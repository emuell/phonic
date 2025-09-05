use std::{f32, f64};

use crate::Error;

// -------------------------------------------------------------------------------------------------

/// Available filter types for the State Variable Filter.
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
#[allow(unused)]
pub enum SvfFilterType {
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
#[derive(Default, Clone, PartialEq)]
pub struct SvfCoefficients {
    filter_type: SvfFilterType,
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

// -------------------------------------------------------------------------------------------------

/// State variable filter (SVF), designed by Andrew Simper of Cytomic.
/// See <http://cytomic.com/files/dsp/SvfLinearTrapOptimised2.pdf>
///
/// The frequency response of this filter is the same as of BZT filters.
///
/// This is a second-order filter. It has a cutoff slope of 12 dB/octave.
/// Q = 0.707 means no resonant peaking.
///
/// This filter will self-oscillate when Q is very high (can be forced by
/// setting the `k` coefficient to zero).
///
/// This filter is stable when modulated at high rates.
#[derive(Default, Clone)]
pub struct SvfFilter {
    coefficients: SvfCoefficients,
    ic1eq: f64,
    ic2eq: f64,
}

#[allow(unused)]
impl SvfCoefficients {
    /// Get currently applied filter type.
    pub fn filter_type(&self) -> SvfFilterType {
        self.filter_type
    }
    pub fn set_filter_type(&mut self, filter_type: SvfFilterType) -> Result<(), Error> {
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
        filter_type: SvfFilterType,
        sample_rate: u32,
        cutoff_frequency: f32,
        q: f32,
        gain: f32,
    ) -> Result<(), Error> {
        if self.filter_type != filter_type
            || self.sample_rate != sample_rate
            || self.cutoff != cutoff_frequency
            || self.q != q
            || self.gain != gain
        {
            self.filter_type = filter_type;
            self.sample_rate = sample_rate;
            self.cutoff = cutoff_frequency;
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
            SvfFilterType::Lowpass => {
                let g = f64::tan(f64::consts::PI * self.cutoff as f64 / self.sample_rate as f64);
                let k = 1.0 / self.q as f64;
                self.a1 = 1.0 / (1.0 + g * (g + k));
                self.a2 = g * self.a1;
                self.a3 = g * self.a2;
                self.m0 = 0.0;
                self.m1 = 0.0;
                self.m2 = 1.0;
            }
            SvfFilterType::Highpass => {
                let g = f64::tan(f64::consts::PI * self.cutoff as f64 / self.sample_rate as f64);
                let k = 1.0 / self.q as f64;
                self.a1 = 1.0 / (1.0 + g * (g + k));
                self.a2 = g * self.a1;
                self.a3 = g * self.a2;
                self.m0 = 1.0;
                self.m1 = -k;
                self.m2 = -1.0;
            }
            SvfFilterType::Bandpass => {
                let g = f64::tan(f64::consts::PI * self.cutoff as f64 / self.sample_rate as f64);
                let k = 1.0 / self.q as f64;
                self.a1 = 1.0 / (1.0 + g * (g + k));
                self.a2 = g * self.a1;
                self.a3 = g * self.a2;
                self.m0 = 0.0;
                self.m1 = 1.0;
                self.m2 = 0.0;
            }
            SvfFilterType::Notch => {
                let g = f64::tan(f64::consts::PI * self.cutoff as f64 / self.sample_rate as f64);
                let k = 1.0 / self.q as f64;
                self.a1 = 1.0 / (1.0 + g * (g + k));
                self.a2 = g * self.a1;
                self.a3 = g * self.a2;
                self.m0 = 1.0;
                self.m1 = -k;
                self.m2 = 0.0;
            }
            SvfFilterType::Peak => {
                let g = f64::tan(f64::consts::PI * self.cutoff as f64 / self.sample_rate as f64);
                let k = 1.0 / self.q as f64;
                self.a1 = 1.0 / (1.0 + g * (g + k));
                self.a2 = g * self.a1;
                self.a3 = g * self.a2;
                self.m0 = 1.0;
                self.m1 = -k;
                self.m2 = -2.0;
            }
            SvfFilterType::Allpass => {
                let g = f64::tan(f64::consts::PI * self.cutoff as f64 / self.sample_rate as f64);
                let k = 1.0 / self.q as f64;
                self.a1 = 1.0 / (1.0 + g * (g + k));
                self.a2 = g * self.a1;
                self.a3 = g * self.a2;
                self.m0 = 1.0;
                self.m1 = -2.0 * k;
                self.m2 = 0.0;
            }
            SvfFilterType::Bell => {
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
            SvfFilterType::Lowshelf => {
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
            SvfFilterType::Highshelf => {
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

#[allow(unused)]
impl SvfFilter {
    /// Sets the filter to an initial response. Use `Svf::default` otherwise.
    pub fn new(
        filter_type: SvfFilterType,
        sample_rate: u32,
        cutoff: f32,
        q: f32,
        gain: f32,
    ) -> Result<Self, Error> {
        let mut svf = Self::default();
        svf.set(filter_type, sample_rate, cutoff, q, gain)?;
        Ok(svf)
    }

    /// Process helper function that calls `process_sample` for each sample in a buffer
    #[inline]
    pub fn process<'a>(&mut self, output: impl Iterator<Item = &'a mut f32>) {
        for sample in output {
            *sample = self.process_sample(*sample as f64) as f32;
        }
    }

    /// Apply the filter on a single sample.
    #[inline]
    pub fn process_sample(&mut self, input: f64) -> f64 {
        let v0 = input;
        let v3 = v0 - self.ic2eq;
        let v1 = self.coefficients.a1 * self.ic1eq + self.coefficients.a2 * v3;
        let v2 = self.ic2eq + self.coefficients.a2 * self.ic1eq + self.coefficients.a3 * v3;
        self.ic1eq = 2.0 * v1 - self.ic1eq;
        self.ic2eq = 2.0 * v2 - self.ic2eq;
        self.coefficients.m0 * v0 + self.coefficients.m1 * v1 + self.coefficients.m2 * v2
    }

    /// Reset state of filter.
    /// Can be used when the audio callback is restarted.
    #[inline]
    pub fn reset(&mut self) {
        self.ic1eq = 0.0;
        self.ic2eq = 0.0;
    }

    /// Set new filter parameters
    /// Parameters:
    /// - filter_type: choose one of the filter types, like peak, lowpass or highpass
    /// - sample_rate: the sample_rate of the audio buffer that the filter should be applied on
    /// - frequency: the frequency in Hz where the cutoff of the filter should be
    /// - q: the steepness of the filter
    /// - gain: the gain boost or decrease of the filter
    #[inline]
    pub fn set(
        &mut self,
        filter_type: SvfFilterType,
        sample_rate: u32,
        cutoff: f32,
        q: f32,
        gain: f32,
    ) -> Result<(), Error> {
        self.coefficients
            .set(filter_type, sample_rate, cutoff, q, gain)
    }

    /// get a reference to the coefficients
    pub fn coefficients(&self) -> &SvfCoefficients {
        &self.coefficients
    }
    /// get a mutable reference to the coefficients
    pub fn coefficients_mut(&mut self) -> &mut SvfCoefficients {
        &mut self.coefficients
    }
}
