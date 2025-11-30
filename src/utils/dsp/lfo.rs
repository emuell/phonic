//! Oscillators for modulation.

use std::f64::consts::PI;

// -------------------------------------------------------------------------------------------------

/// Waveform types for LFO oscillators.
#[derive(
    Debug, Default, Copy, Clone, PartialEq, strum::Display, strum::EnumString, strum::VariantNames,
)]
pub enum LfoWaveform {
    #[default]
    Sine,
    Triangle,
    Sawtooth,
    Square,
}

// -------------------------------------------------------------------------------------------------

/// Simple non bandlimited oscillator which can be used as LFO in effects.
#[derive(Debug, Default, Clone)]
pub struct Lfo {
    phase: f64,
    phase_inc: f64,
    waveform: LfoWaveform,
}

impl Lfo {
    pub fn new(sample_rate: u32, rate: f64, waveform: LfoWaveform) -> Self {
        let phase_inc = 2.0 * PI * rate / sample_rate as f64;
        Self {
            phase: 0.0,
            phase_inc,
            waveform,
        }
    }

    /// Set a new rate in Hz with the given sampling rate.
    pub fn set_rate(&mut self, sample_rate: u32, rate: f64) {
        self.phase_inc = 2.0 * PI * rate / sample_rate as f64;
    }

    /// Set or reset the LFO's phase in degrees.
    pub fn set_phase(&mut self, phase: f64) {
        self.phase = phase;
    }

    /// Advances phase and returns new value
    pub fn next(&mut self) -> f64 {
        let val = match self.waveform {
            LfoWaveform::Sine => self.phase.sin(),
            LfoWaveform::Triangle => {
                // Triangle wave: -1 to 1
                let normalized_phase = self.phase / (2.0 * PI);
                if normalized_phase < 0.5 {
                    4.0 * normalized_phase - 1.0
                } else {
                    -4.0 * normalized_phase + 3.0
                }
            }
            LfoWaveform::Sawtooth => {
                // Sawtooth wave: -1 to 1
                let normalized_phase = self.phase / (2.0 * PI);
                2.0 * normalized_phase - 1.0
            }
            LfoWaveform::Square => {
                // Square wave: -1 or 1
                if self.phase < PI {
                    1.0
                } else {
                    -1.0
                }
            }
        };

        self.phase += self.phase_inc;
        while self.phase >= 2.0 * PI {
            self.phase -= 2.0 * PI;
        }
        val
    }
}
