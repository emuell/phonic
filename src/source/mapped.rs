use crate::utils::buffer::remap_buffer_channels;

use super::{mixed::MixedSource, Source, SourceTime};

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
        let input_channels = source.channel_count();
        assert!(input_channels != 0, "Input channel count must be > 0");
        assert!(output_channels != 0, "Output channel count must be > 0");

        let buffer_size = MixedSource::MAX_MIX_BUFFER_SAMPLES / output_channels * input_channels;
        Self {
            source,
            input_channels,
            output_channels,
            input_buffer: vec![0.0; buffer_size],
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
    fn channel_count(&self) -> usize {
        self.output_channels
    }

    fn sample_rate(&self) -> u32 {
        self.source.sample_rate()
    }

    fn is_exhausted(&self) -> bool {
        self.source.is_exhausted()
    }

    fn weight(&self) -> usize {
        self.source.weight()
    }

    fn write(&mut self, output: &mut [f32], time: &SourceTime) -> usize {
        if output.is_empty() || self.input_channels == self.output_channels {
            // no mapping needed, or pass empty buffers as they are to process messages only
            self.source.write(output, time)
        } else {
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
                let chunk_input = &self.input_buffer[..written];
                let chunk_output = &mut output[total_written
                    ..total_written + (written / self.input_channels) * self.output_channels];

                remap_buffer_channels(
                    chunk_input,
                    self.input_channels,
                    chunk_output,
                    self.output_channels,
                );

                total_written += chunk_output.len();
            }
            total_written
        }
    }
}
