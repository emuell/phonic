use std::{f32, f64};

use strum::{Display, EnumIter, EnumString};

use crate::Error;

// -------------------------------------------------------------------------------------------------

/// Available filter types for the State Variable Filter.
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug, Display, EnumIter, EnumString)]
#[allow(unused)]
pub enum BiquadFilterType {
    #[default]
    Lowpass,
    Highpass,
    Bandpass,
    Notch,
    Peak,
    Allpass,
    Bell,
    Lowshelf,
    Highshelf,
}

// -------------------------------------------------------------------------------------------------

/// The coefficients that hold parameters and necessary data to process the filter.
///
/// See [BiquadFilter] for more info about the filter implementation.
#[derive(Default, Clone, PartialEq)]
pub struct BiquadFilterCoefficients {
    filter_type: BiquadFilterType,
    sample_rate: u32,
    cutoff: f32,
    q: f32,
    gain: f32,
    a1: f64,
    a2: f64,
    a3: f64,
    m0: f64,
    m1: f64,
    m2: f64,
}

#[allow(unused)]
impl BiquadFilterCoefficients {
    pub fn new(
        filter_type: BiquadFilterType,
        sample_rate: u32,
        cutoff: f32,
        q: f32,
        gain: f32,
    ) -> Result<Self, Error> {
        let mut coefficients = BiquadFilterCoefficients::default();
        coefficients.set(filter_type, sample_rate, cutoff, q, gain)?;
        Ok(coefficients)
    }

    /// Get currently applied filter type.
    pub fn filter_type(&self) -> BiquadFilterType {
        self.filter_type
    }
    pub fn set_filter_type(&mut self, filter_type: BiquadFilterType) -> Result<(), Error> {
        if self.filter_type != filter_type {
            self.filter_type = filter_type;
            self.apply()
        } else {
            Ok(())
        }
    }

    /// Get currently applied sample rate
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    pub fn set_sample_rate(&mut self, sample_rate: u32) -> Result<(), Error> {
        if self.sample_rate != sample_rate {
            self.sample_rate = sample_rate;
            self.apply()
        } else {
            Ok(())
        }
    }

    /// The frequency in Hz where the cutoff of the filter should be.
    pub fn cutoff(&self) -> f32 {
        self.cutoff
    }
    /// Set the cutoff frequency in Hz. Must be below nyquist.
    pub fn set_cutoff(&mut self, cutoff: f32) -> Result<(), Error> {
        if self.cutoff != cutoff {
            self.cutoff = cutoff;
            self.apply()
        } else {
            Ok(())
        }
    }

    /// The steepness of the filter.
    pub fn q(&self) -> f32 {
        self.q
    }
    /// Set the resonance (Q factor). Must be > 0.0.
    pub fn set_q(&mut self, q: f32) -> Result<(), Error> {
        if self.q != q {
            self.q = q;
            self.apply()
        } else {
            Ok(())
        }
    }

    pub fn gain(&self) -> f32 {
        self.gain
    }
    /// Set the gain in dB. Only used by Bell, Lowshelf and Highshelf filters.
    pub fn set_gain(&mut self, gain: f32) -> Result<(), Error> {
        if self.gain != gain {
            self.gain = gain;
            self.apply()
        } else {
            Ok(())
        }
    }

    /// Sets new and applies a bach of new filter parameters.
    pub fn set(
        &mut self,
        filter_type: BiquadFilterType,
        sample_rate: u32,
        cutoff: f32,
        q: f32,
        gain: f32,
    ) -> Result<(), Error> {
        if self.filter_type != filter_type
            || self.sample_rate != sample_rate
            || self.cutoff != cutoff
            || self.q != q
            || self.gain != gain
        {
            self.filter_type = filter_type;
            self.sample_rate = sample_rate;
            self.cutoff = cutoff;
            self.q = q;
            self.gain = gain;
            self.apply()
        } else {
            Ok(())
        }
    }

    /// Applies filter filter parameters.
    pub fn apply(&mut self) -> Result<(), Error> {
        if self.sample_rate == 0 {
            return Err(Error::ParameterError(format!(
                "Invalid filter sample-rate: must be > 0, but is {s}",
                s = self.sample_rate
            )));
        }
        if self.q <= 0.0 {
            return Err(Error::ParameterError(format!(
                "Invalid filter Q: must be > 0, but is {q}",
                q = self.q
            )));
        }
        if self.cutoff > self.sample_rate as f32 / 2.0 {
            return Err(Error::ParameterError(format!(
                "Invalid filter frequency: must be > nyquist {n}, but is {f}",
                n = self.sample_rate as f32 / 2.0,
                f = self.cutoff
            )));
        }
        match self.filter_type {
            BiquadFilterType::Lowpass => {
                let g = f64::tan(f64::consts::PI * self.cutoff as f64 / self.sample_rate as f64);
                let k = 1.0 / self.q as f64;
                self.a1 = 1.0 / (1.0 + g * (g + k));
                self.a2 = g * self.a1;
                self.a3 = g * self.a2;
                self.m0 = 0.0;
                self.m1 = 0.0;
                self.m2 = 1.0;
            }
            BiquadFilterType::Highpass => {
                let g = f64::tan(f64::consts::PI * self.cutoff as f64 / self.sample_rate as f64);
                let k = 1.0 / self.q as f64;
                self.a1 = 1.0 / (1.0 + g * (g + k));
                self.a2 = g * self.a1;
                self.a3 = g * self.a2;
                self.m0 = 1.0;
                self.m1 = -k;
                self.m2 = -1.0;
            }
            BiquadFilterType::Bandpass => {
                let g = f64::tan(f64::consts::PI * self.cutoff as f64 / self.sample_rate as f64);
                let k = 1.0 / self.q as f64;
                self.a1 = 1.0 / (1.0 + g * (g + k));
                self.a2 = g * self.a1;
                self.a3 = g * self.a2;
                self.m0 = 0.0;
                self.m1 = 1.0;
                self.m2 = 0.0;
            }
            BiquadFilterType::Notch => {
                let g = f64::tan(f64::consts::PI * self.cutoff as f64 / self.sample_rate as f64);
                let k = 1.0 / self.q as f64;
                self.a1 = 1.0 / (1.0 + g * (g + k));
                self.a2 = g * self.a1;
                self.a3 = g * self.a2;
                self.m0 = 1.0;
                self.m1 = -k;
                self.m2 = 0.0;
            }
            BiquadFilterType::Peak => {
                let g = f64::tan(f64::consts::PI * self.cutoff as f64 / self.sample_rate as f64);
                let k = 1.0 / self.q as f64;
                self.a1 = 1.0 / (1.0 + g * (g + k));
                self.a2 = g * self.a1;
                self.a3 = g * self.a2;
                self.m0 = 1.0;
                self.m1 = -k;
                self.m2 = -2.0;
            }
            BiquadFilterType::Allpass => {
                let g = f64::tan(f64::consts::PI * self.cutoff as f64 / self.sample_rate as f64);
                let k = 1.0 / self.q as f64;
                self.a1 = 1.0 / (1.0 + g * (g + k));
                self.a2 = g * self.a1;
                self.a3 = g * self.a2;
                self.m0 = 1.0;
                self.m1 = -2.0 * k;
                self.m2 = 0.0;
            }
            BiquadFilterType::Bell => {
                let a = f64::powf(10.0, self.gain as f64 / 40.0);
                let g = f64::tan(f64::consts::PI * self.cutoff as f64 / self.sample_rate as f64);
                let k = 1.0 / (self.q as f64 * a);
                self.a1 = 1.0 / (1.0 + g * (g + k));
                self.a2 = g * self.a1;
                self.a3 = g * self.a2;
                self.m0 = 1.0;
                self.m1 = k * (a * a - 1.0);
                self.m2 = 0.0;
            }
            BiquadFilterType::Lowshelf => {
                let a = f64::powf(10.0, self.gain as f64 / 40.0);
                let g = f64::tan(f64::consts::PI * self.cutoff as f64 / self.sample_rate as f64)
                    / f64::sqrt(a);
                let k = 1.0 / self.q as f64;
                self.a1 = 1.0 / (1.0 + g * (g + k));
                self.a2 = g * self.a1;
                self.a3 = g * self.a2;
                self.m0 = 1.0;
                self.m1 = k * (a - 1.0);
                self.m2 = a * a - 1.0;
            }
            BiquadFilterType::Highshelf => {
                let a = f64::powf(10.0, self.gain as f64 / 40.0);
                let g = f64::tan(f64::consts::PI * self.cutoff as f64 / self.sample_rate as f64)
                    * f64::sqrt(a);
                let k = 1.0 / self.q as f64;
                self.a1 = 1.0 / (1.0 + g * (g + k));
                self.a2 = g * self.a1;
                self.a3 = g * self.a2;
                self.m0 = a * a;
                self.m1 = k * (1.0 - a) * a;
                self.m2 = 1.0 - a * a;
            }
        }
        Ok(())
    }
}

// -------------------------------------------------------------------------------------------------

/// State variable biquad filter, designed by Andrew Simper of Cytomic.
/// See <http://cytomic.com/files/dsp/SvfLinearTrapOptimised2.pdf>
///
/// The frequency response of this filter is the same as of BZT filters.
///
/// This is a second-order filter. It has a cutoff slope of 12 dB/octave. Q = 0.707 means no
/// resonant peaking. This filter will self-oscillate when Q is very high.
///
/// This filter is stable when modulated at high rates.
#[derive(Default, Clone)]
pub struct BiquadFilter {
    ic1eq: f64,
    ic2eq: f64,
}

#[allow(unused)]
impl BiquadFilter {
    pub fn new() -> Self {
        Self {
            ic1eq: 0.0,
            ic2eq: 0.0,
        }
    }

    /// Process helper function that calls `process_sample` for each sample in a buffer
    #[inline]
    pub fn process<'a>(
        &mut self,
        coefficients: &BiquadFilterCoefficients,
        output: impl Iterator<Item = &'a mut f32>,
    ) {
        for sample in output {
            *sample = self.process_sample(coefficients, *sample as f64) as f32;
        }
    }

    /// Apply the filter on a single sample.
    #[inline]
    pub fn process_sample(&mut self, coefficients: &BiquadFilterCoefficients, input: f64) -> f64 {
        let v0 = input;
        let v3 = v0 - self.ic2eq;
        let v1 = coefficients.a1 * self.ic1eq + coefficients.a2 * v3;
        let v2 = self.ic2eq + coefficients.a2 * self.ic1eq + coefficients.a3 * v3;
        self.ic1eq = 2.0 * v1 - self.ic1eq;
        self.ic2eq = 2.0 * v2 - self.ic2eq;
        coefficients.m0 * v0 + coefficients.m1 * v1 + coefficients.m2 * v2
    }

    /// Reset state of filter.
    /// Can be used when the audio callback is restarted.
    #[inline]
    pub fn reset(&mut self) {
        self.ic1eq = 0.0;
        self.ic2eq = 0.0;
    }
}
