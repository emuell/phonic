use four_cc::FourCC;

use crate::{
    effect::{Effect, EffectTime},
    parameter::{formatters, FloatParameter, FloatParameterValue, ParameterValueUpdate},
    utils::{buffer::InterleavedBufferMut, db_to_linear, dsp::envelope::EnvelopeFollower},
    Error, Parameter,
};

// -------------------------------------------------------------------------------------------------

/// Stereo noise gate effect with hold and configurable floor level.
pub struct GateEffect {
    // Parameters
    threshold: FloatParameterValue,
    attack_time: FloatParameterValue,
    hold_time: FloatParameterValue,
    release_time: FloatParameterValue,
    range: FloatParameterValue,
    // Internal state
    envelope_follower: EnvelopeFollower,
    hold_counter: u32,
    gate_gain_db: f32,
    attack_coeff: f32,
    release_coeff: f32,
    sample_rate: u32,
    channel_count: usize,
}

impl GateEffect {
    pub const EFFECT_NAME: &str = "Gate";

    pub const THRESHOLD: FloatParameter =
        FloatParameter::new(FourCC(*b"thrs"), "Threshold", -60.0..=0.0, -30.0).with_unit("dB");

    pub const ATTACK_TIME: FloatParameter =
        FloatParameter::new(FourCC(*b"attk"), "Attack", 0.001..=0.5, 0.005).with_unit("ms");

    pub const HOLD_TIME: FloatParameter =
        FloatParameter::new(FourCC(*b"hold"), "Hold", 0.0..=2.0, 0.1).with_unit("ms");

    pub const RELEASE_TIME: FloatParameter =
        FloatParameter::new(FourCC(*b"rels"), "Release", 0.01..=2.0, 0.2).with_unit("ms");

    pub const RANGE: FloatParameter =
        FloatParameter::new(FourCC(*b"rnge"), "Range", -60.0..=0.0, -60.0)
            .with_formatter(formatters::DECIBELS);

    /// Creates a new `GateEffect` with the default parameters.
    pub fn new() -> Self {
        Self {
            threshold: FloatParameterValue::from_description(Self::THRESHOLD),
            attack_time: FloatParameterValue::from_description(Self::ATTACK_TIME),
            hold_time: FloatParameterValue::from_description(Self::HOLD_TIME),
            release_time: FloatParameterValue::from_description(Self::RELEASE_TIME),
            range: FloatParameterValue::from_description(Self::RANGE),
            envelope_follower: EnvelopeFollower::default(),
            hold_counter: 0,
            gate_gain_db: -60.0,
            attack_coeff: 0.0,
            release_coeff: 0.0,
            sample_rate: 0,
            channel_count: 0,
        }
    }

    /// Creates a new `GateEffect` with the given parameters.
    pub fn with_parameters(
        threshold: f32,
        attack_time: f32,
        hold_time: f32,
        release_time: f32,
        range: f32,
    ) -> Self {
        let mut gate = Self::default();
        gate.threshold.set_value(threshold);
        gate.attack_time.set_value(attack_time);
        gate.hold_time.set_value(hold_time);
        gate.release_time.set_value(release_time);
        gate.range.set_value(range);
        gate
    }

    fn update_coefficients(&mut self) {
        if self.sample_rate > 0 {
            self.envelope_follower
                .set_attack_time(self.attack_time.value());
            self.envelope_follower
                .set_release_time(self.release_time.value());
            let sr = self.sample_rate as f32;
            self.attack_coeff = (-1.0 / (self.attack_time.value() * sr)).exp();
            self.release_coeff = (-1.0 / (self.release_time.value() * sr)).exp();
        }
    }
}

impl Default for GateEffect {
    fn default() -> Self {
        Self::new()
    }
}

impl Effect for GateEffect {
    fn name(&self) -> &'static str {
        Self::EFFECT_NAME
    }

    fn weight(&self) -> usize {
        2
    }

    fn parameters(&self) -> Vec<&dyn Parameter> {
        vec![
            self.threshold.description(),
            self.attack_time.description(),
            self.hold_time.description(),
            self.release_time.description(),
            self.range.description(),
        ]
    }

    fn initialize(
        &mut self,
        sample_rate: u32,
        channel_count: usize,
        _max_frames: usize,
    ) -> Result<(), Error> {
        if channel_count != 2 {
            return Err(Error::ParameterError(
                "GateEffect only supports stereo I/O".to_string(),
            ));
        }
        self.sample_rate = sample_rate;
        self.channel_count = channel_count;
        self.envelope_follower = EnvelopeFollower::new(
            sample_rate,
            self.attack_time.value(),
            self.release_time.value(),
        );
        self.envelope_follower.reset(-120.0);
        self.hold_counter = 0;
        self.gate_gain_db = self.range.value();
        self.update_coefficients();
        Ok(())
    }

    fn process(&mut self, mut output: &mut [f32], _time: &EffectTime) {
        debug_assert!(self.channel_count == 2);

        let threshold = self.threshold.value();
        let range_db = self.range.value();
        let hold_samples = (self.hold_time.value() * self.sample_rate as f32) as u32;

        for frame in output.as_frames_mut::<2>() {
            // Peak detection in dB
            let frame_peak = frame[0].abs().max(frame[1].abs());
            let input_db = if frame_peak > 1e-6 {
                20.0 * frame_peak.log10()
            } else {
                -120.0
            };

            // Envelope detection (smoothed level)
            let envelope = self.envelope_follower.run(input_db);

            // Gate state: open / hold / closed
            let target_gain_db = if envelope >= threshold {
                self.hold_counter = hold_samples;
                0.0 // gate open
            } else if self.hold_counter > 0 {
                self.hold_counter -= 1;
                0.0 // hold phase
            } else {
                range_db // gate closed
            };

            // Smooth gate gain with attack/release coefficients
            self.gate_gain_db = if target_gain_db > self.gate_gain_db {
                // Opening: attack
                self.attack_coeff * self.gate_gain_db + (1.0 - self.attack_coeff) * target_gain_db
            } else {
                // Closing: release
                self.release_coeff * self.gate_gain_db + (1.0 - self.release_coeff) * target_gain_db
            };

            // Apply gain
            let gain = if self.gate_gain_db <= -60.0 {
                0.0
            } else {
                db_to_linear(self.gate_gain_db)
            };
            frame[0] *= gain;
            frame[1] *= gain;
        }
    }

    fn process_tail(&self) -> Option<usize> {
        let hold_samples = (self.hold_time.value() * self.sample_rate as f32).ceil() as usize;
        let release_samples = (self.release_time.value() * self.sample_rate as f32).ceil() as usize;
        Some(hold_samples + release_samples)
    }

    fn process_parameter_update(
        &mut self,
        id: FourCC,
        value: &ParameterValueUpdate,
    ) -> Result<(), Error> {
        match id {
            _ if id == Self::THRESHOLD.id() => self.threshold.apply_update(value),
            _ if id == Self::ATTACK_TIME.id() => self.attack_time.apply_update(value),
            _ if id == Self::HOLD_TIME.id() => self.hold_time.apply_update(value),
            _ if id == Self::RELEASE_TIME.id() => self.release_time.apply_update(value),
            _ if id == Self::RANGE.id() => self.range.apply_update(value),
            _ => {
                return Err(Error::ParameterError(format!(
                    "Unknown parameter: '{id}' for effect '{}'",
                    self.name()
                )))
            }
        }
        self.update_coefficients();
        Ok(())
    }
}
