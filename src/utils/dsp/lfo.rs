//! Oscillators for modulation.

use std::f64::consts::PI;

use rand::{rngs::SmallRng, Rng, SeedableRng};

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
    phase: f64,
    phase_inc: f64,
    waveform: LfoWaveform,
    sample_hold_value: f64,
    jitter_current: f64,
    jitter_target: f64,
    jitter_phase: f64,
    rng: SmallRng,
}

impl Default for Lfo {
    fn default() -> Self {
        Self::new(44100, 1.0, LfoWaveform::Sine)
    }
}

impl Lfo {
    pub fn new(sample_rate: u32, rate: f64, waveform: LfoWaveform) -> Self {
        let phase_inc = 2.0 * PI * rate / sample_rate as f64;
        let mut rng = SmallRng::from_os_rng();
        let sample_hold_value = rng.random::<f64>() * 2.0 - 1.0;
        let jitter_current = rng.random::<f64>() * 2.0 - 1.0;
        let jitter_target = rng.random::<f64>() * 2.0 - 1.0;
        Self {
            phase: 0.0,
            phase_inc,
            waveform,
            sample_hold_value,
            jitter_current,
            jitter_target,
            jitter_phase: 0.0,
            rng,
        }
    }

    /// Set a new rate in Hz with the given sample rate.
    pub fn set_rate(&mut self, sample_rate: u32, rate: f64) {
        self.phase_inc = 2.0 * PI * rate / sample_rate as f64;
    }

    /// Set or reset the LFO's phase in degrees.
    pub fn set_phase(&mut self, phase: f64) {
        self.phase = phase;
    }

    /// Set the waveform type.
    pub fn set_waveform(&mut self, waveform: LfoWaveform) {
        self.waveform = waveform;
    }

    /// Advances phase and returns new value
    pub fn run(&mut self) -> f64 {
        let normalized_phase = self.phase / (2.0 * PI);

        let val = match self.waveform {
            LfoWaveform::Sine => self.phase.sin(),
            LfoWaveform::Triangle => {
                if normalized_phase < 0.25 {
                    normalized_phase * 4.0
                } else if normalized_phase < 0.75 {
                    1.0 - (normalized_phase - 0.25) * 4.0
                } else {
                    -1.0 + (normalized_phase - 0.75) * 4.0
                }
            }
            LfoWaveform::RampUp => 2.0 * normalized_phase - 1.0,
            LfoWaveform::RampDown => 1.0 - 2.0 * normalized_phase,
            LfoWaveform::Square => {
                if normalized_phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            LfoWaveform::Random => self.sample_hold_value,
            LfoWaveform::SmoothRandom => {
                // Smooth interpolation between random values using cosine interpolation
                let t = (1.0 - (self.jitter_phase * PI).cos()) * 0.5;
                self.jitter_current * (1.0 - t) + self.jitter_target * t
            }
        };

        self.phase += self.phase_inc;
        if self.phase >= 2.0 * PI {
            self.phase -= 2.0 * PI;
            // Update Random (S&H) value on phase wrap
            self.sample_hold_value = self.rng.random::<f64>() * 2.0 - 1.0;
            // Update Jitter target on phase wrap
            self.jitter_current = self.jitter_target;
            self.jitter_target = self.rng.random::<f64>() * 2.0 - 1.0;
            self.jitter_phase = 0.0;
        }

        // Advance jitter phase
        if self.waveform == LfoWaveform::SmoothRandom {
            self.jitter_phase += self.phase_inc / (2.0 * PI);
            if self.jitter_phase > 1.0 {
                self.jitter_phase = 1.0;
            }
        }

        val
    }
}
