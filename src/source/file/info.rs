use std::{ops::Range, path::Path};

use super::decoder::AudioFileDecoder;
use crate::Error;

// -------------------------------------------------------------------------------------------------

/// Audio file metadata, obtained without decoding any PCM samples.
///
/// See also [`AudioFileBuffer`](super::AudioFileBuffer) to decode audio files.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioFileInfo {
    sample_rate: u32,
    channel_count: usize,
    frame_count: usize,
    loop_range: Option<Range<usize>>,
}

impl AudioFileInfo {
    /// Query audio file metadata without decoding any PCM samples.
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<AudioFileInfo, Error> {
        let mut decoder = AudioFileDecoder::from_file(path)?;
        Ok(Self::from_decoder(&mut decoder))
    }

    /// Query audio file metadata from a raw encoded buffer without decoding any PCM samples.
    pub fn from_buffer(data: Vec<u8>) -> Result<AudioFileInfo, Error> {
        let mut decoder = AudioFileDecoder::from_buffer(data)?;
        Ok(Self::from_decoder(&mut decoder))
    }

    /// Query audio file metadata from the given AudioDecoder instance.
    pub(crate) fn from_decoder(decoder: &mut AudioFileDecoder) -> AudioFileInfo {
        let spec = decoder.signal_spec();
        let loop_points = decoder
            .loops()
            .first()
            .map(|l| (l.start as u64, l.end as u64));
        let frame_count = decoder.count_frames() as usize;
        Self {
            frame_count,
            sample_rate: spec.rate,
            channel_count: spec.channels.count(),
            loop_range: loop_points.map(|p| p.0 as usize..p.1 as usize),
        }
    }

    /// Audio file's sampling rate.
    #[inline]
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Total number of frames in the audio file.
    #[inline]
    pub fn frame_count(&self) -> usize {
        self.frame_count
    }

    /// Audio file's channel count.
    #[inline]
    pub fn channel_count(&self) -> usize {
        self.channel_count
    }

    /// Embedded audio file's loop points as sample indices (NOT frames), if any.
    #[inline]
    pub fn loop_range(&self) -> Option<Range<usize>> {
        self.loop_range.clone()
    }
}
