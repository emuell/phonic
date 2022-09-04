use super::AudioSource;
use crate::utils::resampler::ResamplingQuality;

// -------------------------------------------------------------------------------------------------
/// A source which adjusts the input source to a target channel layout and sample rate

pub struct ConvertedSource {
    converted: Box<dyn AudioSource>,
}

impl ConvertedSource {
    pub fn new<T>(source: T, channel_count: usize, sample_rate: u32) -> Self
    where
        T: AudioSource + Sized,
    {
        if source.sample_rate() != sample_rate {
            let resampled = source.resampled(sample_rate, ResamplingQuality::SincMediumQuality);
            if resampled.channel_count() != channel_count {
                let mapped = resampled.channel_mapped(channel_count);
                Self {
                    converted: Box::new(mapped),
                }
            } else {
                Self {
                    converted: Box::new(resampled),
                }
            }
        } else if source.channel_count() != channel_count {
            let mapped = source.channel_mapped(channel_count);
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
}
