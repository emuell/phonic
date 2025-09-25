use super::{Source, SourceTime};

use crate::utils::{
    buffer::TempBuffer,
    stretcher::{
        bungee::BungeeTimeStretcher,
        signalsmith::{SignalSmithPreset, SignalSmithTimeStretcher},
        AudioTimeStretcher, TimeStretchingSpecs,
    },
};

// -------------------------------------------------------------------------------------------------

/// Time stretcher backend and preset selection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TimeStretchMode {
    /// SignalSmith time stretcher with default preset.
    #[default]
    SignalSmithDefault,
    /// SignalSmith time stretcher with fast preset.
    SignalSmithFast,
    /// Bungee time stretcher.
    Bungee,
}

// -------------------------------------------------------------------------------------------------

/// A source which time-stretches the input source with a custom speed factor.
pub struct StretchedSource<InputSource: Source + 'static> {
    source: InputSource,
    stretcher: Option<Box<dyn AudioTimeStretcher>>,
    input_buffer: TempBuffer,
    output_buffer: TempBuffer,
}

impl<InputSource: Source + 'static> StretchedSource<InputSource> {
    /// Create a new stretched sources with the given sample rate, rate and mode.
    pub fn new(source: InputSource, speed: f64, mode: TimeStretchMode) -> Self {
        let specs = TimeStretchingSpecs::new(speed, source.sample_rate(), source.channel_count());

        let stretcher: Option<Box<dyn AudioTimeStretcher>> = if speed == 1.0 {
            None
        } else {
            Some(match mode {
                TimeStretchMode::SignalSmithDefault | TimeStretchMode::SignalSmithFast => {
                    let preset = if mode == TimeStretchMode::SignalSmithDefault {
                        SignalSmithPreset::Default
                    } else {
                        SignalSmithPreset::Fast
                    };
                    Box::new(
                        SignalSmithTimeStretcher::new(specs, preset)
                            .expect("Failed to create new bungee stretcher instance"),
                    )
                }
                TimeStretchMode::Bungee => Box::new(
                    BungeeTimeStretcher::new(specs)
                        .expect("Failed to create new bungee stretcher instance"),
                ),
            })
        };

        const DEFAULT_CHUNK_SIZE: usize = 512;
        let input_buffer_len = if let Some(stretcher) = &stretcher {
            stretcher
                .max_input_buffer_size()
                .unwrap_or(DEFAULT_CHUNK_SIZE)
        } else {
            0
        };

        let output_buffer_len = if let Some(stretcher) = &stretcher {
            stretcher
                .min_output_buffer_size()
                .unwrap_or(DEFAULT_CHUNK_SIZE)
        } else {
            0
        };

        let input_buffer = TempBuffer::new(input_buffer_len);
        let output_buffer = TempBuffer::new(output_buffer_len);

        Self {
            source,
            stretcher,
            input_buffer,
            output_buffer,
        }
    }
}

impl<InputSource: Source + 'static> Source for StretchedSource<InputSource> {
    fn weight(&self) -> usize {
        2
    }

    fn write(&mut self, output: &mut [f32], time: &SourceTime) -> usize {
        if let Some(stretcher) = self.stretcher.as_deref_mut() {
            let mut total_written = 0;
            while total_written < output.len() {
                if self.output_buffer.is_empty() {
                    self.output_buffer.reset_range();
                    // when there's no input, try fetch some from our source
                    if self.input_buffer.is_empty() {
                        let source_time = time.with_added_frames(
                            (total_written / self.source.channel_count()) as u64,
                        );
                        self.input_buffer.reset_range();
                        let input_read =
                            self.source.write(self.input_buffer.get_mut(), &source_time);
                        self.input_buffer.set_range(0, input_read);
                    }
                    // run stretcher
                    let (input_consumed, output_written) = stretcher
                        .process(self.input_buffer.get(), self.output_buffer.get_mut())
                        .expect("Resampling failed");
                    self.input_buffer.consume(input_consumed);
                    self.output_buffer.set_range(0, output_written);
                    if self.source.is_exhausted() && output_written == 0 {
                        // source and stretcher produced no more output: we're done
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
            self.source.write(output, time)
        }
    }

    fn channel_count(&self) -> usize {
        self.source.channel_count()
    }

    fn sample_rate(&self) -> u32 {
        self.source.sample_rate()
    }

    fn is_exhausted(&self) -> bool {
        self.source.is_exhausted() && self.input_buffer.is_empty() && self.output_buffer.is_empty()
    }
}
