use super::{
    mapped::ChannelMappedSource,
    resampled::{ResampledSource, ResamplingQuality},
    Source, SourceTime,
};

// -------------------------------------------------------------------------------------------------

/// A source which changes the input source's channel layout and sample rate.
pub struct ConvertedSource {
    converted: Box<dyn Source>,
}

impl ConvertedSource {
    pub fn new<InputSource>(
        source: InputSource,
        channel_count: usize,
        sample_rate: u32,
        resample_quality: ResamplingQuality,
    ) -> Self
    where
        InputSource: Source + Sized,
    {
        if source.sample_rate() != sample_rate {
            let resampled = ResampledSource::new(source, sample_rate, resample_quality);
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

impl Source for ConvertedSource {
    fn write(&mut self, output: &mut [f32], time: &SourceTime) -> usize {
        self.converted.write(output, time)
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
