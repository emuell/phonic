#![doc = include_str!("../README.md")]

// private mods (will be partly re-exported)
mod effect;
mod error;
mod output;
mod player;
mod source;

// public, flat re-exports
pub use error::Error;

#[cfg(any(feature = "cpal-output", feature = "sokol-output", doc))]
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

    #[cfg(any(feature = "cpal-output", doc))]
    pub use super::output::cpal::CpalOutput;
    #[cfg(any(feature = "cpal-output", doc))]
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
