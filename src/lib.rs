//! **phonic** is a cross-platform audio playback and DSP library for Rust. It provides a flexible,
//! low-latency audio engine and DSP tools for desktop and web applications.
//!
//! ### Overview
//!
//! - **[`Player`]** is the central component that manages audio playback. It takes an
//!   output device instance, plays sounds and manages DSP effects.
//!
//! - **[`OutputDevice`]** represents the audio backend stream. phonic provides implementations
//!   for different platforms, such as `cpal` for native applications and `web-audio` for WebAssembly.
//!
//! - **[`Source`]** produces audio signals. You can use the built-in [`FileSource`] for playing
//!   back one-shot audio files, [`SynthSource`] for generating synthesized one-shot tones, or create
//!   your own custom source implementation. Files can be played preloaded from RAM or streamed
//!   on-the-fly.
//!
//! - **[`Generator`]** is a source that plays sounds driven by note and parameter events.
//!   Use e.g. a [`Sampler`](crate::generators::Sampler) to play back sample files with
//!   optional AHDSR envelopes, or [`FunDspGenerator`](crate::generators::FunDspGenerator)
//!   to create a custom synth via [fundsp](https://github.com/SamiPerttu/fundsp), or create your
//!   own custom generator.
//!
//! - **[`Effect`]** applies DSP effects to audio signals in a mixer and describes its automatable
//!   properties via [`Parameter`]s. Phonic comes with a basic set of [`effects`], but you can
//!   create your own custom ones too.
//!   Effects are applied within mixers. By default, the player includes one main mixer that routes
//!   all sources through it. For more complex audio routing, create additional mixers using
//!   [`Player::add_mixer`] and route specific sources to them.
//!
//!
//! ### Getting Started
//!
//! Here's a basic example of how to play audio files with DSP effects.
//!
//! ```rust,no_run
//! use std::time::Duration;
//!
//! use phonic::{
//!     DefaultOutputDevice, Player, FilePlaybackOptions, Error,
//!     effects::{ChorusEffect, ReverbEffect, CompressorEffect},
//!     generators::Sampler, GeneratorPlaybackOptions,
//! };
//!
//! fn main() -> Result<(), Error> {
//!     // Create a player with the default audio output device.
//!     let mut player = Player::new(DefaultOutputDevice::open()?, None);
//!
//!     // Store some constants for event scheduling.
//!     let now = player.output_sample_frame_position();
//!     let samples_per_sec = player.output_sample_rate() as u64;
//!
//!     // Add a new sub-mixer with a chorus and reverb effect to the main mixer.
//!     let sub_mixer = {
//!         let new_mixer = player.add_mixer(None)?;
//!         player.add_effect(ChorusEffect::default(), new_mixer.id())?;
//!         player.add_effect(ReverbEffect::default(), new_mixer.id())?;
//!         new_mixer
//!     };
//!
//!     // Add a limiter to the main mixer. All sounds, including the sub mixer's output,
//!     // will be routed through this effect now.
//!     let limiter = player.add_effect(CompressorEffect::new_limiter(), None)?;
//!
//!     // Change effect parameters via the returned handles.
//!     // Schedule a parameter change 3 seconds from now (sample-accurate).
//!     limiter.set_parameter(
//!         CompressorEffect::MAKEUP_GAIN.value_update(3.0),
//!         now + 3 * samples_per_sec
//!     );
//!
//!     // Play a file and get a handle to control it.
//!     let file = player.play_file("path/to/your/file.wav",
//!       FilePlaybackOptions::default().target_mixer(sub_mixer.id())
//!     )?;
//!
//!     // Control playback via the returned handles.
//!     // Schedule a stop 2 seconds from now (sample-accurate)
//!     file.stop(now + 2 * samples_per_sec)?;
//!
//!     // Play another file on the main mixer with scheduled start time.
//!     let some_other_file = player.play_file("path/to/your/some_other_file.wav",
//!       FilePlaybackOptions::default().start_at_time(now + 2 * samples_per_sec)
//!     )?;
//!
//!     // Create a sampler generator to play a sample.
//!     // We configure it to play on the sub-mixer.
//!     let generator = player.play_generator(
//!         Sampler::from_file(
//!             "path/to/instrument_sample.wav",
//!             GeneratorPlaybackOptions::default().target_mixer(sub_mixer.id()),
//!             player.output_channel_count(),
//!             player.output_sample_rate(),
//!         )?,
//!         None
//!      )?;
//!
//!     // Trigger a note on the generator. The `generator` handle is `Send + Sync`, so you
//!     // can also pass it to other threads (e.g. a MIDI thread) to trigger events from there.
//!     generator.note_on(60, Some(1.0), None, None)?;
//!
//!     // The player's audio output stream runs on a separate thread. Keep the
//!     // main thread running here, until all files finished playing.
//!     while file.is_playing() || some_other_file.is_playing() {
//!         std::thread::sleep(Duration::from_millis(100));
//!     }
//!
//!     Ok(())
//! }
//! ```
//!
//! For more advanced usage, such as monitoring playback, sequencing playback, using generator
//! and creating more complex mixer graphs, please see the examples in the `README.md` and the
//! `/examples` directory of the repository.

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
mod generator;
mod modulation;
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
    EffectHandle, EffectId, EffectMovement, FilePlaybackHandle, GeneratorPlaybackHandle,
    MixerHandle, MixerId, NotePlaybackId, PanicHandler, PlaybackId, Player, PlayerConfig,
    SourcePlaybackHandle, SynthPlaybackHandle,
};

pub use effect::{Effect, EffectMessage, EffectMessagePayload, EffectTime};

pub use parameter::{
    Parameter, ParameterPolarity, ParameterScaling, ParameterType, ParameterValueUpdate,
};

pub use source::{
    file::{FilePlaybackOptions, FileSource},
    measured::CpuLoad,
    resampled::ResamplingQuality,
    status::{PlaybackStatusContext, PlaybackStatusEvent},
    synth::{SynthPlaybackMessage, SynthPlaybackOptions, SynthSource},
    Source, SourceTime,
};

pub use generator::{
    Generator, GeneratorPlaybackEvent, GeneratorPlaybackMessage, GeneratorPlaybackOptions,
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

    pub use super::source::empty::EmptySource;

    pub use super::source::file::{
        common::FileSourceImpl,
        preloaded::{PreloadedFileBuffer, PreloadedFileSource},
        streamed::StreamedFileSource,
        FilePlaybackMessage,
    };
    pub use super::source::synth::{
        common::{SynthSourceGenerator, SynthSourceImpl},
        SynthPlaybackMessage,
    };

    #[cfg(feature = "fundsp")]
    pub use super::source::synth::fundsp::FunDspSynthSource;
}

pub mod generators {
    //! Set of basic, common [`Generator`](crate::Generator) source implementations.

    pub use crate::utils::{
        ahdsr::AhdsrParameters, // used by sampler
        dsp::lfo::LfoWaveform,
    };

    pub use super::modulation::{ModulationConfig, ModulationSource, ModulationTarget};

    pub use super::generator::{
        empty::EmptyGenerator,
        sampler::{
            GrainOverlapMode, GrainPlaybackDirection, GrainPlayheadMode, GrainWindowMode,
            GranularParameters, Sampler,
        },
        GeneratorPlaybackEvent, GeneratorPlaybackMessage,
    };

    #[cfg(feature = "fundsp")]
    pub use super::generator::fundsp::FunDspGenerator;
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

// -------------------------------------------------------------------------------------------------

// public re-exports

/// Create unique [`Parameter`] ids.
pub use four_cc;
/// Create custom Generator impls via [generators::FunDspGenerator].
#[cfg(feature = "fundsp")]
pub use fundsp;
