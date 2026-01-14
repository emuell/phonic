// Check feature, target dependencies
#[cfg(all(target_arch = "wasm32", feature = "cpal-output"))]
compile_error!("wasm builds are not compatible with cpal. use web-output instead");

// Note: when cpal-output and web-output features are enabled, use cpal only
#[cfg(feature = "cpal-output")]
pub mod cpal;
#[cfg(feature = "wav-output")]
pub mod wav;
#[cfg(feature = "web-output")]
pub mod web;

/// The enabled audio output type: cpal or web
#[cfg(feature = "cpal-output")]
pub type DefaultOutputDevice = cpal::CpalOutput;
#[cfg(all(feature = "web-output", not(feature = "cpal-output")))]
pub type DefaultOutputDevice = web::WebOutput;

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

/// Audio output stream device backend.
///
/// Represents the platform-specific audio backend that manages the actual audio hardware stream.
/// The [`Player`](crate::Player) requires an `OutputDevice` implementation to play audio.
///
/// Phonic provides different implementations for various platforms:
/// - `CpalOutput`: Cross-platform audio via cpal (desktop).
/// - `WebOutput`: WebAssembly support via Web Audio API.
/// - `WavOutput`: File-based output for offline rendering.
///
/// The `OutputDevice` is passed to [`Player::new()`](crate::Player::new) and manages the
/// real-time audio thread that pulls audio data from [`Source`](crate::Source)s.
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

    /// `true` when audio output is currently suspended by the system.
    /// Only used in web audio backends, other backends never suspend.
    fn is_suspended(&self) -> bool;

    /// Returns `true` while not paused.
    fn is_running(&self) -> bool;
    /// Pause playback without dropping the output source.
    fn pause(&mut self);
    /// Resume from paused playback.
    fn resume(&mut self);

    /// Play given source as main output source.
    fn play(&mut self, source: Box<dyn Source>);
    /// Drop actual source, replacing it with silence.
    fn stop(&mut self);
    /// Release audio device.
    fn close(&mut self);
}
