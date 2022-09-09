use std::time::Duration;

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
/// down volume exponentially on each sample frame with the configured duration.
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
            target_volume: 0.0,
            inertia: 1.0,
            channel_count,
            sample_rate,
        }
    }

    /// Get actual fader state.
    pub fn state(&self) -> FaderState {
        self.state
    }

    // Activate the fader with the given duration.
    pub fn start(&mut self, duration: Duration) {
        self.state = FaderState::IsRunning;
        self.current_volume = 1.0;
        self.target_volume = 0.0;
        // HACK: this is a rough guess and should be calculated properly!
        self.inertia = (1.0 / self.sample_rate as f32) * 4.0 / duration.as_secs_f32();
    }

    // Process fader on the given interleaved output buffer. Returns the modified output range.
    pub fn process(&mut self, output: &mut [f32]) -> usize {
        // return empty handed when there's nothing to do
        if self.state != FaderState::IsRunning {
            return 0;
        }
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
        output.len()
    }
}
