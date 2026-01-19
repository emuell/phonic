use std::{sync::mpsc::SyncSender, sync::Arc, time::Duration};

use crossbeam_queue::ArrayQueue;

use super::{SynthPlaybackMessage, SynthPlaybackOptions, SynthSource};

use crate::{
    player::PlaybackId,
    source::{
        status::{PlaybackStatusContext, PlaybackStatusEvent},
        unique_source_id, Source, SourceTime,
    },
    utils::{
        fader::{FaderState, VolumeFader},
        time::{SampleTime, SampleTimeClock},
    },
    Error,
};

// -------------------------------------------------------------------------------------------------

/// A generic sample generator for [`SynthSourceImpl`].
pub trait SynthSourceGenerator {
    /// Returns true when output is silent oan no more generate calls are required.
    fn is_exhausted(&self) -> bool;

    /// The generator produces sample frames with this number of channels.
    fn channel_count(&self) -> usize;

    /// Fill passed output with generated samples and return samples generated.
    fn generate(&mut self, output: &mut [f32]) -> usize;
}

// -------------------------------------------------------------------------------------------------

/// A synth source which runs an externally defined synth source generator.
pub struct SynthSourceImpl<Generator>
where
    Generator: SynthSourceGenerator + Send + Sync,
{
    generator: Box<Generator>,
    sample_rate: u32,
    channel_count: usize,
    volume_fader: VolumeFader,
    playback_message_queue: Arc<ArrayQueue<SynthPlaybackMessage>>,
    playback_status_send: Option<SyncSender<PlaybackStatusEvent>>,
    playback_status_context: Option<PlaybackStatusContext>,
    playback_id: PlaybackId,
    playback_name: Arc<String>,
    playback_options: SynthPlaybackOptions,
    playback_pos: u64,
    playback_pos_emit_rate: Option<SampleTime>,
    playback_pos_sample_time_clock: SampleTimeClock,
    playback_finished: bool,
}

impl<Generator> SynthSourceImpl<Generator>
where
    Generator: SynthSourceGenerator + Send + Sync,
{
    #[allow(dead_code)]
    pub fn new(
        generator_name: &str,
        generator: Generator,
        options: SynthPlaybackOptions,
        sample_rate: u32,
    ) -> Result<Self, Error> {
        // validate options
        options.validate()?;
        let channel_count = generator.channel_count();
        // create volume fader
        let mut volume_fader = VolumeFader::new(channel_count, sample_rate);
        if let Some(duration) = options.fade_in_duration {
            volume_fader.start_fade_in(duration);
        }
        let playback_message_queue = Arc::new(ArrayQueue::new(128));
        let playback_status_send = None;

        let playback_pos_emit_rate = options
            .playback_pos_emit_rate
            .map(|d| SampleTimeClock::duration_to_sample_time(d, sample_rate));
        let playback_pos_sample_time_clock = SampleTimeClock::new(sample_rate);

        Ok(Self {
            generator: Box::new(generator),
            sample_rate,
            channel_count,
            volume_fader,
            playback_message_queue,
            playback_status_send,
            playback_id: unique_source_id(),
            playback_status_context: None,
            playback_name: Arc::new(generator_name.to_string()),
            playback_options: options,
            playback_pos: 0,
            playback_pos_emit_rate,
            playback_pos_sample_time_clock,
            playback_finished: false,
        })
    }

    fn should_report_pos(&self, time: &SourceTime, is_start_event: bool) -> bool {
        if let Some(emit_rate) = self.playback_pos_emit_rate {
            is_start_event
                || self
                    .playback_pos_sample_time_clock
                    .elapsed(time.pos_in_frames)
                    >= emit_rate
        } else {
            false
        }
    }

    fn send_position_event(&mut self, time: &SourceTime, is_start_event: bool) {
        if let Some(event_send) = &self.playback_status_send {
            if self.should_report_pos(time, is_start_event) {
                self.playback_pos_sample_time_clock
                    .reset(time.pos_in_frames);
                // NB: try_send: we want to ignore full channels on playback pos events and don't want to block
                if let Err(err) = event_send.try_send(PlaybackStatusEvent::Position {
                    id: self.playback_id,
                    context: self.playback_status_context.clone(),
                    path: self.playback_name.clone(),
                    position: self.samples_to_duration(self.playback_pos),
                }) {
                    log::warn!("Failed to send file playback event: {err}")
                }
            }
        }
    }

    fn samples_to_duration(&self, samples: u64) -> Duration {
        let frames = samples / self.channel_count as u64;
        let seconds = frames as f64 / self.sample_rate as f64;
        Duration::from_millis((seconds * 1000.0) as u64)
    }
}

impl<Generator> SynthSource for SynthSourceImpl<Generator>
where
    Generator: SynthSourceGenerator + Send + Sync + 'static,
{
    fn synth_name(&self) -> String {
        self.playback_name.to_string()
    }

    fn playback_id(&self) -> PlaybackId {
        self.playback_id
    }

    fn playback_options(&self) -> &SynthPlaybackOptions {
        &self.playback_options
    }

    fn playback_message_queue(&self) -> Arc<ArrayQueue<SynthPlaybackMessage>> {
        self.playback_message_queue.clone()
    }

    fn playback_status_sender(&self) -> Option<SyncSender<PlaybackStatusEvent>> {
        self.playback_status_send.clone()
    }
    fn set_playback_status_sender(&mut self, sender: Option<SyncSender<PlaybackStatusEvent>>) {
        self.playback_status_send = sender;
    }

    fn playback_status_context(&self) -> Option<PlaybackStatusContext> {
        self.playback_status_context.clone()
    }
    fn set_playback_status_context(&mut self, context: Option<PlaybackStatusContext>) {
        self.playback_status_context = context;
    }
}

impl<Generator> Source for SynthSourceImpl<Generator>
where
    Generator: SynthSourceGenerator + Send + Sync + 'static,
{
    fn channel_count(&self) -> usize {
        self.channel_count
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn is_exhausted(&self) -> bool {
        self.playback_finished
    }

    fn weight(&self) -> usize {
        2
    }

    fn write(&mut self, output: &mut [f32], time: &SourceTime) -> usize {
        // receive playback events
        let mut stop_playing = false;
        if let Some(msg) = self.playback_message_queue.pop() {
            match msg {
                SynthPlaybackMessage::Stop => {
                    if let Some(duration) = self.playback_options.fade_out_duration {
                        if !duration.is_zero() {
                            self.volume_fader.start_fade_out(duration);
                        } else {
                            stop_playing = true;
                        }
                    } else {
                        stop_playing = true;
                    }
                }
            }
        }

        // send Start Event
        if self.playback_pos == 0 {
            let is_start_event = true;
            self.send_position_event(time, is_start_event);
        }

        // return empty handed when playback finished
        if self.playback_finished {
            return 0;
        }

        // run generator
        let written = self.generator.generate(output);

        // apply volume option when <> 1
        if (1.0 - self.playback_options.volume).abs() > 0.0001 {
            let volume = self.playback_options.volume;
            for o in output[0..written].as_mut() {
                *o *= volume;
            }
        }
        // apply volume fader
        self.volume_fader.process(&mut output[0..written]);

        // update playback pos
        self.playback_pos += written as u64;

        // send Position Event
        let is_start_event = false;
        self.send_position_event(time, is_start_event);

        // check if the signal is exhausted and send Stopped event
        let is_exhausted = self.generator.is_exhausted() || written == 0;
        let fade_out_finished = self.volume_fader.state() == FaderState::Finished
            && self.volume_fader.target_volume() == 0.0;
        if stop_playing || is_exhausted || fade_out_finished {
            self.playback_finished = true;
            if let Some(event_send) = &self.playback_status_send {
                if let Err(err) = event_send.try_send(PlaybackStatusEvent::Stopped {
                    id: self.playback_id,
                    context: self.playback_status_context.clone(),
                    path: self.playback_name.clone(),
                    exhausted: self.playback_finished,
                }) {
                    log::warn!("Failed to send synth playback status event: {err}");
                }
            }
        }

        written
    }
}
