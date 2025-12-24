use super::{Source, SourceTime};

// -------------------------------------------------------------------------------------------------

/// A source which does not produce any samples.
#[allow(unused)]
pub struct EmptySource;

impl Source for EmptySource {
    fn write(&mut self, _output: &mut [f32], _time: &SourceTime) -> usize {
        0
    }

    fn channel_count(&self) -> usize {
        0
    }

    fn sample_rate(&self) -> u32 {
        0
    }

    fn is_exhausted(&self) -> bool {
        true
    }
}
