use std::{ops::Range, path::Path};

use symphonia::core::audio::SampleBuffer;

use super::decoder::AudioFileDecoder;
use crate::Error;

// -------------------------------------------------------------------------------------------------

/// Decoded audio file buffer. Usually wrapped into Rc as shared audio data in sources or generators.
///
/// See also [`AudioFileInfo`](super::AudioFileInfo) to query audio file meta data only.
#[derive(PartialEq, Clone)]
pub struct AudioFileBuffer {
    buffer: Vec<f32>,
    sample_rate: u32,
    channel_count: usize,
    loop_range: Option<Range<usize>>,
}

impl AudioFileBuffer {
    /// Create a new shared sample buffer. Returns an error if buffer properties are invalid.
    pub fn new(
        buffer: Vec<f32>,
        channel_count: usize,
        sample_rate: u32,
        loop_range: Option<Range<usize>>,
    ) -> Result<Self, Error> {
        if sample_rate == 0 {
            return Err(Error::ParameterError(
                "file buffer sample rate must be > 0".to_owned(),
            ));
        }
        if channel_count == 0 {
            return Err(Error::ParameterError(
                "file buffer channel count must be > 0".to_owned(),
            ));
        }
        if buffer.is_empty() {
            return Err(Error::ParameterError(
                "file buffer must not be empty".to_owned(),
            ));
        }
        if !buffer.len().is_multiple_of(channel_count) {
            return Err(Error::ParameterError(
                "file buffer length must be a multiple of the channel count".to_owned(),
            ));
        }
        if let Some(loop_range) = &loop_range {
            if loop_range.start >= loop_range.end || loop_range.end > buffer.len() {
                return Err(Error::ParameterError(
                    "file buffer loop range is out of bounds".to_owned(),
                ));
            }
        }
        Ok(Self {
            buffer,
            channel_count,
            sample_rate,
            loop_range,
        })
    }

    /// Create a new audio file buffer from the given audio file path.
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        Self::from_audio_decoder(AudioFileDecoder::from_file(path)?)
    }

    /// Create a new audio file buffer from the given raw **encoded** file stream buffer.
    pub fn from_file_buffer(file_buffer: Vec<u8>) -> Result<Self, Error> {
        Self::from_audio_decoder(AudioFileDecoder::from_buffer(file_buffer)?)
    }

    /// Create a new audio file buffer from the given raw **encoded** file stream buffer.
    pub(crate) fn from_audio_decoder(mut audio_decoder: AudioFileDecoder) -> Result<Self, Error> {
        // get buffer signal specs
        let sample_rate = audio_decoder.signal_spec().rate;
        let channel_count = audio_decoder.signal_spec().channels.count();

        // prealloc entire buffer, when the decoder gives us a frame hint
        let buffer_capacity =
            audio_decoder.codec_params().n_frames.unwrap_or(0) as usize * channel_count + 1;
        let mut buffer = Vec::with_capacity(buffer_capacity);

        // decode the entire file into our buffer in chunks of max_frames_per_packet sizes
        let decode_buffer_capacity = audio_decoder
            .codec_params()
            .max_frames_per_packet
            .unwrap_or(16 * 1024 * channel_count as u64);
        let mut decode_buffer =
            SampleBuffer::<f32>::new(decode_buffer_capacity, audio_decoder.signal_spec());

        while audio_decoder.read_packet(&mut decode_buffer).is_some() {
            buffer.append(&mut decode_buffer.samples().to_vec());
        }
        if buffer.is_empty() {
            // TODO: should pass a proper error here
            return Err(Error::AudioDecodingError(Box::new(
                symphonia::core::errors::Error::DecodeError("failed to decode file"),
            )));
        } else {
            // add one extra empty sample at the end for the cubic resamplers
            buffer.extend(std::iter::repeat_n(0.0, channel_count));
        }

        let mut loop_range = None;
        if let Some(loop_info) = audio_decoder.loops().first() {
            // TODO: for now we only support forward loops
            let loop_start = (loop_info.start as usize * channel_count).min(buffer.len());
            let loop_end = (loop_info.end as usize * channel_count).min(buffer.len());
            if loop_end > loop_start {
                loop_range = Some(loop_start..loop_end);
            }
        }

        Self::new(buffer, channel_count, sample_rate, loop_range)
    }

    /// Access to the shared sample buffer's raw interleaved sample data.
    #[inline]
    pub fn buffer(&self) -> &[f32] {
        &self.buffer
    }

    /// Shared sample buffer's channel layout.
    #[inline]
    pub fn channel_count(&self) -> usize {
        self.channel_count
    }

    /// Shared sample buffer's number of frames.
    #[inline]
    pub fn frame_count(&self) -> usize {
        self.buffer.len() / self.channel_count
    }

    /// Shared sample buffer's sampling rate.
    #[inline]
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Embedded audio file's loop points as sample indices (NOT frames), if any.
    #[inline]
    pub fn loop_range(&self) -> Option<Range<usize>> {
        self.loop_range.clone()
    }
}
