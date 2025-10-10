//! Envelope follower for detecting signal levels.

/// An envelope follower that tracks the amplitude of a signal using attack and release times.
#[derive(Debug, Clone)]
pub struct EnvelopeFollower {
    current_value: f32,
    attack_coeff: f32,
    release_coeff: f32,
    sample_rate: u32,
}

impl EnvelopeFollower {
    /// Create a new envelope follower with the given sample rate and time constants.
    pub fn new(sample_rate: u32, attack_time: f32, release_time: f32) -> Self {
        let mut follower = Self {
            current_value: 0.0,
            attack_coeff: 0.0,
            release_coeff: 0.0,
            sample_rate,
        };
        follower.set_attack_time(attack_time);
        follower.set_release_time(release_time);
        follower
    }

    /// Set a new attack time constant.
    pub fn set_attack_time(&mut self, time: f32) {
        self.attack_coeff = if time > 0.0 {
            (-1.0 / (time * self.sample_rate as f32)).exp()
        } else {
            0.0
        };
    }

    /// Set a new release time constant.
    pub fn set_release_time(&mut self, time: f32) {
        self.release_coeff = if time > 0.0 {
            (-1.0 / (time * self.sample_rate as f32)).exp()
        } else {
            0.0
        };
    }

    /// Process a single input value and return the current envelope value.
    ///
    /// # Arguments
    /// * `input` - The input value (typically in dB)
    ///
    /// # Returns
    /// The current envelope value
    pub fn process(&mut self, input: f32) -> f32 {
        if input > self.current_value {
            // Attack phase
            self.current_value = input + self.attack_coeff * (self.current_value - input);
        } else {
            // Release phase
            self.current_value = input + self.release_coeff * (self.current_value - input);
        }
        self.current_value
    }

    /// Reset the envelope follower to the given value.
    ///
    /// # Arguments
    /// * `value` - The value to reset to (default is 0.0)
    pub fn reset(&mut self, value: f32) {
        self.current_value = value;
    }
}

impl Default for EnvelopeFollower {
    fn default() -> Self {
        Self::new(44100, 0.01, 0.1)
    }
}
