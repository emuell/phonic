//! Shared modulation state management.

use std::collections::HashMap;

use four_cc::FourCC;
use strum::VariantNames;

use crate::{
    modulation::{
        matrix::{ModulationMatrix, ModulationMatrixSlot},
        processor::{
            AhdsrModulationProcessor, KeytrackingModulationProcessor, LfoModulationProcessor,
            VelocityModulationProcessor,
        },
        ModulationConfig, ModulationSource, ModulationTarget,
    },
    utils::{ahdsr::AhdsrParameters, dsp::lfo::LfoWaveform},
    Error,
};

// -------------------------------------------------------------------------------------------------

/// Identifies which slot type and index a modulation source maps to.
#[derive(Debug, Clone, Copy)]
pub enum ModulationSlotType {
    Lfo(usize),      // index into ModulationMatrix.lfo_slots
    Envelope(usize), // index into ModulationMatrix.envelope_slots
    Velocity,        // single velocity slot
    Keytracking,     // single keytracking slot
}

// -------------------------------------------------------------------------------------------------

/// Shared modulation state manager.
///
/// Manages the modulation configuration and provides methods to create and update
/// modulation matrices for each voice. Contains logic that is identical across all generators.
#[derive(Debug)]
pub struct ModulationState {
    config: ModulationConfig,
    /// Maps source FourCC -> slot reference
    source_slot_map: HashMap<FourCC, ModulationSlotType>,
    /// All parameter IDs belonging to modulation sources
    source_parameter_ids: Vec<FourCC>,
}

#[allow(unused)]
impl ModulationState {
    /// Create a new modulation state from configuration.
    pub fn new(config: ModulationConfig) -> Self {
        // Build source slot map
        let mut source_slot_map = HashMap::new();

        let mut lfo_count = 0;
        let mut envelope_count = 0;

        for source_config in &config.sources {
            let slot_type = match source_config {
                ModulationSource::Lfo { .. } => {
                    let lfo = ModulationSlotType::Lfo(lfo_count);
                    lfo_count += 1;
                    lfo
                }
                ModulationSource::Envelope { .. } => {
                    let envelope = ModulationSlotType::Envelope(envelope_count);
                    envelope_count += 1;
                    envelope
                }
                ModulationSource::Velocity { .. } => ModulationSlotType::Velocity,
                ModulationSource::Keytracking { .. } => ModulationSlotType::Keytracking,
            };
            source_slot_map.insert(source_config.id(), slot_type);
        }

        // Collect all parameter IDs from modulation sources
        let mut source_parameter_ids = Vec::new();
        for source_config in &config.sources {
            for param in source_config.parameters() {
                source_parameter_ids.push(param.id());
            }
        }

        Self {
            config,
            source_slot_map,
            source_parameter_ids,
        }
    }

    /// Create a new modulation matrix from this configuration.
    pub fn create_matrix(&self, sample_rate: u32) -> ModulationMatrix {
        let mut matrix = ModulationMatrix::new();

        for source_config in &self.config.sources {
            match source_config {
                ModulationSource::Lfo {
                    rate_param,
                    waveform_param,
                    ..
                } => {
                    let default_rate = rate_param.default_value();
                    // Use the Parameter trait's default_value (returns normalized 0.0-1.0)
                    let default_waveform = LfoWaveform::VARIANTS
                        .iter()
                        .position(|&s| s == waveform_param.default_value())
                        .map(|i| <LfoWaveform as strum::VariantArray>::VARIANTS[i])
                        .expect("Failed to get default LFO waveform value");
                    let source = LfoModulationProcessor::new(
                        sample_rate,
                        default_rate as f64,
                        default_waveform,
                    );
                    matrix.add_lfo_slot(ModulationMatrixSlot::new(source));
                }
                ModulationSource::Envelope {
                    attack_param,
                    hold_param,
                    decay_param,
                    sustain_param,
                    release_param,
                    ..
                } => {
                    let attack = attack_param.default_value();
                    let hold = hold_param.default_value();
                    let decay = decay_param.default_value();
                    let sustain = sustain_param.default_value();
                    let release = release_param.default_value();

                    let mut params = AhdsrParameters::new(
                        std::time::Duration::from_secs_f32(attack),
                        std::time::Duration::from_secs_f32(hold),
                        std::time::Duration::from_secs_f32(decay),
                        sustain,
                        std::time::Duration::from_secs_f32(release),
                    )
                    .unwrap_or_else(|_| AhdsrParameters::default());
                    params
                        .set_sample_rate(sample_rate)
                        .expect("Invalid ahdsr sample rate");

                    let source = AhdsrModulationProcessor::new(params);
                    matrix.add_envelope_slot(ModulationMatrixSlot::new(source));
                }
                ModulationSource::Velocity { .. } => {
                    let source = VelocityModulationProcessor::new(0.0);
                    matrix.set_velocity_slot(ModulationMatrixSlot::new(source));
                }
                ModulationSource::Keytracking { .. } => {
                    let source = KeytrackingModulationProcessor::new(60.0);
                    matrix.set_keytracking_slot(ModulationMatrixSlot::new(source));
                }
            }
        }

        matrix
    }

    /// Check if a parameter ID belongs to a modulation source.
    pub fn is_source_parameter(&self, id: FourCC) -> bool {
        self.source_parameter_ids.contains(&id)
    }

    /// Get modulation source descriptors for the Generator trait.
    pub fn sources(&self) -> Vec<ModulationSource> {
        self.config.sources.clone()
    }

    /// Get modulatable parameter IDs for the Generator trait.
    pub fn targets(&self) -> Vec<ModulationTarget> {
        self.config.targets.clone()
    }

    /// Set or update a modulation routing.
    pub fn set_modulation(
        &self,
        matrix: &mut ModulationMatrix,
        source: FourCC,
        target: FourCC,
        amount: f32,
        bipolar: bool,
    ) -> Result<(), Error> {
        // Validate source exists
        let slot_type = self.source_slot_map.get(&source).ok_or_else(|| {
            Error::ParameterError(format!("Unknown modulation source '{}'", source))
        })?;

        // Validate target exists
        if !self.config.targets.iter().any(|t| t.id == target) {
            return Err(Error::ParameterError(format!(
                "Unknown modulation target '{}'",
                target
            )));
        }

        // Validate amount
        if !(-1.0..=1.0).contains(&amount) {
            return Err(Error::ParameterError(format!(
                "Modulation amount must be in range -1..-1.0 but is {}",
                amount
            )));
        }

        // Update the appropriate slot
        match slot_type {
            ModulationSlotType::Lfo(index) => {
                matrix.update_lfo_target(*index, target, amount, bipolar);
            }
            ModulationSlotType::Envelope(index) => {
                matrix.update_envelope_target(*index, target, amount, bipolar);
            }
            ModulationSlotType::Velocity => {
                matrix.update_velocity_target(target, amount, bipolar);
            }
            ModulationSlotType::Keytracking => {
                matrix.update_keytracking_target(target, amount, bipolar);
            }
        }

        Ok(())
    }

    /// Clear a modulation routing.
    pub fn clear_modulation(
        &self,
        matrix: &mut ModulationMatrix,
        source: FourCC,
        target: FourCC,
    ) -> Result<(), Error> {
        // Clear by setting amount to 0
        self.set_modulation(matrix, source, target, 0.0, false)
    }

    /// Get the modulation configuration.
    pub fn config(&self) -> &ModulationConfig {
        &self.config
    }

    /// Get the source slot map.
    pub fn source_slot_map(&self) -> &HashMap<FourCC, ModulationSlotType> {
        &self.source_slot_map
    }
}
