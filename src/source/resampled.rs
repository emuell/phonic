use super::{AudioSource, AudioSourceTime};

use crate::utils::resampler::{
    cubic::CubicResampler, rubato::RubatoResampler, AudioResampler, ResamplingSpecs,
};

// -------------------------------------------------------------------------------------------------

/// The resampler that ResampledSource should use for resampling.
pub enum ResamplingQuality {
    /// simple and fast, non bandlimited cubic interpolation. Your daily workhorse for e.g. real-time
    /// sampler playback or when CPU resources are are a problem. Downsampling may cause aliasing.
    Fast,
    /// HQ resampling performed via bandlimited `rubato` resampler. When only playing back a few
    /// audio files at once or aliasing is a problem, use this mode for best results.
    HighQuality,
}

// -------------------------------------------------------------------------------------------------

/// A source which resamples the input source, either to adjust source's sample rate to a
/// target rate or to play back a source with a different pitch.
pub struct ResampledSource {
    source: Box<dyn AudioSource>,
    output_sample_rate: u32,
    resampler: Box<dyn AudioResampler>,
    input_buffer: ResampleBuffer,
    output_buffer: ResampleBuffer,
}

impl ResampledSource {
    /// Create a new resampled sources with the given sample rate adjustment.
    pub fn new<InputSource>(
        source: InputSource,
        output_sample_rate: u32,
        quality: ResamplingQuality,
    ) -> Self
    where
        InputSource: AudioSource,
    {
        Self::new_with_speed(source, output_sample_rate, 1.0, quality)
    }
    /// Create a new resampled sources with the given sample rate and playback speed adjument.
    pub fn new_with_speed<InputSource>(
        source: InputSource,
        output_sample_rate: u32,
        speed: f64,
        quality: ResamplingQuality,
    ) -> Self
    where
        InputSource: AudioSource,
    {
        let specs = ResamplingSpecs::new(
            source.sample_rate(),
            (output_sample_rate as f64 / speed) as u32,
            source.channel_count(),
        );
        let resampler: Box<dyn AudioResampler> = match quality {
            ResamplingQuality::HighQuality => Box::new(
                RubatoResampler::new(specs)
                    .expect("Failed to create new rubato resampler instance"),
            ),
            ResamplingQuality::Fast => Box::new(
                CubicResampler::new(specs).expect("Failed to create new cubic resampler instance"),
            ),
        };
        let input_buffer = vec![0.0; resampler.input_buffer_len()];
        let output_buffer = vec![0.0; resampler.output_buffer_len()];
        Self {
            source: Box::new(source),
            resampler,
            output_sample_rate,
            input_buffer: ResampleBuffer {
                buffer: input_buffer,
                start: 0,
                end: 0,
            },
            output_buffer: ResampleBuffer {
                buffer: output_buffer,
                start: 0,
                end: 0,
            },
        }
    }
}

impl AudioSource for ResampledSource {
    fn write(&mut self, output: &mut [f32], time: &AudioSourceTime) -> usize {
        let mut total_written = 0;
        while total_written < output.len() {
            if self.output_buffer.is_empty() {
                // when there's no input, try fetch some from our source
                if self.input_buffer.is_empty() {
                    let source_time = AudioSourceTime {
                        pos_in_frames: time.pos_in_frames
                            + (total_written / self.source.channel_count()) as u64,
                    };
                    let input_read = self
                        .source
                        .write(&mut self.input_buffer.buffer, &source_time);
                    self.input_buffer.buffer[input_read..]
                        .iter_mut()
                        .for_each(|s| *s = 0.0);
                    self.input_buffer.start = 0;
                    self.input_buffer.end = self.input_buffer.buffer.len();
                }
                // run resampler to generate some output
                let (input_consumed, output_written) = self
                    .resampler
                    .process(
                        &self.input_buffer.buffer[self.input_buffer.start..],
                        &mut self.output_buffer.buffer,
                    )
                    .expect("Resampling failed");
                self.input_buffer.start += input_consumed;
                self.output_buffer.start = 0;
                self.output_buffer.end = output_written;
                if output_written == 0 {
                    // resampler produced no more output: we're done
                    break;
                }
            }
            let source = self.output_buffer.get();
            let target = &mut output[total_written..];
            let to_write = self.output_buffer.len().min(target.len());
            target[..to_write].copy_from_slice(&source[..to_write]);
            total_written += to_write;
            self.output_buffer.start += to_write;
        }
        total_written
    }

    fn channel_count(&self) -> usize {
        self.source.channel_count()
    }

    fn sample_rate(&self) -> u32 {
        self.output_sample_rate
    }

    fn is_exhausted(&self) -> bool {
        self.source.is_exhausted() && self.input_buffer.is_empty()
    }
}

// -------------------------------------------------------------------------------------------------

struct ResampleBuffer {
    buffer: Vec<f32>,
    start: usize,
    end: usize,
}

impl ResampleBuffer {
    fn get(&self) -> &[f32] {
        &self.buffer[self.start..self.end]
    }

    fn len(&self) -> usize {
        self.end - self.start
    }

    fn is_empty(&self) -> bool {
        self.start >= self.end
    }
}
