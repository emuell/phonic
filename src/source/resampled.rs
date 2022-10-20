use super::{AudioSource, AudioSourceTime};

use crate::utils::{
    buffer::TempBuffer,
    resampler::{cubic::CubicResampler, rubato::RubatoResampler, AudioResampler, ResamplingSpecs},
};

// -------------------------------------------------------------------------------------------------

/// The resampler that ResampledSource should use for resampling.
#[derive(Copy, Clone, PartialEq, PartialOrd, Debug)]
pub enum ResamplingQuality {
    /// simple and fast, non bandlimited cubic interpolation. Your daily workhorse for e.g. real-time
    /// sampler playback or when CPU resources are are a problem. Downsampling may cause aliasing.
    Default,
    /// HQ resampling performed via bandlimited `rubato` resampler. When only playing back a few
    /// audio files at once or aliasing is a problem, use this mode for best results.
    HighQuality,
}

// -------------------------------------------------------------------------------------------------

/// A source which resamples the input source to adjust the source's sample rate.
pub struct ResampledSource {
    source: Box<dyn AudioSource>,
    output_sample_rate: u32,
    resampler: Box<dyn AudioResampler>,
    input_buffer: TempBuffer,
    output_buffer: TempBuffer,
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
        let resampler_specs = ResamplingSpecs::new(
            source.sample_rate(),
            (output_sample_rate as f64 / speed) as u32,
            source.channel_count(),
        );
        let resampler: Box<dyn AudioResampler> = match quality {
            ResamplingQuality::HighQuality => Box::new(
                RubatoResampler::new(resampler_specs)
                    .expect("Failed to create new rubato resampler instance"),
            ),
            ResamplingQuality::Default => Box::new(
                CubicResampler::new(resampler_specs)
                    .expect("Failed to create new cubic resampler instance"),
            ),
        };
        const DEFAULT_CHUNK_SIZE: usize = 512;
        let input_buffer_len = resampler
            .max_input_buffer_size()
            .unwrap_or(DEFAULT_CHUNK_SIZE);
        let output_buffer_len = DEFAULT_CHUNK_SIZE;
        Self {
            source: Box::new(source),
            resampler,
            output_sample_rate,
            input_buffer: TempBuffer::new(input_buffer_len),
            output_buffer: TempBuffer::new(output_buffer_len),
        }
    }
}

impl AudioSource for ResampledSource {
    fn write(&mut self, output: &mut [f32], time: &AudioSourceTime) -> usize {
        let mut total_written = 0;
        while total_written < output.len() {
            if self.output_buffer.is_empty() {
                self.output_buffer.reset_range();
                // when there's no input, try fetch some from our source
                if self.input_buffer.is_empty() {
                    let source_time = time
                        .with_added_frames((total_written / self.source.channel_count()) as u64);
                    self.input_buffer.reset_range();
                    let input_read = self.source.write(self.input_buffer.get_mut(), &source_time);

                    // fill up with zeros if resampler needs more samples
                    if let Some(required_input_len) = self.resampler.required_input_buffer_size() {
                        if self.input_buffer.len() < required_input_len {
                            self.input_buffer.set_range(0, required_input_len);
                            for o in &mut self.input_buffer.get_mut()[input_read..] {
                                *o = 0.0;
                            }
                        }
                    }
                }
                // run resampler to generate some output
                let (input_consumed, output_written) = self
                    .resampler
                    .process(self.input_buffer.get(), self.output_buffer.get_mut())
                    .expect("Resampling failed");
                self.input_buffer.consume(input_consumed);
                self.output_buffer.set_range(0, output_written);
                if output_written == 0 {
                    // resampler produced no more output: we're done
                    break;
                }
            }
            let target = &mut output[total_written..];
            let written = self.output_buffer.copy_to(target);
            self.output_buffer.consume(written);
            total_written += written;
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
        self.source.is_exhausted() && self.input_buffer.is_empty() && self.output_buffer.is_empty()
    }
}
