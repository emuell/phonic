use std::fmt::Debug;

use four_cc::FourCC;

use crate::utils::dsp::lfo::LfoWaveform;

use super::processor::{
    AhdsrModulationProcessor, KeytrackingModulationProcessor, LfoModulationProcessor,
    ModulationProcessor, ModulationProcessorTarget, VelocityModulationProcessor,
    MODULATION_PROCESSOR_BLOCK_SIZE,
};

// -------------------------------------------------------------------------------------------------

/// Container for a modulation processor with its target routings.
///
/// Processes modulation in blocks (up to [`MAX_MODULATION_BLOCK_SIZE`]),
/// caching results for efficient per-sample access. Used by [`ModulationMatrix`].
#[derive(Debug, Clone)]
pub struct ModulationMatrixSlot<P: ModulationProcessor> {
    /// The modulation processor (LFO, envelope, velocity, keytracking)
    pub processor: P,
    /// List of parameter targets this source modulates
    pub targets: Vec<ModulationProcessorTarget>,
    /// Enabled state
    pub enabled: bool,
    /// Block buffer for processed modulation values (reused across calls)
    /// Size matches MAX_BLOCK_SIZE
    pub block_buffer: [f32; MODULATION_PROCESSOR_BLOCK_SIZE],
}

impl<S: ModulationProcessor> ModulationMatrixSlot<S> {
    /// Create a new modulation slot with the given source.
    pub fn new(source: S) -> Self {
        /// Maximum expected targets connected to a modulation processor.
        const MAX_TARGETS: usize = 4;
        Self {
            processor: source,
            targets: Vec::with_capacity(MAX_TARGETS),
            enabled: true,
            block_buffer: [0.0; MODULATION_PROCESSOR_BLOCK_SIZE],
        }
    }

    /// Add a modulation target.
    pub fn add_target(&mut self, target: ModulationProcessorTarget) {
        self.targets.push(target);
    }

    /// Remove all targets.
    #[allow(unused)]
    pub fn clear_targets(&mut self) {
        self.targets.clear();
    }

    /// Update target amount for a specific parameter ID.
    /// If target doesn't exist and amount is non-zero, adds it.
    /// If target exists and amount is zero, removes it.
    /// Otherwise updates the amount.
    pub fn update_target(&mut self, parameter_id: FourCC, amount: f32, bipolar: bool) {
        let threshold = 0.001;
        if let Some(target) = self
            .targets
            .iter_mut()
            .find(|t| t.parameter_id == parameter_id)
        {
            if amount.abs() < threshold {
                // Remove target if amount is effectively zero
                self.targets.retain(|t| t.parameter_id != parameter_id);
            } else {
                // Update existing target
                target.amount = amount;
                target.bipolar = bipolar;
            }
        } else if amount.abs() >= threshold {
            // Add new target if amount is non-zero
            self.add_target(ModulationProcessorTarget::new(
                parameter_id,
                amount,
                bipolar,
            ));
        }
    }

    /// Process modulation block (calls source's process_block) and memorizes its output.
    pub fn process(&mut self, block_size: usize) {
        assert!(
            block_size <= MODULATION_PROCESSOR_BLOCK_SIZE,
            "Invalid block size"
        );
        if self.enabled && self.processor.is_active() {
            self.processor.process(&mut self.block_buffer[..block_size]);
        } else {
            // If disabled or inactive, fill with zeros
            self.block_buffer[..block_size].fill(0.0);
        }
    }
}

// -------------------------------------------------------------------------------------------------

/// Per-voice modulation matrix containing all modulation sources and their routings.
///
/// Created from [`ModulationConfig`](crate::modulation::ModulationConfig). Processes all sources
/// in blocks, providing block or per-sample modulation output for all target parameters.
#[derive(Debug, Clone)]
pub struct ModulationMatrix {
    /// LFO slots (typically 2 or 4 LFOs)
    pub lfo_slots: Vec<ModulationMatrixSlot<LfoModulationProcessor>>,
    /// Envelope slots (typically 1 or 2 AHDSR envelopes)
    pub envelope_slots: Vec<ModulationMatrixSlot<AhdsrModulationProcessor>>,
    /// Velocity slot (single instance, optional)
    pub velocity_slot: Option<ModulationMatrixSlot<VelocityModulationProcessor>>,
    /// Keytracking slot (single instance, optional)
    pub keytracking_slot: Option<ModulationMatrixSlot<KeytrackingModulationProcessor>>,
    /// Current block size: may be less than MAX_MODULATION_BLOCK_SIZE, but never more
    current_output_size: usize,
}

#[allow(unused)]
impl ModulationMatrix {
    /// Create a new empty modulation matrix.
    pub fn new() -> Self {
        // Prealloc for typical usage
        Self {
            lfo_slots: Vec::with_capacity(4),
            envelope_slots: Vec::with_capacity(2),
            velocity_slot: None,
            keytracking_slot: None,
            current_output_size: 0,
        }
    }

    /// Add an LFO slot.
    pub fn add_lfo_slot(&mut self, slot: ModulationMatrixSlot<LfoModulationProcessor>) {
        self.lfo_slots.push(slot);
    }

    /// Add an envelope slot.
    pub fn add_envelope_slot(&mut self, slot: ModulationMatrixSlot<AhdsrModulationProcessor>) {
        self.envelope_slots.push(slot);
    }

    /// Set velocity slot.
    pub fn set_velocity_slot(&mut self, slot: ModulationMatrixSlot<VelocityModulationProcessor>) {
        self.velocity_slot = Some(slot);
    }

    /// Set keytracking slot.
    pub fn set_keytracking_slot(
        &mut self,
        slot: ModulationMatrixSlot<KeytrackingModulationProcessor>,
    ) {
        self.keytracking_slot = Some(slot);
    }

    /// Process all enabled modulation processors for the next chunk of samples.
    ///
    /// # Arguments
    /// * `chunk_size` - Number of samples to process (up to MAX_MODULATION_BLOCK_SIZE)
    pub fn process(&mut self, chunk_size: usize) {
        assert!(
            chunk_size <= MODULATION_PROCESSOR_BLOCK_SIZE,
            "Chunk must be < MAX_MODULATION_BLOCK_SIZE, but is: {chunk_size}"
        );

        // Process all enabled slots
        for slot in &mut self.lfo_slots {
            slot.process(chunk_size);
        }
        for slot in &mut self.envelope_slots {
            slot.process(chunk_size);
        }
        if let Some(slot) = &mut self.velocity_slot {
            slot.process(chunk_size);
        }
        if let Some(slot) = &mut self.keytracking_slot {
            slot.process(chunk_size);
        }

        // Memorize valid size
        self.current_output_size = chunk_size;
    }

    /// Last processed, valid modulation output value size.
    pub fn output_size(&self) -> usize {
        self.current_output_size
    }

    /// Get accumulated preprocessed modulation values for a single parameter.
    ///
    /// Writes the sum of all modulation processors targeting the given parameter to the output buffer.
    /// The output buffer must be at least `output_size` long.
    pub fn output(&self, parameter_id: FourCC, output: &mut [f32]) {
        let block_size = self.current_output_size;
        debug_assert!(
            output.len() >= block_size,
            "Output buffer too small for block size"
        );

        let apply_unipolar_block =
            |output: &mut [f32], input: &[f32], amount: f32, bipolar: bool| {
                if bipolar {
                    // Transform unipolar [0.0, 1.0] to bipolar [-1.0, 1.0] target
                    for (out, &inp) in output.iter_mut().zip(input) {
                        let mod_value = (inp - 0.5) * 2.0;
                        *out += mod_value * amount;
                    }
                } else {
                    // Use as-is
                    for (out, &inp) in output.iter_mut().zip(input) {
                        *out += inp * amount;
                    }
                }
            };

        let apply_bipolar_block =
            |output: &mut [f32], input: &[f32], amount: f32, bipolar: bool| {
                if bipolar {
                    // Use as-is
                    for (o, &i) in output.iter_mut().zip(input) {
                        *o += i * amount;
                    }
                } else {
                    // Transform bipolar [-1.0, 1.0] to unipolar [0.0, 1.0] target
                    for (o, &i) in output.iter_mut().zip(input) {
                        let mod_value = (i + 1.0) / 2.0;
                        *o += mod_value * amount;
                    }
                }
            };

        // Initialize output with zeros
        output[..block_size].fill(0.0);

        // Accumulate modulation from all LFO slots
        for slot in &self.lfo_slots {
            if slot.enabled {
                for target in &slot.targets {
                    if target.parameter_id == parameter_id {
                        // bipolar LFO to unipolar or bipolar target
                        apply_bipolar_block(
                            &mut output[..block_size],
                            &slot.block_buffer[..block_size],
                            target.amount,
                            target.bipolar,
                        );
                    }
                }
            }
        }

        // Accumulate modulation from all envelope slots
        for slot in &self.envelope_slots {
            if slot.enabled {
                for target in &slot.targets {
                    if target.parameter_id == parameter_id {
                        // unipolar envelope to unipolar or bipolar target
                        apply_unipolar_block(
                            &mut output[..block_size],
                            &slot.block_buffer[..block_size],
                            target.amount,
                            target.bipolar,
                        );
                    }
                }
            }
        }

        // Accumulate modulation from velocity slot
        if let Some(slot) = &self.velocity_slot {
            if slot.enabled {
                for target in &slot.targets {
                    if target.parameter_id == parameter_id {
                        // unipolar velocity to unipolar or bipolar target
                        apply_unipolar_block(
                            &mut output[..block_size],
                            &slot.block_buffer[..block_size],
                            target.amount,
                            target.bipolar,
                        );
                    }
                }
            }
        }

        // Accumulate modulation from keytracking slot
        if let Some(slot) = &self.keytracking_slot {
            if slot.enabled {
                for target in &slot.targets {
                    if target.parameter_id == parameter_id {
                        // unipolar keytracking to unipolar or bipolar target
                        apply_unipolar_block(
                            &mut output[..block_size],
                            &slot.block_buffer[..block_size],
                            target.amount,
                            target.bipolar,
                        );
                    }
                }
            }
        }
    }

    /// Get accumulated preprocessed modulation value for a parameter at a specific sample position.
    ///
    /// Returns the sum of all modulation processors targeting the given parameter,
    /// weighted by their amounts.
    #[inline]
    pub fn output_at(&self, parameter_id: FourCC, sample_index: usize) -> f32 {
        debug_assert!(
            sample_index < self.current_output_size,
            "Sample index out of bounds"
        );

        let mut total = 0.0;

        let apply_unipolar = |raw_value: f32, bipolar: bool| -> f32 {
            if bipolar {
                // Transform unipolar [0.0, 1.0] to bipolar [-1.0, 1.0] target
                (raw_value - 0.5) * 2.0
            } else {
                // Use as-is
                raw_value
            }
        };

        let apply_bipolar = |raw_value: f32, bipolar: bool| -> f32 {
            if bipolar {
                // Use as-is
                raw_value
            } else {
                // Transform bipolar [-1.0, 1.0] to unipolar [0.0, 1.0] target
                (raw_value + 1.0) / 2.0
            }
        };

        // Accumulate modulation from all LFO slots
        for slot in &self.lfo_slots {
            if slot.enabled {
                for target in &slot.targets {
                    if target.parameter_id == parameter_id {
                        let raw_value = slot.block_buffer[sample_index];
                        let mod_value = apply_bipolar(raw_value, target.bipolar);
                        total += mod_value * target.amount;
                    }
                }
            }
        }

        // Accumulate modulation from all envelope slots
        for slot in &self.envelope_slots {
            if slot.enabled {
                for target in &slot.targets {
                    if target.parameter_id == parameter_id {
                        let raw_value = slot.block_buffer[sample_index];
                        let mod_value = apply_unipolar(raw_value, target.bipolar);
                        total += mod_value * target.amount;
                    }
                }
            }
        }

        // Accumulate modulation from velocity slot
        if let Some(slot) = &self.velocity_slot {
            if slot.enabled {
                for target in &slot.targets {
                    if target.parameter_id == parameter_id {
                        let raw_value = slot.block_buffer[sample_index];
                        let mod_value = apply_unipolar(raw_value, target.bipolar);
                        total += mod_value * target.amount;
                    }
                }
            }
        }

        // Accumulate modulation from keytracking slot
        if let Some(slot) = &self.keytracking_slot {
            if slot.enabled {
                for target in &slot.targets {
                    if target.parameter_id == parameter_id {
                        let raw_value = slot.block_buffer[sample_index];
                        let mod_value = apply_unipolar(raw_value, target.bipolar);
                        total += mod_value * target.amount;
                    }
                }
            }
        }

        total
    }

    /// Reset & (Re)Trigger & all sources.
    pub fn note_on(&mut self, note: u8, volume: f32) {
        for slot in &mut self.lfo_slots {
            slot.processor.reset();
        }
        for slot in &mut self.envelope_slots {
            slot.processor.reset();
            slot.processor.note_on(1.0); // 1.0 for full modulation depth
        }
        if let Some(slot) = &mut self.velocity_slot {
            slot.processor.set_velocity(volume);
        }
        if let Some(slot) = &mut self.keytracking_slot {
            slot.processor.set_midi_note(note as f32);
        }
    }

    /// Trigger note-off for all envelope sources.
    pub fn note_off(&mut self) {
        for slot in &mut self.envelope_slots {
            slot.processor.note_off();
        }
    }

    /// Update LFO rate for a specific LFO slot.
    pub fn update_lfo_rate(&mut self, lfo_index: usize, rate: f64) {
        if let Some(slot) = self.lfo_slots.get_mut(lfo_index) {
            slot.processor.set_rate(rate);
        }
    }

    /// Update LFO waveform for a specific LFO slot.
    pub fn update_lfo_waveform(&mut self, lfo_index: usize, waveform: LfoWaveform) {
        if let Some(slot) = self.lfo_slots.get_mut(lfo_index) {
            slot.processor.set_waveform(waveform);
        }
    }

    /// Update LFO target amount for a specific LFO slot and parameter.
    pub fn update_lfo_target(
        &mut self,
        lfo_index: usize,
        parameter_id: FourCC,
        amount: f32,
        bipolar: bool,
    ) {
        if let Some(slot) = self.lfo_slots.get_mut(lfo_index) {
            slot.update_target(parameter_id, amount, bipolar);
        }
    }

    /// Update envelope attack time for a specific envelope slot.
    pub fn update_envelope_attack(&mut self, env_index: usize, attack: f32) {
        if let Some(slot) = self.envelope_slots.get_mut(env_index) {
            slot.processor.set_attack(attack);
        }
    }

    /// Update envelope hold time for a specific envelope slot.
    pub fn update_envelope_hold(&mut self, env_index: usize, hold: f32) {
        if let Some(slot) = self.envelope_slots.get_mut(env_index) {
            slot.processor.set_hold(hold);
        }
    }

    /// Update envelope decay time for a specific envelope slot.
    pub fn update_envelope_decay(&mut self, env_index: usize, decay: f32) {
        if let Some(slot) = self.envelope_slots.get_mut(env_index) {
            slot.processor.set_decay(decay);
        }
    }

    /// Update envelope sustain level for a specific envelope slot.
    pub fn update_envelope_sustain(&mut self, env_index: usize, sustain: f32) {
        if let Some(slot) = self.envelope_slots.get_mut(env_index) {
            slot.processor.set_sustain(sustain);
        }
    }

    /// Update envelope release time for a specific envelope slot.
    pub fn update_envelope_release(&mut self, env_index: usize, release: f32) {
        if let Some(slot) = self.envelope_slots.get_mut(env_index) {
            slot.processor.set_release(release);
        }
    }

    /// Update envelope target amount for a specific envelope slot and parameter.
    pub fn update_envelope_target(
        &mut self,
        env_index: usize,
        parameter_id: FourCC,
        amount: f32,
        bipolar: bool,
    ) {
        if let Some(slot) = self.envelope_slots.get_mut(env_index) {
            slot.update_target(parameter_id, amount, bipolar);
        }
    }

    /// Update velocity target amount for a specific parameter.
    pub fn update_velocity_target(&mut self, parameter_id: FourCC, amount: f32, bipolar: bool) {
        if let Some(slot) = &mut self.velocity_slot {
            slot.update_target(parameter_id, amount, bipolar);
        }
    }

    /// Update keytracking target amount for a specific parameter.
    pub fn update_keytracking_target(&mut self, parameter_id: FourCC, amount: f32, bipolar: bool) {
        if let Some(slot) = &mut self.keytracking_slot {
            slot.update_target(parameter_id, amount, bipolar);
        }
    }
}

impl Default for ModulationMatrix {
    fn default() -> Self {
        Self::new()
    }
}
