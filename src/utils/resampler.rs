//! AudioResampling trait for resampling interleaved audio (sources).

use crate::Error;

pub(crate) mod cubic;
pub(crate) mod rubato;

// -------------------------------------------------------------------------------------------------

/// AudioResampler specs.
#[derive(Copy, Clone)]
pub struct ResamplingSpecs {
    input_rate: u32,
    output_rate: u32,
    channel_count: usize,
}

impl ResamplingSpecs {
    pub fn new(input_rate: u32, output_rate: u32, channel_count: usize) -> Self {
        Self {
            input_rate,
            output_rate,
            channel_count,
        }
    }

    pub fn input_ratio(&self) -> f64 {
        self.input_rate as f64 / self.output_rate as f64
    }
    pub fn output_ratio(&self) -> f64 {
        self.output_rate as f64 / self.input_rate as f64
    }
}

// -------------------------------------------------------------------------------------------------

/// Audio resampler interface.
///
/// Depending on the implementation, there might be an constrain on the input buffer size which
/// is fed into the resampler in order to process something. Outputs never should have such
/// constrains.
pub trait AudioResampler: Send + Sync {
    /// required or suggested input buffer length in order to process some output.
    fn required_input_buffer_size(&self) -> Option<usize>;
    /// maximum input buffer length constrain for processing, if there is some.
    fn max_input_buffer_size(&self) -> Option<usize>;

    /// process interleaved input samples to the given interleaved output buffers.
    /// Input buffer size must fit the given required_input_buffer_size constrain, if there is
    /// some. The very last processing call can use a zero length input in order to flush any
    /// pending buffered outputs - if any.
    /// returns ResamplerError or (input_consumed, output_written) on success.
    fn process(&mut self, input: &[f32], output: &mut [f32]) -> Result<(usize, usize), Error>;

    /// Reset internal resampler state. Make an existing resampler ready for a new source.
    fn reset(&mut self);
}
