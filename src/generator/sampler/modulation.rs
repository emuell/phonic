use std::{collections::HashMap, str::FromStr};

use four_cc::FourCC;

use crate::{
    utils::dsp::{lfo::LfoWaveform, modulation::ModulationMatrix},
    Error,
};

use super::Sampler;

// -------------------------------------------------------------------------------------------------

/// Slot type for modulation sources in the modulation matrix
#[derive(Debug, Clone, Copy)]
enum ModulationSlotType {
    Lfo(usize),
    Velocity,
    Keytracking,
}

impl TryFrom<FourCC> for ModulationSlotType {
    type Error = Error;

    fn try_from(value: FourCC) -> Result<Self, Self::Error> {
        match value {
            id if id == Sampler::MOD_SOURCE_LFO1.id() => Ok(ModulationSlotType::Lfo(0)),
            id if id == Sampler::MOD_SOURCE_LFO2.id() => Ok(ModulationSlotType::Lfo(1)),
            id if id == Sampler::MOD_SOURCE_VELOCITY.id() => Ok(ModulationSlotType::Velocity),
            id if id == Sampler::MOD_SOURCE_KEYTRACK.id() => Ok(ModulationSlotType::Keytracking),
            _ => Err(Error::ParameterError(format!(
                "Unknown modulation source: {}",
                value
            ))),
        }
    }
}

// -------------------------------------------------------------------------------------------------

/// Modulation state containing all LFO parameters and routing configuration for the sampler
#[derive(Debug)]
pub struct SamplerModulationState {
    lfo1_rate: f32,
    lfo1_waveform: LfoWaveform,
    lfo2_rate: f32,
    lfo2_waveform: LfoWaveform,
    routing: HashMap<(FourCC, FourCC), (f32, bool)>, // (source, target) -> (amount, bipolar)
}

impl SamplerModulationState {
    pub fn new() -> Self {
        Self {
            lfo1_rate: Sampler::MOD_LFO1_RATE.default_value(),
            lfo1_waveform: LfoWaveform::from_str(Sampler::MOD_LFO1_WAVEFORM.default_value())
                .expect("Failed to parse default LFO waveform string"),
            lfo2_rate: Sampler::MOD_LFO2_RATE.default_value(),
            lfo2_waveform: LfoWaveform::from_str(Sampler::MOD_LFO2_WAVEFORM.default_value())
                .expect("Failed to parse default LFO waveform string"),
            routing: HashMap::with_capacity(
                Sampler::MODULATION_SOURCES.len()
                    * Sampler::GRAIN_MODULATION_TARGET_PARAMETERS.len(),
            ),
        }
    }

    /// Validate that a modulation source and target are compatible
    pub fn validate_routing(source: FourCC, target: FourCC) -> Result<(), Error> {
        // Check if source exists
        if !Sampler::MODULATION_SOURCES.iter().any(|s| s.id == source) {
            return Err(Error::ParameterError(format!(
                "Invalid modulation source: {}",
                source
            )));
        }

        // Check if target is modulatable
        if !Sampler::GRAIN_MODULATION_TARGET_PARAMETERS
            .iter()
            .any(|&p| p.id() == target)
        {
            return Err(Error::ParameterError(format!(
                "Parameter {} is not modulatable",
                target
            )));
        }

        Ok(())
    }

    /// Upate raw modulation values. Applied next time a voice is started or in `update_voice_modulation`
    pub fn update_lfo1_rate(&mut self, rate: f32) {
        self.lfo1_rate = rate;
    }
    pub fn update_lfo1_waveform(&mut self, waveform: LfoWaveform) {
        self.lfo1_waveform = waveform;
    }
    pub fn update_lfo2_rate(&mut self, rate: f32) {
        self.lfo2_rate = rate;
    }
    pub fn update_lfo2_waveform(&mut self, waveform: LfoWaveform) {
        self.lfo2_waveform = waveform;
    }

    /// Set or update a modulation routing
    pub fn set_routing(&mut self, source: FourCC, target: FourCC, amount: f32, bipolar: bool) {
        if amount.abs() < 0.001 {
            // Remove if effectively zero
            self.routing.remove(&(source, target));
        } else {
            self.routing.insert((source, target), (amount, bipolar));
        }
    }

    /// Clear a modulation routing
    pub fn clear_routing(&mut self, source: FourCC, target: FourCC) {
        self.routing.remove(&(source, target));
    }

    /// Initialize a voice's modulation matrix with current modulation state
    pub fn start_voice_modulation(
        &self,
        modulation_matrix: &mut ModulationMatrix,
        note: u8,
        velocity: f32,
    ) {
        // Update LFO 1 configuration and clear targets
        if let Some(slot) = modulation_matrix.lfo_slots.get_mut(0) {
            slot.source.set_rate(self.lfo1_rate as f64);
            slot.source.set_waveform(self.lfo1_waveform);
            slot.clear_targets();
        }

        // Update LFO 2 configuration and clear targets
        if let Some(slot) = modulation_matrix.lfo_slots.get_mut(1) {
            slot.source.set_rate(self.lfo2_rate as f64);
            slot.source.set_waveform(self.lfo2_waveform);
            slot.clear_targets();
        }

        // Update Velocity source and clear targets
        if let Some(ref mut slot) = modulation_matrix.velocity_slot {
            slot.source.set_velocity(velocity);
            slot.clear_targets();
        }

        // Update Keytracking source and clear targets
        if let Some(ref mut slot) = modulation_matrix.keytracking_slot {
            slot.source.set_midi_note(note as f32);
            slot.clear_targets();
        }

        // Apply/update all enabled targets
        for ((source_id, target_id), (amount, bipolar)) in &self.routing {
            if let Ok(slot_type) = ModulationSlotType::try_from(*source_id) {
                match slot_type {
                    ModulationSlotType::Lfo(lfo_index) => {
                        if let Some(slot) = modulation_matrix.lfo_slots.get_mut(lfo_index) {
                            slot.update_target(*target_id, *amount, *bipolar);
                        }
                    }
                    ModulationSlotType::Velocity => {
                        if let Some(ref mut slot) = modulation_matrix.velocity_slot {
                            slot.update_target(*target_id, *amount, *bipolar);
                        }
                    }
                    ModulationSlotType::Keytracking => {
                        if let Some(ref mut slot) = modulation_matrix.keytracking_slot {
                            slot.update_target(*target_id, *amount, *bipolar);
                        }
                    }
                }
            }
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
        match ModulationSlotType::try_from(source_id)? {
            ModulationSlotType::Lfo(lfo_index) => {
                modulation_matrix.update_lfo_target(lfo_index, target_id, amount, bipolar);
            }
            ModulationSlotType::Velocity => {
                modulation_matrix.update_velocity_target(target_id, amount, bipolar);
            }
            ModulationSlotType::Keytracking => {
                modulation_matrix.update_keytracking_target(target_id, amount, bipolar);
            }
        }

        Ok(())
    }
}
