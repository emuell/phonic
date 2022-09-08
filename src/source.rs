pub mod converted;
pub mod empty;
pub mod file;
pub mod mapped;
pub mod mixed;
pub mod playback;
pub mod resampled;
pub mod synth;

use self::{converted::ConvertedSource, mapped::ChannelMappedSource, resampled::ResampledSource};
use crate::utils::resampler::ResamplingQuality;

// -------------------------------------------------------------------------------------------------

/// AudioSource types produce audio samples in `f32` format and are `Send`able across threads.
///
/// The output buffer is a raw interleaved buffer, which is going to be written by the source
/// in their [`channel_count`] and [`sample_rate`] specs. Specs may not change during runtime,
/// so following sources don't have to adapt to new specs.
///
/// [`write`] is called in the realtime audio thread, so it should not allocate memory or block!
pub trait AudioSource: Send + 'static {
    /// Write at most of `output.len()` samples into the interleaved `output`
    /// Returns the number of written **samples** (not frames).
    fn write(&mut self, output: &mut [f32]) -> usize;
    /// This source's output channel layout.
    fn channel_count(&self) -> usize;
    /// This source's output sample rate.
    fn sample_rate(&self) -> u32;
    /// returns if the source finished playback. Exhaused sources should only return 0 on [`write`]
    /// and could be removed from a source render graph.
    fn is_exhausted(&self) -> bool;

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
