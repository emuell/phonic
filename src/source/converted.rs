use super::AudioSource;
use crate::{
    mapped::ChannelMappedSource, resampled::Quality as ResamplingQuality,
    resampled::ResampledSource,
};

// -------------------------------------------------------------------------------------------------

/// A source which adjusts the input source to a target channel layout and sample rate.
pub struct ConvertedSource {
    converted: Box<dyn AudioSource>,
}

impl ConvertedSource {
    pub fn new<InputSource>(
        source: InputSource,
        channel_count: usize,
        sample_rate: u32,
        resample_quality: ResamplingQuality,
    ) -> Self
    where
        InputSource: AudioSource + Sized,
    {
        Self::new_with_speed(source, channel_count, sample_rate, 1.0, resample_quality)
    }

    pub fn new_with_speed<InputSource>(
        source: InputSource,
        channel_count: usize,
        sample_rate: u32,
        speed: f64,
        resample_quality: ResamplingQuality,
    ) -> Self
    where
        InputSource: AudioSource + Sized,
    {
        if source.sample_rate() != sample_rate || speed != 1.0 {
            let resampled =
                ResampledSource::new_with_speed(source, sample_rate, speed, resample_quality);
            if resampled.channel_count() != channel_count {
                let mapped = ChannelMappedSource::new(resampled, channel_count);
                Self {
                    converted: Box::new(mapped),
                }
            } else {
                Self {
                    converted: Box::new(resampled),
                }
            }
        } else if source.channel_count() != channel_count {
            let mapped = ChannelMappedSource::new(source, channel_count);
            Self {
                converted: Box::new(mapped),
            }
        } else {
            Self {
                converted: Box::new(source),
            }
        }
    }
}

impl AudioSource for ConvertedSource {
    fn write(&mut self, output: &mut [f32]) -> usize {
        self.converted.write(output)
    }

    fn channel_count(&self) -> usize {
        self.converted.channel_count()
    }

    fn sample_rate(&self) -> u32 {
        self.converted.sample_rate()
    }

    fn is_exhausted(&self) -> bool {
        self.converted.is_exhausted()
    }
}
