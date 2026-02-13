//! Modulation configuration and state management for FunDSP generators.

use std::collections::HashMap;

use four_cc::FourCC;

use crate::{
    modulation::{
        matrix::ModulationMatrix,
        processor::MODULATION_PROCESSOR_BLOCK_SIZE,
        state::{ModulationSlotType, ModulationState},
        ModulationConfig, ModulationSource, ModulationTarget,
    },
    utils::{dsp::lfo::LfoWaveform, fundsp::SharedBuffer},
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

    /// Check if a parameter ID belongs to a modulation source.
    pub fn is_source_parameter(&self, id: FourCC) -> bool {
        self.inner.is_source_parameter(id)
    }

    /// Get modulation source descriptors for the Generator trait.
    pub fn sources(&self) -> Vec<ModulationSource> {
        self.inner.sources()
    }

    /// Get modulatable parameter IDs for the Generator trait.
    pub fn targets(&self) -> Vec<ModulationTarget> {
        self.inner.targets()
    }

    /// Apply a parameter update to a modulation matrix.
    ///
    /// Reads the current shared parameter value and pushes it to the matrix.
    pub fn apply_parameter_update(
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

// -------------------------------------------------------------------------------------------------

/// Sampler modulation state within a FunDspVoice. Holds and processes the modulation matrix.
pub(crate) struct FunDSpModulationVoiceState {
    matrix: ModulationMatrix,
    shared_buffers: HashMap<FourCC, SharedBuffer>,
    temp_buffer: [f32; MODULATION_PROCESSOR_BLOCK_SIZE],
}

impl FunDSpModulationVoiceState {
    /// Create a new voice state with the given matrix and shared buffers
    pub fn new(matrix: ModulationMatrix, shared_buffers: HashMap<FourCC, SharedBuffer>) -> Self {
        let temp_buffer = [0.0; MODULATION_PROCESSOR_BLOCK_SIZE];
        Self {
            matrix,
            shared_buffers,
            temp_buffer,
        }
    }

    /// Access to the modulation matrix.
    #[inline]
    #[allow(unused)]
    pub fn matrix(&self) -> &ModulationMatrix {
        &self.matrix
    }

    /// Mutable access to the modulation matrix.
    #[inline]
    pub fn matrix_mut(&mut self) -> &mut ModulationMatrix {
        &mut self.matrix
    }

    /// Start modulation processing when the voice starts playing.
    pub fn start(&mut self, note: u8, volume: f32) {
        self.matrix.note_on(note, volume);
    }

    /// Stop modulation processing when the voice stops playing.
    pub fn stop(&mut self) {
        self.matrix.note_off();
    }

    /// Process modulation block and fill shared buffers.
    pub fn process(&mut self, chunk_size: usize) {
        debug_assert!(
            chunk_size <= MODULATION_PROCESSOR_BLOCK_SIZE,
            "Frames exceeds maximum block size"
        );

        // Process matrix
        self.matrix.process(chunk_size);

        // Write processed outputs into shared buffers
        for (param_id, shared_buffer) in &mut self.shared_buffers {
            self.matrix
                .output(*param_id, &mut self.temp_buffer[..chunk_size]);
            shared_buffer.write(&self.temp_buffer[..chunk_size]);
        }
    }

    /// Clear all shared buffers.
    pub fn clear(&mut self) {
        for buffer in self.shared_buffers.values_mut() {
            buffer.clear();
        }
    }
}
