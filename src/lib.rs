#![doc = include_str!("../README.md")]

// private mods (will be partly re-exported)
mod effect;
mod error;
#[cfg(any(feature = "cpal", feature = "sokol", doc))]
mod output;
mod player;
mod source;

// public, flat re-exports
pub use error::Error;

#[cfg(any(feature = "cpal", doc))]
pub use output::AudioHostId;
#[cfg(any(feature = "cpal", feature = "sokol", doc))]
pub use output::{DefaultOutputDevice, DefaultOutputSink, OutputDevice, OutputSink};

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
