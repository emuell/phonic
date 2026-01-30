//! Oscillators for modulation.

use rand::{rngs::SmallRng, Rng, SeedableRng};

// -------------------------------------------------------------------------------------------------

/// See https://web.archive.org/web/20171228230531/http://forum.devmaster.net/t/fast-and-accurate-sine-cosine/9648
/// x must be in range [-PI to PI]
fn sine_approx(x: f32) -> f32 {
    use std::f32::consts::PI;
    debug_assert!((-PI..=PI).contains(&x));

    const B: f32 = 4.0 / PI;
    const C: f32 = -4.0 / (PI * PI);
    const P: f32 = 0.225;

    let y = B * x + C * x * x.abs();
    P * (y * y.abs() - y) + y
}

// -------------------------------------------------------------------------------------------------

/// Waveform types for LFO oscillators.
#[derive(
    Debug, Default, Copy, Clone, PartialEq, strum::Display, strum::EnumString, strum::VariantNames,
)]
pub enum LfoWaveform {
    #[default]
    Sine,
    Triangle,
    #[strum(serialize = "Ramp Up")]
    RampUp,
    #[strum(serialize = "Ramp Down")]
    RampDown,
    Square,
    Random,
    #[strum(serialize = "Smooth Random")]
    SmoothRandom,
}

// -------------------------------------------------------------------------------------------------

/// Simple non bandlimited oscillator which can be used as LFO in effects.
#[derive(Debug, Clone)]
pub struct Lfo {
    phase: f32,
    phase_inc: f32,
    waveform: LfoWaveform,
    sample_hold_value: f32,
    jitter_current: f32,
    jitter_target: f32,
    rng: SmallRng,
}

impl Default for Lfo {
    fn default() -> Self {
        Self::new(44100, 1.0, LfoWaveform::Sine)
    }
}

impl Lfo {
    pub fn new(sample_rate: u32, rate: f64, waveform: LfoWaveform) -> Self {
        let phase = 0.0;
        let phase_inc = (rate / sample_rate as f64) as f32;
        let mut rng = SmallRng::from_os_rng();
        let sample_hold_value = rng.random::<f32>() * 2.0 - 1.0;
        let jitter_current = rng.random::<f32>() * 2.0 - 1.0;
        let jitter_target = rng.random::<f32>() * 2.0 - 1.0;
        Self {
            phase,
            phase_inc,
            waveform,
            sample_hold_value,
            jitter_current,
            jitter_target,
            rng,
        }
    }

    /// Set a new rate in Hz with the given sample rate.
    pub fn set_rate(&mut self, sample_rate: u32, rate: f64) {
        self.phase_inc = (rate / sample_rate as f64) as f32;
    }

    /// Set or reset the LFO's phase (normalized [0, 1]).
    pub fn set_phase(&mut self, phase: f32) {
        self.phase = phase.rem_euclid(1.0);
    }

    /// Set or reset the LFO's phase in degrees [0, 2*PI].
    pub fn set_phase_degrees(&mut self, phase: f32) {
        self.set_phase(phase / std::f32::consts::TAU);
    }

    /// Set the waveform type.
    pub fn set_waveform(&mut self, waveform: LfoWaveform) {
        self.waveform = waveform;
    }

    /// Advances phase and returns new value
    pub fn run(&mut self) -> f32 {
        let value = match self.waveform {
            LfoWaveform::Sine => {
                let p = if self.phase < 0.5 {
                    self.phase * std::f32::consts::TAU
                } else {
                    (self.phase - 1.0) * std::f32::consts::TAU
                };
                sine_approx(p)
            }
            LfoWaveform::Triangle => {
                if self.phase < 0.25 {
                    self.phase * 4.0
                } else if self.phase < 0.75 {
                    2.0 - self.phase * 4.0
                } else {
                    self.phase * 4.0 - 4.0
                }
            }
            LfoWaveform::RampUp => self.phase * 2.0 - 1.0,
            LfoWaveform::RampDown => 1.0 - self.phase * 2.0,
            LfoWaveform::Square => {
                if self.phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            LfoWaveform::Random => self.sample_hold_value,
            LfoWaveform::SmoothRandom => {
                // Smooth interpolation between random values using cosine interpolation
                let p = std::f32::consts::FRAC_PI_2 - self.phase * std::f32::consts::PI;
                let t = (1.0 - sine_approx(p)) * 0.5;
                self.jitter_current + t * (self.jitter_target - self.jitter_current)
            }
        };

        if matches!(
            self.waveform,
            LfoWaveform::Random | LfoWaveform::SmoothRandom
        ) {
            self.advance_phase_random();
        } else {
            self.advance_phase();
        }

        value
    }

    /// Write lfo values into the given output buffer, filling the entire buffer.
    pub fn process(&mut self, output: &mut [f32]) {
        match self.waveform {
            LfoWaveform::Sine => {
                for sample in output {
                    let p = if self.phase < 0.5 {
                        self.phase * std::f32::consts::TAU
                    } else {
                        (self.phase - 1.0) * std::f32::consts::TAU
                    };
                    *sample = sine_approx(p);
                    self.advance_phase();
                }
            }
            LfoWaveform::Triangle => {
                for sample in output {
                    *sample = if self.phase < 0.25 {
                        self.phase * 4.0
                    } else if self.phase < 0.75 {
                        2.0 - self.phase * 4.0
                    } else {
                        self.phase * 4.0 - 4.0
                    };
                    self.advance_phase();
                }
            }
            LfoWaveform::RampUp => {
                for sample in output {
                    *sample = self.phase * 2.0 - 1.0;
                    self.advance_phase();
                }
            }
            LfoWaveform::RampDown => {
                for sample in output {
                    *sample = 1.0 - self.phase * 2.0;
                    self.advance_phase();
                }
            }
            LfoWaveform::Square => {
                for sample in output {
                    *sample = if self.phase < 0.5 { 1.0 } else { -1.0 };
                    self.advance_phase();
                }
            }
            LfoWaveform::Random => {
                for sample in output {
                    *sample = self.sample_hold_value;
                    self.advance_phase_random();
                }
            }
            LfoWaveform::SmoothRandom => {
                for sample in output {
                    // Smooth interpolation between random values using cosine interpolation
                    let p = std::f32::consts::FRAC_PI_2 - self.phase * std::f32::consts::PI;
                    let t = (1.0 - sine_approx(p)) * 0.5;
                    *sample = self.jitter_current + t * (self.jitter_target - self.jitter_current);
                    self.advance_phase_random();
                }
            }
        }
    }

    #[inline]
    fn advance_phase(&mut self) {
        self.phase += self.phase_inc;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
    }

    #[inline]
    fn advance_phase_random(&mut self) {
        self.phase += self.phase_inc;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
            // Update Random (S&H) value on phase wrap
            self.sample_hold_value = self.rng.random::<f32>() * 2.0 - 1.0;
            // Update Jitter target on phase wrap
            self.jitter_current = self.jitter_target;
            self.jitter_target = self.rng.random::<f32>() * 2.0 - 1.0;
        }
    }
}
