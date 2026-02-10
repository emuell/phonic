use four_cc::FourCC;

use crate::{
    modulation::{
        matrix::ModulationMatrix,
        state::{ModulationSlotType, ModulationState},
        ModulationConfig, ModulationSource,
    },
    utils::dsp::lfo::LfoWaveform,
    Error,
};

// -------------------------------------------------------------------------------------------------

/// Modulation state for the sampler generator.
///
/// Wraps shared `ModulationState`.
#[derive(Debug)]
pub struct SamplerModulationState {
    inner: ModulationState,
}

impl SamplerModulationState {
    pub fn new(config: ModulationConfig) -> Self {
        let inner = ModulationState::new(config);
        Self { inner }
    }

    /// Create a new modulation matrix from this configuration.
    pub fn create_matrix(&self, sample_rate: u32) -> ModulationMatrix {
        self.inner.create_matrix(sample_rate)
    }

    /// Apply a parameter update to all voice modulation matrices.
    pub fn apply_parameter_update(
        &mut self,
        id: FourCC,
        rate: Option<f32>,
        waveform: Option<LfoWaveform>,
        voices: &mut [super::voice::SamplerVoice],
    ) -> Result<(), Error> {
        // Find which source this parameter belongs to
        for source_config in self.inner.config().sources.iter() {
            match source_config {
                ModulationSource::Lfo {
                    rate_param,
                    waveform_param,
                    ..
                } => {
                    let source_id = source_config.id();
                    let lfo_index = if let Some(ModulationSlotType::Lfo(index)) =
                        self.inner.source_slot_map().get(&source_id)
                    {
                        *index
                    } else {
                        continue;
                    };

                    if id == rate_param.id() {
                        if let Some(rate) = rate {
                            // Update all voices
                            for voice in voices {
                                voice
                                    .modulation_matrix()
                                    .update_lfo_rate(lfo_index, rate as f64);
                            }
                        }
                        return Ok(());
                    } else if id == waveform_param.id() {
                        if let Some(waveform) = waveform {
                            // Update all voices
                            for voice in voices {
                                voice
                                    .modulation_matrix()
                                    .update_lfo_waveform(lfo_index, waveform);
                            }
                        }
                        return Ok(());
                    }
                }
                ModulationSource::Envelope { .. } => {
                    panic!("Not expecting envelope modulation source for a sampler");
                }
                ModulationSource::Velocity { .. } | ModulationSource::Keytracking { .. } => {
                    // No parameters to update
                }
            }
        }

        Err(Error::ParameterError(format!(
            "Invalid/unknown modulation parameter {id}"
        )))
    }

    /// Initialize a voice's modulation matrix with per-note values
    pub fn start_voice_modulation(
        &self,
        modulation_matrix: &mut ModulationMatrix,
        note: u8,
        velocity: f32,
    ) {
        if let Some(ref mut slot) = modulation_matrix.velocity_slot {
            slot.processor.set_velocity(velocity);
        }
        if let Some(ref mut slot) = modulation_matrix.keytracking_slot {
            slot.processor.set_midi_note(note as f32);
        }
    }

    /// Update modulation routing in a single voice's modulation matrix
    pub fn update_voice_modulation(
        &self,
        modulation_matrix: &mut ModulationMatrix,
        source_id: FourCC,
        target_id: FourCC,
        amount: f32,
        bipolar: bool,
    ) -> Result<(), Error> {
        match self.inner.source_slot_map().get(&source_id) {
            Some(ModulationSlotType::Lfo(lfo_index)) => {
                modulation_matrix.update_lfo_target(*lfo_index, target_id, amount, bipolar);
            }
            Some(ModulationSlotType::Envelope(_)) => {
                panic!("Not expecting envelope modulation source for a sampler");
            }
            Some(ModulationSlotType::Velocity) => {
                modulation_matrix.update_velocity_target(target_id, amount, bipolar);
            }
            Some(ModulationSlotType::Keytracking) => {
                modulation_matrix.update_keytracking_target(target_id, amount, bipolar);
            }
            None => {
                return Err(Error::ParameterError(format!(
                    "Unknown modulation source: {}",
                    source_id
                )));
            }
        }

        Ok(())
    }

    /// Get the modulation configuration.
    pub fn config(&self) -> &ModulationConfig {
        self.inner.config()
    }
}
