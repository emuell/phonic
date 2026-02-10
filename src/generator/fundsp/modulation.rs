//! Modulation configuration and state management for FunDSP generators.

use std::collections::HashMap;

use four_cc::FourCC;

use crate::{
    modulation::{
        matrix::ModulationMatrix,
        state::{ModulationSlotType, ModulationState},
        ModulationConfig, ModulationSource, ModulationTarget,
    },
    utils::dsp::lfo::LfoWaveform,
};

use super::parameter::SharedParameterValue;

// -------------------------------------------------------------------------------------------------

/// Per-generator modulation state manager for FunDSP generators.
///
/// Wraps shared `ModulationState` and adds FunDSP-specific parameter propagation logic.
pub(crate) struct FunDspModulationState {
    inner: ModulationState,
}

impl FunDspModulationState {
    /// Create a new modulation state from configuration.
    pub fn new(config: ModulationConfig) -> Self {
        Self {
            inner: ModulationState::new(config),
        }
    }

    /// Create a new modulation matrix from this configuration.
    pub fn create_matrix(&self, sample_rate: u32) -> ModulationMatrix {
        self.inner.create_matrix(sample_rate)
    }

    /// Get modulation source descriptors for the Generator trait.
    pub fn modulation_sources(&self) -> Vec<ModulationSource> {
        self.inner.sources()
    }

    /// Get modulatable parameter IDs for the Generator trait.
    pub fn modulation_targets(&self) -> Vec<ModulationTarget> {
        self.inner.targets()
    }

    /// Check if a parameter ID belongs to a modulation source.
    pub fn is_source_parameter(&self, id: FourCC) -> bool {
        self.inner.is_source_parameter(id)
    }

    /// Apply a parameter update to a modulation matrix.
    ///
    /// Reads the current shared parameter value and pushes it to the matrix.
    pub fn apply_parameter_to_matrix(
        &self,
        matrix: &mut ModulationMatrix,
        param_id: FourCC,
        shared_params: &HashMap<FourCC, SharedParameterValue>,
    ) {
        // Find which source this parameter belongs to
        for source_config in self.inner.config().sources.iter() {
            match source_config {
                ModulationSource::Lfo {
                    rate_param,
                    waveform_param,
                    ..
                } => {
                    if let Some(ModulationSlotType::Lfo(lfo_index)) =
                        self.inner.source_slot_map().get(&source_config.id())
                    {
                        if param_id == rate_param.id() {
                            if let Some(param) = shared_params.get(&param_id) {
                                let rate = param.shared().value();
                                matrix.update_lfo_rate(*lfo_index, rate as f64);
                            }
                        } else if param_id == waveform_param.id() {
                            if let Some(param) = shared_params.get(&param_id) {
                                let waveform_index = param.shared().value().round() as usize;
                                let waveform =
                                    <LfoWaveform as strum::VariantArray>::VARIANTS[waveform_index];
                                matrix.update_lfo_waveform(*lfo_index, waveform);
                            }
                        }
                    }
                }
                ModulationSource::Envelope {
                    attack_param,
                    hold_param,
                    decay_param,
                    sustain_param,
                    release_param,
                    ..
                } => {
                    if let Some(ModulationSlotType::Envelope(env_index)) =
                        self.inner.source_slot_map().get(&source_config.id())
                    {
                        // Update individual envelope parameters
                        if param_id == attack_param.id() {
                            if let Some(param) = shared_params.get(&param_id) {
                                matrix.update_envelope_attack(*env_index, param.shared().value());
                            }
                        } else if param_id == hold_param.id() {
                            if let Some(param) = shared_params.get(&param_id) {
                                matrix.update_envelope_hold(*env_index, param.shared().value());
                            }
                        } else if param_id == decay_param.id() {
                            if let Some(param) = shared_params.get(&param_id) {
                                matrix.update_envelope_decay(*env_index, param.shared().value());
                            }
                        } else if param_id == sustain_param.id() {
                            if let Some(param) = shared_params.get(&param_id) {
                                matrix.update_envelope_sustain(*env_index, param.shared().value());
                            }
                        } else if param_id == release_param.id() {
                            if let Some(param) = shared_params.get(&param_id) {
                                matrix.update_envelope_release(*env_index, param.shared().value());
                            }
                        }
                    }
                }
                ModulationSource::Velocity { .. } | ModulationSource::Keytracking { .. } => {
                    // Velocity and keytracking have no parameters
                }
            }
        }
    }

    /// Set or update a modulation routing.
    pub fn set_modulation(
        &self,
        matrix: &mut ModulationMatrix,
        source: FourCC,
        target: FourCC,
        amount: f32,
        bipolar: bool,
    ) -> Result<(), crate::Error> {
        self.inner
            .set_modulation(matrix, source, target, amount, bipolar)
    }

    /// Clear a modulation routing.
    pub fn clear_modulation(
        &self,
        matrix: &mut ModulationMatrix,
        source: FourCC,
        target: FourCC,
    ) -> Result<(), crate::Error> {
        self.inner.clear_modulation(matrix, source, target)
    }
}
