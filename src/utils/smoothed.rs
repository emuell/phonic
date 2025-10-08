use std::fmt::{Debug, Display};

use crate::utils::{buffer::scale_buffer, panning_factors};

// -------------------------------------------------------------------------------------------------

/// Provides smooth transitions between a current and target f32 value.
/// Smoothing usually needs to be applied to avoid clicks in e.g. volume or other DSP parameter changes.
pub trait SmoothedValue: Debug {
    /// Access to the current, possibly ramped value.
    #[must_use]
    fn current(&self) -> f32;
    /// Access to the target value.
    #[must_use]
    fn target(&self) -> f32;

    /// Ramp, if needed, and get the current ramped value, else returns the target value.
    #[must_use]
    fn next(&mut self) -> f32 {
        if self.need_ramp() {
            self.ramp();
            self.current()
        } else {
            self.target()
        }
    }

    /// Test if ramping is necessary. When ramping is not necessary, parameter changes
    /// may be applied in blocks without calling `next` or `ramp`, which usually is faster.
    #[must_use]
    fn need_ramp(&self) -> bool;
    /// Move current to target value, when ramping is necessary, else does nothing.
    fn ramp(&mut self);

    /// Set current and target to the same value.
    fn init(&mut self, amount: f32);
    /// Set a new target value and ramp current, when current is different from the target.
    fn set_target(&mut self, target: f32);

    /// Update sample rate of the smoothed value. Smoothed values are expected to be called
    /// once per audio frame and the ramping scales with the sample rate.
    fn set_sample_rate(&mut self, sample_rate: u32);
}

impl Display for dyn SmoothedValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.need_ramp() {
            f.write_fmt(format_args!("{}(->{})", self.current(), self.target()))
        } else {
            f.write_fmt(format_args!("{})", self.target()))
        }
    }
}

// -------------------------------------------------------------------------------------------------

/// Apply a smoothed volume value to a buffer,
pub fn apply_smoothed_gain(buffer: &mut [f32], smoothed: &mut impl SmoothedValue) {
    if smoothed.need_ramp() {
        for s in buffer.iter_mut() {
            *s *= smoothed.next();
        }
    } else {
        let gain = smoothed.target();
        if (1.0 - gain).abs() > 0.000001 {
            scale_buffer(buffer, gain);
        }
    }
}

/// Apply a smoothed panning value to a buffer.
pub fn apply_smoothed_panning(
    buffer: &mut [f32],
    channel_count: usize,
    smoothed: &mut impl SmoothedValue,
) {
    if channel_count >= 2 {
        if smoothed.need_ramp() {
            match channel_count {
                2 => {
                    for frame in buffer.chunks_exact_mut(2) {
                        let (pan_l, pan_r) = panning_factors(smoothed.next());
                        frame[0] *= pan_l;
                        frame[1] *= pan_r;
                    }
                }
                _ => {
                    for frame in buffer.chunks_exact_mut(channel_count) {
                        // TODO: handle multi channel layouts beyond stereo
                        let (pan_l, pan_r) = panning_factors(smoothed.next());
                        frame[0] *= pan_l;
                        frame[1] *= pan_r;
                    }
                }
            }
        } else {
            let pan = smoothed.target();
            if pan.abs() > 0.000001 {
                let (pan_l, pan_r) = panning_factors(pan);
                match channel_count {
                    2 => {
                        for frame in buffer.chunks_exact_mut(2) {
                            frame[0] *= pan_l;
                            frame[1] *= pan_r;
                        }
                    }
                    _ => {
                        for frame in buffer.chunks_exact_mut(channel_count) {
                            // TODO: handle multi channel layouts beyond stereo
                            frame[0] *= pan_l;
                            frame[1] *= pan_r;
                        }
                    }
                }
            }
        }
    } else {
        // can't apply panning to mono signals
    }
}

// -------------------------------------------------------------------------------------------------

/// Exponential smoothed value for smooth ramping, using an inertial exponential approach.
///
/// The value changes gradually towards the target based on the configurable inertia factor.
/// This should be the default smoothed value for volume alike parameters.
#[derive(Debug, Clone)]
pub struct ExponentialSmoothedValue {
    current: f32,
    target: f32,
    inertia: f32,
    sample_rate_comp: f32,
}

impl ExponentialSmoothedValue {
    pub const DEFAULT_INERTIA: f32 = 0.02;

    const UNINITIALIZED_SAMPLE_RATE: u32 = 66666;
    const UNINITIALIZED_SAMPLE_RATE_COMP: f32 = 44100.0 / Self::UNINITIALIZED_SAMPLE_RATE as f32;

    pub const fn new(value: f32, sample_rate: u32) -> Self {
        Self::with_inertia(value, Self::DEFAULT_INERTIA, sample_rate)
    }

    pub const fn with_inertia(value: f32, inertia: f32, sample_rate: u32) -> Self {
        assert!(inertia > 0.0 && inertia <= 1.0, "Invalid inertia");
        assert!(sample_rate > 0, "Invalid sample rate");

        let current = value;
        let target = value;
        let sample_rate_comp = 44100.0 / sample_rate as f32;

        ExponentialSmoothedValue {
            current,
            target,
            inertia,
            sample_rate_comp,
        }
    }

    #[inline(always)]
    pub fn inertia(&self) -> f32 {
        self.inertia
    }

    pub fn set_inertia(&mut self, inertia: f32) {
        assert!(inertia > 0.0 && inertia <= 1.0, "Invalid inertia");
        self.inertia = inertia;
    }

    pub fn reset(&mut self) {
        self.init(self.target);
    }
}

impl SmoothedValue for ExponentialSmoothedValue {
    #[inline(always)]
    fn current(&self) -> f32 {
        self.current
    }

    #[inline(always)]
    fn target(&self) -> f32 {
        self.target
    }

    fn need_ramp(&self) -> bool {
        debug_assert!(
            self.sample_rate_comp != Self::UNINITIALIZED_SAMPLE_RATE_COMP,
            "Call 'set_sample_rate' for default constructed smoothed values before using them!"
        );
        const EPSILON: f32 = f32::EPSILON * 100.0;
        let inertia_add = (self.target - self.current) * self.inertia * self.sample_rate_comp;
        let next = self.current + inertia_add;
        (self.current - next).abs() > EPSILON
    }

    fn ramp(&mut self) {
        debug_assert!(
            self.sample_rate_comp != Self::UNINITIALIZED_SAMPLE_RATE_COMP,
            "Call 'set_sample_rate' for default constructed smoothed values before using them!"
        );
        self.current += (self.target - self.current) * self.inertia * self.sample_rate_comp;
    }

    fn init(&mut self, amount: f32) {
        self.target = amount;
        self.current = amount;
    }

    fn set_target(&mut self, target: f32) {
        self.target = target;
        if !self.need_ramp() {
            self.current = self.target;
        }
    }

    fn set_sample_rate(&mut self, sample_rate: u32) {
        self.sample_rate_comp = 44100.0 / sample_rate as f32;
    }
}

impl Default for ExponentialSmoothedValue {
    fn default() -> Self {
        Self::new(0.0, Self::UNINITIALIZED_SAMPLE_RATE)
    }
}

impl From<f32> for ExponentialSmoothedValue {
    fn from(value: f32) -> Self {
        Self::new(value, Self::UNINITIALIZED_SAMPLE_RATE)
    }
}

// -------------------------------------------------------------------------------------------------

/// Linear smoothed value for ramping linearly towards the target over a specified duration or steps.
/// Provides direct access to step size and duration control.
#[derive(Debug, Clone)]
pub struct LinearSmoothedValue {
    current: f32,
    target: f32,
    step: f32,
    current_step: f32,
    num_pending_steps: u32,
    sample_rate_comp: f32,
}

impl LinearSmoothedValue {
    pub const DEFAULT_STEP: f32 = 0.01;

    const UNINITIALIZED_SAMPLE_RATE: u32 = 66666;
    const UNINITIALIZED_SAMPLE_RATE_COMP: f32 = 44100.0 / Self::UNINITIALIZED_SAMPLE_RATE as f32;

    pub const fn new(value: f32, sample_rate: u32) -> Self {
        Self::with_step(value, Self::DEFAULT_STEP, sample_rate)
    }

    pub const fn with_step(value: f32, step: f32, sample_rate: u32) -> Self {
        assert!(step > 0.0, "Invalid step");
        assert!(sample_rate > 0, "Invalid sample rate");

        let current = value;
        let target = value;
        let current_step = 0.0;
        let num_pending_steps = 0;
        let sample_rate_comp = 44100.0 / sample_rate as f32;

        LinearSmoothedValue {
            current,
            target,
            step,
            current_step,
            num_pending_steps,
            sample_rate_comp,
        }
    }

    #[inline(always)]
    pub fn step(&self) -> f32 {
        self.step
    }

    pub fn set_step(&mut self, step: f32) {
        assert!(step > 0.0, "Invalid step");
        self.step = step;
        self.current_step = if self.current > self.target {
            -self.step * self.sample_rate_comp
        } else {
            self.step * self.sample_rate_comp
        };

        let pending_steps = (self.target - self.current) / self.current_step;
        self.num_pending_steps = pending_steps.round().max(0.0) as u32;

        if self.num_pending_steps == 0 {
            self.current = self.target;
        }
    }

    pub fn set_target_with_duration(&mut self, target: f32, duration_in_samples: Option<u32>) {
        assert!(
            duration_in_samples.is_none_or(|d| d > 0),
            "Invalid duration"
        );

        self.target = target;

        if self.current == self.target {
            self.num_pending_steps = 0;
        } else {
            if let Some(dur_samples) = duration_in_samples {
                self.num_pending_steps = dur_samples;
                self.current_step = (self.target - self.current) / dur_samples as f32;
                self.step = self.current_step.abs();
            } else {
                self.current_step = if self.current > self.target {
                    -self.step * self.sample_rate_comp
                } else {
                    self.step * self.sample_rate_comp
                };

                let pending_steps = (self.target - self.current) / self.current_step;
                self.num_pending_steps = pending_steps.round().max(0.0) as u32;
            }

            if self.num_pending_steps == 0 {
                self.current = self.target;
            }
        }
    }

    pub fn reset(&mut self) {
        self.init(self.target);
    }
}

impl SmoothedValue for LinearSmoothedValue {
    #[inline(always)]
    fn current(&self) -> f32 {
        self.current
    }

    #[inline(always)]
    fn target(&self) -> f32 {
        self.target
    }

    #[inline(always)]
    fn need_ramp(&self) -> bool {
        debug_assert!(
            self.sample_rate_comp != Self::UNINITIALIZED_SAMPLE_RATE_COMP,
            "Call 'set_sample_rate' for default constructed smoothed values before using them!"
        );
        self.num_pending_steps > 0
    }

    fn ramp(&mut self) {
        debug_assert!(
            self.sample_rate_comp != Self::UNINITIALIZED_SAMPLE_RATE_COMP,
            "Call 'set_sample_rate' for default constructed smoothed values before using them!"
        );
        if self.num_pending_steps > 0 {
            self.current += self.current_step;
            self.num_pending_steps -= 1;
            if self.num_pending_steps == 0 {
                self.current = self.target;
            }
        }
    }

    fn init(&mut self, amount: f32) {
        self.target = amount;
        self.current = amount;
        self.num_pending_steps = 0;
    }

    fn set_target(&mut self, target: f32) {
        self.set_target_with_duration(target, None);
    }

    fn set_sample_rate(&mut self, sample_rate: u32) {
        self.sample_rate_comp = 44100.0 / sample_rate as f32;
        self.current_step = if self.current > self.target {
            -self.step * self.sample_rate_comp
        } else {
            self.step * self.sample_rate_comp
        };
    }
}

impl Default for LinearSmoothedValue {
    fn default() -> Self {
        Self::new(0.0, Self::UNINITIALIZED_SAMPLE_RATE)
    }
}

impl From<f32> for LinearSmoothedValue {
    fn from(value: f32) -> Self {
        Self::new(value, Self::UNINITIALIZED_SAMPLE_RATE)
    }
}

// -------------------------------------------------------------------------------------------------

/// Sigmoid smoothed value for ramping using a sigmoid function for smooth, non-linear transitions.
/// Uses an S-shaped curve for acceleration and deceleration, providing control over ramp duration.
#[derive(Debug, Clone)]
pub struct SigmoidSmoothedValue {
    initial: f32,
    current: f32,
    target: f32,
    t: f32,
    step: f32,
    sample_rate_comp: f32,
}

impl SigmoidSmoothedValue {
    pub const DEFAULT_DURATION: usize = 1000;

    const SIGMOID_T_RANGE_MIN: f32 = -5.0;
    const SIGMOID_T_RANGE_MAX: f32 = 5.0;

    const UNINITIALIZED_SAMPLE_RATE: u32 = 66666;
    const UNINITIALIZED_SAMPLE_RATE_COMP: f32 = 44100.0 / Self::UNINITIALIZED_SAMPLE_RATE as f32;

    pub const fn new(value: f32, sample_rate: u32) -> Self {
        Self::with_duration(value, Self::DEFAULT_DURATION, sample_rate)
    }

    pub const fn with_duration(value: f32, duration: usize, sample_rate: u32) -> Self {
        assert!(duration > 0, "Invalid duration");
        assert!(sample_rate > 0, "Invalid sample rate");

        let range = Self::SIGMOID_T_RANGE_MAX - Self::SIGMOID_T_RANGE_MIN;
        let sample_rate_comp = 44100.0 / sample_rate as f32;

        let initial = value;
        let current = value;
        let target = value;

        let t = Self::SIGMOID_T_RANGE_MAX;
        let step = range / duration as f32 * sample_rate_comp;

        SigmoidSmoothedValue {
            initial,
            current,
            target,
            t,
            step,
            sample_rate_comp,
        }
    }

    pub fn duration(&self) -> usize {
        let range = Self::SIGMOID_T_RANGE_MAX - Self::SIGMOID_T_RANGE_MIN;
        (range / self.step) as usize
    }

    pub fn set_duration(&mut self, duration: usize) {
        assert!(duration > 0, "Invalid duration");
        let range = Self::SIGMOID_T_RANGE_MAX - Self::SIGMOID_T_RANGE_MIN;
        self.step = range / (duration as f32 * self.sample_rate_comp);
    }

    pub fn reset(&mut self) {
        self.init(self.target);
    }
}

impl SmoothedValue for SigmoidSmoothedValue {
    #[inline(always)]
    fn current(&self) -> f32 {
        self.current
    }

    #[inline(always)]
    fn target(&self) -> f32 {
        self.target
    }

    #[inline(always)]
    fn need_ramp(&self) -> bool {
        debug_assert!(
            self.sample_rate_comp != Self::UNINITIALIZED_SAMPLE_RATE_COMP,
            "Call 'set_sample_rate' for default constructed smoothed values before using them!"
        );
        self.t < Self::SIGMOID_T_RANGE_MAX
    }

    fn ramp(&mut self) {
        debug_assert!(
            self.sample_rate_comp != Self::UNINITIALIZED_SAMPLE_RATE_COMP,
            "Call 'set_sample_rate' for default constructed smoothed values before using them!"
        );
        if self.t < Self::SIGMOID_T_RANGE_MAX {
            let sigmoid_coeff = 1.0 / (1.0 + (-self.t).exp());
            self.current = self.initial + sigmoid_coeff * (self.target - self.initial);
            self.t += self.step;
        }
    }

    fn init(&mut self, value: f32) {
        self.target = value;
        self.current = value;
        self.initial = value;
        self.t = Self::SIGMOID_T_RANGE_MAX;
    }

    fn set_target(&mut self, value: f32) {
        self.initial = self.current;
        self.target = value;
        if self.current != self.target {
            self.t = Self::SIGMOID_T_RANGE_MIN;
        }
    }

    fn set_sample_rate(&mut self, sample_rate: u32) {
        let duration = self.duration();
        self.sample_rate_comp = 44100.0 / sample_rate as f32;
        self.set_duration(duration);
    }
}

impl Default for SigmoidSmoothedValue {
    fn default() -> Self {
        Self::new(0.0, Self::UNINITIALIZED_SAMPLE_RATE)
    }
}

impl From<f32> for SigmoidSmoothedValue {
    fn from(value: f32) -> Self {
        let mut s = Self::default();
        s.init(value);
        s
    }
}

// -------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exp_smoothed_value() {
        // Test new
        let val = ExponentialSmoothedValue::new(0.0, 44100);
        assert_eq!(val.current(), 0.0);
        assert_eq!(val.target(), 0.0);
        assert_eq!(val.inertia(), ExponentialSmoothedValue::DEFAULT_INERTIA);

        // Test init
        let mut val = ExponentialSmoothedValue::new(0.0, 44100);
        val.init(1.0);
        assert_eq!(val.current(), 1.0);
        assert_eq!(val.target(), 1.0);

        // Test set_target no ramp
        let mut val = ExponentialSmoothedValue::new(0.0, 44100);
        val.set_target(0.0);
        assert_eq!(val.current(), 0.0);
        assert_eq!(val.target(), 0.0);
        assert!(!val.need_ramp());

        // Test set_target with ramp
        let mut val = ExponentialSmoothedValue::new(0.0, 44100);
        val.set_target(1.0);
        assert_eq!(val.target(), 1.0);
        assert!(val.need_ramp());
        val.ramp();
        assert!(val.current() > 0.0);

        // Test multi ramps
        let mut val = ExponentialSmoothedValue::new(0.0, 44100);
        val.set_target(1.0);
        let initial = val.current();
        for _ in 0..10 {
            val.ramp();
        }
        assert!(val.current() > initial);
        assert!(val.current() < val.target());
        assert!(val.need_ramp());

        // Test different inertia
        let mut val1 = ExponentialSmoothedValue::new(0.0, 44100);
        val1.set_inertia(0.1);
        val1.set_target(1.0);
        val1.ramp();
        let current_high_inertia = val1.current();

        let mut val2 = ExponentialSmoothedValue::new(0.0, 44100);
        val2.set_inertia(0.01);
        val2.set_target(1.0);
        val2.ramp();
        let current_low_inertia = val2.current();

        assert!(
            current_high_inertia > current_low_inertia,
            "Higher inertia should approach faster"
        );
    }

    #[test]
    fn test_linear_smoothed_value() {
        // Test new
        let val = LinearSmoothedValue::new(0.0, 44100);
        assert_eq!(val.current(), 0.0);
        assert_eq!(val.target(), 0.0);
        assert_eq!(val.step(), LinearSmoothedValue::DEFAULT_STEP);

        // Test init
        let mut val = LinearSmoothedValue::new(0.0, 44100);
        val.init(1.0);
        assert_eq!(val.current(), 1.0);
        assert_eq!(val.target(), 1.0);
        assert!(!val.need_ramp());

        // Test set_target no ramp
        let mut val = LinearSmoothedValue::new(0.0, 44100);
        val.set_target(0.0);
        assert_eq!(val.current(), 0.0);
        assert_eq!(val.target(), 0.0);
        assert!(!val.need_ramp());

        // Test set_target with duration
        let mut val = LinearSmoothedValue::new(0.0, 44100);
        val.set_target_with_duration(1.0, Some(10));
        assert_eq!(val.target(), 1.0);
        assert!(val.need_ramp());
        val.ramp();
        assert!(val.current() > 0.0);

        // Test reach target
        let mut val = LinearSmoothedValue::new(0.0, 44100);
        val.set_target_with_duration(1.0, Some(5));
        for _ in 0..5 {
            val.ramp();
        }
        assert!(!val.need_ramp());
        assert_eq!(val.current(), 1.0);
        assert_eq!(val.target(), 1.0);

        // Test set_step
        let mut val = LinearSmoothedValue::new(0.0, 44100);
        val.set_step(0.05);
        assert_eq!(val.step(), 0.05);
        val.set_target_with_duration(1.0, None);
        assert!(val.need_ramp());
        let step_val = 0.05;
        let steps_needed = (1.0f32 / step_val).ceil() as u32;
        assert_eq!(val.num_pending_steps, steps_needed);
    }

    #[test]
    fn test_sigmoid_smoothed_value() {
        // Test new
        let val = SigmoidSmoothedValue::new(0.0, 44100);
        assert_eq!(val.current(), 0.0);
        assert_eq!(val.target(), 0.0);

        // Test init
        let mut val = SigmoidSmoothedValue::new(0.0, 44100);
        val.init(1.0);
        assert_eq!(val.current(), 1.0);
        assert_eq!(val.target(), 1.0);
        assert!(!val.need_ramp());

        // Test set_target
        let mut val = SigmoidSmoothedValue::new(0.0, 44100);
        val.set_target(1.0);
        assert_eq!(val.target(), 1.0);
        assert!(val.need_ramp());
        val.ramp();
        assert!(val.current() > 0.0 && val.current() < 1.0);

        // Test ramp pattern
        let mut val = SigmoidSmoothedValue::new(0.0, 44100);
        val.set_target(1.0);
        let default_step = (SigmoidSmoothedValue::SIGMOID_T_RANGE_MAX
            - SigmoidSmoothedValue::SIGMOID_T_RANGE_MIN)
            / SigmoidSmoothedValue::DEFAULT_DURATION as f32;
        let total_ramps = ((SigmoidSmoothedValue::SIGMOID_T_RANGE_MAX
            - SigmoidSmoothedValue::SIGMOID_T_RANGE_MIN)
            / default_step) as usize;
        for _ in 0..total_ramps {
            val.ramp();
        }
        assert!((val.current() - val.target()).abs() < 0.01); // should reach target

        // Test set_duration
        let mut val = SigmoidSmoothedValue::new(0.0, 44100);
        val.set_duration(2000);
        assert_eq!(val.duration(), 2000);
        val.set_target(1.0);
        assert_eq!(val.target(), 1.0);
        // After setting duration, step is adjusted
        let expected_step = (SigmoidSmoothedValue::SIGMOID_T_RANGE_MAX
            - SigmoidSmoothedValue::SIGMOID_T_RANGE_MIN)
            / 2000.0;
        assert!((val.step - expected_step).abs() < 0.001);
    }
}
