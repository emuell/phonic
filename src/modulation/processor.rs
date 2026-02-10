use std::fmt::Debug;

use four_cc::FourCC;

use crate::utils::{
    ahdsr::{AhdsrEnvelope, AhdsrParameters, AhdsrStage},
    dsp::lfo::{Lfo, LfoWaveform},
};

// -------------------------------------------------------------------------------------------------

/// Maximum block size for modulation processing (samples).
/// This should be a a SIMD friendly size for optimal performance.
pub const MODULATION_PROCESSOR_BLOCK_SIZE: usize = 64;

// -------------------------------------------------------------------------------------------------

/// Generates time-varying modulation signals for parameter automation.
///
/// Implemented by LFOs, envelopes, velocity, and keytracking. Outputs modulation
/// values in blocks for use in [`ModulationMatrixSlot`](crate::modulation::matrix::ModulationMatrixSlot).
pub trait ModulationProcessor: Debug + Clone + Send {
    /// Initialize/reset the modulation processor (called on note-on or when source is enabled).
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

/// LFO modulation processor (wraps existing Lfo).
///
/// Output: bipolar [-1.0, 1.0]
#[derive(Debug, Clone)]
pub struct LfoModulationProcessor {
    lfo: Lfo,
    sample_rate: u32,
    rate: f64,
    waveform: LfoWaveform,
}

impl LfoModulationProcessor {
    /// Create a new LFO modulation processor.
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

impl ModulationProcessor for LfoModulationProcessor {
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

/// AHDSR envelope modulation processor (wraps existing AhdsrEnvelope).
///
/// Output: unipolar [0.0, 1.0]
#[derive(Clone)]
pub struct AhdsrModulationProcessor {
    envelope: AhdsrEnvelope,
    parameters: AhdsrParameters,
}

// Manual Debug implementation since AhdsrEnvelope and AhdsrParameters don't derive Debug
impl std::fmt::Debug for AhdsrModulationProcessor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AhdsrModulationProcessor")
            .field("stage", &self.envelope.stage())
            .field("output", &self.envelope.output())
            .finish()
    }
}

#[allow(unused)]
impl AhdsrModulationProcessor {
    /// Create a new AHDSR envelope modulation processor.
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

    /// Update attack time.
    pub fn set_attack(&mut self, attack: f32) {
        let _ = self
            .parameters
            .set_attack_time(std::time::Duration::from_secs_f32(attack));
    }

    /// Update hold time.
    pub fn set_hold(&mut self, hold: f32) {
        let _ = self
            .parameters
            .set_hold_time(std::time::Duration::from_secs_f32(hold));
    }

    /// Update decay time.
    pub fn set_decay(&mut self, decay: f32) {
        let _ = self
            .parameters
            .set_decay_time(std::time::Duration::from_secs_f32(decay));
    }

    /// Update sustain level.
    pub fn set_sustain(&mut self, sustain: f32) {
        let _ = self.parameters.set_sustain_level(sustain);
    }

    /// Update release time.
    pub fn set_release(&mut self, release: f32) {
        let _ = self
            .parameters
            .set_release_time(std::time::Duration::from_secs_f32(release));
    }
}

impl ModulationProcessor for AhdsrModulationProcessor {
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

/// Velocity modulation processor (static per note).
///
/// Output: unipolar [0.0, 1.0]
#[derive(Debug, Clone)]
pub struct VelocityModulationProcessor {
    velocity: f32,
}

impl VelocityModulationProcessor {
    /// Create a new velocity modulation processor.
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

impl ModulationProcessor for VelocityModulationProcessor {
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

/// Keytracking modulation processor (note pitch as modulation, static per note).
///
/// Output: unipolar [0.0, 1.0] where 0.0 = MIDI note 0, 1.0 = MIDI note 127
///
/// Common use case: Filter cutoff tracking keyboard (higher notes = brighter filter)
#[derive(Debug, Clone)]
pub struct KeytrackingModulationProcessor {
    note_pitch: f32, // Normalized MIDI note (0.0-1.0)
}

impl KeytrackingModulationProcessor {
    /// Create a new keytracking modulation processor.
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

impl ModulationProcessor for KeytrackingModulationProcessor {
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

/// Routing from a modulation source to a target parameter.
///
/// Specifies the parameter ID, modulation depth, and polarity transform.
/// Used by [`ModulationMatrixSlot`](crate::modulation::matrix::ModulationMatrixSlot) to route processor output.
#[derive(Debug, Clone)]
pub struct ModulationProcessorTarget {
    /// Parameter ID to modulate
    pub parameter_id: FourCC,
    /// Modulation amount/depth (0.0 = none, 1.0 = full range)
    pub amount: f32,
    /// Bipolar mode: if true, 0.5 is center, modulation goes +/-
    /// If false, 0.0 is minimum, modulation only goes positive
    pub bipolar: bool,
}

impl ModulationProcessorTarget {
    /// Create a new modulation target.
    pub fn new(parameter_id: FourCC, amount: f32, bipolar: bool) -> Self {
        Self {
            parameter_id,
            amount,
            bipolar,
        }
    }
}
