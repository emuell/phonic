use std::{
    fmt::Display,
    sync::{Arc, Mutex},
    time::Duration,
};

use super::{Source, SourceTime};

use crate::utils::{
    buffer::InterleavedBuffer,
    time::{SampleTime, SampleTimeClock},
};

// -------------------------------------------------------------------------------------------------

/// Audio level metrics of a source.
#[derive(Debug, Clone, Default)]
pub struct AudioLevel {
    /// Per-channel peak amplitude (linear). 1.0 = 0 dBFS
    pub peak: Vec<f32>,
    /// Per-channel RMS level (linear).
    pub rms: Vec<f32>,
}

impl AudioLevel {
    /// Peak level in dBFS for the given channel. Returns `f32::NEG_INFINITY` for silence.
    pub fn peak_db(&self, channel: usize) -> f32 {
        self.peak
            .get(channel)
            .copied()
            .map(|p| {
                if p > 0.0 {
                    20.0 * p.log10()
                } else {
                    f32::NEG_INFINITY
                }
            })
            .unwrap_or(f32::NEG_INFINITY)
    }

    /// RMS level in dBFS for the given channel. Returns `f32::NEG_INFINITY` for silence.
    pub fn rms_db(&self, channel: usize) -> f32 {
        self.rms
            .get(channel)
            .copied()
            .map(|r| {
                if r > 0.0 {
                    20.0 * r.log10()
                } else {
                    f32::NEG_INFINITY
                }
            })
            .unwrap_or(f32::NEG_INFINITY)
    }
}

impl Display for AudioLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let peak_strings: Vec<String> = (0..self.peak.len())
            .map(|ch| format!("{:.1}", self.peak_db(ch)))
            .collect();
        write!(f, "Peak: [{}] dBFS", peak_strings.join(", "))?;
        Ok(())
    }
}

// -------------------------------------------------------------------------------------------------

/// A thread-safe handle to a [`AudioLevelState`].
pub type SharedAudioLevelState = Arc<Mutex<AudioLevelState>>;

// -------------------------------------------------------------------------------------------------

/// Audio level measurement state, shared between a `MeteredSource` and its handle.
pub struct AudioLevelState {
    channel_count: usize,
    peak_hold: Vec<f32>,
    sum_square: Vec<f64>,
    collected_frames: u64,
    update_interval: SampleTime,
    update_interval_clock: SampleTimeClock,
    audio_level: AudioLevel,
}

impl AudioLevelState {
    pub fn new(update_interval: Duration, channel_count: usize, sample_rate: u32) -> Self {
        Self {
            channel_count,
            peak_hold: vec![0.0; channel_count],
            sum_square: vec![0.0; channel_count],
            collected_frames: 0,
            update_interval: SampleTimeClock::duration_to_sample_time(update_interval, sample_rate),
            update_interval_clock: SampleTimeClock::new(sample_rate),
            audio_level: AudioLevel {
                peak: vec![0.0; channel_count],
                rms: vec![0.0; channel_count],
            },
        }
    }

    /// Returns the last computed audio level.
    pub fn audio_level(&self) -> &AudioLevel {
        &self.audio_level
    }

    /// Update audio level
    pub fn record(&mut self, output: &[f32], time: &SourceTime) {
        let channel_count = self.channel_count;
        if channel_count == 0 || output.is_empty() {
            return;
        }

        // Accumulate peak and sum-of-squares per channel.
        for frame in output.frames(channel_count) {
            for (channel, &sample) in frame.enumerate() {
                let abs_sample = sample.abs();
                if abs_sample > self.peak_hold[channel] {
                    self.peak_hold[channel] = abs_sample;
                }
                self.sum_square[channel] += (sample as f64) * (sample as f64);
            }
        }

        // Publish results at the configured interval.
        self.collected_frames += (output.len() / channel_count) as u64;

        if self.update_interval_clock.elapsed(time.pos_in_frames) >= self.update_interval {
            for channel in 0..channel_count {
                self.audio_level.peak[channel] = self.peak_hold[channel];
                self.audio_level.rms[channel] = if self.collected_frames > 0 {
                    (self.sum_square[channel] / self.collected_frames as f64).sqrt() as f32
                } else {
                    0.0
                };
            }

            self.update_interval_clock.reset(time.pos_in_frames);
            self.collected_frames = 0;
            self.peak_hold.fill(0.0);
            self.sum_square.fill(0.0);
        }
    }
}

// -------------------------------------------------------------------------------------------------

/// A source wrapper that measures the audio output level of an inner source.
///
/// When no `update_interval` is provided, the wrapper is a transparent pass-through.
pub struct MeteredSource<S: Source> {
    source: S,
    state: Option<SharedAudioLevelState>,
}

impl<S: Source> MeteredSource<S> {
    /// Wraps a source to measure its audio output levels.
    /// Pass `None`as update_interval to disable metering entirely.
    pub fn new(source: S, update_interval: Option<Duration>) -> Self {
        if let Some(update_interval) = update_interval {
            let channel_count = source.channel_count();
            Self {
                state: Some(Arc::new(Mutex::new(AudioLevelState::new(
                    update_interval,
                    channel_count,
                    source.sample_rate(),
                )))),
                source,
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

    /// Returns a thread-safe handle to the metering state, when metering is enabled.
    pub(crate) fn state(&self) -> Option<SharedAudioLevelState> {
        self.state.clone()
    }
}

impl<S: Source> Source for MeteredSource<S> {
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
        let written = self.source.write(output, time);
        if written > 0 {
            if let Some(state) = &self.state {
                if let Ok(mut state) = state.try_lock() {
                    state.record(&output[..written], time);
                }
            }
        }
        written
    }
}
