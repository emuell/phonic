//! **phonic** is a cross-platform audio playback and DSP library for Rust, providing a flexible,
//! low-latency audio engine and related tools for games and music applications.
//!
//! ### Overview
//!
//! - **[`Player`]** is the central component that manages audio playback. It takes an
//!   output device instance, plays sounds and manages DSP effects.
//!
//! - **[`OutputDevice`]** represents the audio backend stream. phonic provides implementations
//!   for different platforms, such as `cpal` for native applications and `sokol` for WebAssembly.
//!   The [`DefaultOutputDevice`] is an alias for the recommended output device for the current
//!   build target.
//!
//! - **[`Source`]** produces audio signals. You can use the built-in [`FileSource`] for playing
//!   back audio files, [`SynthSource`] for generating synthesized tones or can create your own
//!   source implementation. File sources can be preloaded into memory or streamed on-the-fly.
//!
//! - **[`Effect`]** applies DSP effects to audio signals signals. By default, only one
//!   mixer is present in the player and will route all sources through it. To create more complex
//!   routings you can create sub-mixers via [`Player::add_mixer`] and route sources to them.
//!   Each mixer instance has its own chain of DSP effects, which can be set up via 
//!   [`Player::add_effect`]. You can use the built in effects impls in [`effects`] or create
//!   your own ones.
//!
//! ### Getting Started
//!
//! Here's a basic example of how to play audio files with DSP effects.
//!
//! ```rust,no_run
//! use phonic::{
//!     DefaultOutputDevice, Player, FilePlaybackOptions, Error,
//!     effects::{ReverbEffect, CompressorEffect},
//! };
//!
//! fn main() -> Result<(), Error> {
//!     // Open the default audio output device.
//!     let output_device = DefaultOutputDevice::open()?;
//!     // Create a player for the given output device.
//!     let mut player = Player::new(output_device, None);
//!
//!     // Add a new sub-mixer to the main mixer.
//!     let sub_mixer_id = player.add_mixer(None)?;
//!     // Add a reverb effect to this mixer.
//!     player.add_effect(ReverbEffect::default(), sub_mixer_id)?;
//!
//!     // Add a limiter to the main mixer. All sounds, including the sub mixer's output
//!     // will be routed through this effect now.
//!     player.add_effect(CompressorEffect::default_limiter(), None)?;
//!
//!     // Play a file with default options on the sub mixer. It will start playing immediately.
//!     player.play_file("path/to/your/file.wav",
//!       FilePlaybackOptions::default().target_mixer(sub_mixer_id)
//!     )?;
//!     // Play a file with default options on the main mixer and schedule it to be played
//!     // two seconds ahead of the current output time.
//!     player.play_file("path/to/your/some_other_file.wav",
//!       FilePlaybackOptions::default()
//!         .start_at_time(player.output_sample_frame_position() + 
//!            2 * player.output_sample_rate() as u64)
//!     )?;
//!
//!     // The player's audio output stream runs on a separate thread, so we need to keep
//!     // the main thread alive to hear the audio.
//!     std::thread::sleep(std::time::Duration::from_secs(5));
//!
//!     Ok(())
//! }
//! ```
//!
//! For more advanced usage, such as monitoring playback, sequencing source playback or managing
//! creating more complex mixer graphs, please see the examples in the `README.md` and the `/examples`
//! directory of the repository.

#![cfg_attr(all(doc, docsrs), feature(doc_auto_cfg))]

// private mods (will be partly re-exported)
mod effect;
mod error;
mod output;
mod player;
mod source;

// public, flat re-exports
pub use error::Error;

#[cfg(any(feature = "cpal-output", feature = "sokol-output"))]
pub use output::DefaultOutputDevice;
pub use output::OutputDevice;

pub use player::{
    EffectId, MixerId, PlaybackId, PlaybackStatusContext, PlaybackStatusEvent, Player,
};

pub use effect::{Effect, EffectMessage, EffectMessagePayload, EffectTime};
pub use source::{
    file::{FilePlaybackMessage, FilePlaybackOptions, FileSource},
    resampled::ResamplingQuality,
    synth::{SynthPlaybackMessage, SynthPlaybackOptions, SynthSource},
    Source, SourceTime,
};

pub mod outputs {
    //! Default [`OutputDevice`](super::OutputDevice) implementations.

    #[cfg(feature = "cpal-output")]
    pub use super::output::cpal::CpalOutput;
    #[cfg(feature = "cpal-output")]
    pub use super::output::AudioHostId;

    #[cfg(feature = "sokol-output")]
    pub use super::output::sokol::SokolOutput;

    #[cfg(feature = "wav-output")]
    pub use super::output::wav::WavOutputDevice;
}

pub mod sources {
    //! Set of basic, common File & Synth tone [`Source`](super::Source) implementations.

    // synths
    pub use super::source::synth::common::{SynthSourceGenerator, SynthSourceImpl};
    #[cfg(feature = "dasp")]
    pub use super::source::synth::dasp::DaspSynthSource;

    // files
    pub use super::source::file::{
        common::FileSourceImpl, preloaded::PreloadedFileSource, streamed::StreamedFileSource,
    };
}

pub mod effects {
    //! Set of basic, common DSP [`Effect`](super::Effect) implementations.

    pub use super::effect::{
        chorus::{ChorusEffect, ChorusEffectMessage},
        compressor::{CompressorEffect, CompressorEffectMessage},
        dcfilter::{DcFilterEffect, DcFilterEffectMessage},
        filter::{FilterEffect, FilterEffectMessage, FilterEffectType},
        reverb::{ReverbEffect, ReverbEffectMessage},
    };
}

pub mod utils;
