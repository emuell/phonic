use crate::{
    utils::{
        buffer::{clear_buffer, TempBuffer},
        resampler::{
            cubic::CubicResampler, rubato::RubatoResampler, AudioResampler, ResamplingSpecs,
        },
    },
    Source, SourceTime,
};

// -------------------------------------------------------------------------------------------------

/// The resampler quality an audio source can be resampled with.
#[derive(Copy, Clone, PartialEq, PartialOrd, Debug)]
pub enum ResamplingQuality {
    /// Simple and fast, non bandlimited cubic interpolation. Your daily workhorse for e.g. real-time
    /// sampler playback or when CPU resources are are a problem. Downsampling may cause aliasing.
    Default,
    /// HQ resampling performed via bandlimited `rubato` resampler. When only playing back a few
    /// audio files at once or aliasing is a problem, use this mode for best results.
    HighQuality,
}

// -------------------------------------------------------------------------------------------------

/// A source which resamples the input source to adjust the source's sample rate.
pub struct ResampledSource<InputSource: Source + 'static> {
    source: InputSource,
    output_sample_rate: u32,
    resampler: Option<Box<dyn AudioResampler>>,
    input_buffer: TempBuffer,
    output_buffer: TempBuffer,
}

impl<InputSource: Source + 'static> ResampledSource<InputSource> {
    /// Create a new resampled sources with the given sample rate adjustment.
    pub fn new(source: InputSource, output_sample_rate: u32, quality: ResamplingQuality) -> Self
    where
        InputSource: Source,
    {
        Self::new_with_speed(source, output_sample_rate, 1.0, quality)
    }
    /// Create a new resampled sources with the given sample rate and playback speed adjustment.
    pub fn new_with_speed(
        source: InputSource,
        output_sample_rate: u32,
        speed: f64,
        quality: ResamplingQuality,
    ) -> Self
    where
        InputSource: Source,
    {
        let resampler_specs = ResamplingSpecs::new(
            source.sample_rate(),
            (output_sample_rate as f64 / speed) as u32,
            source.channel_count(),
        );
        let resampler: Option<Box<dyn AudioResampler>> =
            if resampler_specs.input_rate == resampler_specs.output_rate {
                None
            } else {
                match quality {
                    ResamplingQuality::HighQuality => Some(Box::new(
                        RubatoResampler::new(resampler_specs)
                            .expect("Failed to create new rubato resampler instance"),
                    )),
                    ResamplingQuality::Default => Some(Box::new(
                        CubicResampler::new(resampler_specs)
                            .expect("Failed to create new cubic resampler instance"),
                    )),
                }
            };
        const DEFAULT_CHUNK_SIZE: usize = 512;
        let input_buffer_len = if let Some(resampler) = &resampler {
            resampler
                .max_input_buffer_size()
                .unwrap_or(DEFAULT_CHUNK_SIZE * source.channel_count())
        } else {
            0
        };
        let input_buffer = TempBuffer::new(input_buffer_len);

        let output_buffer_len = if resampler.is_some() {
            DEFAULT_CHUNK_SIZE * source.channel_count()
        } else {
            0
        };
        let output_buffer = TempBuffer::new(output_buffer_len);

        Self {
            source,
            resampler,
            output_sample_rate,
            input_buffer,
            output_buffer,
        }
    }
}

impl<InputSource: Source + 'static> Source for ResampledSource<InputSource> {
    fn write(&mut self, output: &mut [f32], time: &SourceTime) -> usize {
        if let Some(resampler) = self.resampler.as_deref_mut() {
            if !output.is_empty() {
                let mut total_written = 0;
                while total_written < output.len() {
                    if self.output_buffer.is_empty() {
                        self.output_buffer.reset_range();
                        // fetch new input from our source
                        if self.input_buffer.is_empty() {
                            let source_time = time.with_added_frames(
                                (total_written / self.source.channel_count()) as u64,
                            );
                            self.input_buffer.reset_range();
                            let input_read =
                                self.source.write(self.input_buffer.get_mut(), &source_time);
                            // pad, fill up up missing inputs with zeros if the resampler has an input buffer constrain
                            // this should only happen for exhausted sources...
                            if input_read < self.input_buffer.len() {
                                let required_input_len =
                                    resampler.required_input_buffer_size().unwrap_or(0);
                                if self.input_buffer.len() < required_input_len {
                                    self.input_buffer.set_range(0, required_input_len);
                                    clear_buffer(&mut self.input_buffer.get_mut()[input_read..]);
                                }
                            }
                        }
                        // run resampler to generate some output
                        let (input_consumed, output_written) = resampler
                            .process(self.input_buffer.get(), self.output_buffer.get_mut())
                            .expect("Resampling failed");
                        self.input_buffer.consume(input_consumed);
                        self.output_buffer.set_range(0, output_written);
                        if self.source.is_exhausted() && output_written == 0 {
                            // source and resampler produced no more output: we're done
                            break;
                        }
                    }
                    let target = &mut output[total_written..];
                    let written = self.output_buffer.copy_to(target);
                    self.output_buffer.consume(written);
                    total_written += written;
                }
                total_written
            } else {
                // pass empty buffers as they are, to process messages only
                self.source.write(output, time)
            }
        } else {
            // no resampling needed, just process the source as it is
            self.source.write(output, time)
        }
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

    fn weight(&self) -> usize {
        let resampler_weight = if self.resampler.is_some() { 1 } else { 0 };
        self.source.weight() + resampler_weight
    }
}
