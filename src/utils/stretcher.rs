//! AudioTimeStretcher trait for time stretching interleaved audio (sources).

use crate::Error;

#[cfg(feature = "bungee-timestretch")]
pub(crate) mod bungee;

// -------------------------------------------------------------------------------------------------

/// AudioResampler specs.
#[derive(Copy, Clone)]
pub struct TimeStretchingSpecs {
    pub speed: f64,
    pub sample_rate: u32,
    pub channel_count: usize,
}

impl TimeStretchingSpecs {
    pub fn new(speed: f64, sample_rate: u32, channel_count: usize) -> Self {
        debug_assert!(speed > 0.0 && speed < 100.0);
        Self {
            speed,
            sample_rate,
            channel_count,
        }
    }
}

// -------------------------------------------------------------------------------------------------

/// Audio time stretcher interface.
///
/// Depending on the implementation, there might be an constrain on the input buffer size which
/// is fed into the stretcher in order to process something. Outputs never should have such
/// constrains.
pub trait AudioTimeStretcher: Send + Sync {
    /// Maximum input buffer length constrain for processing, if there is some.
    fn max_input_buffer_size(&self) -> Option<usize>;
    /// Minimum output buffer length constrain for processing, if there is some.
    fn min_output_buffer_size(&self) -> Option<usize>;

    /// Process interleaved input samples to the given interleaved output buffers.
    ///
    /// Input buffer size must fit the given required_input_buffer_size constrain, if there is
    /// some. The very last processing call can use a zero length input in order to flush any
    /// pending buffered outputs - if any.
    ///
    /// Returns `ResamplerError` or a `(input_consumed, output_written)` tuple on success.
    fn process(&mut self, input: &[f32], output: &mut [f32]) -> Result<(usize, usize), Error>;
}
