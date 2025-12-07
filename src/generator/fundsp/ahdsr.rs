//! FunDSP AudioNode wrapper for the AHDSR envelope.

use fundsp::hacker32::*;
use std::time::Duration;

use crate::utils::ahdsr::{AhdsrEnvelope, AhdsrParameters};

// -------------------------------------------------------------------------------------------------

/// FunDSP AudioNode that wraps an [`AhdsrEnvelope`] with shared parameter control.
///
/// This node has no inputs and one output. It reads the gate signal from a shared
/// variable and outputs the envelope value. All AHDSR parameters (attack, hold, decay,
/// sustain, release) are also read from shared variables, allowing real-time control.
#[derive(Clone)]
pub struct SharedAhdsrNode {
    params: AhdsrParameters,
    envelope: AhdsrEnvelope,
    gate_shared: Shared,
    attack_shared: Shared,
    hold_shared: Shared,
    decay_shared: Shared,
    sustain_shared: Shared,
    release_shared: Shared,
    last_gate: f32,
}

impl SharedAhdsrNode {
    /// Create a new AHDSR node with the given sample rate and shared parameter controls.
    ///
    /// # Arguments
    /// * `gate` - Shared variable for gate signal (> 0.0 = note on, <= 0.0 = note off)
    /// * `attack` - Shared variable for attack time in seconds
    /// * `hold` - Shared variable for hold time in seconds
    /// * `decay` - Shared variable for decay time in seconds
    /// * `sustain` - Shared variable for sustain level (0.0..=1.0)
    /// * `release` - Shared variable for release time in seconds
    pub fn new(
        gate: Shared,
        attack: Shared,
        hold: Shared,
        decay: Shared,
        sustain: Shared,
        release: Shared,
    ) -> Self {
        Self {
            params: AhdsrParameters::default(),
            envelope: AhdsrEnvelope::new(),
            gate_shared: gate,
            attack_shared: attack,
            hold_shared: hold,
            decay_shared: decay,
            sustain_shared: sustain,
            release_shared: release,
            last_gate: 0.0,
        }
    }
}

impl AudioNode for SharedAhdsrNode {
    const ID: u64 = 101; // Unique ID for this node type
    type Inputs = U0;
    type Outputs = U1;

    #[inline]
    fn tick(&mut self, _input: &Frame<f32, Self::Inputs>) -> Frame<f32, Self::Outputs> {
        // Update parameters from shared values
        let attack = self.attack_shared.value();
        let hold = self.hold_shared.value();
        let decay = self.decay_shared.value();
        let sustain = self.sustain_shared.value();
        let release = self.release_shared.value();

        let _ = self.params.set_attack_time(Duration::from_secs_f32(attack));
        let _ = self.params.set_hold_time(Duration::from_secs_f32(hold));
        let _ = self.params.set_decay_time(Duration::from_secs_f32(decay));
        let _ = self.params.set_sustain_level(sustain);
        let _ = self
            .params
            .set_release_time(Duration::from_secs_f32(release));

        // Handle gate transitions
        let gate = self.gate_shared.value();
        if gate > 0.0 && self.last_gate <= 0.0 {
            // Gate went high: trigger note on
            self.envelope.note_on(&self.params, 1.0);
        } else if gate <= 0.0 && self.last_gate > 0.0 {
            // Gate went low: trigger note off
            self.envelope.note_off(&self.params);
        }
        self.last_gate = gate;

        // Process envelope and return output
        let output = self.envelope.process(&self.params);
        [output].into()
    }

    fn process(&mut self, size: usize, _input: &BufferRef, output: &mut BufferMut) {
        // Update parameters from shared values
        let attack = self.attack_shared.value();
        let hold = self.hold_shared.value();
        let decay = self.decay_shared.value();
        let sustain = self.sustain_shared.value();
        let release = self.release_shared.value();

        let _ = self.params.set_attack_time(Duration::from_secs_f32(attack));
        let _ = self.params.set_hold_time(Duration::from_secs_f32(hold));
        let _ = self.params.set_decay_time(Duration::from_secs_f32(decay));
        let _ = self.params.set_sustain_level(sustain);
        let _ = self
            .params
            .set_release_time(Duration::from_secs_f32(release));

        // Handle gate transitions
        let gate = self.gate_shared.value();
        if gate > 0.0 && self.last_gate <= 0.0 {
            // Gate went high: trigger note on
            self.envelope.note_on(&self.params, 1.0);
        } else if gate <= 0.0 && self.last_gate > 0.0 {
            // Gate went low: trigger note off
            self.envelope.note_off(&self.params);
        }
        self.last_gate = gate;

        // Process the envelope using the optimized buffer method
        debug_assert!(self.inputs() == 0);
        debug_assert!(self.outputs() == 1);

        // Process the envelope in the output
        let output_buffer = output.channel_f32_mut(0);
        self.envelope
            .process_buffer(&self.params, &mut output_buffer[0..size]);
    }

    fn reset(&mut self) {
        self.envelope.reset();
        self.last_gate = 0.0;
    }

    fn set_sample_rate(&mut self, sample_rate: f64) {
        let _ = self.params.set_sample_rate(sample_rate as u32);
    }
}

// -------------------------------------------------------------------------------------------------

/// Helper function to create an AHDSR node wrapped in a fundsp `An`.
///
/// # Arguments
/// * `gate` - Shared variable for gate signal (> 0.0 = note on, <= 0.0 = note off)
/// * `attack` - Shared variable for attack time in seconds
/// * `hold` - Shared variable for hold time in seconds
/// * `decay` - Shared variable for decay time in seconds
/// * `sustain` - Shared variable for sustain level (0.0..=1.0)
/// * `release` - Shared variable for release time in seconds
///
/// # Example
/// ```rust
/// use phonic::{fundsp::hacker32::*, generators::shared_ahdsr};
///
/// let gate = shared(0.0);
/// let attack = shared(0.01);
/// let hold = shared(0.0);
/// let decay = shared(0.1);
/// let sustain = shared(0.7);
/// let release = shared(0.5);
///
/// let envelope = shared_ahdsr(gate, attack, hold, decay, sustain, release);
/// ```
pub fn shared_ahdsr(
    gate: Shared,
    attack: Shared,
    hold: Shared,
    decay: Shared,
    sustain: Shared,
    release: Shared,
) -> An<SharedAhdsrNode> {
    An(SharedAhdsrNode::new(
        gate, attack, hold, decay, sustain, release,
    ))
}

// -------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ahdsr_node_creation() {
        let gate = shared(0.0);
        let attack = shared(0.01);
        let hold = shared(0.0);
        let decay = shared(0.1);
        let sustain = shared(0.7);
        let release = shared(0.5);

        let mut node = SharedAhdsrNode::new(gate, attack, hold, decay, sustain, release);
        node.set_sample_rate(44100.0);

        // Should start in idle state with zero output
        let output = node.tick(&Frame::default());
        assert_eq!(output[0], 0.0);
    }

    #[test]
    fn test_ahdsr_node_gate_trigger() {
        let gate = shared(0.0);
        let attack = shared(0.01);
        let hold = shared(0.0);
        let decay = shared(0.1);
        let sustain = shared(0.7);
        let release = shared(0.5);

        let mut node = SharedAhdsrNode::new(gate.clone(), attack, hold, decay, sustain, release);
        node.set_sample_rate(44100.0);

        // Trigger note on
        gate.set_value(1.0);
        let output = node.tick(&Frame::default());

        // Output should be non-zero after gate trigger
        assert!(output[0] > 0.0);
    }

    #[test]
    fn test_ahdsr_node_reset() {
        let gate = shared(1.0);
        let attack = shared(0.0);
        let hold = shared(0.01);
        let decay = shared(0.1);
        let sustain = shared(0.7);
        let release = shared(0.5);

        let mut node = SharedAhdsrNode::new(gate, attack, hold, decay, sustain, release);
        node.set_sample_rate(44100.0);

        // Process some samples
        for _ in 0..100 {
            node.tick(&Frame::default());
        }

        // Reset should return to attack stage
        node.reset();
        let output = node.tick(&Frame::default());
        assert_eq!(output[0], 1.0);
    }
}
