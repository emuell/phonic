//! Audio output device implementations and traits.

// Validate feature compatibility
#[cfg(all(target_arch = "wasm32", feature = "cpal-output"))]
compile_error!("wasm32 targets are incompatible with cpal-output. Use web-output instead");

#[cfg(feature = "cpal-output")]
pub mod cpal;
#[cfg(feature = "wav-output")]
pub mod wav;
#[cfg(feature = "web-output")]
pub mod web;

use super::source::Source;

// -------------------------------------------------------------------------------------------------

/// The default audio output device for the current platform and feature configuration.
///
/// Determined by compile-time feature flags and target archs:
/// - `cpal-output` (if not wasm32)
/// - `web-output` (for wasm32 targets)
#[cfg(feature = "cpal-output")]
pub type DefaultOutputDevice = cpal::CpalOutput;

#[cfg(all(feature = "web-output", not(feature = "cpal-output")))]
pub type DefaultOutputDevice = web::WebOutput;

// -------------------------------------------------------------------------------------------------

/// Available audio host drivers for cpal output devices.
///
/// Represents different audio backends available on various platforms.
/// The default variant uses the system-preferred audio host.
#[cfg(feature = "cpal-output")]
#[derive(Debug, Clone, Copy)]
pub enum AudioHostId {
    /// System default audio host
    Default,
    /// Windows: Audio Stream Input/Output (ASIO)
    #[cfg(target_os = "windows")]
    Asio,
    /// Windows: Windows Audio Session API (WASAPI)
    #[cfg(target_os = "windows")]
    Wasapi,
    /// Linux: Advanced Linux Sound Architecture
    #[cfg(target_os = "linux")]
    Alsa,
    /// Linux: JACK Audio Connection Kit
    #[cfg(target_os = "linux")]
    Jack,
}

// -------------------------------------------------------------------------------------------------

/// Platform-agnostic audio output device interface, abstracts platform-specific audio backends
/// for [`Player`](crate::Player).
pub trait OutputDevice: Send {
    /// Returns the output buffer channel count (e.g., 1 for mono, 2 for stereo).
    fn channel_count(&self) -> usize;
    /// Returns the output sample rate in Hz.
    fn sample_rate(&self) -> u32;
    /// Returns the current playhead position in **samples** (not frames).
    /// Tracks the total number of samples output since device creation.
    fn sample_position(&self) -> u64;

    /// Returns the current output volume (typically 0.0 to 1.0).
    fn volume(&self) -> f32;
    /// Sets the output volume.
    fn set_volume(&mut self, volume: f32);

    /// Returns `true` if audio output is suspended by the system.
    /// Only relevant for web audio backends; always returns `false` for desktop backends.
    fn is_suspended(&self) -> bool;

    /// Returns `true` if audio is currently playing (not paused).
    fn is_running(&self) -> bool;
    /// Pauses playback while keeping the output source active.
    fn pause(&mut self);
    /// Resumes playback from a paused state.
    fn resume(&mut self);

    /// Starts playback of a new audio source.
    fn play(&mut self, source: Box<dyn Source>);
    /// Stops playback and replaces the source with silence.
    fn stop(&mut self);

    /// Releases the audio device and cleans up resources.
    fn close(&mut self);
}
