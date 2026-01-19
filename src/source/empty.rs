use super::{Source, SourceTime};

// -------------------------------------------------------------------------------------------------

/// A source which does not produce any samples.
///
/// Can be useful when a temorary placeholder source is needed.
#[derive(Debug, Clone)]
pub struct EmptySource {
    channel_count: usize,
    sample_rate: u32,
}

impl EmptySource {
    pub fn new(channel_count: usize, sample_rate: u32) -> Self {
        Self {
            channel_count,
            sample_rate,
        }
    }
}

impl Default for EmptySource {
    fn default() -> Self {
        Self::new(2, 44100)
    }
}

impl Source for EmptySource {
    fn channel_count(&self) -> usize {
        self.channel_count
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn is_exhausted(&self) -> bool {
        true
    }

    fn weight(&self) -> usize {
        0
    }

    fn write(&mut self, _output: &mut [f32], _time: &SourceTime) -> usize {
        0
    }
}
