use super::AudioSource;

// -------------------------------------------------------------------------------------------------

/// Empty audio source. Does not produce any samples.
pub struct EmptySource;

impl AudioSource for EmptySource {
    fn write(&mut self, _output: &mut [f32]) -> usize {
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
