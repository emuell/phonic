pub mod converted;
pub mod empty;
pub mod file;
pub mod mapped;
pub mod mixed;
pub mod resampled;

use self::converted::ConvertedSource;
use self::mapped::ChannelMappedSource;
use self::resampled::ResampledSource;
use crate::utils::resampler::ResamplingQuality;

// Types that can produce audio samples in `f32` format. `Send`able across threads.
pub trait AudioSource: Send + 'static {
    // Write at most of `output.len()` samples into the `output`. Returns the
    // number of written samples. Should take care to always output a full
    // frame, and should _never_ block.
    fn write(&mut self, output: &mut [f32]) -> usize;
    fn channel_count(&self) -> usize;
    fn sample_rate(&self) -> u32;

    fn channel_mapped(self, output_channels: usize) -> ChannelMappedSource<Self>
    where
        Self: Sized,
    {
        ChannelMappedSource::new(self, output_channels)
    }

    fn resampled(self, output_sample_rate: u32, quality: ResamplingQuality) -> ResampledSource<Self>
    where
        Self: Sized,
    {
        ResampledSource::new(self, output_sample_rate, quality)
    }

    fn converted(self, output_channels: usize, output_sample_rate: u32) -> ConvertedSource
    where
        Self: Sized,
    {
        ConvertedSource::new(self, output_channels, output_sample_rate)
    }
}
