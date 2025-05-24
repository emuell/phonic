// Check feature, target dependencies
#[cfg(all(target_arch = "wasm32", feature = "cpal"))]
compile_error!("wasm builds are not compatible with cpal. use sokol-output instead");

// Note: when cpal and sokol features are enabled, we use cpal only
#[cfg(feature = "cpal")]
pub mod cpal;
#[cfg(all(feature = "sokol", not(feature = "cpal")))]
pub mod sokol;

/// The enabled audio output type: cpal or sokol
#[cfg(feature = "cpal")]
pub type DefaultOutputDevice = cpal::CpalOutput;

#[cfg(all(feature = "sokol", not(feature = "cpal")))]
pub type DefaultOutputDevice = sokol::SokolOutput;

/// Available audio hosts for cpal output (platform specific)
#[cfg(feature = "cpal")]
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
pub type DefaultOutputSink = <DefaultOutputDevice as OutputDevice>::Sink;

use super::source::Source;

// -------------------------------------------------------------------------------------------------

/// OutputDevice controller
pub trait OutputSink {
    /// true when audio output is currently suspended by the system: only used in Sokol's
    /// WebAudio backend, all other backends return false
    fn suspended(&self) -> bool;

    /// Actual device's output sample buffer channel count.
    fn channel_count(&self) -> usize;
    /// Actual device's output sample rate.
    fn sample_rate(&self) -> u32;
    /// Actual device's output playhead position in **samples** (NOT frames).
    fn sample_position(&self) -> u64;

    /// Get actual output volume.
    fn volume(&self) -> f32;
    /// Set a new output volume.
    fn set_volume(&mut self, volume: f32);

    /// Play given source as main output source.
    fn play(&mut self, source: impl Source);
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

/// OutputDevice implementation: provides a sink controller.
pub trait OutputDevice {
    type Sink: OutputSink;
    fn sink(&self) -> Self::Sink;
}
