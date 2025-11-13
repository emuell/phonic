//! Volume fading, de-clicking tools.

use std::time::Duration;

use crate::utils::buffer::scale_buffer;

// -------------------------------------------------------------------------------------------------

#[derive(PartialEq, Clone, Copy)]
pub enum FaderState {
    /// Fading was not started and thus is bypassed.
    Stopped,
    /// Fader is actively processing.
    IsRunning,
    /// Fader got started and finished processing.
    Finished,
}

// -------------------------------------------------------------------------------------------------

/// Fades out a sample buffer by applying a decaying volume ramp.
///
/// Fader initially is disabled and needs to be started first. Fading is applied by ramping
/// volume exponentially on each sample frame with the configured duration.
#[derive(Clone, Copy)]
pub struct VolumeFader {
    state: FaderState,
    current_volume: f32,
    target_volume: f32,
    inertia: f32,
    channel_count: usize,
    sample_rate: u32,
}

impl VolumeFader {
    /// Create a new bypassed fader with the given signal specs.
    pub fn new(channel_count: usize, sample_rate: u32) -> Self {
        Self {
            state: FaderState::Stopped,
            current_volume: 1.0,
            target_volume: 1.0,
            inertia: 1.0,
            channel_count,
            sample_rate,
        }
    }

    /// Get actual fader state.
    pub fn state(&self) -> FaderState {
        self.state
    }

    /// Get target volume.
    pub fn target_volume(&self) -> f32 {
        self.target_volume
    }

    // Activate the fade with the given duration.
    pub fn start_fade_in(&mut self, duration: Duration) {
        if self.state == FaderState::IsRunning {
            self.start(self.current_volume, 1.0, duration)
        } else {
            self.start(0.0, 1.0, duration)
        }
    }
    // Activate the fade with the given duration.
    pub fn start_fade_out(&mut self, duration: Duration) {
        if self.state == FaderState::IsRunning {
            self.start(self.current_volume, 0.0, duration)
        } else {
            self.start(1.0, 0.0, duration)
        }
    }
    // Activate the fader with the given start, end values and duration.
    pub fn start(&mut self, from: f32, to: f32, duration: Duration) {
        if duration.is_zero() {
            self.current_volume = to;
            self.target_volume = to;
            self.state = FaderState::Finished;
        } else {
            self.state = FaderState::IsRunning;
            self.current_volume = from;
            self.target_volume = to;
            // Calculate inertia to reach 99% completion at the specified duration:
            // 0.01 = e^(-duration/t) => t = -duration/ln(0.01) ≈ duration/ln(100)
            // Per-sample inertia = 1 - e^(-1/(sample_rate * t))
            const LN100: f32 = 4.605;
            let samples_duration = self.sample_rate as f32 * duration.as_secs_f32() / LN100;
            self.inertia = 1.0 - (-1.0 / samples_duration).exp();
        }
    }

    /// Reset the fader to its initial state.
    #[inline]
    pub fn reset(&mut self) {
        self.state = FaderState::Stopped;
        self.current_volume = 1.0;
        self.target_volume = 1.0;
    }

    // Process fader on the given interleaved output buffer.
    pub fn process(&mut self, output: &mut [f32]) {
        if self.state != FaderState::IsRunning {
            if self.target_volume != 1.0 {
                scale_buffer(output, self.target_volume);
            }
        } else {
            for f in output.chunks_exact_mut(self.channel_count) {
                // ramp per frame
                self.current_volume += (self.target_volume - self.current_volume) * self.inertia;
                // apply per sample
                for s in f.iter_mut() {
                    *s *= self.current_volume;
                }
            }
            // check if we've finished fading
            if (self.current_volume - self.target_volume).abs() < 0.0001 {
                self.state = FaderState::Finished;
            }
        }
    }
}
