//! Global playback transport - timing information shared between the player and sequencers.

// -------------------------------------------------------------------------------------------------

/// Timing context for sequencers: sample rate, tempo, and time signature.
#[derive(Debug, Clone, Copy)]
pub struct Transport {
    sample_rate: u32,
    beats_per_minute: f64,
    beats_per_bar: usize,
}

impl Transport {
    /// Create a new transport instance.
    ///
    /// Panics if any of the properties are <= 0.
    pub const fn new(sample_rate: u32, beats_per_minute: f64, beats_per_bar: usize) -> Self {
        assert!(sample_rate > 0, "Invalid transport sample rate");
        assert!(beats_per_minute > 0.0, "Invalid transport BPM");
        assert!(beats_per_bar > 0, "Invalid transport BPP");
        Self {
            sample_rate,
            beats_per_minute,
            beats_per_bar,
        }
    }

    #[inline(always)]
    pub const fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    #[inline(always)]
    pub const fn beats_per_minute(&self) -> f64 {
        self.beats_per_minute
    }
    #[inline(always)]
    pub const fn beats_per_bar(&self) -> usize {
        self.beats_per_bar
    }

    /// Number of samples that fit into one beat at the current tempo and sample rate.
    #[inline]
    pub fn samples_per_beat(&self) -> f64 {
        60.0 / self.beats_per_minute * self.sample_rate as f64
    }
    /// Number of samples that fit into one bar.
    #[inline]
    pub fn samples_per_bar(&self) -> f64 {
        self.samples_per_beat() * self.beats_per_bar as f64
    }

    /// Convert a sample count to wall-clock seconds.
    #[inline]
    pub fn samples_to_seconds(&self, samples: u64) -> f64 {
        samples as f64 / self.sample_rate as f64
    }
    /// Convert wall-clock seconds to a sample count.
    #[inline]
    pub fn seconds_to_samples(&self, seconds: f64) -> u64 {
        (seconds * self.sample_rate as f64) as u64
    }

    /// Convert a sample count to beats.
    #[inline]
    pub fn samples_to_beats(&self, samples: u64) -> f64 {
        samples as f64 / self.samples_per_beat()
    }
    /// Convert beats to a sample count.
    #[inline]
    pub fn beats_to_samples(&self, beats: f64) -> u64 {
        (beats * self.samples_per_beat()) as u64
    }

    /// Convert a sample count to bars.
    #[inline]
    pub fn samples_to_bars(&self, samples: u64) -> f64 {
        samples as f64 / self.samples_per_bar()
    }
    /// Convert bars to a sample count.
    #[inline]
    pub fn bars_to_samples(&self, bars: f64) -> u64 {
        (bars * self.samples_per_bar()) as u64
    }
}
