// Check feature, target dependencies
#[cfg(all(target_arch = "wasm32", feature = "cpal-output"))]
compile_error!("wasm builds are not compatible with cpal. use sokol-output instead");

// Note: when cpal and sokol features are enabled, we use cpal only
#[cfg(feature = "cpal-output")]
pub mod cpal;
#[cfg(feature = "sokol-output")]
pub mod sokol;
#[cfg(feature = "wav-output")]
pub mod wav;

/// The enabled audio output type: cpal or sokol
#[cfg(feature = "cpal-output")]
pub type DefaultOutputDevice = cpal::CpalOutput;
#[cfg(all(feature = "sokol-output", not(feature = "cpal-output")))]
pub type DefaultOutputDevice = sokol::SokolOutput;

/// Available audio hosts for cpal output devices (platform specific)
#[cfg(feature = "cpal-output")]
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

use super::source::Source;

// -------------------------------------------------------------------------------------------------

/// Audio output stream device.
pub trait OutputDevice: Send {
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

    /// true when audio output is currently suspended by the system.
    /// Only used in Sokol's WebAudio backend, other backends do never suspend.
    fn is_suspended(&self) -> bool;

    /// Returns true while not paused.
    fn is_running(&self) -> bool;
    /// Pause playback without dropping the output source.
    fn pause(&mut self);
    /// Resume from paused playback.
    fn resume(&mut self);

    /// Play given source as main output source.
    fn play(&mut self, source: Box<dyn Source>);
    /// Drop actual source, replacing it with silence
    fn stop(&mut self);
    /// Release audio device
    fn close(&mut self);
}
