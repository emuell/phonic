//! FunDSP-based synth source.

use std::{
    sync::{mpsc::SyncSender, Arc},
    time::Duration,
};

use crossbeam_queue::ArrayQueue;
use fundsp::prelude32::*;

use super::{
    common::{SynthSourceGenerator, SynthSourceImpl},
    SynthPlaybackMessage, SynthPlaybackOptions, SynthSource,
};
use crate::{
    source::Source,
    utils::{buffer::max_abs_sample, time::SampleTimeClock},
    Error, PlaybackStatusContext, PlaybackStatusEvent, Player, SynthPlaybackHandle,
};

// -------------------------------------------------------------------------------------------------

// -60dB as audio silence
const SILENCE_THRESHOLD: f32 = 1e-6;
// 2 seconds of silence to ensure full decay
const EXHAUSTION_DURATION: Duration = Duration::from_secs(2);

// -------------------------------------------------------------------------------------------------

/// A synth generator which runs a FunDSP [`AudioNode`] until it is exhausted.
pub struct FunDspSynthSourceGenerator {
    audio_unit: Box<dyn AudioUnit>,
    is_exhausted: bool,
    silence_samples_count: usize,
    exhaustion_threshold_samples: usize,
}

impl FunDspSynthSourceGenerator {
    pub fn new(mut audio_unit: Box<dyn AudioUnit>, sample_rate: u32) -> Self {
        assert!(
            [1, 2].contains(&audio_unit.outputs()),
            "Only mono or stereo generator units are supported"
        );
        audio_unit.set_sample_rate(sample_rate as f64);
        audio_unit.allocate();

        let is_exhausted = false;
        let silence_samples_count = 0;
        let exhaustion_threshold_samples =
            SampleTimeClock::duration_to_sample_time(EXHAUSTION_DURATION, sample_rate) as usize;

        Self {
            audio_unit,
            is_exhausted,
            silence_samples_count,
            exhaustion_threshold_samples,
        }
    }
}

impl SynthSourceGenerator for FunDspSynthSourceGenerator {
    fn channel_count(&self) -> usize {
        self.audio_unit.outputs()
    }

    fn is_exhausted(&self) -> bool {
        self.is_exhausted
    }

    fn generate(&mut self, output: &mut [f32]) -> usize {
        if self.is_exhausted {
            return 0;
        }

        const BLOCK_SIZE: usize = 64; // Block size for SIMD processing

        let channel_count = self.audio_unit.outputs();
        let frame_count = output.len() / channel_count;

        for block_start in (0..frame_count).step_by(BLOCK_SIZE) {
            let block_end = std::cmp::min(block_start + BLOCK_SIZE, frame_count);
            let block_len = block_end - block_start;

            match channel_count {
                1 => {
                    // Mono processing
                    let input_buffer = BufferArray::<U1>::new();
                    let mut output_buffer = BufferArray::<U1>::new();

                    self.audio_unit.process(
                        block_len,
                        &input_buffer.buffer_ref(),
                        &mut output_buffer.buffer_mut(),
                    );

                    // Check for silence in the block
                    let max_abs = max_abs_sample(output_buffer.channel_f32(0));
                    if max_abs < SILENCE_THRESHOLD {
                        self.silence_samples_count =
                            self.silence_samples_count.saturating_add(block_len);
                    } else {
                        self.silence_samples_count = 0;
                    }

                    // Copy to output
                    let start_offset = block_start * channel_count;
                    for index in 0..block_len {
                        let sample = output_buffer.at_f32(0, index);
                        output[start_offset + index] = sample;
                    }
                }
                2 => {
                    // Stereo processing
                    let input_buffer = BufferArray::<U2>::new();
                    let mut output_buffer = BufferArray::<U2>::new();

                    self.audio_unit.process(
                        block_len,
                        &input_buffer.buffer_ref(),
                        &mut output_buffer.buffer_mut(),
                    );

                    // Check for silence in both channels
                    let max_abs_left = max_abs_sample(output_buffer.channel_f32(0));
                    let max_abs_right = max_abs_sample(output_buffer.channel_f32(1));
                    let max_abs = max_abs_left.max(max_abs_right);

                    if max_abs < SILENCE_THRESHOLD {
                        self.silence_samples_count =
                            self.silence_samples_count.saturating_add(block_len);
                    } else {
                        self.silence_samples_count = 0;
                    }

                    // Interleave to output
                    let start_offset = block_start * channel_count;
                    for index in 0..block_len {
                        let left = output_buffer.at_f32(0, index);
                        let right = output_buffer.at_f32(1, index);
                        let frame_start = start_offset + index * channel_count;
                        output[frame_start] = left;
                        output[frame_start + 1] = right;
                    }
                }
                _ => {
                    panic!(
                        "Unexpected output channel count in FunDspSynthSourceGenerator. \
                         Only mono or stereo outputs are supported"
                    );
                }
            }
        }

        // Mark as exhausted if we've had enough consecutive silent samples
        if self.silence_samples_count >= self.exhaustion_threshold_samples {
            self.is_exhausted = true;
        }

        output.len()
    }
}

// -------------------------------------------------------------------------------------------------

/// A [`SynthSource`] which runs a FunDSP `AudioNode` until it is exhausted.
pub struct FunDspSynthSource(SynthSourceImpl<FunDspSynthSourceGenerator>);

impl FunDspSynthSource {
    /// Create a new fundsp synth source. Usually created via [`Player::play_fundsp_synth`].
    pub fn new(
        generator_name: &str,
        audio_unit: Box<dyn AudioUnit>,
        options: SynthPlaybackOptions,
        sample_rate: u32,
    ) -> Result<Self, Error> {
        let generator = FunDspSynthSourceGenerator::new(audio_unit, sample_rate);
        Ok(Self(SynthSourceImpl::new(
            generator_name,
            generator,
            options,
            sample_rate,
        )?))
    }
}

impl SynthSource for FunDspSynthSource {
    fn synth_name(&self) -> String {
        self.0.synth_name()
    }

    fn playback_id(&self) -> crate::PlaybackId {
        self.0.playback_id()
    }

    fn playback_options(&self) -> &SynthPlaybackOptions {
        self.0.playback_options()
    }

    fn playback_message_queue(&self) -> Arc<ArrayQueue<SynthPlaybackMessage>> {
        self.0.playback_message_queue()
    }

    fn playback_status_sender(&self) -> Option<SyncSender<PlaybackStatusEvent>> {
        self.0.playback_status_sender()
    }

    fn set_playback_status_sender(&mut self, sender: Option<SyncSender<PlaybackStatusEvent>>) {
        self.0.set_playback_status_sender(sender)
    }

    fn playback_status_context(&self) -> Option<crate::source::status::PlaybackStatusContext> {
        self.0.playback_status_context()
    }

    fn set_playback_status_context(
        &mut self,
        context: Option<crate::source::status::PlaybackStatusContext>,
    ) {
        self.0.set_playback_status_context(context)
    }
}

impl Source for FunDspSynthSource {
    fn write(&mut self, output: &mut [f32], time: &crate::SourceTime) -> usize {
        self.0.write(output, time)
    }

    fn channel_count(&self) -> usize {
        self.0.channel_count()
    }

    fn sample_rate(&self) -> u32 {
        self.0.sample_rate()
    }

    fn is_exhausted(&self) -> bool {
        self.0.is_exhausted()
    }
}

// -------------------------------------------------------------------------------------------------

impl Player {
    /// Play a mono or stereo [FunDSP](https://github.com/SamiPerttu/fundsp) AudioNode with the
    /// given options. See [`SynthPlaybackOptions`] for more info about available options.
    ///
    /// The node will play until it is exhausted (stops producing audio for more than 2 seconds).
    ///
    /// Example one-shot sound:
    /// ```rust, no_run
    /// use phonic::fundsp::prelude32::*;
    ///
    /// // Create a 2-second sine wave at 440 Hz
    /// let duration = 2.0;
    /// let freq = 440.0;
    /// let node = envelope(|t| if t < duration { 1.0 } else { 0.0 }) * sine_hz(freq);
    /// let audio_unit: Box<dyn AudioUnit> = Box::new(node);
    /// ```
    #[cfg(feature = "fundsp")]
    pub fn play_fundsp_synth(
        &mut self,
        generator_name: &str,
        audio_unit: Box<dyn AudioUnit>,
        options: SynthPlaybackOptions,
    ) -> Result<SynthPlaybackHandle, Error> {
        self.play_fundsp_synth_with_context(generator_name, audio_unit, options, None)
    }

    /// Play a mono or stereo [FunDSP](https://github.com/SamiPerttu/fundsp) AudioNode with the
    /// given options and a custom playback status context.
    #[cfg(feature = "fundsp")]
    pub fn play_fundsp_synth_with_context(
        &mut self,
        generator_name: &str,
        audio_unit: Box<dyn AudioUnit>,
        options: SynthPlaybackOptions,
        context: Option<PlaybackStatusContext>,
    ) -> Result<SynthPlaybackHandle, Error> {
        // create synth source
        let source = FunDspSynthSource::new(
            generator_name,
            audio_unit,
            options,
            self.output_sample_rate(),
        )?;
        // and play it
        self.play_synth_source_with_context(source, options.start_time, context)
    }
}
