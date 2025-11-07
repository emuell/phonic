use crate::{
    source::mixed::{EffectProcessor, MixedSource},
    utils::buffer::{add_buffers, max_abs_sample},
    Source, SourceTime,
};

// -------------------------------------------------------------------------------------------------

/// Wraps a sub-mixer with silence detection for auto-bypass optimization.
///
/// Tracks silence duration to determine if the sub-mixer is producing audible output,
/// allowing the parent mixer to optimize effect processing.
pub(super) struct SubMixerProcessor {
    mixer: Box<MixedSource>,
    silence_counter: usize,
}

impl SubMixerProcessor {
    pub fn new(mixer: Box<MixedSource>) -> Self {
        Self {
            mixer,
            silence_counter: 0,
        }
    }

    /// Process the sub-mixer and check if it produced audible output.
    /// Returns true if the sub-mixer is producing audible audio.
    pub fn process(
        &mut self,
        output: &mut [f32],
        mix_buffer: &mut [f32],
        channel_count: usize,
        sample_rate: u32,
        time: &SourceTime,
    ) -> bool {
        // Run mixer
        let written = self.mixer.write(mix_buffer, time);
        // Check if the sub-mixer produced audible output
        let max_sample = max_abs_sample(&mix_buffer[..written]);
        if max_sample < EffectProcessor::SILENCE_THRESHOLD {
            let frames_processed = output.len() / channel_count;
            self.silence_counter += frames_processed;
            // Consider sub-mixer silent after x seconds of silence
            if self.silence_counter < EffectProcessor::SILENCE_SECONDS * sample_rate as usize {
                // Add sub mixer output to the main output buffer
                add_buffers(&mut output[..written], &mix_buffer[..written]);
                true
            } else {
                false
            }
        } else {
            // Reset silence counter if we detect audio
            self.silence_counter = 0;
            // Add sub mixer output to the main output buffer
            add_buffers(&mut output[..written], &mix_buffer[..written]);
            true
        }
    }
}
