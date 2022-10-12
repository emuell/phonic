#[cfg(feature = "cpal")]
pub mod cpal;
#[cfg(feature = "cubeb")]
pub mod cubeb;

/// The enabled audio output type: cpal or cubeb
#[cfg(feature = "cpal")]
pub type DefaultAudioOutput = cpal::CpalOutput;

#[cfg(feature = "cubeb")]
#[cfg(not(feature = "cpal"))]
pub type DefaultAudioOutput = cubeb::CubebOutput;

/// The enabled audio output sink type: cpal or cubeb
pub type DefaultAudioSink = <DefaultAudioOutput as AudioOutput>::Sink;

use super::source::AudioSource;

// -------------------------------------------------------------------------------------------------

/// AudioOutput controller
pub trait AudioSink {
    /// Signal specs.
    fn channel_count(&self) -> usize;
    fn sample_rate(&self) -> u32;
    /// Actual device's output playhead position in samples (NOT sample frames).
    fn sample_position(&self) -> u64;

    /// Get actual output volume.
    fn volume(&self) -> f32;
    /// Set a new output volume.
    fn set_volume(&mut self, volume: f32);

    /// Play given source as main output source.
    fn play(&mut self, source: impl AudioSource);
    /// Drop actual source, replacing it with silence
    fn stop(&mut self);
    /// Pause playback without dropping the ouput source.
    fn pause(&mut self);
    /// Resume from paused playback.
    fn resume(&mut self);

    /// Release audio device
    fn close(&mut self);
}

// -------------------------------------------------------------------------------------------------

/// AudioOutput implementation: provides a sink controller.
pub trait AudioOutput {
    type Sink: AudioSink;
    fn sink(&self) -> Self::Sink;
}
