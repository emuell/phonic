use rubato::{SincFixedIn, SincInterpolationParameters, SincInterpolationType, VecResampler};

use super::{AudioResampler, ResamplingSpecs};

use crate::{
    utils::buffer::{interleaved_to_planar, planar_to_interleaved, TempBuffer},
    Error,
};

// -------------------------------------------------------------------------------------------------

/// `AudioResampler` impl which is using a bandlimited SincFixedIn `rubato` resampler.
pub struct RubatoResampler {
    spec: ResamplingSpecs,
    resampler: SincFixedIn<f32>,
    input: Vec<Vec<f32>>,
    output: Vec<Vec<f32>>,
    pending: TempBuffer,
}

impl RubatoResampler {
    pub fn new(spec: ResamplingSpecs) -> Result<Self, Error> {
        const CHUNK_SIZE: usize = 256;
        let parameters = SincInterpolationParameters {
            f_cutoff: 0.95,
            interpolation: SincInterpolationType::Cubic,
            oversampling_factor: 128,
            sinc_len: 256,
            window: rubato::WindowFunction::BlackmanHarris2,
        };
        match SincFixedIn::new(
            spec.output_ratio(),
            1.0,
            parameters,
            CHUNK_SIZE,
            spec.channel_count,
        ) {
            Err(err) => Err(Error::ResamplingError(Box::new(err))),
            Ok(resampler) => {
                let input = resampler.input_buffer_allocate(true);
                let output = resampler.output_buffer_allocate(true);
                let pending = TempBuffer::new(spec.channel_count * resampler.output_frames_max());
                Ok(Self {
                    resampler,
                    spec,
                    input,
                    output,
                    pending,
                })
            }
        }
    }
}

impl AudioResampler for RubatoResampler {
    fn required_input_buffer_size(&self) -> Option<usize> {
        Some(self.resampler.input_frames_next() * self.spec.channel_count)
    }
    fn max_input_buffer_size(&self) -> Option<usize> {
        Some(self.resampler.input_frames_max() * self.spec.channel_count)
    }

    fn process(&mut self, input: &[f32], output: &mut [f32]) -> Result<(usize, usize), Error> {
        debug_assert!(
            input.is_empty() || input.len() >= self.required_input_buffer_size().unwrap(),
            "invalid input buffer specs"
        );

        if self.spec.input_rate == self.spec.output_rate {
            // Bypass conversion in case the sample rates are equal.
            let min = input.len().min(output.len());
            output[..min].copy_from_slice(&input[..min]);
            return Ok((min, min));
        }

        // flush pending outs
        if !self.pending.is_empty() {
            let input_consumed = 0;
            let output_written = self.pending.copy_to(output);
            self.pending.consume(output_written);
            return Ok((input_consumed, output_written));
        }

        // when there is no more pending output and no more input we're done
        if input.is_empty() {
            return Ok((0, 0));
        }

        // convert inputs to planar
        interleaved_to_planar(input, &mut self.input);
        // reset outputs
        for output_channel in self.output.iter_mut() {
            output_channel.resize(output_channel.capacity(), 0.0);
        }
        // resample and convert and memorize outputs
        match self
            .resampler
            .process_into_buffer(&self.input, &mut self.output, None)
        {
            Err(err) => Err(Error::ResamplingError(Box::new(err))),
            Ok((consumed_input_frames, generated_output_frames)) => {
                assert!(consumed_input_frames == self.input[0].len());
                assert!(generated_output_frames <= self.output[0].len());
                // resize output to apply the currently generated output frames
                for output_channel in self.output.iter_mut() {
                    output_channel.resize(generated_output_frames, 0.0);
                }
                if self.output.len() * self.output[0].len() > output.len() {
                    // copy what fits to output, store rest into pending
                    self.pending
                        .set_range(0, self.output.len() * self.output[0].len());
                    planar_to_interleaved(&self.output, self.pending.get_mut());

                    let input_consumed = self.input.len() * self.input[0].len();
                    let output_written = self.pending.copy_to(output);
                    self.pending.consume(output_written);

                    Ok((input_consumed, output_written))
                } else {
                    // copy entire result to output
                    planar_to_interleaved(&self.output, output);

                    let input_consumed = self.input.len() * self.input[0].len();
                    let output_written = self.output.len() * self.output[0].len();

                    Ok((input_consumed, output_written))
                }
            }
        }
    }

    fn update(&mut self, input_rate: u32, output_rate: u32) -> Result<(), Error> {
        self.spec.input_rate = input_rate;
        self.spec.output_rate = output_rate;
        let new_ratio = self.spec.output_ratio();
        let ramp = false;
        self.resampler
            .set_resample_ratio(new_ratio, ramp)
            .map_err(|err| Error::ResamplingError(Box::new(err)))
    }

    fn reset(&mut self) {
        // there's no reset functionality in rubato
    }
}

unsafe impl Send for RubatoResampler {}
unsafe impl Sync for RubatoResampler {}
