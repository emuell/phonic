// Check feature, target dependencies
#[cfg(all(target_arch = "wasm32", feature = "cpal"))]
compile_error!("wasm32 builds are not compatible with cpal. use sokol instead");

// Note: when cpal and cubeb features are enabled, we use cpal only
#[cfg(feature = "cpal")]
pub mod cpal;
#[cfg(all(feature = "sokol", not(feature = "cpal")))]
pub mod sokol;

/// The enabled audio output type: cpal or cubeb
#[cfg(feature = "cpal")]
pub type DefaultAudioOutput = cpal::CpalOutput;

#[cfg(all(feature = "sokol", not(feature = "cpal")))]
pub type DefaultAudioOutput = sokol::SokolOutput;

/// Available audio hosts (platform specific)
pub enum AudioHostId {
    Default, // system default
    #[cfg(target_os = "windows")]
    Asio,
    #[cfg(target_os = "windows")]
    Wasapi,
    #[cfg(target_os = "linux")]
    Alsa,
    #[cfg(target_os = "linux")]
    Jack,
}

/// The enabled audio output sink type: cpal or sokol
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
