use std::{
    fmt::Display,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use super::{super::utils::time::SampleTimeClock, Source, SourceTime};

// -------------------------------------------------------------------------------------------------

/// CPU load of a playing source, typically accessed via the source's playback handle.
#[derive(Debug, Copy, Clone, Default)]
pub struct CpuLoad {
    /// Average CPU load over the measurement interval.
    /// A value of 1.0 means the source took as much CPU time as the duration of the audio it produced.
    pub average: f32,
    /// Peak CPU load observed during the last measurement interval.
    pub peak: f32,
}

impl Display for CpuLoad {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "{:.2}% ({:.2}%)",
            self.average * 100.0,
            self.peak * 100.0
        ))
    }
}

// -------------------------------------------------------------------------------------------------

/// A source wrapper that measures the CPU load of an inner source, when passing a valid
/// measure duration. When no measure duration is set, it only runs the inner source.
pub struct MeasuredSource<S: Source> {
    source: S,
    state: Option<SharedMeasurementState>,
}

impl<S: Source> MeasuredSource<S> {
    /// Wraps a source to measure its CPU load, when update_interval is Some.
    pub fn new(source: S, update_interval: Option<Duration>) -> Self {
        if let Some(update_interval) = update_interval {
            Self {
                source,
                state: Some(Arc::new(Mutex::new(MeasurementState::new(update_interval)))),
            }
        } else {
            Self {
                source,
                state: None,
            }
        }
    }

    /// Returns a reference to the wrapped source.
    #[allow(unused)]
    #[inline]
    pub(crate) fn source(&self) -> &S {
        &self.source
    }

    /// Returns a thread-safe handle to the measurement state, when measuring is enabled.
    pub(crate) fn state(&self) -> Option<SharedMeasurementState> {
        self.state.clone()
    }
}

impl<S: Source> Source for MeasuredSource<S> {
    fn sample_rate(&self) -> u32 {
        self.source.sample_rate()
    }

    fn channel_count(&self) -> usize {
        self.source.channel_count()
    }

    fn is_exhausted(&self) -> bool {
        self.source.is_exhausted()
    }

    fn weight(&self) -> usize {
        self.source.weight()
    }

    fn write(&mut self, output: &mut [f32], time: &SourceTime) -> usize {
        if let Some(state) = &self.state {
            let start_time = Instant::now();
            let written_samples = self.source.write(output, time);
            let processing_time = start_time.elapsed();

            if written_samples > 0 {
                if let Ok(mut state) = state.try_lock() {
                    let frames_written = written_samples as u64 / self.channel_count() as u64;
                    let sample_rate = self.sample_rate();
                    state.record(processing_time, frames_written, sample_rate);
                }
            }

            written_samples
        } else {
            self.source.write(output, time)
        }
    }
}

// -------------------------------------------------------------------------------------------------

/// A thread-safe handle to a `MeasurementState`.
pub type SharedMeasurementState = Arc<Mutex<MeasurementState>>;

// -------------------------------------------------------------------------------------------------

#[derive(Debug)]
pub struct MeasurementState {
    total_processing_time: Duration,
    total_frames_processed: u64,
    peak_load_in_interval: f32,
    last_update: Instant,
    cpu_load: CpuLoad,
    update_interval: Duration,
}

impl MeasurementState {
    pub fn new(update_interval: Duration) -> Self {
        Self {
            total_processing_time: Duration::from_secs(0),
            total_frames_processed: 0,
            peak_load_in_interval: 0.0,
            last_update: Instant::now(),
            cpu_load: CpuLoad::default(),
            update_interval,
        }
    }

    pub fn cpu_load(&self) -> CpuLoad {
        self.cpu_load
    }

    pub(self) fn record(
        &mut self,
        processing_time: Duration,
        frames_processed: u64,
        sample_rate: u32,
    ) {
        self.total_processing_time += processing_time;
        self.total_frames_processed += frames_processed;

        if frames_processed > 0 {
            let audio_time_secs =
                SampleTimeClock::sample_time_to_duration(frames_processed, sample_rate)
                    .as_secs_f32();
            let processing_time_secs = processing_time.as_secs_f32();
            if audio_time_secs > 0.0 {
                let current_load = processing_time_secs / audio_time_secs;
                if current_load > self.peak_load_in_interval {
                    self.peak_load_in_interval = current_load;
                }
            }
        }

        let now = Instant::now();
        if now.duration_since(self.last_update) >= self.update_interval {
            let avg_load = if self.total_frames_processed > 0 {
                let total_audio_duration = Duration::from_secs_f64(
                    self.total_frames_processed as f64 / sample_rate as f64,
                );
                if total_audio_duration.as_secs_f64() > 0.0 {
                    (self.total_processing_time.as_secs_f64() / total_audio_duration.as_secs_f64())
                        as f32
                } else {
                    0.0
                }
            } else {
                0.0
            };

            self.cpu_load.average = avg_load;
            self.cpu_load.peak = self.peak_load_in_interval;

            self.total_processing_time = Duration::from_secs(0);
            self.total_frames_processed = 0;
            self.peak_load_in_interval = 0.0;
            self.last_update = now;
        }
    }
}
