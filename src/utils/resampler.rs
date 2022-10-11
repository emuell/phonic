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

    pub fn channel_count(&self) -> usize {
        self.channel_count
    }
}

// -------------------------------------------------------------------------------------------------

/// Audio resampler interface.
pub trait AudioResampler: Send + Sync {
    /// required or suggested input buffer length for processing.
    fn input_buffer_len(&self) -> usize;
    /// required or suggested output buffer length for processing.
    fn output_buffer_len(&self) -> usize;

    /// process interleaved input samples to the given interleaved output buffers.
    /// returns ResamplerError or (input_consumed, output_written) on success.
    fn process(&mut self, input: &[f32], output: &mut [f32]) -> Result<(usize, usize), Error>;
}
