//! State Variable Filter (SVF) using the topology-preserving transform (TPT)
//! as described by Andy Simper / Cytomic.
//!
//! Reference: Andy Simper, "Linear Trapezoidal Integrated SVF", Cytomic Technical Papers, 2013.
//! <http://cytomic.com/files/dsp/SvfLinearTrapOptimised2.pdf>

use std::f64;

use crate::Error;

// -------------------------------------------------------------------------------------------------

/// Available filter types for the SVF Filter.
#[derive(
    Debug, Default, Copy, Clone, PartialEq, strum::Display, strum::EnumString, strum::VariantNames,
)]
pub enum SvfFilterType {
    #[default]
    Lowpass,
    Highpass,
    Bandpass,
}

// -------------------------------------------------------------------------------------------------

/// The coefficients that hold parameters and necessary data to process the SVF filter.
///
/// See [SvfFilter] for more info about the filter implementation.
#[derive(Default, Clone, PartialEq)]
pub struct SvfFilterCoefficients {
    filter_type: SvfFilterType,
    sample_rate: u32,
    cutoff: f32,
    resonance: f32,
    g: f64,
    k: f64,
    a1: f64,
    a2: f64,
    a3: f64,
}

#[allow(unused)]
impl SvfFilterCoefficients {
    pub fn new(
        filter_type: SvfFilterType,
        sample_rate: u32,
        cutoff: f32,
        resonance: f32,
    ) -> Result<Self, Error> {
        let mut coefficients = SvfFilterCoefficients::default();
        coefficients.set(filter_type, sample_rate, cutoff, resonance)?;
        Ok(coefficients)
    }

    /// Get currently applied filter type.
    #[inline]
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

    /// Get currently applied sample rate.
    #[inline]
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
    #[inline]
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

    /// The resonance amount (0.0 .. 1.0).
    #[inline]
    pub fn resonance(&self) -> f32 {
        self.resonance
    }
    /// Set the resonance amount. Must be in range 0.0 .. 1.0.
    pub fn set_resonance(&mut self, resonance: f32) -> Result<(), Error> {
        if self.resonance != resonance {
            self.resonance = resonance;
            self.apply()
        } else {
            Ok(())
        }
    }

    /// Sets and applies a batch of new filter parameters.
    pub fn set(
        &mut self,
        filter_type: SvfFilterType,
        sample_rate: u32,
        cutoff: f32,
        resonance: f32,
    ) -> Result<(), Error> {
        if self.filter_type != filter_type
            || self.sample_rate != sample_rate
            || self.cutoff != cutoff
            || self.resonance != resonance
        {
            self.filter_type = filter_type;
            self.sample_rate = sample_rate;
            self.cutoff = cutoff;
            self.resonance = resonance;
            self.apply()
        } else {
            Ok(())
        }
    }

    /// Applies filter parameters.
    pub fn apply(&mut self) -> Result<(), Error> {
        if self.sample_rate == 0 {
            return Err(Error::ParameterError(format!(
                "Invalid filter sample-rate: must be > 0, but is {s}",
                s = self.sample_rate
            )));
        }
        if self.resonance < 0.0 || self.resonance > 1.0 {
            return Err(Error::ParameterError(format!(
                "Invalid filter resonance: must be in 0.0..1.0, but is {r}",
                r = self.resonance
            )));
        }
        if self.cutoff > self.sample_rate as f32 / 2.0 {
            return Err(Error::ParameterError(format!(
                "Invalid filter frequency: must be <= nyquist {n}, but is {f}",
                n = self.sample_rate as f32 / 2.0,
                f = self.cutoff
            )));
        }
        // Warped cutoff via bilinear transform: g = tan(pi * fc / fs)
        self.g = f64::tan(f64::consts::PI * self.cutoff as f64 / self.sample_rate as f64);
        // Damping coefficient: k = 2 * (1 - resonance * 0.97)
        // At resonance=0: k=2 (no resonance, Butterworth-ish)
        // At resonance=1: k~0.06 (near self-oscillation)
        self.k = (2.0 * (1.0 - self.resonance as f64 * 0.97)).max(0.03);
        // Precompute the matrix coefficients
        self.a1 = 1.0 / (1.0 + self.g * (self.g + self.k));
        self.a2 = self.g * self.a1;
        self.a3 = self.g * self.a2;
        Ok(())
    }
}

// -------------------------------------------------------------------------------------------------

/// State variable filter using the Cytomic (Andy Simper) TPT topology.
///
/// This is a second-order filter with a cutoff slope of 12 dB/octave.
#[derive(Clone)]
pub struct SvfFilter {
    ic1eq: f64,
    ic2eq: f64,
}

impl Default for SvfFilter {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(unused)]
impl SvfFilter {
    pub fn new() -> Self {
        Self {
            ic1eq: 0.0,
            ic2eq: 0.0,
        }
    }

    /// Process helper function that calls `process_sample` for each sample in a buffer.
    #[inline]
    pub fn process<'a>(
        &mut self,
        coefficients: &SvfFilterCoefficients,
        output: impl Iterator<Item = &'a mut f32>,
    ) {
        for sample in output {
            *sample = self.process_sample(coefficients, *sample as f64) as f32;
        }
    }

    /// Apply the filter on a single sample.
    #[inline]
    pub fn process_sample(&mut self, coefficients: &SvfFilterCoefficients, input: f64) -> f64 {
        let v3 = input - self.ic2eq;
        let v1 = coefficients.a1 * self.ic1eq + coefficients.a2 * v3;
        let v2 = self.ic2eq + coefficients.a2 * self.ic1eq + coefficients.a3 * v3;
        self.ic1eq = 2.0 * v1 - self.ic1eq;
        self.ic2eq = 2.0 * v2 - self.ic2eq;
        match coefficients.filter_type {
            SvfFilterType::Lowpass => v2,
            SvfFilterType::Bandpass => v1,
            SvfFilterType::Highpass => input - coefficients.k * v1 - v2,
        }
    }

    /// Reset state of filter.
    #[inline]
    pub fn reset(&mut self) {
        self.ic1eq = 0.0;
        self.ic2eq = 0.0;
    }
}
