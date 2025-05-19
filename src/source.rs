use std::time::Instant;

// -------------------------------------------------------------------------------------------------

pub mod converted;
pub mod empty;
pub mod file;
pub mod mapped;
pub mod mixed;
pub mod resampled;
pub mod synth;

// -------------------------------------------------------------------------------------------------

/// Timing info for [`Source`] impls.
#[derive(Debug, Copy, Clone, PartialEq, PartialOrd)]
pub struct SourceTime {
    /// Buffer time in absolute sample frames since playback started.
    pub pos_in_frames: u64,
    /// Buffer pos in elapsed wallclock time units since playback started.
    pub pos_instant: Instant,
}

impl SourceTime {
    /// Create a new SourceTime with default values.
    pub fn new() -> Self {
        Self {
            pos_in_frames: 0,
            pos_instant: Instant::now(),
        }
    }

    /// return a new SourceTime with a frame time which is this times's frame time
    /// plus the given amount in frames.
    pub fn with_added_frames(&self, frames: u64) -> Self {
        let mut copy = *self;
        copy.add_frames(frames);
        copy
    }

    /// Move pos in frames by the given amount in frames.
    pub fn add_frames(&mut self, frames: u64) {
        self.pos_in_frames += frames;
    }
}

impl Default for SourceTime {
    fn default() -> Self {
        Self::new()
    }
}

// -------------------------------------------------------------------------------------------------

/// Source types produce audio samples in `f32` format and can be `Send` and `Sync`ed
/// across threads.
///
/// The output buffer is a raw interleaved buffer, which is going to be written by the source
/// in the specified `channel_count` and `sample_rate` specs. Specs may not change during runtime,
/// so following sources don't have to adapt to new specs.
///
/// `write` is called in the realtime audio thread, so it must not block!
pub trait Source: Send + Sync + 'static {
    /// The source's output sample rate.
    fn sample_rate(&self) -> u32;
    /// The source's output channel count.
    fn channel_count(&self) -> usize;

    /// returns if the source finished playback. Exhausted sources should only return 0 on `write`
    /// and can be removed from a source render graph.
    fn is_exhausted(&self) -> bool;

    /// Write at most of `output.len()` samples into the interleaved `output`
    /// The given [`SourceTime`] parameter specifies which absolute time this buffer in the
    /// final output stream refers to. It can be used to schedule and apply real-time events.
    /// Returns the number of written **samples** (not frames).
    fn write(&mut self, output: &mut [f32], time: &SourceTime) -> usize;
}
