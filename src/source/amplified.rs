use crate::utils::buffer::scale_buffer;

use super::{Source, SourceTime};

// -------------------------------------------------------------------------------------------------

/// A source which applies a volume factor to some other source's output
pub struct AmplifiedSource {
    source: Box<dyn Source>,
    volume: f32,
}

impl AmplifiedSource {
    pub fn new<InputSource>(source: InputSource, volume: f32) -> Self
    where
        InputSource: Source,
    {
        debug_assert!(volume >= 0.0, "Invalid volume factor");
        Self {
            source: Box::new(source),
            volume,
        }
    }
}

impl Source for AmplifiedSource {
    fn write(&mut self, output: &mut [f32], time: &SourceTime) -> usize {
        // write input source
        let written = self.source.write(output, time);
        // apply volume
        if (1.0 - self.volume).abs() > 0.001 {
            let written_out = &mut output[0..written];
            scale_buffer(written_out, self.volume);
        }
        written
    }

    fn channel_count(&self) -> usize {
        self.source.channel_count()
    }

    fn sample_rate(&self) -> u32 {
        self.source.sample_rate()
    }

    fn is_exhausted(&self) -> bool {
        self.source.is_exhausted()
    }
}
