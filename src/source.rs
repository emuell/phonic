pub mod decoded;
pub mod empty;
pub mod mapped;
pub mod mixed;
pub mod resampled;

// Types that can produce audio samples in `f32` format. `Send`able across threads.
pub trait AudioSource: Send + 'static {
    // Write at most of `output.len()` samples into the `output`. Returns the
    // number of written samples. Should take care to always output a full
    // frame, and should _never_ block.
    fn write(&mut self, output: &mut [f32]) -> usize;
    fn channel_count(&self) -> usize;
    fn sample_rate(&self) -> u32;
}
