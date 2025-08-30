use crate::utils::{buffer::scale_buffer, panning_factors};

// Sigmoid ramp constants
const SIGMOID_T_RANGE_MIN: f32 = -5.0;
const SIGMOID_T_RANGE_MAX: f32 = 5.0;
const SIGMOID_DEFAULT_STEP: f32 = (SIGMOID_T_RANGE_MAX - SIGMOID_T_RANGE_MIN) / 1000.0;

// Default ramp steps
const DEFAULT_INERTIA: f32 = 0.02;
const DEFAULT_STEP: f32 = 0.01;

// -------------------------------------------------------------------------------------------------

/// Provides smooth transitions between a current and target f32 value.
/// Smoothing usually needs to be applied to avoid clicks in e.g. volume or other DSP parameter changes.
pub trait SmoothedValue {
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
    /// may be applied in blocks, which usually is faster.
    #[must_use]
    fn need_ramp(&self) -> bool;
    /// Move current to target value, when ramping is necessary.
    fn ramp(&mut self);

    /// Set current and target to the same value.
    fn init(&mut self, amount: f32);
    /// Set a new target value and ramp current, when current is different from the target.
    fn set_target(&mut self, target: f32);
}

// -------------------------------------------------------------------------------------------------

/// Apply a smoothed volume value to a buffer,
pub fn apply_smoothed_gain(buffer: &mut [f32], smoothed: &mut impl SmoothedValue) {
    if smoothed.need_ramp() {
        for s in buffer.iter_mut() {
            *s *= smoothed.next();
        }
    } else {
        let v = smoothed.target();
        if (1.0 - v).abs() > 0.001 {
            scale_buffer(buffer, v);
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
            if pan.abs() > 0.001 {
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

/// Exponential smoothed value for smooth ramping using an inertial exponential approach.
/// The value changes gradually towards the target based on the configurable inertia factor.
/// This should be the default smoothed value for volume alike parameters.
#[derive(Debug, Clone)]
pub struct ExponentialSmoothedValue {
    current: f32,
    target: f32,
    inertia: f32,
    sample_rate_comp: f32,
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
        const EPSILON: f32 = f32::EPSILON * 100.0;
        let inertia_add = (self.target - self.current) * self.inertia * self.sample_rate_comp;
        let next = self.current + inertia_add;
        (self.current - next).abs() > EPSILON
    }

    fn ramp(&mut self) {
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
}

impl ExponentialSmoothedValue {
    pub fn new(sample_rate: u32) -> Self {
        let sample_rate_comp = 44100.0 / sample_rate as f32;
        ExponentialSmoothedValue {
            current: 0.0,
            target: 0.0,
            inertia: DEFAULT_INERTIA,
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
        self.num_pending_steps > 0
    }

    fn ramp(&mut self) {
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
}

impl LinearSmoothedValue {
    pub fn new(sample_rate: u32) -> Self {
        let comp = 44100.0 / sample_rate as f32;
        LinearSmoothedValue {
            current: 0.0,
            target: 0.0,
            step: DEFAULT_STEP,
            current_step: 0.0,
            num_pending_steps: 0,
            sample_rate_comp: comp,
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

// -------------------------------------------------------------------------------------------------

/// Sigmoid smoothed value for ramping using a sigmoid function for smooth, non-linear transitions.
/// Uses an S-shaped curve for acceleration and deceleration, providing control over ramp duration.
pub struct SigmoidSmoothedValue {
    initial: f32,
    current: f32,
    target: f32,
    t: f32,
    step: f32,
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
        self.t < SIGMOID_T_RANGE_MAX
    }

    fn ramp(&mut self) {
        if self.need_ramp() {
            let sigmoid_coeff = 1.0 / (1.0 + (-self.t).exp());
            self.current = self.initial + sigmoid_coeff * (self.target - self.initial);
            self.t += self.step;
        }
    }

    fn init(&mut self, amount: f32) {
        self.target = amount;
        self.current = amount;
        self.initial = amount;
        self.t = SIGMOID_T_RANGE_MAX;
    }

    fn set_target(&mut self, target: f32) {
        self.initial = self.current;
        self.target = target;
        if self.current != self.target {
            self.t = SIGMOID_T_RANGE_MIN;
        }
    }
}

impl SigmoidSmoothedValue {
    pub fn new(_sample_rate: u32) -> Self {
        SigmoidSmoothedValue {
            initial: 0.0,
            current: 0.0,
            target: 0.0,
            t: SIGMOID_T_RANGE_MAX,
            step: SIGMOID_DEFAULT_STEP,
        }
    }

    pub fn duration(&self) -> usize {
        let range = SIGMOID_T_RANGE_MAX - SIGMOID_T_RANGE_MIN;
        (range / self.step) as usize
    }

    pub fn set_duration(&mut self, duration: usize) {
        assert!(duration > 0, "Invalid duration");
        let range = SIGMOID_T_RANGE_MAX - SIGMOID_T_RANGE_MIN;
        self.step = range / duration as f32;
    }

    pub fn reset(&mut self) {
        self.init(self.target);
    }
}

// -------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exp_smoothed_value() {
        // Test new
        let val = ExponentialSmoothedValue::new(44100);
        assert_eq!(val.current(), 0.0);
        assert_eq!(val.target(), 0.0);
        assert_eq!(val.inertia(), DEFAULT_INERTIA);

        // Test init
        let mut val = ExponentialSmoothedValue::new(44100);
        val.init(1.0);
        assert_eq!(val.current(), 1.0);
        assert_eq!(val.target(), 1.0);

        // Test set_target no ramp
        let mut val = ExponentialSmoothedValue::new(44100);
        val.set_target(0.0);
        assert_eq!(val.current(), 0.0);
        assert_eq!(val.target(), 0.0);
        assert!(!val.need_ramp());

        // Test set_target with ramp
        let mut val = ExponentialSmoothedValue::new(44100);
        val.set_target(1.0);
        assert_eq!(val.target(), 1.0);
        assert!(val.need_ramp());
        val.ramp();
        assert!(val.current() > 0.0);

        // Test multi ramps
        let mut val = ExponentialSmoothedValue::new(44100);
        val.set_target(1.0);
        let initial = val.current();
        for _ in 0..10 {
            val.ramp();
        }
        assert!(val.current() > initial);
        assert!(val.current() < val.target());
        assert!(val.need_ramp());

        // Test different inertia
        let mut val1 = ExponentialSmoothedValue::new(44100);
        val1.set_inertia(0.1);
        val1.set_target(1.0);
        val1.ramp();
        let current_high_inertia = val1.current();

        let mut val2 = ExponentialSmoothedValue::new(44100);
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
        let val = LinearSmoothedValue::new(44100);
        assert_eq!(val.current(), 0.0);
        assert_eq!(val.target(), 0.0);
        assert_eq!(val.step(), DEFAULT_STEP);

        // Test init
        let mut val = LinearSmoothedValue::new(44100);
        val.init(1.0);
        assert_eq!(val.current(), 1.0);
        assert_eq!(val.target(), 1.0);
        assert!(!val.need_ramp());

        // Test set_target no ramp
        let mut val = LinearSmoothedValue::new(44100);
        val.set_target(0.0);
        assert_eq!(val.current(), 0.0);
        assert_eq!(val.target(), 0.0);
        assert!(!val.need_ramp());

        // Test set_target with duration
        let mut val = LinearSmoothedValue::new(44100);
        val.set_target_with_duration(1.0, Some(10));
        assert_eq!(val.target(), 1.0);
        assert!(val.need_ramp());
        val.ramp();
        assert!(val.current() > 0.0);

        // Test reach target
        let mut val = LinearSmoothedValue::new(44100);
        val.set_target_with_duration(1.0, Some(5));
        for _ in 0..5 {
            val.ramp();
        }
        assert!(!val.need_ramp());
        assert_eq!(val.current(), 1.0);
        assert_eq!(val.target(), 1.0);

        // Test set_step
        let mut val = LinearSmoothedValue::new(44100);
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
        let val = SigmoidSmoothedValue::new(44100);
        assert_eq!(val.current(), 0.0);
        assert_eq!(val.target(), 0.0);

        // Test init
        let mut val = SigmoidSmoothedValue::new(44100);
        val.init(1.0);
        assert_eq!(val.current(), 1.0);
        assert_eq!(val.target(), 1.0);
        assert!(!val.need_ramp());

        // Test set_target
        let mut val = SigmoidSmoothedValue::new(44100);
        val.set_target(1.0);
        assert_eq!(val.target(), 1.0);
        assert!(val.need_ramp());
        val.ramp();
        assert!(val.current() > 0.0 && val.current() < 1.0);

        // Test ramp pattern
        let mut val = SigmoidSmoothedValue::new(44100);
        val.set_target(1.0);
        let total_ramps =
            ((SIGMOID_T_RANGE_MAX - SIGMOID_T_RANGE_MIN) / SIGMOID_DEFAULT_STEP) as usize;
        for _ in 0..total_ramps {
            val.ramp();
        }
        assert!((val.current() - val.target()).abs() < 0.1); // should reach target

        // Test set_duration
        let mut val = SigmoidSmoothedValue::new(44100);
        val.set_duration(2000);
        assert_eq!(val.duration(), 2000);
        val.set_target(1.0);
        assert_eq!(val.target(), 1.0);
        // After setting duration, step is adjusted
        let expected_step = (SIGMOID_T_RANGE_MAX - SIGMOID_T_RANGE_MIN) / 2000.0;
        assert!((val.step - expected_step).abs() < 0.001);
    }
}
