use basedrop::Owned;

use crate::{utils::buffer::max_abs_sample, Effect, SourceTime};

// -------------------------------------------------------------------------------------------------

/// Wraps an effect with auto-bypass logic and tail management.
///
/// Automatically bypasses effects when effect input is silent and the effect's tail has expired,
/// calling `process_started` and `process_stopped` on state transitions. Tracks tail duration using
/// `process_tail` or silence detection for effects that don't implement it.
pub(super) struct EffectProcessor {
    pub(super) effect: Owned<Box<dyn Effect>>,
    bypassed: bool,
    tail_counter: usize,
    silence_counter: usize,
}

impl EffectProcessor {
    /// Threshold for detecting silence in effect output (approximately -60dB)
    pub const SILENCE_THRESHOLD: f32 = 0.001;
    /// Number of seconds that we should let an effect running before treating it as bypassed
    pub const SILENCE_SECONDS: usize = 2;

    pub fn new(effect: Owned<Box<dyn Effect>>) -> Self {
        Self {
            effect,
            bypassed: true,
            tail_counter: 0,
            silence_counter: usize::MAX,
        }
    }

    /// Process this effect with full bypass logic and tail management.
    /// Returns true when the effect processed output.
    pub fn process(
        &mut self,
        output: &mut [f32],
        channel_count: usize,
        sample_rate: u32,
        input_bypassed: bool,
        time: &SourceTime,
    ) -> bool {
        // Handle bypass state transitions
        self.update_bypass_state(self.should_bypass(input_bypassed));

        if !self.bypassed {
            // Process effect if not bypassed
            self.effect.process(output, time);

            if input_bypassed {
                // Sources are inactive, update tail counters to bypass in future calls
                self.update_tail_counters(output, channel_count, sample_rate);
            } else {
                // Sources are active, reset tail counter and silence counter
                self.reset_tail_counters();
            }
            // is active
            true
        } else {
            // is bypassed
            false
        }
    }

    /// Check if this effect should be bypassed based on source activity and tail state.
    #[inline]
    fn should_bypass(&self, input_bypassed: bool) -> bool {
        // Only bypass if input is silent AND we've confirmed the tail has expired
        input_bypassed && self.tail_counter == 0 && self.silence_counter == usize::MAX
    }

    /// Handle bypass state transitions, calling process_start/end as needed.
    fn update_bypass_state(&mut self, should_bypass: bool) {
        if should_bypass && !self.bypassed {
            // Entering bypass: call process_stop
            self.effect.process_stopped();
            self.bypassed = true;
            // println!("Bypassing effect: {}", self.effect.name());
        } else if !should_bypass && self.bypassed {
            // Leaving bypass: call process_start
            self.effect.process_started();
            self.bypassed = false;
            self.reset_tail_counters();
            // println!("Activating effect: {}", self.effect.name());
        }
    }

    /// Update tail counter while there is no audible input signal.
    fn update_tail_counters(&mut self, output: &[f32], channel_count: usize, sample_rate: u32) {
        if let Some(tail_frames) = self.effect.process_tail() {
            if tail_frames == usize::MAX {
                // Don't apply tails: effect never wants to auto-bypass
                self.tail_counter = tail_frames;
            } else {
                // Effect provided a known tail duration
                if self.tail_counter == usize::MAX {
                    // Initialize tail counter when sources stop
                    self.tail_counter = tail_frames;
                } else {
                    // Decrement tail counter
                    let frames_processed = output.len() / channel_count;
                    self.tail_counter = self.tail_counter.saturating_sub(frames_processed);
                }
            }
            // reset unused silence counter
            self.silence_counter = usize::MAX;
        } else {
            // Unknown tail: detect silence manually
            let max_sample = max_abs_sample(output);
            if max_sample < Self::SILENCE_THRESHOLD {
                let frames_processed = output.len() / channel_count;
                self.silence_counter = self.silence_counter.saturating_add(frames_processed);
                // Consider effect silent after x seconds of silence
                if self.silence_counter >= Self::SILENCE_SECONDS * sample_rate as usize {
                    // Mark as ready to bypass
                    self.tail_counter = 0;
                    self.silence_counter = usize::MAX;
                }
            } else {
                // Reset silence counter if we detect audio
                self.silence_counter = 0;
            }
        }
    }

    /// Update tail counter when there is an audible input signal.
    #[inline]
    fn reset_tail_counters(&mut self) {
        self.tail_counter = usize::MAX;
        self.silence_counter = 0;
    }
}
