use std::time::Duration;

// -------------------------------------------------------------------------------------------------

pub type SampleTime = u64;

// -------------------------------------------------------------------------------------------------

/// A clock that measures and converts wall-clock time in terms of sample frames at a fixed
/// sample rate. This clock tracks sample times directly as u64 values.
#[derive(Debug, Clone)]
pub struct SampleTimeClock {
    sample_rate: u32,
    start_time: SampleTime,
}

impl SampleTimeClock {
    /// Create a new SampleTimeClock with the given sample rate.
    pub fn new(sample_rate: u32) -> Self {
        assert!(sample_rate > 0, "Invalid sample rate");
        Self {
            sample_rate,
            start_time: 0,
        }
    }

    /// Convert a duration to sample frames with the given sample rate.
    pub fn duration_to_sample_time(duration: Duration, sample_rate: u32) -> SampleTime {
        debug_assert!(sample_rate > 0, "Invalid sample rate");
        (duration.as_secs_f64() * sample_rate as f64) as SampleTime
    }

    /// Convert sample frames to a duration with the given sample rate.
    pub fn sample_time_to_duration(sample_time: SampleTime, sample_rate: u32) -> Duration {
        debug_assert!(sample_rate > 0, "Invalid sample rate");
        Duration::from_secs_f64(sample_time as f64 / sample_rate as f64)
    }

    /// Reset the clock to start counting from the given sample time.
    pub fn reset(&mut self, current_time: SampleTime) {
        self.start_time = current_time;
    }

    /// Get the elapsed time since the last reset in sample frames.
    pub fn elapsed(&self, current_time: SampleTime) -> SampleTime {
        current_time.saturating_sub(self.start_time)
    }

    /// Get the elapsed time since the last reset as a duration.
    pub fn elapsed_duration(&self, current_time: u64) -> Duration {
        Self::sample_time_to_duration(self.elapsed(current_time), self.sample_rate)
    }
}
