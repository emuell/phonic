use super::{AudioSource, AudioSourceTime};

// -------------------------------------------------------------------------------------------------

/// A source which does not produce any samples.
pub struct EmptySource;

impl AudioSource for EmptySource {
    fn write(&mut self, _output: &mut [f32], _time: &AudioSourceTime) -> usize {
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
