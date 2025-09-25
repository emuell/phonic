use crate::{
    utils::{
        buffer::{copy_buffers, TempBuffer},
        stretcher::{AudioTimeStretcher, TimeStretchingSpecs},
    },
    Error,
};

use signalsmith_stretch::Stretch;

// -------------------------------------------------------------------------------------------------

/// SignalSmith preset config
pub enum SignalSmithPreset {
    Default,
    Fast,
}

// -------------------------------------------------------------------------------------------------

/// SignalSmith-based audio time stretcher implementation.
pub struct SignalSmithTimeStretcher {
    speed: f64,
    stretcher: Stretch,
    channel_count: usize,
    output_buffer: Vec<f32>,
    pending_output: TempBuffer,
    pending_latency_frames: usize,
}

impl SignalSmithTimeStretcher {
    // The maximum number of input frames supported by this stretcher impl.
    const MAX_INPUT_FRAMES: usize = 1024;

    pub fn new(specs: TimeStretchingSpecs, preset: SignalSmithPreset) -> Result<Self, Error> {
        let speed = specs.speed;
        let sample_rate = specs.sample_rate;
        let channel_count = specs.channel_count;

        // create stretcher
        let stretcher = match preset {
            SignalSmithPreset::Default => {
                Stretch::preset_default(channel_count as u32, sample_rate)
            }
            SignalSmithPreset::Fast => Stretch::preset_cheaper(channel_count as u32, sample_rate),
        };

        // max output_latency to flush the remaining latency buffer after we got no more inputs
        let max_output_frames = ((Self::MAX_INPUT_FRAMES as f64 / speed).ceil() as usize)
            .max(stretcher.output_latency());
        let output_buffer = vec![0.0; max_output_frames * channel_count];

        let pending_output = TempBuffer::new(max_output_frames * channel_count);

        let pending_latency_frames =
            (stretcher.input_latency() as f64 / speed).ceil() as usize + stretcher.output_latency();

        Ok(Self {
            speed,
            stretcher,
            channel_count,
            output_buffer,
            pending_output,
            pending_latency_frames,
        })
    }
}

impl AudioTimeStretcher for SignalSmithTimeStretcher {
    fn max_input_buffer_size(&self) -> Option<usize> {
        Some(Self::MAX_INPUT_FRAMES * self.channel_count)
    }

    fn min_output_buffer_size(&self) -> Option<usize> {
        Some((Self::MAX_INPUT_FRAMES as f64 / self.speed).ceil() as usize * self.channel_count)
    }

    fn process(&mut self, input: &[f32], output: &mut [f32]) -> Result<(usize, usize), Error> {
        if self.speed == 1.0 {
            // Bypass conversion in case the ratio is 1.0
            let min = input.len().min(output.len());
            copy_buffers(&mut output[..min], &input[..min]);
            return Ok((min, min));
        }

        let mut input_consumed = 0;
        let mut output_written = 0;

        // First, output all pending data
        if !self.pending_output.is_empty() {
            let copied = self.pending_output.copy_to(&mut output[output_written..]);
            self.pending_output.consume(copied);
            output_written += copied;

            if output_written == output.len() {
                return Ok((input_consumed, output_written));
            }
        }

        // When the input is empty, we still need to flush all pending outputs
        if input.is_empty() {
            debug_assert!(self.output_buffer.capacity() >= self.stretcher.output_latency());
            debug_assert!(self.pending_output.capacity() >= self.stretcher.output_latency());

            // Prepare last output buffer
            self.output_buffer
                .resize(self.stretcher.output_latency() * self.channel_count, 0.0);

            // Process
            self.stretcher.flush(&mut self.output_buffer);

            let remaining_output_samples = output.len() - output_written;
            let samples_to_write = self.output_buffer.len();
            if samples_to_write <= remaining_output_samples {
                // Everything fits into the output buffer
                copy_buffers(
                    &mut output[output_written..output_written + samples_to_write],
                    &self.output_buffer,
                );
                output_written += samples_to_write;
            } else {
                // Not everything fits, use pending buffer.
                self.pending_output.set_range(0, samples_to_write);
                copy_buffers(self.pending_output.get_mut(), &self.output_buffer);

                let copied = self.pending_output.copy_to(&mut output[output_written..]);
                self.pending_output.consume(copied);
                output_written += copied;
            }
        } else {
            // Prepare output buffers
            let input_frames = (input.len() / self.channel_count).min(Self::MAX_INPUT_FRAMES);
            let output_frames_f = input_frames as f64 / self.speed;
            let output_frames = output_frames_f.ceil() as usize;

            self.output_buffer
                .resize(output_frames * self.channel_count, 0.0);

            // Process
            self.stretcher.process(input, &mut self.output_buffer);
            input_consumed = input_frames * self.channel_count;

            // Skip empty latency buffers with the first process calls
            let frames_to_skip = self.pending_latency_frames.min(output_frames);
            self.pending_latency_frames -= frames_to_skip;

            let frames_to_write = output_frames - frames_to_skip;
            if frames_to_write > 0 {
                // In-place move data to remove latency frames from the beginning and truncate.
                self.output_buffer.copy_within(
                    frames_to_skip * self.channel_count..output_frames * self.channel_count,
                    0,
                );
                self.output_buffer
                    .truncate(frames_to_write * self.channel_count);

                let remaining_output_samples = output.len() - output_written;
                let samples_to_write = frames_to_write * self.channel_count;
                if samples_to_write <= remaining_output_samples {
                    // Everything fits into the output buffer
                    copy_buffers(
                        &mut output[output_written..output_written + samples_to_write],
                        &self.output_buffer,
                    );
                    output_written += samples_to_write;
                } else {
                    // Not everything fits, use pending buffer.
                    self.pending_output.set_range(0, samples_to_write);
                    copy_buffers(self.pending_output.get_mut(), &self.output_buffer);

                    let copied = self.pending_output.copy_to(&mut output[output_written..]);
                    self.pending_output.consume(copied);
                    output_written += copied;
                }
            }
        }

        Ok((input_consumed, output_written))
    }
}

// -------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::AudioTimeStretcher;
    use super::*;
    use crate::utils::buffer::InterleavedBufferMut;

    #[test]
    fn test_signalsmith_stretcher() {
        const SPEED: f64 = 0.7;
        const CHANNEL_COUNT: usize = 2;
        const FRAME_COUNT: usize = 1024;

        // Test stereo resampling at 44.1kHz with the given speed
        let specs = TimeStretchingSpecs::new(SPEED, 44100, 2);
        let mut resampler =
            SignalSmithTimeStretcher::new(specs, SignalSmithPreset::Default).unwrap();

        // Create a simple sine wave for testing
        let mut input_buffer = vec![0.0f32; CHANNEL_COUNT * FRAME_COUNT];
        for (i, frame) in input_buffer
            .as_frames_mut::<CHANNEL_COUNT>()
            .iter_mut()
            .enumerate()
        {
            frame[0] = (i as f32 * 0.1).sin();
            frame[1] = (i as f32 * 0.1).sin();
        }

        // Estimated output size
        let mut output_buffer =
            vec![0.0f32; ((CHANNEL_COUNT * FRAME_COUNT) as f64 / SPEED).ceil() as usize];

        // Process until we receive some output
        let (mut input_consumed, mut output_written);

        loop {
            (input_consumed, output_written) = resampler
                .process(&input_buffer, &mut output_buffer)
                .unwrap();

            assert!(input_consumed >= input_buffer.len());
            if output_written > 0 {
                break;
            }
        }

        // Verify the output buffer received some data
        assert!(
            output_buffer.iter().any(|s| s.abs() > 0.1),
            "Output buffer should contain non-zero data"
        );
    }
}
