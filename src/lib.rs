//! **phonic** is a cross-platform audio playback and DSP library for Rust. It provides a flexible,
//! low-latency audio engine and related tools for desktop and web-based music applications
//!
//! ### Overview
//!
//! - **[`Player`]** is the central component that manages audio playback. It takes an
//!   output device instance, plays sounds and manages DSP effects.
//!
//! - **[`OutputDevice`]** represents the audio backend stream. phonic provides implementations
//!   for different platforms, such as `cpal` for native applications and `webauddio` for WebAssembly.
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
//!     effects::{ChorusEffect, ReverbEffect, CompressorEffect},
//! };
//!
//! fn main() -> Result<(), Error> {
//!     // Create a player with the default audio output device.
//!     let mut player = Player::new(DefaultOutputDevice::open()?, None);
//!
//!     // Add a new sub-mixer with a chorus and reverb effect to the main mixer.
//!     let sub_mixer_id = {
//!         let mixer_id = player.add_mixer(None)?;
//!         player.add_effect(ChorusEffect::default(), mixer_id)?;
//!         player.add_effect(ReverbEffect::default(), mixer_id)?;
//!         mixer_id
//!     };
//!
//!     // Add a limiter to the main mixer. All sounds, including the sub mixer's output
//!     // will be routed through this effect now.
//!     player.add_effect(CompressorEffect::new_limiter(), None)?;
//!
//!     // Play a file with default options on the sub mixer. It will start playing immediately
//!     // and will be routed through a chorus and reverb effect and the main mixer's limiter.
//!     player.play_file("path/to/your/file.wav",
//!       FilePlaybackOptions::default().target_mixer(sub_mixer_id)
//!     )?;
//!
//!     // Play a file with default options on the main mixer and schedule it two seconds ahead
//!     // of the current output time. It will be routed through the limiter effect only.
//!     let sample_rate = player.output_sample_rate() as u64;
//!     player.play_file("path/to/your/some_other_file.wav",
//!       FilePlaybackOptions::default().start_at_time(
//!         player.output_sample_frame_position() + 2 * sample_rate)
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

// enable feature config when building for docs.rs
#![cfg_attr(docsrs, feature(doc_cfg))]
// enable experimental ASM features for emscripten js! macros
#![cfg_attr(
    target_os = "emscripten",
    feature(asm_experimental_arch),
    feature(macro_metavar_expr_concat)
)]

// -------------------------------------------------------------------------------------------------

// private mods (partly re-exported)

mod effect;
mod error;
mod output;
mod parameter;
mod player;
mod source;

// public, flat re-exports (common types and traits)
pub use error::Error;

#[cfg(any(feature = "cpal-output", feature = "web-output"))]
pub use output::DefaultOutputDevice;
pub use output::OutputDevice;

pub use player::{
    EffectId, EffectMovement, MixerId, PlaybackId, PlaybackStatusContext, PlaybackStatusEvent,
    Player,
};

pub use effect::{Effect, EffectMessage, EffectMessagePayload, EffectTime};
pub use parameter::{
    ClonableParameter, Parameter, ParameterScaling, ParameterType, ParameterValueUpdate,
};

pub use source::{
    file::{FilePlaybackMessage, FilePlaybackOptions, FileSource},
    resampled::ResamplingQuality,
    synth::{SynthPlaybackMessage, SynthPlaybackOptions, SynthSource},
    Source, SourceTime,
};

// -------------------------------------------------------------------------------------------------

// public, modularized re-exports (common trait impls)

pub mod outputs {
    //! Default [`OutputDevice`](super::OutputDevice) implementations.

    #[cfg(feature = "cpal-output")]
    pub use super::output::cpal::CpalOutput;
    #[cfg(feature = "cpal-output")]
    pub use super::output::AudioHostId;

    #[cfg(feature = "web-output")]
    pub use super::output::web::WebOutput;

    #[cfg(feature = "wav-output")]
    pub use super::output::wav::WavOutput;
}

pub mod sources {
    //! Set of basic, common File & Synth tone [`Source`](super::Source) implementations.

    pub use super::source::file::{
        common::FileSourceImpl,
        preloaded::{PreloadedFileBuffer, PreloadedFileSource},
        streamed::StreamedFileSource,
    };
    pub use super::source::synth::common::{SynthSourceGenerator, SynthSourceImpl};
    #[cfg(feature = "dasp")]
    pub use super::source::synth::dasp::DaspSynthSource;
}

pub mod parameters {
    //! Effect [`Parameter`](super::Parameter) implementations.

    pub use super::parameter::{
        BooleanParameter, BooleanParameterValue, EnumParameter, EnumParameterValue, FloatParameter,
        FloatParameterValue, IntegerParameter, IntegerParameterValue, SmoothedParameterValue,
    };
}

pub mod effects {
    //! Set of basic, common DSP [`Effect`](super::Effect) implementations.

    pub use super::effect::{
        chorus::{ChorusEffect, ChorusEffectFilterType, ChorusEffectMessage},
        compressor::CompressorEffect,
        dcfilter::{DcFilterEffect, DcFilterEffectMode},
        distortion::{DistortionEffect, DistortionType},
        eq5::Eq5Effect,
        filter::{FilterEffect, FilterEffectType},
        gain::GainEffect,
        reverb::{ReverbEffect, ReverbEffectMessage},
    };
}

// -------------------------------------------------------------------------------------------------

// public mods

pub mod utils;
