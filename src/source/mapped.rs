use super::AudioSource;

// -------------------------------------------------------------------------------------------------

/// A source which changes the channel layout
pub struct ChannelMappedSource {
    source: Box<dyn AudioSource>,
    input_channels: usize,
    output_channels: usize,
    buffer: Vec<f32>,
}

impl ChannelMappedSource {
    pub fn new<InputSource>(source: InputSource, output_channels: usize) -> Self
    where
        InputSource: AudioSource,
    {
        const BUFFER_SIZE: usize = 16 * 1024;
        let input_channels = source.channel_count();
        Self {
            source: Box::new(source),
            input_channels,
            output_channels,
            buffer: vec![0.0; BUFFER_SIZE],
        }
    }
}

impl AudioSource for ChannelMappedSource {
    fn write(&mut self, output: &mut [f32]) -> usize {
        let input_max = (output.len() / self.output_channels) * self.input_channels;
        let buffer_max = input_max.min(self.buffer.len());
        let written = self.source.write(&mut self.buffer[..buffer_max]);
        let input = &self.buffer[..written];
        let input_frames = input.chunks_exact(self.input_channels);
        let output_frames = output.chunks_exact_mut(self.output_channels);
        match self.input_channels {
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

    fn channel_count(&self) -> usize {
        self.output_channels
    }

    fn sample_rate(&self) -> u32 {
        self.source.sample_rate()
    }
}
