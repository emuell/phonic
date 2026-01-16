use crate::{
    source::{
        measured::MeasuredSource,
        mixed::{EffectProcessor, MixedSource},
    },
    utils::buffer::{add_buffers, max_abs_sample},
    Source, SourceTime,
};

// -------------------------------------------------------------------------------------------------

// Parallel mixer processing
mod processing;
mod sync;

pub(crate) use processing::{SubMixerProcessingResult, SubMixerThreadPool};

// -------------------------------------------------------------------------------------------------

// -------------------------------------------------------------------------------------------------

/// Wraps a sub-mixer with silence detection for auto-bypass optimization.
///
/// Tracks silence duration to determine if the sub-mixer is producing audible output,
/// allowing the parent mixer to optimize effect processing.
pub(crate) struct SubMixerProcessor {
    mixer: Box<MeasuredSource<MixedSource>>,
    pub(super) silence_counter: usize,
    /// Temporary output buffer for parallel processing.
    /// Each mixer owns its buffer - workers write directly to it to avoid copying.
    /// Pre-allocated to avoid allocations in the audio path.
    pub(super) temp_output_buffer: Vec<f32>,
}

impl SubMixerProcessor {
    pub fn new(mixer: Box<MeasuredSource<MixedSource>>) -> Self {
        Self {
            mixer,
            silence_counter: 0,
            temp_output_buffer: vec![0.0; MixedSource::MAX_MIX_BUFFER_SAMPLES],
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

    /// Estimate the processing weight of this sub-mixer for load balancing.
    ///
    /// The weight is calculated based on the number of active sources and effects,
    /// both in this mixer and recursively in nested sub-mixers. Silent mixers
    /// (past the silence threshold) have reduced weight since they're bypassed.
    ///
    /// Returns a weight value >= 1.
    pub(crate) fn estimate_processing_weight(&self) -> usize {
        // Silent mixers are those with very high silence_counter values
        const SILENCE_FRAME_THRESHOLD: usize = 96000; // ~2 seconds at 48kHz

        /// Weight factors for processing cost estimation.
        const WEIGHT_PER_SOURCE: usize = 1;
        const WEIGHT_PER_EFFECT: usize = 2;
        const WEIGHT_PER_GENERATOR: usize = 3; // TODO

        // Access internal MixedSource
        let mixed_source = self.mixer.source();

        // Count direct sources
        let source_count = mixed_source.playing_source_count();

        // Count direct effects
        let effect_count = mixed_source.effect_count();

        // Recursively count sources/effects in submixers
        let (nested_sources, nested_effects) = mixed_source
            .submixer_iter()
            .map(|submixer| submixer.estimate_processing_weight_recursive())
            .fold((0, 0), |(s_acc, e_acc), (s, e)| (s_acc + s, e_acc + e));

        let total_sources = source_count + nested_sources;
        let total_effects = effect_count + nested_effects;

        // Combine with configurable weight factors
        let weight = total_sources * WEIGHT_PER_SOURCE + total_effects * WEIGHT_PER_EFFECT;

        // Reduce weight for silent mixers (already bypassed)
        if self.silence_counter > SILENCE_FRAME_THRESHOLD {
            (weight / 4).max(1) // Silent mixers cost much less, but at least 1
        } else {
            weight.max(1) // Minimum weight of 1
        }
    }

    /// Helper for recursive weight calculation.
    /// Returns (source_count, effect_count) for this mixer and all nested sub-mixers.
    fn estimate_processing_weight_recursive(&self) -> (usize, usize) {
        let mixed_source = self.mixer.source();

        let direct_sources = mixed_source.playing_source_count();
        let direct_effects = mixed_source.effect_count();

        // Recurse into nested submixers
        let (nested_sources, nested_effects) = mixed_source
            .submixer_iter()
            .map(|submixer| submixer.estimate_processing_weight_recursive())
            .fold((0, 0), |(s_acc, e_acc), (s, e)| (s_acc + s, e_acc + e));

        (
            direct_sources + nested_sources,
            direct_effects + nested_effects,
        )
    }
}
