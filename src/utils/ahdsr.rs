//! Classic AHDSR envelope with optional curve scaling factors for attack, decay and release.

use std::time::Duration;

use crate::Error;

// -------------------------------------------------------------------------------------------------

/// Current processing stage in a [`AhdsrEnvelope`].
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub enum AhdsrStage {
    #[default]
    /// Before attack and after release (zero volume).
    Idle,
    Attack,
    Hold,
    Decay,
    Sustain,
    Release,
}

// -------------------------------------------------------------------------------------------------

/// AHDSR envelope parameters that define the envelope shape for a [`AhdsrEnvelope`].
#[derive(Clone)]
pub struct AhdsrParameters {
    sample_rate: u32,
    attack_time: Duration,
    attack_scaling: f32,
    attack_rate: f32,
    hold_time: Duration,
    decay_time: Duration,
    decay_scaling: f32,
    decay_rate: f32,
    sustain_level: f32,
    release_time: Duration,
    release_scaling: f32,
    release_rate: f32,
}

impl AhdsrParameters {
    const UNINITIALIZED_SAMPLE_RATE: u32 = 66666;
    const EULER_DIV_2: f32 = std::f32::consts::E / 2.0;

    /// Create new AHDSR parameters with sustain level and attack, hold, decay, and release time
    /// durations without scaling. See [`Self::setup`] for parameter info.
    ///
    /// Note that by default no valid sample rate is set. When using the parameters within a
    /// [`AhdsrEnvelope`], make sure you set a valid rate before calling process.
    pub fn new(
        attack_time: Duration,
        hold_time: Duration,
        decay_time: Duration,
        sustain_level: f32,
        release_time: Duration,
    ) -> Result<Self, Error> {
        let mut parameters = Self::zeroed();
        parameters.sample_rate = Self::UNINITIALIZED_SAMPLE_RATE;
        parameters.setup(
            attack_time,
            hold_time,
            decay_time,
            sustain_level,
            release_time,
        )?;
        Ok(parameters)
    }

    /// Create new AHDSR parameters with sustain level and attack, hold, decay, release time
    /// durations and  scaling factors. See [`Self::setup_with_scaling`] for parameter info.
    ///
    /// Note that by default no valid sample rate is set. When using the parameters within a
    /// [`AhdsrEnvelope`], make sure you set a valid rate before calling process.
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_scaling(
        attack_time: Duration,
        attack_scaling: f32,
        hold_time: Duration,
        decay_time: Duration,
        decay_scaling: f32,
        sustain_level: f32,
        release_time: Duration,
        release_scaling: f32,
    ) -> Result<Self, Error> {
        let mut parameters = Self::zeroed();
        parameters.sample_rate = Self::UNINITIALIZED_SAMPLE_RATE;
        parameters.setup_with_scaling(
            attack_time,
            attack_scaling,
            hold_time,
            decay_time,
            decay_scaling,
            sustain_level,
            release_time,
            release_scaling,
        )?;
        Ok(parameters)
    }

    fn zeroed() -> Self {
        Self {
            sample_rate: 0,
            attack_time: Duration::ZERO,
            attack_scaling: 0.0,
            attack_rate: 0.0,
            hold_time: Duration::ZERO,
            decay_time: Duration::ZERO,
            decay_scaling: 0.0,
            decay_rate: 0.0,
            sustain_level: 0.0,
            release_time: Duration::ZERO,
            release_scaling: 0.0,
            release_rate: 0.0,
        }
    }

    /// Get currently applied sample rate.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Set a new sample rate and recalculate internal rates if needed.
    pub fn set_sample_rate(&mut self, sample_rate: u32) -> Result<(), Error> {
        if self.sample_rate != sample_rate {
            self.sample_rate = sample_rate;
            self.setup(
                self.attack_time,
                self.hold_time,
                self.decay_time,
                self.sustain_level,
                self.release_time,
            )
        } else {
            Ok(())
        }
    }

    /// Get the sustain level.
    pub fn sustain_level(&self) -> f32 {
        self.sustain_level
    }
    /// Set the sustain level.
    pub fn set_sustain_level(&mut self, level: f32) -> Result<(), Error> {
        if !(0.0..=1.0).contains(&level) {
            return Err(Error::ParameterError(format!(
                "Invalid sustain level: {}. Must be in range [0.0, 1.0]",
                level
            )));
        }
        self.sustain_level = level;
        Ok(())
    }

    /// Get attack time duration.
    pub fn attack_time(&self) -> Duration {
        self.attack_time
    }
    /// Set the attack rate based on a time duration. Attack can be zero
    /// to completely skip the attack phase.
    pub fn set_attack_time(&mut self, time: Duration) -> Result<(), Error> {
        self.attack_time = time;
        let time_secs = time.as_secs_f32();
        if time_secs == 0.0 {
            self.attack_rate = f32::MAX;
        } else {
            self.attack_rate = 1.0 / (time_secs * self.sample_rate as f32);
        }
        Ok(())
    }

    /// Get attack scaling.
    pub fn attack_scaling(&self) -> f32 {
        self.attack_scaling
    }
    /// Set the attack scaling factor.
    ///
    /// Scaling should be in range [-1.0, 1.0].
    /// 0.0 = linear, positive = logarithmic (fast start), negative = exponential (slow start).
    pub fn set_attack_scaling(&mut self, scaling: f32) -> Result<(), Error> {
        if !(-1.0..=1.0).contains(&scaling) {
            return Err(Error::ParameterError(format!(
                "Invalid attack scaling: {}. Must be in range [-1.0, 1.0]",
                scaling
            )));
        }
        self.attack_scaling = scaling;
        Ok(())
    }

    /// Get hold time duration.
    pub fn hold_time(&self) -> Duration {
        self.hold_time
    }
    /// Set the hold time duration.
    pub fn set_hold_time(&mut self, time: Duration) -> Result<(), Error> {
        self.hold_time = time;
        Ok(())
    }

    /// Get decay time duration.
    pub fn decay_time(&self) -> Duration {
        self.decay_time
    }
    /// Set the decay rate based on a time duration.
    pub fn set_decay_time(&mut self, time: Duration) -> Result<(), Error> {
        self.decay_time = time;
        if time.is_zero() {
            self.decay_rate = f32::MAX;
        } else {
            self.decay_rate =
                (1.0 - self.sustain_level) / (time.as_secs_f32() * self.sample_rate as f32);
        }
        Ok(())
    }

    /// Get decay scaling.
    pub fn decay_scaling(&self) -> f32 {
        self.decay_scaling
    }
    /// Set the decay scaling factor.
    ///
    /// Scaling should be in range [-1.0, 1.0].
    /// 0.0 = linear, positive = logarithmic (fast start), negative = exponential (slow start).
    pub fn set_decay_scaling(&mut self, scaling: f32) -> Result<(), Error> {
        if !(-1.0..=1.0).contains(&scaling) {
            return Err(Error::ParameterError(format!(
                "Invalid decay scaling: {}. Must be in range [-1.0, 1.0]",
                scaling
            )));
        }
        self.decay_scaling = scaling;
        Ok(())
    }

    /// Get release time duration.
    pub fn release_time(&self) -> Duration {
        self.release_time
    }
    /// Set the release rate based on a time duration.
    pub fn set_release_time(&mut self, time: Duration) -> Result<(), Error> {
        self.release_time = time;
        let time_secs = time.as_secs_f32();
        if time_secs == 0.0 {
            self.release_rate = f32::MAX;
        } else {
            self.release_rate = 1.0 / (time_secs * self.sample_rate as f32);
        }
        Ok(())
    }

    /// Get release scaling.
    pub fn release_scaling(&self) -> f32 {
        self.release_scaling
    }
    /// Set the release scaling factor.
    ///
    /// Scaling should be in range [-1.0, 1.0].
    /// 0.0 = linear, positive = logarithmic (fast start), negative = exponential (slow start).
    pub fn set_release_scaling(&mut self, scaling: f32) -> Result<(), Error> {
        if !(-1.0..=1.0).contains(&scaling) {
            return Err(Error::ParameterError(format!(
                "Invalid release scaling: {}. Must be in range [-1.0, 1.0]",
                scaling
            )));
        }
        self.release_scaling = scaling;
        Ok(())
    }

    /// Set sustain level, attack, hold, decay, and release time durations without scaling.
    ///
    /// sustain_level is in range [0.0, 1.0].
    pub fn setup(
        &mut self,
        attack_time: Duration,
        hold_time: Duration,
        decay_time: Duration,
        sustain_level: f32,
        release_time: Duration,
    ) -> Result<(), Error> {
        self.set_attack_time(attack_time)?;
        self.set_hold_time(hold_time)?;
        self.set_decay_time(decay_time)?;
        self.set_sustain_level(sustain_level)?;
        self.set_release_time(release_time)?;
        Ok(())
    }

    /// Set sustain level, attack, hold, decay, and release time durations with scaling.
    ///
    /// sustain_level is in range [0.0, 1.0], scaling is in range [-1.0 1.0].
    #[allow(clippy::too_many_arguments)]
    pub fn setup_with_scaling(
        &mut self,
        attack_time: Duration,
        attack_scaling: f32,
        hold_time: Duration,
        decay_time: Duration,
        decay_scaling: f32,
        sustain_level: f32,
        release_time: Duration,
        release_scaling: f32,
    ) -> Result<(), Error> {
        self.set_attack_time(attack_time)?;
        self.set_attack_scaling(attack_scaling)?;
        self.set_hold_time(hold_time)?;
        self.set_decay_time(decay_time)?;
        self.set_decay_scaling(decay_scaling)?;
        self.set_sustain_level(sustain_level)?;
        self.set_release_time(release_time)?;
        self.set_release_scaling(release_scaling)?;
        Ok(())
    }

    /// Apply scaling on a normalized target value.
    ///
    /// Value should be in range [0.0, 1.0].
    /// Scaling should be in range [-1.0, 1.0].
    ///
    /// Scaling is logarithmic (with positive values) or exponential with (negative values)
    /// using the following shape (Wolfram alpha notation):
    /// `plot x^(1 + (-s)^(e/2) * 16) for x=0 to 1, s=-1 to 1`
    #[inline]
    fn apply_scaling(value: f32, scaling: f32) -> f32 {
        debug_assert!(
            (0.0..=1.0).contains(&value),
            "Value must be in range [0.0, 1.0]"
        );
        debug_assert!(
            (-1.0..=1.0).contains(&scaling),
            "Scaling must be in range [-1.0, 1.0]"
        );
        if scaling == 0.0 || value == 0.0 {
            // linear or zero
            value
        } else {
            // pow'ed
            let scaling = -scaling;
            if scaling > 0.0 {
                value.powf(1.0 + scaling.powf(Self::EULER_DIV_2) * 16.0)
            } else {
                1.0 - (1.0 - value).powf(1.0 + (-scaling).powf(Self::EULER_DIV_2) * 16.0)
            }
        }
    }
}

impl Default for AhdsrParameters {
    fn default() -> Self {
        Self::new(
            Duration::from_millis(10),
            Duration::from_secs(1),
            Duration::from_millis(500),
            0.75,
            Duration::from_secs(1),
        )
        .expect("Default AHDSR parameters should be valid")
    }
}

// -------------------------------------------------------------------------------------------------

/// Classic AHDSR envelope with externally defined parameter state.
///
/// Parameters are defined in an external struct which must be passed to the process function.
#[derive(Default, Clone)]
pub struct AhdsrEnvelope {
    stage: AhdsrStage,
    target_volume: f32,
    hold_samples_remaining: f32,
    release_output: f32,
    output: f32,
}

impl AhdsrEnvelope {
    const SILENCE: f32 = 0.001; // -60dB

    /// Create a new AHDSR envelope with default state.
    pub fn new() -> Self {
        Self {
            stage: AhdsrStage::Idle,
            target_volume: 0.0,
            hold_samples_remaining: 0.0,
            release_output: 0.0,
            output: 0.0,
        }
    }

    /// Return the envelope's current stage.
    #[inline(always)]
    pub fn stage(&self) -> AhdsrStage {
        self.stage
    }

    /// Return the envelope's current (last processed) output value.
    #[inline(always)]
    pub fn output(&self) -> f32 {
        self.output
    }

    /// Sets target volume from the given velocity volume and state to Attack.
    pub fn note_on(&mut self, parameters: &AhdsrParameters, volume: f32) {
        self.target_volume = volume;

        if parameters.attack_rate == f32::MAX {
            // Skip attack, go to hold or decay
            self.output = volume;
            if parameters.hold_time > Duration::ZERO {
                self.stage = AhdsrStage::Hold;
                self.hold_samples_remaining =
                    parameters.hold_time.as_secs_f32() * parameters.sample_rate as f32;
            } else {
                self.stage = AhdsrStage::Decay;
            }
        } else {
            self.output = 0.0;
            self.stage = AhdsrStage::Attack;
        }
    }

    /// Set target volume to 0 and state to Release.
    pub fn note_off(&mut self, parameters: &AhdsrParameters) {
        if parameters.release_time > Duration::ZERO {
            self.target_volume = 0.0;
            self.release_output = self.output;
            if self.release_output > f32::EPSILON {
                self.stage = AhdsrStage::Release;
            } else {
                self.stage = AhdsrStage::Idle;
            }
        } else {
            self.output = 0.0;
            self.release_output = 0.0;
            self.stage = AhdsrStage::Idle;
        }
    }

    /// Immediately stop the voice and set state to Idle.
    #[allow(dead_code)]
    pub fn reset(&mut self) {
        self.output = 0.0;
        self.stage = AhdsrStage::Idle;
    }

    /// Compute and return one output sample. Will return 0.0 and do nothing
    /// at all in Idle stage.
    #[inline]
    pub fn run(&mut self, parameters: &AhdsrParameters) -> f32 {
        debug_assert!(
            parameters.sample_rate != AhdsrParameters::UNINITIALIZED_SAMPLE_RATE,
            "Set a valid sample rate in ahdsr parameters before processing!"
        );

        match self.stage {
            AhdsrStage::Attack => {
                self.output += parameters.attack_rate;
                if self.output >= self.target_volume {
                    self.output = self.target_volume;
                    self.target_volume = parameters.sustain_level;
                    // After attack, go to hold or decay
                    if !parameters.hold_time.is_zero() {
                        self.stage = AhdsrStage::Hold;
                        self.hold_samples_remaining =
                            parameters.hold_time.as_secs_f32() * parameters.sample_rate as f32;
                    } else {
                        self.stage = AhdsrStage::Decay;
                    }
                }
            }

            AhdsrStage::Hold => {
                self.hold_samples_remaining -= 1.0;
                if self.hold_samples_remaining <= 0.0 {
                    if parameters.decay_time.is_zero() {
                        self.stage = AhdsrStage::Sustain;
                    } else {
                        self.stage = AhdsrStage::Decay;
                    }
                }
                // Output stays at target_volume during hold
            }

            AhdsrStage::Decay => {
                if self.output > parameters.sustain_level {
                    self.output -= parameters.decay_rate;
                    if self.output <= parameters.sustain_level {
                        self.output = parameters.sustain_level;
                        self.stage = AhdsrStage::Sustain;
                    }
                } else {
                    // attack target < sustain level
                    self.output += parameters.decay_rate;
                    if self.output >= parameters.sustain_level {
                        self.output = parameters.sustain_level;
                        self.stage = AhdsrStage::Sustain;
                    }
                }
            }

            AhdsrStage::Sustain => {
                // nothing to do (waiting for release trigger)
            }

            AhdsrStage::Release => {
                // Apply release level dynamically based on output at note_off time
                self.output -= self.release_output * parameters.release_rate;
                if self.output <= Self::SILENCE {
                    self.output = 0.0;
                    self.stage = AhdsrStage::Idle;
                }
            }

            AhdsrStage::Idle => {
                // nothing to do
            }
        }

        // Apply scaling based on current stage (Hold stage uses no scaling)
        match self.stage {
            AhdsrStage::Attack if parameters.attack_scaling != 0.0 => {
                let progress = self.output / self.target_volume.max(f32::EPSILON);
                let scaled_progress =
                    AhdsrParameters::apply_scaling(progress, parameters.attack_scaling);
                scaled_progress * self.target_volume
            }
            AhdsrStage::Decay if parameters.decay_scaling != 0.0 => {
                let range = (self.target_volume - parameters.sustain_level)
                    .abs()
                    .max(f32::EPSILON);
                let progress = if self.target_volume > parameters.sustain_level {
                    (self.target_volume - self.output) / range
                } else {
                    (self.output - self.target_volume) / range
                };
                let scaled_progress =
                    AhdsrParameters::apply_scaling(progress, parameters.decay_scaling);
                if self.target_volume > parameters.sustain_level {
                    self.target_volume - (scaled_progress * range)
                } else {
                    self.target_volume + (scaled_progress * range)
                }
            }
            AhdsrStage::Release if parameters.release_scaling != 0.0 => {
                let initial_release_level = self.output.max(f32::EPSILON);
                let progress = 1.0 - (self.output / initial_release_level);
                let scaled_progress =
                    AhdsrParameters::apply_scaling(progress, parameters.release_scaling);
                initial_release_level * (1.0 - scaled_progress)
            }
            _ => self.output,
        }
    }

    /// Process a buffer of samples, writing envelope values to output.
    /// This can be more efficient than calling `process()` per sample, especially
    /// in idle and sustain stages.
    #[inline]
    pub fn process(&mut self, parameters: &AhdsrParameters, output: &mut [f32]) {
        debug_assert!(
            parameters.sample_rate != AhdsrParameters::UNINITIALIZED_SAMPLE_RATE,
            "Set a valid sample rate in ahdsr parameters before processing!"
        );
        match self.stage {
            AhdsrStage::Idle => {
                // If we're in idle the output is empty
                output.fill(0.0);
            }
            AhdsrStage::Sustain => {
                // If we're in sustain stage, we can fill the buffer with a constant value
                output.fill(self.output);
            }
            _ => {
                // General case: process sample by sample
                for sample in output.iter_mut() {
                    *sample = self.run(parameters);
                }
            }
        }
    }
}

// -------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_construction() {
        let parameters = AhdsrParameters::default();
        let env = AhdsrEnvelope::new();
        assert_eq!(env.stage(), AhdsrStage::Idle);
        assert_eq!(
            parameters.sample_rate(),
            AhdsrParameters::UNINITIALIZED_SAMPLE_RATE
        );
    }

    #[test]
    fn test_note_on_triggers_attack() -> Result<(), Box<Error>> {
        let mut parameters = AhdsrParameters::default();
        parameters.set_sample_rate(44100)?;
        parameters.setup(
            Duration::from_millis(100),
            Duration::ZERO,
            Duration::from_millis(100),
            0.5,
            Duration::from_millis(100),
        )?;
        let mut env = AhdsrEnvelope::new();
        env.note_on(&parameters, 1.0);
        assert_eq!(env.stage(), AhdsrStage::Attack);
        Ok(())
    }

    #[test]
    fn test_note_off_triggers_release() -> Result<(), Box<Error>> {
        let mut parameters = AhdsrParameters::default();
        parameters.set_sample_rate(44100)?;
        parameters.setup(
            Duration::from_millis(100),
            Duration::ZERO,
            Duration::from_millis(100),
            0.5,
            Duration::from_millis(100),
        )?;
        let mut env = AhdsrEnvelope::new();
        env.note_on(&parameters, 1.0);
        let _ = env.run(&parameters);
        env.note_off(&parameters);
        assert_eq!(env.stage(), AhdsrStage::Release);
        Ok(())
    }

    #[test]
    #[should_panic]
    fn test_process_no_sample_rate() {
        let parameters = AhdsrParameters::default();
        let mut env = AhdsrEnvelope::new();
        env.run(&parameters); // no valid SR set
    }

    #[test]
    fn test_reset_goes_to_idle() {
        let parameters = AhdsrParameters::default();
        let mut env = AhdsrEnvelope::new();
        env.note_on(&parameters, 1.0);
        env.reset();
        assert_eq!(env.stage(), AhdsrStage::Idle);
    }

    #[test]
    fn test_scaling() {
        let result = AhdsrParameters::apply_scaling(0.5, 0.0);
        assert!((result - 0.5).abs() < 1e-10);

        // Positive scaling should give logarithmic curve (fast start)
        let result = AhdsrParameters::apply_scaling(0.5, 0.5);
        assert!(result > 0.5); // Should be less than linear at midpoint

        // Negative scaling should give exponential curve (slow start)
        let result = AhdsrParameters::apply_scaling(0.5, -0.5);
        assert!(result < 0.5); // Should be more than linear at midpoint
    }
}
