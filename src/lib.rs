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

#[cfg(feature = "dasp")]
pub use source::synth::dasp::DaspSynthSource;

pub use source::{
    file::{
        preloaded::PreloadedFileSource, streamed::StreamedFileSource, FilePlaybackOptions,
        FileSource,
    },
    mixed::MixedSource,
    resampled::ResamplingQuality,
    synth::{SynthPlaybackOptions, SynthSource},
    Source, SourceTime,
};

pub use effect::Effect;

// public mods
pub mod utils;

pub mod effects {
    //! Set of basic, common DSP effect implementations.

    pub use super::effect::{
        chorus::{ChorusEffect, ChorusEffectMessage},
        compressor::{CompressorEffect, CompressorEffectMessage},
        dcfilter::{DcFilterEffect, DcFilterEffectMessage},
        filter::{FilterEffect, FilterEffectMessage, FilterEffectType},
        reverb::{ReverbEffect, ReverbEffectMessage},
    };
}
