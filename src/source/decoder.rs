use std::{fs::File, io, path::Path, time::Duration};

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

pub struct AudioDecoder {
    track_id: u32, // Internal track index.
    decoder: Box<dyn Decoder>,
    format: Box<dyn FormatReader>,
}

impl AudioDecoder {
    pub fn new(path: String) -> Result<Self, Error> {
        // Create a media source. Note that the MediaSource trait is automatically implemented for File,
        // among other types.
        let file = Box::new(File::open(Path::new(&path))?);

        // Create the media source stream using the boxed media source from above.
        let mss = MediaSourceStream::new(file, Default::default());

        // Create a hint to help the format registry guess what format reader is appropriate. In this
        // example we'll leave it empty.
        let hint = Hint::new();

        // Use the default options when reading and decoding.
        let format_opts: FormatOptions = Default::default();
        let metadata_opts: MetadataOptions = Default::default();
        let decoder_opts: DecoderOptions = Default::default();

        // Probe the media source stream for a format.
        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &format_opts, &metadata_opts)
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

        // Create a decoder for the track.
        let decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &decoder_opts)
            .map_err(|err| Error::AudioDecodingError(Box::new(err)))?;

        Ok(Self {
            track_id: track.id,
            decoder,
            format,
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
                    log::error!("format error: {}", err);
                    return None; // We cannot recover from format errors, quit.
                }
            };
            while !self.format.metadata().is_latest() {
                // Consume any new metadata that has been read since the last
                // packet.
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
                    log::error!("io decode error: {}", err);
                    continue;
                }
                Err(SymphoniaError::DecodeError(err)) => {
                    // The packet failed to decode due to invalid data, skip the packet.
                    log::error!("decode error: {}", err);
                    continue;
                }
                Err(err) => {
                    log::error!("fatal decode error: {}", err);
                    return None;
                }
            };
        }
    }
}
