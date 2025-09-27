use crate::{
    utils::{
        buffer::{copy_buffers, interleaved_to_planar, planar_to_interleaved, TempBuffer},
        stretcher::{AudioTimeStretcher, TimeStretchingSpecs},
    },
    Error,
};

use bungee_rs::Stream;

// -------------------------------------------------------------------------------------------------

/// Bungee-based audio resampler implementation.
/// This uses the Bungee audio stretching library to perform sample rate conversion
/// by treating resampling as a time-stretching operation with pitch compensation.
pub struct BungeeTimeStretcher {
    speed: f64,
    channel_count: usize,
    /// Bungee stream processor
    stream: Stream,
    // Pre-allocated buffers to avoid allocations in the process method.
    planar_input: Vec<Vec<f32>>,
    planar_output: Vec<Vec<f32>>,
    pending_output: TempBuffer,
    pending_latency_frames: Option<usize>,
    remaining_latency_frames: Option<usize>,
}

impl BungeeTimeStretcher {
    // The maximum number of input frames supported by this stretcher impl.
    const MAX_INPUT_FRAMES: usize = 1024;

    pub fn new(specs: TimeStretchingSpecs) -> Result<Self, Error> {
        let speed = specs.speed;
        let sample_rate = specs.sample_rate;
        let channel_count = specs.channel_count;

        // Initialize the stretcher with the output sample rate
        let stream = Stream::new(sample_rate as usize, channel_count, Self::MAX_INPUT_FRAMES)
            .map_err(|err| {
                Error::ResamplingError(format!("Failed to create stretcher stream: {err}").into())
            })?;

        let planar_input = vec![vec![0.0; Self::MAX_INPUT_FRAMES]; channel_count];

        // Pre-allocate with a bit of margin
        let max_output_frames = (Self::MAX_INPUT_FRAMES as f64 / speed).ceil() as usize + 16;
        let planar_output = vec![vec![0.0; max_output_frames]; channel_count];
        let pending_output = TempBuffer::new(max_output_frames * channel_count);
        let pending_latency_frames = None;
        let remaining_latency_frames = None;

        Ok(Self {
            speed,
            channel_count,
            stream,
            planar_input,
            planar_output,
            pending_output,
            pending_latency_frames,
            remaining_latency_frames,
        })
    }
}

impl AudioTimeStretcher for BungeeTimeStretcher {
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

        // Process new input or flush outputs only
        let process_outputs_only = input.is_empty();

        let (input_frames, output_frames_f) = if process_outputs_only {
            // flush pending output only when inputs are silent
            let remaining_frames = *self
                .remaining_latency_frames
                .get_or_insert_with(|| self.stream.latency().ceil() as usize);

            let input_frames = remaining_frames.min(Self::MAX_INPUT_FRAMES);
            let output_frames_f = input_frames as f64 / self.speed;

            self.remaining_latency_frames
                .replace(remaining_frames - input_frames);

            (input_frames, output_frames_f)
        } else {
            // process inputs and outputs
            let input_frames = (input.len() / self.channel_count).min(Self::MAX_INPUT_FRAMES);
            let output_frames_f = input_frames as f64 / self.speed;
            (input_frames, output_frames_f)
        };

        // skip processing when there's no input and output is completely flushed
        if input_frames == 0 {
            return Ok((0, 0));
        }

        // Prepare input buffers
        if !process_outputs_only {
            for ch in self.planar_input.iter_mut() {
                debug_assert!(ch.capacity() >= input_frames);
                ch.resize(input_frames, 0.0);
            }
            interleaved_to_planar(input, &mut self.planar_input);
        }

        // Prepare output buffers
        let max_output_frames = output_frames_f.ceil() as usize;
        for ch in self.planar_output.iter_mut() {
            debug_assert!(ch.capacity() >= max_output_frames);
            ch.resize(max_output_frames, 0.0);
        }

        // Process on prepared planar data
        if !process_outputs_only {
            assert!(
                self.planar_input[0].len() >= input_frames,
                "Temporary input buffer is too small"
            );
        }
        assert!(
            self.planar_output[0].len() >= output_frames_f.ceil() as usize,
            "Temporary output buffer is too small"
        );
        let frames_processed = self.stream.process(
            if input.is_empty() {
                None
            } else {
                Some(&self.planar_input)
            },
            &mut self.planar_output,
            input_frames,
            output_frames_f,
            1.0, // pitch
        );
        if !process_outputs_only {
            input_consumed = input_frames * self.channel_count;
        }

        // Initialize latency after the first process call
        let pending_latency_frames = *self
            .pending_latency_frames
            .get_or_insert_with(|| (self.stream.latency() / self.speed).ceil() as usize);

        if frames_processed > 0 {
            // skip empty latency buffers with the first process calls
            let frames_to_skip = pending_latency_frames.min(frames_processed);
            self.pending_latency_frames
                .replace(pending_latency_frames - frames_to_skip);

            let frames_to_write = frames_processed - frames_to_skip;
            if frames_to_write > 0 {
                // In-place move data to remove latency frames from the beginning and truncate.
                for ch in self.planar_output.iter_mut() {
                    ch.copy_within(frames_to_skip..frames_processed, 0);
                    ch.truncate(frames_to_write);
                }

                let remaining_output_samples = output.len() - output_written;
                let samples_to_write = frames_to_write * self.channel_count;
                if samples_to_write <= remaining_output_samples {
                    // Everything fits into the output buffer
                    planar_to_interleaved(
                        &self.planar_output,
                        &mut output[output_written..output_written + samples_to_write],
                    );
                    output_written += samples_to_write;
                } else {
                    // Not everything fits, use pending buffer.
                    self.pending_output.set_range(0, samples_to_write);
                    planar_to_interleaved(&self.planar_output, self.pending_output.get_mut());

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
    fn test_bungee_resampler() {
        const SPEED: f64 = 0.7;
        const CHANNEL_COUNT: usize = 2;
        const FRAME_COUNT: usize = 1024;

        // Test stereo resampling at 44.1kHz with the given speed
        let specs = TimeStretchingSpecs::new(SPEED, 44100, 2);
        let mut resampler = BungeeTimeStretcher::new(specs).unwrap();

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
