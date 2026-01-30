use std::fmt::Debug;

use four_cc::FourCC;

use crate::utils::{
    ahdsr::{AhdsrEnvelope, AhdsrParameters, AhdsrStage},
    dsp::lfo::{Lfo, LfoWaveform},
};

// -------------------------------------------------------------------------------------------------

/// Maximum block size for modulation processing (samples).
/// This matches FunDSP's block size for optimal performance.
pub const MAX_MODULATION_BLOCK_SIZE: usize = 64;

/// Maximum expected targets connected to a modulation source.
/// This is just a hint for preallocating memory.
const MAX_TARGETS: usize = 4;

// -------------------------------------------------------------------------------------------------

/// Modulation sources that processes audio as modulation values.
pub trait ModulationSource: Debug + Clone + Send {
    /// Initialize/reset the modulation source (called on note-on or when source is enabled).
    fn reset(&mut self, sample_rate: u32);

    /// Check if source is active (for envelopes: not idle, for LFOs: always true).
    fn is_active(&self) -> bool;

    /// Process a block of samples and write modulation values to the given output buffer.
    ///
    /// # Exected output ranges:
    /// - LFOs: bipolar [-1.0, 1.0]
    /// - Envelopes: unipolar [0.0, 1.0]
    /// - Velocity/Keytracking: unipolar [0.0, 1.0]
    fn process(&mut self, output: &mut [f32]);
}

// -------------------------------------------------------------------------------------------------

/// LFO modulation source (wraps existing Lfo).
///
/// Output: bipolar [-1.0, 1.0]
#[derive(Debug, Clone)]
pub struct LfoModulationSource {
    lfo: Lfo,
    sample_rate: u32,
    rate: f64,
    waveform: LfoWaveform,
}

impl LfoModulationSource {
    /// Create a new LFO modulation source.
    pub fn new(sample_rate: u32, rate: f64, waveform: LfoWaveform) -> Self {
        let lfo = Lfo::new(sample_rate, rate, waveform);
        Self {
            lfo,
            sample_rate,
            rate,
            waveform,
        }
    }

    /// Get current LFO rate.
    #[allow(unused)]
    pub fn rate(&self) -> f64 {
        self.rate
    }
    /// Set LFO rate in Hz.
    pub fn set_rate(&mut self, rate: f64) {
        self.rate = rate;
        self.lfo.set_rate(self.sample_rate, rate);
    }

    /// Get current waveform.
    #[allow(unused)]
    pub fn waveform(&self) -> LfoWaveform {
        self.waveform
    }
    /// Set LFO waveform.
    pub fn set_waveform(&mut self, waveform: LfoWaveform) {
        self.waveform = waveform;
        self.lfo.set_waveform(waveform);
    }
}

impl ModulationSource for LfoModulationSource {
    fn reset(&mut self, sample_rate: u32) {
        self.sample_rate = sample_rate;
        self.lfo = Lfo::new(sample_rate, self.rate, self.waveform);
    }

    fn is_active(&self) -> bool {
        true // LFOs are always active
    }

    fn process(&mut self, output: &mut [f32]) {
        self.lfo.process(output)
    }
}

// -------------------------------------------------------------------------------------------------

/// AHDSR envelope modulation source (wraps existing AhdsrEnvelope).
///
/// Output: unipolar [0.0, 1.0]
#[derive(Clone)]
pub struct AhdsrModulationSource {
    envelope: AhdsrEnvelope,
    parameters: AhdsrParameters,
}

// Manual Debug implementation since AhdsrEnvelope and AhdsrParameters don't derive Debug
impl std::fmt::Debug for AhdsrModulationSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AhdsrModulationSource")
            .field("stage", &self.envelope.stage())
            .field("output", &self.envelope.output())
            .finish()
    }
}

#[allow(unused)]
impl AhdsrModulationSource {
    /// Create a new AHDSR envelope modulation source.
    pub fn new(parameters: AhdsrParameters) -> Self {
        let envelope = AhdsrEnvelope::new();
        Self {
            envelope,
            parameters,
        }
    }

    /// Trigger the envelope (called on note-on).
    /// Volume parameter scales the envelope output (typically 1.0 for modulation).
    pub fn note_on(&mut self, volume: f32) {
        self.envelope.note_on(&self.parameters, volume);
    }
    /// Release the envelope (called on note-off).
    pub fn note_off(&mut self) {
        self.envelope.note_off(&self.parameters);
    }

    /// Get current envelope stage.
    #[allow(unused)]
    pub fn stage(&self) -> AhdsrStage {
        self.envelope.stage()
    }

    /// Get current parameters.
    #[allow(unused)]
    pub fn parameters(&self) -> &AhdsrParameters {
        &self.parameters
    }
    /// Update envelope parameters.
    pub fn set_parameters(&mut self, parameters: AhdsrParameters) {
        self.parameters = parameters;
    }
}

impl ModulationSource for AhdsrModulationSource {
    fn reset(&mut self, _sample_rate: u32) {
        self.envelope = AhdsrEnvelope::new();
        self.envelope.note_on(&self.parameters, 1.0); // Full volume for modulation
    }

    fn is_active(&self) -> bool {
        self.envelope.stage() != AhdsrStage::Idle
    }

    fn process(&mut self, output: &mut [f32]) {
        self.envelope.process(&self.parameters, output);
    }
}

// -------------------------------------------------------------------------------------------------

/// Velocity modulation source (static per note).
///
/// Output: unipolar [0.0, 1.0]
#[derive(Debug, Clone)]
pub struct VelocityModulationSource {
    velocity: f32,
}

impl VelocityModulationSource {
    /// Create a new velocity modulation source.
    ///
    /// # Arguments
    /// * `velocity` - Note velocity (0.0-1.0)
    pub fn new(velocity: f32) -> Self {
        debug_assert!(
            (0.0..=1.0).contains(&velocity),
            "Velocity must be in range [0.0, 1.0]"
        );
        Self { velocity }
    }

    /// Get current velocity.
    #[allow(unused)]
    pub fn velocity(&self) -> f32 {
        self.velocity
    }

    /// Set velocity (for parameter updates).
    pub fn set_velocity(&mut self, velocity: f32) {
        debug_assert!(
            (0.0..=1.0).contains(&velocity),
            "Velocity must be in range [0.0, 1.0]"
        );
        self.velocity = velocity;
    }
}

impl ModulationSource for VelocityModulationSource {
    fn reset(&mut self, _sample_rate: u32) {
        // Velocity is static, nothing to reset
    }

    fn is_active(&self) -> bool {
        true // Velocity is always active (static value)
    }

    fn process(&mut self, output: &mut [f32]) {
        output.fill(self.velocity);
    }
}

// -------------------------------------------------------------------------------------------------

/// Keytracking modulation source (note pitch as modulation, static per note).
///
/// Output: unipolar [0.0, 1.0] where 0.0 = MIDI note 0, 1.0 = MIDI note 127
///
/// Common use case: Filter cutoff tracking keyboard (higher notes = brighter filter)
#[derive(Debug, Clone)]
pub struct KeytrackingModulationSource {
    note_pitch: f32, // Normalized MIDI note (0.0-1.0)
}

impl KeytrackingModulationSource {
    /// Create a new keytracking modulation source.
    ///
    /// # Arguments
    /// * `midi_note` - MIDI note number (0-127)
    pub fn new(midi_note: f32) -> Self {
        debug_assert!(
            (0.0..=127.0).contains(&midi_note),
            "MIDI note must be in range [0.0, 127.0]"
        );
        let note_pitch = midi_note / 127.0;
        Self { note_pitch }
    }

    /// Get current note pitch (normalized 0.0-1.0).
    #[allow(unused)]
    pub fn note_pitch(&self) -> f32 {
        self.note_pitch
    }

    /// Set note pitch from MIDI note number.
    pub fn set_midi_note(&mut self, midi_note: f32) {
        debug_assert!(
            (0.0..=127.0).contains(&midi_note),
            "MIDI note must be in range [0.0, 127.0]"
        );
        self.note_pitch = midi_note / 127.0;
    }
}

impl ModulationSource for KeytrackingModulationSource {
    fn reset(&mut self, _sample_rate: u32) {
        // Keytracking is static, nothing to reset
    }

    fn is_active(&self) -> bool {
        true // Keytracking is always active (static value)
    }

    fn process(&mut self, output: &mut [f32]) {
        output.fill(self.note_pitch);
    }
}

// -------------------------------------------------------------------------------------------------

/// Modulation target specification.
///
/// Defines which parameter a modulation source should affect and by how much.
#[derive(Debug, Clone)]
pub struct ModulationTarget {
    /// Parameter ID to modulate
    pub parameter_id: FourCC,
    /// Modulation amount/depth (0.0 = none, 1.0 = full range)
    pub amount: f32,
    /// Bipolar mode: if true, 0.5 is center, modulation goes +/-
    /// If false, 0.0 is minimum, modulation only goes positive
    pub bipolar: bool,
}

impl ModulationTarget {
    /// Create a new modulation target.
    pub fn new(parameter_id: FourCC, amount: f32, bipolar: bool) -> Self {
        Self {
            parameter_id,
            amount,
            bipolar,
        }
    }
}

// -------------------------------------------------------------------------------------------------

/// Modulation slot containing a modulation source and its targets.
///
/// Each slot processes its source in blocks and caches the results for per-sample access.
#[derive(Debug, Clone)]
pub struct ModulationSlot<S: ModulationSource> {
    /// The modulation source (LFO, envelope, velocity, keytracking)
    pub source: S,
    /// List of parameter targets this source modulates
    pub targets: Vec<ModulationTarget>,
    /// Enabled state
    pub enabled: bool,
    /// Block buffer for processed modulation values (reused across calls)
    /// Size matches MAX_BLOCK_SIZE
    pub block_buffer: [f32; MAX_MODULATION_BLOCK_SIZE],
}

impl<S: ModulationSource> ModulationSlot<S> {
    /// Create a new modulation slot with the given source.
    pub fn new(source: S) -> Self {
        Self {
            source,
            targets: Vec::with_capacity(MAX_TARGETS),
            enabled: true,
            block_buffer: [0.0; MAX_MODULATION_BLOCK_SIZE],
        }
    }

    /// Add a modulation target.
    pub fn add_target(&mut self, target: ModulationTarget) {
        self.targets.push(target);
    }

    /// Remove all targets.
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
            self.add_target(ModulationTarget::new(parameter_id, amount, bipolar));
        }
    }

    /// Process modulation block (calls source's process_block) and memorizes its output.
    pub fn process(&mut self, block_size: usize) {
        assert!(
            block_size <= MAX_MODULATION_BLOCK_SIZE,
            "Invalid block size"
        );
        if self.enabled && self.source.is_active() {
            self.source.process(&mut self.block_buffer[..block_size]);
        } else {
            // If disabled or inactive, fill with zeros
            self.block_buffer[..block_size].fill(0.0);
        }
    }
}

// -------------------------------------------------------------------------------------------------

/// Runtime configurable Modulation matrix, containing multiple modulation sources.
///
/// Processes all enabled modulation sources in blocks and caches the results for per-sample
/// access during audio processing.
#[derive(Debug, Clone)]
pub struct ModulationMatrix {
    /// LFO slots (typically 2 or 4 LFOs)
    pub lfo_slots: Vec<ModulationSlot<LfoModulationSource>>,
    /// Envelope slots (typically 1 or 2 AHDSR envelopes)
    pub envelope_slots: Vec<ModulationSlot<AhdsrModulationSource>>,
    /// Velocity slot (single instance, optional)
    pub velocity_slot: Option<ModulationSlot<VelocityModulationSource>>,
    /// Keytracking slot (single instance, optional)
    pub keytracking_slot: Option<ModulationSlot<KeytrackingModulationSource>>,
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
    pub fn add_lfo_slot(&mut self, slot: ModulationSlot<LfoModulationSource>) {
        self.lfo_slots.push(slot);
    }

    /// Add an envelope slot.
    pub fn add_envelope_slot(&mut self, slot: ModulationSlot<AhdsrModulationSource>) {
        self.envelope_slots.push(slot);
    }

    /// Set velocity slot.
    pub fn set_velocity_slot(&mut self, slot: ModulationSlot<VelocityModulationSource>) {
        self.velocity_slot = Some(slot);
    }

    /// Set keytracking slot.
    pub fn set_keytracking_slot(&mut self, slot: ModulationSlot<KeytrackingModulationSource>) {
        self.keytracking_slot = Some(slot);
    }

    /// Process all enabled modulation sources for the next chunk of samples.
    ///
    /// # Arguments
    /// * `chunk_size` - Number of samples to process (up to MAX_MODULATION_BLOCK_SIZE)
    pub fn process(&mut self, chunk_size: usize) {
        assert!(
            chunk_size <= MAX_MODULATION_BLOCK_SIZE,
            "Chunk must be < MAX_MODULATION_BLOCK_SIZE, but is: {chunk_size}"
        );

        // Process all enabled slots
        for slot in &mut self.lfo_slots {
            slot.process(chunk_size);
        }
        for slot in &mut self.envelope_slots {
            slot.process(chunk_size);
        }
        if let Some(ref mut slot) = self.velocity_slot {
            slot.process(chunk_size);
        }
        if let Some(ref mut slot) = self.keytracking_slot {
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
    /// Writes the sum of all modulation sources targeting the given parameter to the output buffer.
    /// The output buffer must be at least `output_size` long.
    pub fn modulation_output(&self, parameter_id: FourCC, output: &mut [f32]) {
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
        if let Some(ref slot) = self.velocity_slot {
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
        if let Some(ref slot) = self.keytracking_slot {
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
    /// Returns the sum of all modulation sources targeting the given parameter,
    /// weighted by their amounts.
    #[inline]
    pub fn modulation_output_at(&self, parameter_id: FourCC, sample_index: usize) -> f32 {
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
        if let Some(ref slot) = self.velocity_slot {
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
        if let Some(ref slot) = self.keytracking_slot {
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

    /// Reset all modulation sources (called on note-on).
    pub fn reset(&mut self, sample_rate: u32) {
        for slot in &mut self.lfo_slots {
            slot.source.reset(sample_rate);
        }
        for slot in &mut self.envelope_slots {
            slot.source.reset(sample_rate);
        }
        if let Some(ref mut slot) = self.velocity_slot {
            slot.source.reset(sample_rate);
        }
        if let Some(ref mut slot) = self.keytracking_slot {
            slot.source.reset(sample_rate);
        }
        self.current_output_size = 0;
    }

    /// Trigger note-on for all envelope sources.
    /// Volume parameter scales the envelope output (typically 1.0 for full modulation depth).
    pub fn note_on(&mut self, volume: f32) {
        for slot in &mut self.envelope_slots {
            slot.source.note_on(volume);
        }
    }

    /// Trigger note-off for all envelope sources.
    pub fn note_off(&mut self) {
        for slot in &mut self.envelope_slots {
            slot.source.note_off();
        }
    }

    // --- Helper methods for updating modulation parameters without destroying state ---

    /// Update LFO rate for a specific LFO slot.
    pub fn update_lfo_rate(&mut self, lfo_index: usize, rate: f64) {
        if let Some(slot) = self.lfo_slots.get_mut(lfo_index) {
            slot.source.set_rate(rate);
        }
    }

    /// Update LFO waveform for a specific LFO slot.
    pub fn update_lfo_waveform(&mut self, lfo_index: usize, waveform: LfoWaveform) {
        if let Some(slot) = self.lfo_slots.get_mut(lfo_index) {
            slot.source.set_waveform(waveform);
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

    /// Update envelope parameters for a specific envelope slot.
    pub fn update_envelope_parameters(&mut self, env_index: usize, parameters: AhdsrParameters) {
        if let Some(slot) = self.envelope_slots.get_mut(env_index) {
            slot.source.set_parameters(parameters);
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
        if let Some(ref mut slot) = self.velocity_slot {
            slot.update_target(parameter_id, amount, bipolar);
        }
    }

    /// Update keytracking target amount for a specific parameter.
    pub fn update_keytracking_target(&mut self, parameter_id: FourCC, amount: f32, bipolar: bool) {
        if let Some(ref mut slot) = self.keytracking_slot {
            slot.update_target(parameter_id, amount, bipolar);
        }
    }
}

impl Default for ModulationMatrix {
    fn default() -> Self {
        Self::new()
    }
}
