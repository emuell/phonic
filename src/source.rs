use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Instant,
};

// -------------------------------------------------------------------------------------------------

pub(crate) mod playback;
pub mod status;

pub mod amplified;
pub mod converted;
pub mod empty;
pub mod file;
pub mod guarded;
pub mod mapped;
pub mod measured;
pub mod mixed;
pub mod panned;
pub mod resampled;
#[cfg(feature = "time-stretching")]
pub mod stretched;
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

    /// Returns a new SourceTime with a frame time which is this time's frame time
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

/// Audio signal producer that generates new or processes existing audio samples.
///
/// Sources are the fundamental building blocks of audio playback in phonic. They produce audio
/// samples in `f32` format and can represent various kinds of audio:
/// - Audio file playback via [`FileSource`](crate::FileSource)
/// - Synthesized tones via [`SynthSource`](crate::SynthSource)
/// - Note-driven instruments via [`Generator`](crate::Generator) (which is a specialized `Source`)
/// - DSP operations like mixing, panning, resampling, etc.
///
/// The output buffer is a raw interleaved buffer, which is written by the source in the specified
/// `channel_count` and `sample_rate` specs. Specs may not change during runtime, so downstream
/// sources don't have to adapt to new specs.
///
/// **Important:** `write` is called in real-time audio threads, so it must not block!
pub trait Source: Send + Sync + 'static {
    /// Convert the Source impl into a boxed `dyn Source`.
    ///
    /// Avoids double boxing when a generator impl already is a `Box<dyn Source>`.
    fn into_box(self) -> Box<dyn Source>
    where
        Self: Sized,
    {
        Box::new(self)
    }

    /// The source's output sample rate.
    fn sample_rate(&self) -> u32;
    /// The source's output channel count.
    fn channel_count(&self) -> usize;

    /// Returns whether the source finished playback. Exhausted sources should only return 0
    /// on `write` and may be removed from a source render graph.
    fn is_exhausted(&self) -> bool;

    /// Return a rough **estimate** of the processing costs for this source in range `~1..10`,
    /// where 1 means pretty lightweight and 10 very CPU intensive. This is used in parallel
    /// processing to distribute work loads evenly before or without actual CPU measurements.
    fn weight(&self) -> usize;

    /// Write at most `output.len()` samples into the interleaved `output`
    /// The given [`SourceTime`] parameter specifies which absolute time this buffer in the
    /// final output stream refers to. It can be used to schedule and apply real-time events.
    /// Returns the number of written **samples** (not frames).
    fn write(&mut self, output: &mut [f32], time: &SourceTime) -> usize;
}

// -------------------------------------------------------------------------------------------------

/// Generates a unique source id for a program run, by counting atomically upwards from 1.
pub(crate) fn unique_source_id() -> usize {
    static ID_COUNTER: AtomicUsize = AtomicUsize::new(1);
    ID_COUNTER.fetch_add(1, Ordering::Relaxed)
}

// -------------------------------------------------------------------------------------------------

/// Allow adding/using boxed `dyn Source`s as `Source` impls.
impl Source for Box<dyn Source> {
    fn into_box(self) -> Box<dyn Source> {
        self
    }

    fn sample_rate(&self) -> u32 {
        (**self).sample_rate()
    }

    fn channel_count(&self) -> usize {
        (**self).channel_count()
    }

    fn is_exhausted(&self) -> bool {
        (**self).is_exhausted()
    }

    fn weight(&self) -> usize {
        (**self).weight()
    }

    fn write(&mut self, output: &mut [f32], time: &SourceTime) -> usize {
        (**self).write(output, time)
    }
}
