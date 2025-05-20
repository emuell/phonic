use super::{AudioResampler, ResamplingSpecs};
use crate::Error;

// -------------------------------------------------------------------------------------------------

/// Interpolate a single channel of interleaved audio with cubic interpolation.
#[derive(Clone)]
struct CubicInterpolator {
    input: [f32; 4],
    sub_pos: f32,
    ratio: f32,
    is_initialized: bool,
}

impl CubicInterpolator {
    pub fn new(ratio: f32) -> Self {
        let input = [0.0, 0.0, 0.0, 0.0];
        let sub_pos = 0.0;
        let is_initialized = false;
        Self {
            input,
            sub_pos,
            ratio,
            is_initialized,
        }
    }

    pub fn reset(&mut self) {
        self.input.fill(0.0);
        self.sub_pos = 0.0;
        self.is_initialized = false;
    }

    pub fn process(
        &mut self,
        input: &[f32],
        output: &mut [f32],
        channel_index: usize,
        channel_count: usize,
    ) -> (usize, usize) {
        debug_assert!(input.len() % channel_count == 0);
        debug_assert!(output.len() % channel_count == 0);

        let num_in = input.len() / channel_count;
        let num_out = output.len() / channel_count;

        let mut num_consumed = 0;
        let mut num_produced = 0;

        if (self.ratio - 1.0).abs() < 0.000001 {
            // Bypass conversion in case the sample rates are equal.
            let min = input.len().min(output.len());
            output[..min].copy_from_slice(&input[..min]);
            return (min, min);
        }

        // preload our input buffer
        if !self.is_initialized && input.len() >= 3 {
            self.is_initialized = true;
            for f in 0..3 {
                unsafe {
                    self.push_sample(*input.get_unchecked(f * channel_count + channel_index));
                }
                num_consumed += 1;
            }
        }

        // downsample
        if self.ratio < 1.0 {
            while num_produced < num_out {
                if self.sub_pos >= 1.0 {
                    if num_consumed == num_in {
                        break;
                    }
                    unsafe {
                        self.push_sample(
                            *input.get_unchecked(num_consumed * channel_count + channel_index),
                        );
                    }
                    num_consumed += 1;
                    self.sub_pos -= 1.0;
                }

                unsafe {
                    *output.get_unchecked_mut(num_produced * channel_count + channel_index) =
                        self.interpolate(self.sub_pos);
                }
                num_produced += 1;
                self.sub_pos += self.ratio;
            }
        }
        // upsample
        else {
            'outer_loop: while num_produced < num_out {
                while self.sub_pos < self.ratio {
                    if num_consumed == num_in {
                        break 'outer_loop;
                    }
                    unsafe {
                        self.push_sample(
                            *input.get_unchecked(num_consumed * channel_count + channel_index),
                        );
                    }
                    num_consumed += 1;
                    self.sub_pos += 1.0;
                }

                self.sub_pos -= self.ratio;
                unsafe {
                    *output.get_unchecked_mut(num_produced * channel_count + channel_index) =
                        self.interpolate(1.0 - self.sub_pos);
                }
                num_produced += 1;
            }
        }

        (num_consumed * channel_count, num_produced * channel_count)
    }

    #[inline]
    fn push_sample(&mut self, new_value: f32) {
        self.input[3] = self.input[2];
        self.input[2] = self.input[1];
        self.input[1] = self.input[0];
        self.input[0] = new_value;
    }

    #[inline]
    fn interpolate(&self, fraction: f32) -> f32 {
        debug_assert!((0.0..=1.0).contains(&fraction));

        // Given a previous frame, a current frame, the two next frames, and a fraction from
        // 0.0 to 1.0 between the current frame and next frame, get an approximated frame.
        // This is the 4-point, 3rd-order Hermite interpolation x-form algorithm from "Polynomial
        // Interpolators for High-Quality Resampling of Oversampled Audio" by Olli Niemitalo, p. 43:
        // http://yehar.com/blog/wp-content/uploads/2009/08/deip.pdf
        let ym1 = self.input[3];
        let y0 = self.input[2];
        let y1 = self.input[1];
        let y2 = self.input[0];
        let c0 = y0;
        let c1 = (y1 - ym1) * 0.5;
        let c2 = ym1 - y0 * 2.5 + y1 * 2.0 - y2 * 0.5;
        let c3 = (y2 - ym1) * 0.5 + (y0 - y1) * 1.5;
        ((c3 * fraction + c2) * fraction + c1) * fraction + c0
    }
}

// -------------------------------------------------------------------------------------------------

/// Simple cubic interpolater without bandlimiting. Designed to sound good while being fast and
/// not necessarily as HQ as possible. Suitable for samplers which are playing loads of samples at
/// the same time.
pub struct CubicResampler {
    spec: ResamplingSpecs,
    interpolators: Vec<CubicInterpolator>,
}

impl CubicResampler {
    pub fn new(spec: ResamplingSpecs) -> Result<Self, Error> {
        Ok(Self {
            spec,
            interpolators: vec![
                CubicInterpolator::new(spec.input_ratio() as f32);
                spec.channel_count
            ],
        })
    }
}

impl AudioResampler for CubicResampler {
    fn required_input_buffer_size(&self) -> Option<usize> {
        None
    }
    fn max_input_buffer_size(&self) -> Option<usize> {
        None
    }

    fn process(&mut self, input: &[f32], output: &mut [f32]) -> Result<(usize, usize), Error> {
        let channel_count = self.spec.channel_count;
        let mut result = (0, 0);
        for (channel_index, interpolator) in self.interpolators.iter_mut().enumerate() {
            result = interpolator.process(input, output, channel_index, channel_count);
        }
        Ok(result)
    }

    fn reset(&mut self) {
        for interpolator in self.interpolators.iter_mut() {
            interpolator.reset();
        }
    }
}
