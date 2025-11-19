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
    attack_scaling: f64,
    attack_rate: f64,
    hold_time: Duration,
    decay_time: Duration,
    decay_scaling: f64,
    decay_rate: f64,
    sustain_level: f64,
    release_time: Duration,
    release_scaling: f64,
    release_rate: f64,
}

impl AhdsrParameters {
    const UNINITIALIZED_SAMPLE_RATE: u32 = 66666;
    const EULER_DIV_2: f64 = std::f64::consts::E / 2.0;

    /// Create new AHDSR parameters with sustain level and attack, hold, decay, and release time
    /// durations without scaling. See [`Self::setup`] for parameter info.
    ///
    /// Note that by default no valid sample rate is set. When using the params within a
    /// [`AhdsrEnvelope`], make sure you set a valid rate before calling process.
    pub fn new(
        attack_time: Duration,
        hold_time: Duration,
        decay_time: Duration,
        sustain_level: f64,
        release_time: Duration,
    ) -> Result<Self, Error> {
        let mut params = Self::zeroed();
        params.sample_rate = Self::UNINITIALIZED_SAMPLE_RATE;
        params.setup(
            attack_time,
            hold_time,
            decay_time,
            sustain_level,
            release_time,
        )?;
        Ok(params)
    }

    /// Create new AHDSR parameters with sustain level and attack, hold, decay, release time
    /// durations and  scaling factors. See [`Self::setup_with_scaling`] for parameter info.
    ///
    /// Note that by default no valid sample rate is set. When using the params within a
    /// [`AhdsrEnvelope`], make sure you set a valid rate before calling process.
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_scaling(
        attack_time: Duration,
        attack_scaling: f64,
        hold_time: Duration,
        decay_time: Duration,
        decay_scaling: f64,
        sustain_level: f64,
        release_time: Duration,
        release_scaling: f64,
    ) -> Result<Self, Error> {
        let mut params = Self::zeroed();
        params.sample_rate = Self::UNINITIALIZED_SAMPLE_RATE;
        params.setup_with_scaling(
            attack_time,
            attack_scaling,
            hold_time,
            decay_time,
            decay_scaling,
            sustain_level,
            release_time,
            release_scaling,
        )?;
        Ok(params)
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

    /// Set the sustain level.
    pub fn set_sustain_level(&mut self, level: f64) -> Result<(), Error> {
        if !(0.0..=1.0).contains(&level) {
            return Err(Error::ParameterError(format!(
                "Invalid sustain level: {}. Must be in range [0.0, 1.0]",
                level
            )));
        }
        self.sustain_level = level;
        Ok(())
    }

    /// Set the attack rate based on a time duration. Attack can be zero
    /// to completely skip the attack phase.
    pub fn set_attack_time(&mut self, time: Duration) -> Result<(), Error> {
        self.attack_time = time;
        let time_secs = time.as_secs_f64();
        if time_secs == 0.0 {
            self.attack_rate = f64::MAX;
        } else {
            self.attack_rate = 1.0 / (time_secs * self.sample_rate as f64);
        }
        Ok(())
    }

    /// Set the attack scaling factor.
    ///
    /// Scaling should be in range [-1.0, 1.0].
    /// 0.0 = linear, positive = logarithmic (fast start), negative = exponential (slow start).
    pub fn set_attack_scaling(&mut self, scaling: f64) -> Result<(), Error> {
        if !(-1.0..=1.0).contains(&scaling) {
            return Err(Error::ParameterError(format!(
                "Invalid attack scaling: {}. Must be in range [-1.0, 1.0]",
                scaling
            )));
        }
        self.attack_scaling = scaling;
        Ok(())
    }

    /// Set the hold time duration.
    pub fn set_hold_time(&mut self, time: Duration) -> Result<(), Error> {
        self.hold_time = time;
        Ok(())
    }

    /// Set the decay rate based on a time duration.
    /// Duration must be strictly greater than zero.
    pub fn set_decay_time(&mut self, time: Duration) -> Result<(), Error> {
        let time_secs = time.as_secs_f64();
        if time_secs == 0.0 {
            return Err(Error::ParameterError(format!(
                "Invalid decay time: {:?}. Must be > 0",
                time
            )));
        }
        self.decay_time = time;
        self.decay_rate = (1.0 - self.sustain_level) / (time_secs * self.sample_rate as f64);
        Ok(())
    }

    /// Set the decay scaling factor.
    ///
    /// Scaling should be in range [-1.0, 1.0].
    /// 0.0 = linear, positive = logarithmic (fast start), negative = exponential (slow start).
    pub fn set_decay_scaling(&mut self, scaling: f64) -> Result<(), Error> {
        if !(-1.0..=1.0).contains(&scaling) {
            return Err(Error::ParameterError(format!(
                "Invalid decay scaling: {}. Must be in range [-1.0, 1.0]",
                scaling
            )));
        }
        self.decay_scaling = scaling;
        Ok(())
    }

    /// Set the release rate based on a time duration.
    pub fn set_release_time(&mut self, time: Duration) -> Result<(), Error> {
        self.release_time = time;
        let time_secs = time.as_secs_f64();
        if time_secs == 0.0 {
            self.release_rate = f64::MAX;
        } else {
            self.release_rate = self.sustain_level / (time_secs * self.sample_rate as f64);
        }
        Ok(())
    }

    /// Set the release scaling factor.
    ///
    /// Scaling should be in range [-1.0, 1.0].
    /// 0.0 = linear, positive = logarithmic (fast start), negative = exponential (slow start).
    pub fn set_release_scaling(&mut self, scaling: f64) -> Result<(), Error> {
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
        sustain_level: f64,
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
        attack_scaling: f64,
        hold_time: Duration,
        decay_time: Duration,
        decay_scaling: f64,
        sustain_level: f64,
        release_time: Duration,
        release_scaling: f64,
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
    fn apply_scaling(value: f64, scaling: f64) -> f64 {
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
    target_volume: f64,
    hold_samples_remaining: f64,
    release_output: f64,
    output: f64,
}

impl AhdsrEnvelope {
    const SILENCE: f64 = 0.001; // -60dB

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

    /// Sets target volume from the given velocity volume and state to Attack.
    pub fn note_on(&mut self, params: &AhdsrParameters, volume: f64) {
        self.target_volume = volume;

        if params.attack_rate == f64::MAX {
            // Skip attack, go to hold or decay
            self.output = volume;
            if params.hold_time > Duration::ZERO {
                self.stage = AhdsrStage::Hold;
                self.hold_samples_remaining =
                    params.hold_time.as_secs_f64() * params.sample_rate as f64;
            } else {
                self.stage = AhdsrStage::Decay;
            }
        } else {
            self.output = 0.0;
            self.stage = AhdsrStage::Attack;
        }
    }

    /// Set target volume to 0 and state to Release.
    pub fn note_off(&mut self, params: &AhdsrParameters) {
        if params.release_time > Duration::ZERO {
            self.target_volume = 0.0;
            self.release_output = self.output;
            self.stage = AhdsrStage::Release;
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
    pub fn process(&mut self, params: &AhdsrParameters) -> f64 {
        debug_assert!(
            params.sample_rate != AhdsrParameters::UNINITIALIZED_SAMPLE_RATE,
            "Set a valid sample rate in ahdsr parameters before processing!"
        );

        match self.stage {
            AhdsrStage::Attack => {
                self.output += params.attack_rate;
                if self.output >= self.target_volume {
                    self.output = self.target_volume;
                    self.target_volume = params.sustain_level;
                    // After attack, go to hold or decay
                    if !params.hold_time.is_zero() {
                        self.stage = AhdsrStage::Hold;
                        self.hold_samples_remaining =
                            params.hold_time.as_secs_f64() * params.sample_rate as f64;
                    } else {
                        self.stage = AhdsrStage::Decay;
                    }
                }
            }

            AhdsrStage::Hold => {
                self.hold_samples_remaining -= 1.0;
                if self.hold_samples_remaining <= 0.0 {
                    self.stage = AhdsrStage::Decay;
                }
                // Output stays at target_volume during hold
            }

            AhdsrStage::Decay => {
                if self.output > params.sustain_level {
                    self.output -= params.decay_rate;
                    if self.output <= params.sustain_level {
                        self.output = params.sustain_level;
                        self.stage = AhdsrStage::Sustain;
                    }
                } else {
                    // attack target < sustain level
                    self.output += params.decay_rate;
                    if self.output >= params.sustain_level {
                        self.output = params.sustain_level;
                        self.stage = AhdsrStage::Sustain;
                    }
                }
            }

            AhdsrStage::Sustain => {
                // nothing to do (waiting for release trigger)
            }

            AhdsrStage::Release => {
                // Apply release level dynamically based on output at note_off time
                self.output -= self.release_output / params.sustain_level * params.release_rate;
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
            AhdsrStage::Attack if params.attack_scaling != 0.0 => {
                let progress = self.output / self.target_volume.max(f64::EPSILON);
                let scaled_progress =
                    AhdsrParameters::apply_scaling(progress, params.attack_scaling);
                scaled_progress * self.target_volume
            }
            AhdsrStage::Decay if params.decay_scaling != 0.0 => {
                let range = (self.target_volume - params.sustain_level)
                    .abs()
                    .max(f64::EPSILON);
                let progress = if self.target_volume > params.sustain_level {
                    (self.target_volume - self.output) / range
                } else {
                    (self.output - self.target_volume) / range
                };
                let scaled_progress =
                    AhdsrParameters::apply_scaling(progress, params.decay_scaling);
                if self.target_volume > params.sustain_level {
                    self.target_volume - (scaled_progress * range)
                } else {
                    self.target_volume + (scaled_progress * range)
                }
            }
            AhdsrStage::Release if params.release_scaling != 0.0 => {
                let initial_release_level = self.output.max(f64::EPSILON);
                let progress = 1.0 - (self.output / initial_release_level);
                let scaled_progress =
                    AhdsrParameters::apply_scaling(progress, params.release_scaling);
                initial_release_level * (1.0 - scaled_progress)
            }
            _ => self.output,
        }
    }
}

// -------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_construction() {
        let params = AhdsrParameters::default();
        let env = AhdsrEnvelope::new();
        assert_eq!(env.stage(), AhdsrStage::Idle);
        assert_eq!(
            params.sample_rate(),
            AhdsrParameters::UNINITIALIZED_SAMPLE_RATE
        );
    }

    #[test]
    fn test_note_on_triggers_attack() -> Result<(), Box<Error>> {
        let mut params = AhdsrParameters::default();
        params.setup(
            Duration::from_millis(100),
            Duration::ZERO,
            Duration::from_millis(100),
            0.5,
            Duration::from_millis(100),
        )?;
        let mut env = AhdsrEnvelope::new();
        env.note_on(&params, 1.0);
        assert_eq!(env.stage(), AhdsrStage::Attack);
        Ok(())
    }

    #[test]
    fn test_note_off_triggers_release() -> Result<(), Box<Error>> {
        let mut params = AhdsrParameters::default();
        params.setup(
            Duration::from_millis(100),
            Duration::ZERO,
            Duration::from_millis(100),
            0.5,
            Duration::from_millis(100),
        )?;
        let mut env = AhdsrEnvelope::new();
        env.note_on(&params, 1.0);
        env.note_off(&params);
        assert_eq!(env.stage(), AhdsrStage::Release);
        Ok(())
    }

    #[test]
    #[should_panic]
    fn test_process_no_sample_rate() {
        let params = AhdsrParameters::default();
        let mut env = AhdsrEnvelope::new();
        env.process(&params); // no valid SR set
    }

    #[test]
    fn test_reset_goes_to_idle() {
        let params = AhdsrParameters::default();
        let mut env = AhdsrEnvelope::new();
        env.note_on(&params, 1.0);
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
