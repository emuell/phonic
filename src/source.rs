pub mod converted;
pub mod empty;
pub mod file;
pub mod mapped;
pub mod mixed;
pub mod resampled;

use self::{converted::ConvertedSource, mapped::ChannelMappedSource, resampled::ResampledSource};
use crate::utils::resampler::ResamplingQuality;

// Types that can produce audio samples in `f32` format. `Send`able across threads.
pub trait AudioSource: Send + 'static {
    /// Write at most of `output.len()` samples into the interleaved `output`
    /// Returns the number of written samples.
    fn write(&mut self, output: &mut [f32]) -> usize;
    /// This source's output channel layout
    fn channel_count(&self) -> usize;
    /// This source's output sample rate
    fn sample_rate(&self) -> u32;

    /// Shortcut for creating a new source from self with a remapped channel layout
    fn channel_mapped(self, output_channels: usize) -> ChannelMappedSource
    where
        Self: AudioSource + Sized,
    {
        ChannelMappedSource::new(self, output_channels)
    }
    /// Shortcut for creating a new source from self with a matched sample rate
    fn resampled(
        self,
        output_sample_rate: u32,
        resample_quality: ResamplingQuality,
    ) -> ResampledSource
    where
        Self: AudioSource + Sized,
    {
        ResampledSource::new(self, output_sample_rate, resample_quality)
    }
    /// Shortcut for creating a new source with the given signal specs
    fn converted(
        self,
        output_channels: usize,
        output_sample_rate: u32,
        resample_quality: ResamplingQuality,
    ) -> ConvertedSource
    where
        Self: AudioSource + Sized,
    {
        ConvertedSource::new(self, output_channels, output_sample_rate, resample_quality)
    }
}
