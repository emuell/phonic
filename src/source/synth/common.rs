use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use crossbeam_channel::Sender;
use crossbeam_queue::ArrayQueue;

use super::{SynthPlaybackMessage, SynthPlaybackOptions, SynthSource};
use crate::{
    player::{PlaybackId, PlaybackStatusContext, PlaybackStatusEvent},
    source::{Source, SourceTime},
    utils::{
        fader::{FaderState, VolumeFader},
        unique_usize_id,
    },
    Error,
};

// -------------------------------------------------------------------------------------------------

/// A generic sample generator for SynthSourceImpl.
pub trait SynthSourceGenerator {
    /// Fill passed output with generated samples and return samples generated.
    fn generate(&mut self, output: &mut [f32]) -> usize;
    /// returns true when output is silent oan no more generate calls are required.
    fn is_exhausted(&self) -> bool;
}

// -------------------------------------------------------------------------------------------------

/// A synth source which runs an externally defined synth source generator.
pub struct SynthSourceImpl<Generator>
where
    Generator: SynthSourceGenerator + Send + Sync,
{
    generator: Box<Generator>,
    sample_rate: u32,
    volume_fader: VolumeFader,
    playback_message_queue: Arc<ArrayQueue<SynthPlaybackMessage>>,
    playback_status_send: Option<Sender<PlaybackStatusEvent>>,
    playback_status_context: Option<PlaybackStatusContext>,
    playback_id: PlaybackId,
    playback_name: Arc<String>,
    playback_options: SynthPlaybackOptions,
    playback_pos: u64,
    playback_pos_report_instant: Instant,
    playback_finished: bool,
}

impl<Generator> SynthSourceImpl<Generator>
where
    Generator: SynthSourceGenerator + Send + Sync,
{
    const CHANNEL_COUNT: usize = 1;

    #[allow(dead_code)]
    pub fn new(
        generator: Generator,
        generator_name: &str,
        options: SynthPlaybackOptions,
        sample_rate: u32,
        event_send: Option<Sender<PlaybackStatusEvent>>,
    ) -> Result<Self, Error> {
        // validate options
        options.validate()?;
        // create volume fader
        let mut volume_fader = VolumeFader::new(Self::CHANNEL_COUNT, sample_rate);
        if let Some(duration) = options.fade_in_duration {
            volume_fader.start_fade_in(duration);
        }
        let playback_message_queue = Arc::new(ArrayQueue::new(128));
        Ok(Self {
            generator: Box::new(generator),
            sample_rate,
            volume_fader,
            playback_message_queue,
            playback_status_send: event_send,
            playback_id: unique_usize_id(),
            playback_status_context: None,
            playback_name: Arc::new(generator_name.to_string()),
            playback_options: options,
            playback_pos: 0,
            playback_pos_report_instant: Instant::now(),
            playback_finished: false,
        })
    }

    fn should_report_pos(&mut self) -> bool {
        if let Some(report_duration) = self.playback_options.playback_pos_emit_rate {
            let should_report = self.playback_pos_report_instant.elapsed() >= report_duration;
            self.playback_pos_report_instant = Instant::now();
            should_report
        } else {
            false
        }
    }

    fn samples_to_duration(&self, samples: u64) -> Duration {
        let frames = samples / Self::CHANNEL_COUNT as u64;
        let seconds = frames as f64 / self.sample_rate as f64;
        Duration::from_millis((seconds * 1000.0) as u64)
    }
}

impl<Generator> SynthSource for SynthSourceImpl<Generator>
where
    Generator: SynthSourceGenerator + Send + Sync + 'static,
{
    fn playback_id(&self) -> PlaybackId {
        self.playback_id
    }

    fn playback_options(&self) -> &SynthPlaybackOptions {
        &self.playback_options
    }

    fn playback_message_queue(&self) -> Arc<ArrayQueue<SynthPlaybackMessage>> {
        self.playback_message_queue.clone()
    }

    fn playback_status_sender(&self) -> Option<Sender<PlaybackStatusEvent>> {
        self.playback_status_send.clone()
    }
    fn set_playback_status_sender(&mut self, sender: Option<Sender<PlaybackStatusEvent>>) {
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
    fn write(&mut self, output: &mut [f32], _time: &SourceTime) -> usize {
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
        if self.should_report_pos() {
            if let Some(event_send) = &self.playback_status_send {
                // NB: try_send: we want to ignore full channels on playback pos events and don't want to block
                if let Err(err) = event_send.try_send(PlaybackStatusEvent::Position {
                    id: self.playback_id,
                    context: self.playback_status_context.clone(),
                    path: self.playback_name.clone(),
                    position: self.samples_to_duration(self.playback_pos),
                }) {
                    log::warn!("Failed to send playback event: {}", err)
                }
            }
        }

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
                    log::warn!("failed to send synth playback status event: {}", err);
                }
            }
        }

        written
    }

    fn channel_count(&self) -> usize {
        Self::CHANNEL_COUNT
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn is_exhausted(&self) -> bool {
        self.playback_finished
    }
}
