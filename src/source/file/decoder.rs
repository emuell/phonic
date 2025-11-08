use std::{
    fs::File,
    io::{self, Seek},
    path::Path,
    time::Duration,
};

use byteorder::{ByteOrder, LittleEndian};
use riff::{Chunk, ChunkId};

use symphonia::core::{
    audio::{SampleBuffer, SignalSpec},
    codecs::{CodecParameters, Decoder, DecoderOptions},
    conv::ConvertibleSample,
    errors::Error as SymphoniaError,
    formats::{FormatOptions, FormatReader, SeekMode, SeekTo},
    io::MediaSourceStream,
    meta::MetadataOptions,
    probe::Hint,
    units::TimeStamp,
};

use crate::error::Error;

// -------------------------------------------------------------------------------------------------

/// Loop mode direction from a decoded audio file
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioDecoderLoopMode {
    Forward,
    Alternating,
    Backward,
    Unknown,
}

/// Loop info from a decoded audio file
#[derive(Debug, Clone)]
pub struct AudioDecoderLoopInfo {
    #[allow(unused)]
    pub mode: AudioDecoderLoopMode,
    pub start: u32,
    pub end: u32,
}

pub struct AudioDecoder {
    track_id: u32, // Internal track index.
    decoder: Box<dyn Decoder>,
    format: Box<dyn FormatReader>,
    loops: Vec<AudioDecoderLoopInfo>,
}

impl AudioDecoder {
    /// Create a new decoder from the given file path.
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        let mut file = File::open(path.as_ref())?;
        let loops = Self::parse_loop_metadata(&mut file).unwrap_or_default();
        file.seek(io::SeekFrom::Start(0))?;

        let file = Box::new(file);
        let source_stream = MediaSourceStream::new(file, Default::default());
        let mut decoder = Self::from_source_stream(source_stream)?;
        decoder.loops = loops;

        Ok(decoder)
    }

    /// Create a new decoder from the given buffer. The buffer unfortunately must get copied as
    /// Symphonia does not allow reading non static buffer refs at the time being...
    pub fn from_buffer(buffer: Vec<u8>) -> Result<Self, Error> {
        let mut cursor = io::Cursor::new(buffer);
        let loops = Self::parse_loop_metadata(&mut cursor).unwrap_or_default();
        cursor.seek(io::SeekFrom::Start(0))?;

        let cursor = Box::new(cursor);
        let source_stream = MediaSourceStream::new(cursor, Default::default());
        let mut decoder = Self::from_source_stream(source_stream)?;
        decoder.loops = loops;

        Ok(decoder)
    }

    /// Create a new decoder from the given Symphonia MediaSourceStream
    pub fn from_source_stream(source_stream: MediaSourceStream) -> Result<Self, Error> {
        // Unused hint to help the format registry guess what format reader is appropriate.
        let hint = Hint::new();

        // Use the default options when reading and decoding.
        let format_opts: FormatOptions = Default::default();
        let metadata_opts: MetadataOptions = Default::default();
        let decoder_opts: DecoderOptions = Default::default();

        // Probe the media source stream for a format.
        let probed = symphonia::default::get_probe()
            .format(&hint, source_stream, &format_opts, &metadata_opts)
            .map_err(|_| Error::MediaFileProbeError)?;

        // Get the format reader yielded by the probe operation.
        let format = probed.format;

        // Get the default track.
        let track = match format.default_track() {
            Some(t) => t,
            None => {
                return Err(Error::MediaFileNotFound);
            }
        };
        let track_id = track.id;

        // Create a decoder for the track.
        let decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &decoder_opts)
            .map_err(|err| Error::AudioDecodingError(Box::new(err)))?;

        // there are no loops in symphonia meta infos: must parse those separately.
        let loops = vec![];

        Ok(Self {
            track_id,
            decoder,
            format,
            loops,
        })
    }

    pub fn codec_params(&self) -> &CodecParameters {
        self.decoder.codec_params()
    }

    pub fn signal_spec(&self) -> SignalSpec {
        SignalSpec {
            rate: self.codec_params().sample_rate.unwrap(),
            channels: self.codec_params().channels.unwrap(),
        }
    }

    pub fn loops(&self) -> &[AudioDecoderLoopInfo] {
        &self.loops
    }

    pub fn seek(&mut self, time: Duration) -> Result<TimeStamp, Error> {
        let seeked_to = self
            .format
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time: time.as_secs_f64().into(),
                    track_id: Some(self.track_id),
                },
            )
            .map_err(|_| Error::MediaFileSeekError)?;
        Ok(seeked_to.actual_ts)
    }

    /// Detect file container and parse loop metadata accordingly.
    fn parse_loop_metadata<R: io::Read + io::Seek>(
        reader: &mut R,
    ) -> Result<Vec<AudioDecoderLoopInfo>, Error> {
        let mut magic = [0u8; 4];
        let read = reader
            .read(&mut magic)
            .map_err(|_| Error::MediaFileProbeError)?;
        reader.seek(io::SeekFrom::Start(0))?;
        if read < 4 {
            return Ok(vec![]);
        }
        match &magic {
            b"RIFF" => Self::parse_riff_loops(reader),
            b"fLaC" => Self::parse_flac_loops(reader),
            _ => Ok(vec![]),
        }
    }

    fn parse_riff_loops<R: io::Read + io::Seek>(
        reader: &mut R,
    ) -> Result<Vec<AudioDecoderLoopInfo>, Error> {
        const RIFF_ID: ChunkId = ChunkId { value: *b"RIFF" };
        const WAVE_ID: ChunkId = ChunkId { value: *b"WAVE" };
        const SMPL_ID: ChunkId = ChunkId { value: *b"smpl" };

        // read RIFF_ID chunk
        let riff_chunk = Chunk::read(reader, 0).map_err(|_| Error::MediaFileProbeError)?;
        if riff_chunk.id() != RIFF_ID
            || riff_chunk
                .read_type(reader)
                .map_err(|_| Error::MediaFileProbeError)?
                != WAVE_ID
        {
            return Err(Error::MediaFileProbeError);
        }

        // find SMPL_ID chunk
        let mut smpl_chunk = None;
        for child in riff_chunk.iter(reader) {
            match child {
                Ok(child) if child.id() == SMPL_ID => {
                    smpl_chunk = Some(child);
                    break;
                }
                Ok(_) => {
                    // try next chunk
                    continue;
                }
                Err(_) => {
                    // stop on errors
                    break;
                }
            }
        }

        // read SMPL_ID chunk
        if let Some(chunk) = smpl_chunk {
            if let Ok(data) = chunk.read_contents(reader) {
                return Self::parse_smpl_body(&data);
            }
        }

        Err(Error::MediaFileProbeError) // No smpl chunk found or failed to read
    }

    /// Minimal FLAC metadata parser that scans Application blocks for embedded RIFF "smpl" data.
    fn parse_flac_loops<R: io::Read + io::Seek>(
        reader: &mut R,
    ) -> Result<Vec<AudioDecoderLoopInfo>, Error> {
        // Expect "fLaC" marker at start.
        let mut marker = [0u8; 4];
        reader
            .read_exact(&mut marker)
            .map_err(|_| Error::MediaFileProbeError)?;
        if &marker != b"fLaC" {
            return Err(Error::MediaFileProbeError);
        }

        // Iterate metadata blocks.
        let mut loops: Vec<AudioDecoderLoopInfo> = Vec::new();
        loop {
            // METADATA_BLOCK_HEADER:
            // 1 byte: [is_last(1 bit) | block_type(7 bits)]
            // 3 bytes: length (24-bit big endian)
            let mut header = [0u8; 4];
            reader
                .read_exact(&mut header)
                .map_err(|_| Error::MediaFileProbeError)?;
            let is_last = (header[0] & 0x80) != 0;
            let block_type = header[0] & 0x7F;
            let length: usize =
                ((header[1] as usize) << 16) | ((header[2] as usize) << 8) | (header[3] as usize);
            // Application block type is 2.
            if block_type == 2 {
                if length < 4 {
                    // Malformed application block; skip payload if any.
                    let mut sink = vec![0u8; length];
                    reader
                        .read_exact(&mut sink)
                        .map_err(|_| Error::MediaFileProbeError)?;
                } else {
                    // Read 4-byte application ID then payload.
                    let mut app_id = [0u8; 4];
                    reader
                        .read_exact(&mut app_id)
                        .map_err(|_| Error::MediaFileProbeError)?;
                    let payload_len = length - 4;
                    let mut payload = vec![0u8; payload_len];
                    reader
                        .read_exact(&mut payload)
                        .map_err(|_| Error::MediaFileProbeError)?;
                    // Try to parse the payload as an smpl chunk.
                    if payload.len() >= 8 && &payload[0..4] == b"smpl" {
                        // Ensure we don't read beyond payload.
                        let size = LittleEndian::read_u32(&payload[4..8]) as usize;
                        let max_len = payload.len().saturating_sub(8);
                        let body_len = size.min(max_len);
                        let body = &payload[8..8 + body_len];
                        if let Ok(mut new_loops) = Self::parse_smpl_body(body) {
                            loops.append(&mut new_loops);
                        }
                    }
                }
            } else {
                // Skip payload of this block.
                let mut sink = vec![0u8; length];
                reader
                    .read_exact(&mut sink)
                    .map_err(|_| Error::MediaFileProbeError)?;
            }

            if is_last {
                break;
            }
        }
        Ok(loops)
    }

    /// Parse the contents of a RIFF "smpl" chunk body (without the 8-byte RIFF chunk header).
    fn parse_smpl_body(data: &[u8]) -> Result<Vec<AudioDecoderLoopInfo>, Error> {
        // The smpl chunk header must be >= 36 bytes long.
        if data.len() < 36 {
            return Err(Error::MediaFileProbeError);
        }
        // number of loops is at offset 28..32 (little endian)
        let num_loops = LittleEndian::read_u32(&data[28..32]) as usize;
        let mut loops = Vec::with_capacity(num_loops);
        let mut loop_data_start = 36;

        for _ in 0..num_loops {
            if loop_data_start + 24 > data.len() {
                break; // No more complete loop entries present.
            }
            let loop_slice = &data[loop_data_start..loop_data_start + 24];

            let loop_type = LittleEndian::read_u32(&loop_slice[4..8]);
            let loop_mode = match loop_type {
                0 => AudioDecoderLoopMode::Forward,
                1 => AudioDecoderLoopMode::Alternating,
                2 => AudioDecoderLoopMode::Backward,
                _ => AudioDecoderLoopMode::Unknown,
            };
            let loop_start = LittleEndian::read_u32(&loop_slice[8..12]);
            let loop_end = LittleEndian::read_u32(&loop_slice[12..16]);

            loops.push(AudioDecoderLoopInfo {
                mode: loop_mode,
                start: loop_start,
                end: loop_end,
            });

            loop_data_start += 24;
        }

        Ok(loops)
    }

    /// Read a next packet of audio from this decoder.  Returns `None` in case
    /// of EOF or internal error.
    pub fn read_packet<S>(&mut self, samples: &mut SampleBuffer<S>) -> Option<TimeStamp>
    where
        S: ConvertibleSample,
    {
        loop {
            // Demux an encoded packet from the media format.
            let packet = match self.format.next_packet() {
                Ok(packet) => packet,
                Err(SymphoniaError::IoError(io)) if io.kind() == io::ErrorKind::UnexpectedEof => {
                    return None; // End of this stream.
                }
                Err(err) => {
                    log::error!("Audio file decoder format error: {err}");
                    return None; // We cannot recover from format errors, quit.
                }
            };
            while !self.format.metadata().is_latest() {
                // Consume any new metadata that has been read since the last packet.
            }
            // If the packet does not belong to the selected track, skip over it.
            if packet.track_id() != self.track_id {
                continue;
            }
            // Decode the packet into an audio buffer.
            match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    // Interleave the samples into the buffer.
                    samples.copy_interleaved_ref(decoded);
                    return Some(packet.ts());
                }
                Err(SymphoniaError::IoError(err)) => {
                    // The packet failed to decode due to an IO error, skip the packet.
                    log::error!("Audio file decoder I/O error: {err}");
                    continue;
                }
                Err(SymphoniaError::DecodeError(err)) => {
                    // The packet failed to decode due to invalid data, skip the packet.
                    log::error!("Audio file decoder error: {err}");
                    continue;
                }
                Err(err) => {
                    log::error!("Audio file decoder fatal error: {err}");
                    return None;
                }
            };
        }
    }
}
