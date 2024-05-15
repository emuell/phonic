use std::sync::Arc;

use crossbeam_channel::Sender;
use crossbeam_queue::ArrayQueue;
use fundsp::prelude::*;

use crate::{
    player::{AudioFilePlaybackId, AudioFilePlaybackStatusContext, AudioFilePlaybackStatusEvent},
    source::{synth::SynthPlaybackOptions, AudioSource, AudioSourceTime},
    Error,
};

use super::{
    common::{SynthSourceGenerator, SynthSourceImpl},
    SynthPlaybackMessage, SynthSource,
};

// -------------------------------------------------------------------------------------------------

/// A synth source generator which runs a fundsp::AudioUnit64 generator (no inputs, one output).
/// When no audible signal was generated for more than half a second it's treated as "exhausted".
pub(crate) struct FunDspSynthGenerator {
    unit: Box<dyn AudioUnit64>,
    sample_rate: u32,
    silence_count: u64,
    is_exhausted: bool,
}

impl FunDspSynthGenerator {
    pub fn new(mut unit: impl AudioUnit64 + 'static, sample_rate: u32) -> Self {
        // set target sample rate to unit
        unit.reset();
        // preallocate all needed memory in the main thread to avoid allocating in
        // real-time threads later on...
        unit.allocate();
        Self {
            unit: Box::new(unit),
            sample_rate,
            silence_count: 0,
            is_exhausted: false,
        }
    }
}

impl SynthSourceGenerator for FunDspSynthGenerator {
    fn generate(&mut self, output: &mut [f32]) -> usize {
        // run unit in MAX_BUFFER_SIZE blocks
        let mut temp_buffer = [0f64; MAX_BUFFER_SIZE];
        let mut exhausted_test_sum = 0.0;
        let mut written = 0;
        while written < output.len() {
            let frames_left = output.len() - written;
            let to_write = std::cmp::Ord::min(frames_left, MAX_BUFFER_SIZE);
            self.unit.process(to_write, &[], &mut [&mut temp_buffer]);
            let out = &mut output[written..written + to_write];
            for (o, i) in out.iter_mut().zip(temp_buffer) {
                *o = i as f32;
                exhausted_test_sum += i;
            }
            written += to_write;
        }

        // check if output is exhausted (produced silence for longer than half a second)
        if !self.is_exhausted {
            if (exhausted_test_sum / written as f64).abs() < 0.0000001 {
                self.silence_count += written as u64;
                if self.silence_count > self.sample_rate as u64 / 2 {
                    self.is_exhausted = true;
                }
            } else {
                self.silence_count = 0;
            }
        }
        written
    }

    fn is_exhausted(&self) -> bool {
        self.is_exhausted
    }
}

unsafe impl Sync for FunDspSynthGenerator {}

// -------------------------------------------------------------------------------------------------

/// A synth source which runs a fundsp::AudioUnit64 generator.
pub struct FunDspSynthSource(SynthSourceImpl<FunDspSynthGenerator>);

impl FunDspSynthSource {
    pub fn new(
        unit: impl AudioUnit64 + 'static,
        unit_name: &str,
        options: SynthPlaybackOptions,
        sample_rate: u32,
        event_send: Option<Sender<AudioFilePlaybackStatusEvent>>,
    ) -> Result<Self, Error> {
        Ok(Self(SynthSourceImpl::new(
            FunDspSynthGenerator::new(unit, sample_rate),
            unit_name,
            options,
            sample_rate,
            event_send,
        )?))
    }
}

impl SynthSource for FunDspSynthSource {
    fn playback_id(&self) -> AudioFilePlaybackId {
        self.0.playback_id()
    }

    fn playback_message_queue(&self) -> Arc<ArrayQueue<SynthPlaybackMessage>> {
        self.0.playback_message_queue()
    }

    fn playback_status_sender(&self) -> Option<Sender<AudioFilePlaybackStatusEvent>> {
        self.0.playback_status_sender()
    }
    fn set_playback_status_sender(&mut self, sender: Option<Sender<AudioFilePlaybackStatusEvent>>) {
        self.0.set_playback_status_sender(sender);
    }

    fn playback_status_context(&self) -> Option<AudioFilePlaybackStatusContext> {
        self.0.playback_status_context()
    }
    fn set_playback_status_context(&mut self, context: Option<AudioFilePlaybackStatusContext>) {
        self.0.set_playback_status_context(context);
    }
}

impl AudioSource for FunDspSynthSource {
    fn write(&mut self, output: &mut [f32], time: &AudioSourceTime) -> usize {
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
