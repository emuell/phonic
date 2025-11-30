//! DC removal filter.

// -------------------------------------------------------------------------------------------------

/// DC filter modes.
#[derive(
    Debug, Default, Copy, Clone, PartialEq, strum::Display, strum::EnumString, strum::VariantNames,
)]
#[allow(unused)]
pub enum DcFilterMode {
    /// ~1Hz cutoff: very gentle, might not remove all DC offset.
    Slow,
    /// ~5Hz cutoff: good for most cases.
    #[default]
    Default,
    /// ~20Hz cutoff: aggressive, might affect very low frequencies.
    Fast,
}

// -------------------------------------------------------------------------------------------------

/// DC blocker filter based on a one-pole high-pass filter.
/// `y(n) = x(n) - x(n-1) + R * y(n-1)`
#[derive(Debug, Clone)]
pub struct DcFilter {
    y1: f64, // previous output
    x1: f64, // previous input
    r: f64,  // coefficient
}

impl DcFilter {
    /// Create a new DC filter with a default cutoff.
    pub fn new() -> Self {
        Self {
            y1: 0.0,
            x1: 0.0,
            r: 0.999,
        }
    }

    pub fn init(&mut self, sample_rate: u32, mode: DcFilterMode) {
        let amount_coef = match mode {
            DcFilterMode::Slow => 1.0,
            DcFilterMode::Default => 5.0,
            DcFilterMode::Fast => 20.0,
        };
        self.r = 1.0 - (std::f64::consts::TAU * (amount_coef / sample_rate as f64));
    }

    /// Process helper function that calls `process_sample` for each sample in a buffer
    #[inline]
    pub fn process<'a>(&mut self, output: impl Iterator<Item = &'a mut f32>) {
        for sample in output {
            *sample = self.process_sample(*sample as f64) as f32;
        }
    }

    /// Process a single sample.
    #[inline]
    pub fn process_sample(&mut self, sample: f64) -> f64 {
        self.y1 = sample - self.x1 + self.r * self.y1;
        self.x1 = sample;
        self.y1
    }
}

impl Default for DcFilter {
    fn default() -> Self {
        Self::new()
    }
}
