use super::{AudioSource, AudioSourceTime};

// -------------------------------------------------------------------------------------------------

/// A source which changes the channel layout of some other source.
pub struct ChannelMappedSource {
    source: Box<dyn AudioSource>,
    input_channels: usize,
    output_channels: usize,
    input_buffer: Vec<f32>,
}

impl ChannelMappedSource {
    pub fn new<InputSource>(source: InputSource, output_channels: usize) -> Self
    where
        InputSource: AudioSource,
    {
        const BUFFER_SIZE: usize = 256;
        let input_channels = source.channel_count();
        Self {
            source: Box::new(source),
            input_channels,
            output_channels,
            input_buffer: vec![0.0; BUFFER_SIZE * input_channels],
        }
    }
}

impl AudioSource for ChannelMappedSource {
    fn write(&mut self, output: &mut [f32], time: &AudioSourceTime) -> usize {
        let mut total_written = 0;
        while total_written < output.len() {
            // read as much input as we can to fill the entire output
            let input_max =
                ((output.len() - total_written) / self.output_channels) * self.input_channels;
            let buffer_max = input_max.min(self.input_buffer.len());

            let source_time = AudioSourceTime::with_frames_added(
                time,
                (total_written / self.output_channels) as u64,
            );
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
