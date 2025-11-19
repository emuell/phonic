use super::{Source, SourceTime};

// -------------------------------------------------------------------------------------------------

/// A source which changes the channel layout of some other source.
pub struct ChannelMappedSource<InputSource: Source + 'static> {
    source: InputSource,
    input_channels: usize,
    output_channels: usize,
    input_buffer: Vec<f32>,
}

impl<InputSource: Source + 'static> ChannelMappedSource<InputSource> {
    pub fn new(source: InputSource, output_channels: usize) -> Self
    where
        InputSource: Source,
    {
        const BUFFER_SIZE: usize = 256;
        let input_channels = source.channel_count();
        Self {
            source,
            input_channels,
            output_channels,
            input_buffer: vec![0.0; BUFFER_SIZE * input_channels],
        }
    }

    /// Access to the wrapped source.
    #[allow(unused)]
    pub fn input_source(&self) -> &InputSource {
        &self.source
    }
    /// Mut access to the wrapped source.
    pub fn input_source_mut(&mut self) -> &mut InputSource {
        &mut self.source
    }
}

impl<InputSource: Source + 'static> Source for ChannelMappedSource<InputSource> {
    fn write(&mut self, output: &mut [f32], time: &SourceTime) -> usize {
        if !output.is_empty() {
            let mut total_written = 0;
            while total_written < output.len() {
                // read as much input as we can to fill the entire output
                let input_max =
                    ((output.len() - total_written) / self.output_channels) * self.input_channels;
                let buffer_max = input_max.min(self.input_buffer.len());

                let source_time =
                    time.with_added_frames((total_written / self.output_channels) as u64);
                let written = self
                    .source
                    .write(&mut self.input_buffer[..buffer_max], &source_time);
                if written == 0 {
                    // source is exhausted
                    break;
                }

                // convert
                let input = &self.input_buffer[..written];
                let target = &mut output[total_written
                    ..total_written + (written / self.input_channels) * self.output_channels];

                let input_frames = input.chunks_exact(self.input_channels);
                let output_frames = target.chunks_exact_mut(self.output_channels);
                total_written += match self.input_channels {
                    1 => {
                        match self.output_channels {
                            1 => {
                                let mut written = 0_usize;
                                for (i, o) in input_frames.zip(output_frames) {
                                    o[0] = i[0];
                                    written += 1;
                                }
                                written
                            }
                            c => {
                                let mut written = 0_usize;
                                for (i, o) in input_frames.zip(output_frames) {
                                    o[0] = i[0];
                                    o[1] = i[0];
                                    // Assume the rest is is implicitly silence.
                                    written += c;
                                }
                                written
                            }
                        }
                    }
                    _ => {
                        match self.output_channels {
                            1 => {
                                let mut written = 0_usize;
                                for (i, o) in input_frames.zip(output_frames) {
                                    o[0] = i[0];
                                    written += 1;
                                }
                                written
                            }
                            c => {
                                let mut written = 0_usize;
                                for (i, o) in input_frames.zip(output_frames) {
                                    o[0] = i[0];
                                    o[1] = i[1];
                                    // Assume the rest is is implicitly silence.
                                    written += c;
                                }
                                written
                            }
                        }
                    }
                }
            }
            total_written
        } else {
            // pass empty buffers as they are, to process messages only
            self.source.write(output, time)
        }
    }

    fn channel_count(&self) -> usize {
        self.output_channels
    }

    fn sample_rate(&self) -> u32 {
        self.source.sample_rate()
    }

    fn is_exhausted(&self) -> bool {
        self.source.is_exhausted()
    }
}
