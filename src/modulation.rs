//! Modulation system for parameter automation.
//!
//! Provides modulation matrix architecture where sources (LFOs, envelopes, velocity, keytracking) can route to
//! target parameters with configurable depth and polarity.

use four_cc::FourCC;

use crate::parameter::{EnumParameter, FloatParameter, Parameter, ParameterPolarity};

// -------------------------------------------------------------------------------------------------

pub(crate) mod matrix;
pub(crate) mod processor;
pub(crate) mod state;

// -------------------------------------------------------------------------------------------------

/// Configuration for a modulation source for a modulation source (e.g. LFO, AHDSR envelope,
/// velocity, keytracking) within a [`ModulationConfig`] as used by [`Generator`](crate::Generator).
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum ModulationSource {
    /// LFO with rate and waveform parameters.
    Lfo {
        id: FourCC,
        name: &'static str,
        rate_param: FloatParameter,
        waveform_param: EnumParameter,
    },
    /// AHDSR envelope with attack, hold, decay, sustain, and release parameters.
    Envelope {
        id: FourCC,
        name: &'static str,
        attack_param: FloatParameter,
        hold_param: FloatParameter,
        decay_param: FloatParameter,
        sustain_param: FloatParameter,
        release_param: FloatParameter,
    },
    /// Velocity (static per note, no parameters).
    Velocity { id: FourCC, name: &'static str },
    /// Keytracking (static per note, no parameters).
    Keytracking { id: FourCC, name: &'static str },
}

impl ModulationSource {
    /// Get the source ID.
    pub fn id(&self) -> FourCC {
        match self {
            Self::Lfo { id, .. } => *id,
            Self::Envelope { id, .. } => *id,
            Self::Velocity { id, .. } => *id,
            Self::Keytracking { id, .. } => *id,
        }
    }

    /// Get the source name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Lfo { name, .. } => name,
            Self::Envelope { name, .. } => name,
            Self::Velocity { name, .. } => name,
            Self::Keytracking { name, .. } => name,
        }
    }

    /// Get parameter descriptors for this modulation source.
    pub fn parameters(&self) -> Vec<&dyn Parameter> {
        match self {
            Self::Lfo {
                rate_param,
                waveform_param,
                ..
            } => vec![rate_param as &dyn Parameter, waveform_param],
            Self::Envelope {
                attack_param,
                hold_param,
                decay_param,
                sustain_param,
                release_param,
                ..
            } => vec![
                attack_param as &dyn Parameter,
                hold_param,
                decay_param,
                sustain_param,
                release_param,
            ],
            Self::Velocity { .. } | Self::Keytracking { .. } => vec![],
        }
    }

    /// Get the polarity of this modulation source.
    pub fn polarity(&self) -> ParameterPolarity {
        match self {
            Self::Lfo { .. } => ParameterPolarity::Bipolar,
            Self::Envelope { .. } | Self::Velocity { .. } | Self::Keytracking { .. } => {
                ParameterPolarity::Unipolar
            }
        }
    }
}

// -------------------------------------------------------------------------------------------------

/// Identifies a modulatable target parameter by ID and name within a [`ModulationSource`]
/// as used by [`Generator`](crate::Generator).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModulationTarget {
    id: FourCC,
    name: &'static str,
}

impl ModulationTarget {
    /// Create a new modulation target.
    pub const fn new(id: FourCC, name: &'static str) -> Self {
        Self { id, name }
    }

    /// Unique identifier for this modulation source.
    #[inline]
    pub const fn id(&self) -> FourCC {
        self.id
    }

    /// Human-readable name for display.
    #[inline]
    pub const fn name(&self) -> &'static str {
        self.name
    }
}

// -------------------------------------------------------------------------------------------------

/// Defines available modulation sources and targets for a [`Generator`](crate::Generator).
#[derive(Debug, Clone, Default)]
pub struct ModulationConfig {
    /// Available modulation sources
    pub sources: Vec<ModulationSource>,
    /// Parameters that can receive modulation
    pub targets: Vec<ModulationTarget>,
}

impl ModulationConfig {
    /// Get all modulation source parameters of all configs.
    pub fn source_parameters(&self) -> Vec<Box<dyn Parameter>> {
        self.sources
            .iter()
            .flat_map(|source_config| {
                source_config
                    .parameters()
                    .iter()
                    .map(|p| p.dyn_clone())
                    .collect::<Vec<_>>()
            })
            .collect()
    }
}
