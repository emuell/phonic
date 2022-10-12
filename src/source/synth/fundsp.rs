use std::time::{Duration, Instant};

use crossbeam_channel::{unbounded, Receiver, Sender};

use super::{SynthPlaybackMessage, SynthPlaybackOptions, SynthSource};
use crate::{
    player::{AudioFilePlaybackId, AudioFilePlaybackStatusEvent},
    source::{AudioSource, AudioSourceTime},
    utils::{
        fader::{FaderState, VolumeFader},
        unique_usize_id,
    },
};

use fundsp::prelude::*;

// -------------------------------------------------------------------------------------------------

/// A synth source which runs a fundsp::AudioUnit64 generator (no inputs, one output).
/// When no audible signal was generated for more than half a second the source will stop playing.
pub struct FunDspSynthSource {
    unit: Box<dyn fundsp::audiounit::AudioUnit64>,
    sample_rate: u32,
    volume: f32,
    volume_fader: VolumeFader,
    fade_out_duration: Option<Duration>,
    send: Sender<SynthPlaybackMessage>,
    recv: Receiver<SynthPlaybackMessage>,
    event_send: Option<Sender<AudioFilePlaybackStatusEvent>>,
    playback_id: AudioFilePlaybackId,
    playback_name: String,
    playback_pos: u64,
    playback_pos_report_instant: Instant,
    playback_pos_emit_rate: Option<Duration>,
    playback_finished: bool,
    playback_silence_count: u64,
}

impl FunDspSynthSource {
    const CHANNEL_COUNT: usize = 1;

    pub fn new(
        mut unit: impl AudioUnit64 + 'static,
        unit_name: &str,
        options: SynthPlaybackOptions,
        sample_rate: u32,
        event_send: Option<Sender<AudioFilePlaybackStatusEvent>>,
    ) -> Self {
        // ensure unit is using the correct sample rate
        unit.reset(Some(sample_rate as f64));
        // create volume fader
        let mut volume_fader = VolumeFader::new(Self::CHANNEL_COUNT, sample_rate);
        if let Some(duration) = options.fade_in_duration {
            volume_fader.start_fade_in(duration);
        }
        let (send, recv) = unbounded::<SynthPlaybackMessage>();
        Self {
            unit: Box::new(unit),
            sample_rate,
            volume: options.volume,
            volume_fader,
            fade_out_duration: options.fade_out_duration,
            send,
            recv,
            event_send,
            playback_id: unique_usize_id(),
            playback_name: unit_name.to_string(),
            playback_pos: 0,
            playback_pos_report_instant: Instant::now(),
            playback_pos_emit_rate: options.playback_pos_emit_rate,
            playback_finished: false,
            playback_silence_count: 0,
        }
    }

    fn should_report_pos(&self) -> bool {
        if let Some(report_duration) = self.playback_pos_emit_rate {
            self.playback_pos_report_instant.elapsed() >= report_duration
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

impl SynthSource for FunDspSynthSource {
    fn playback_message_sender(&self) -> Sender<SynthPlaybackMessage> {
        self.send.clone()
    }

    fn playback_id(&self) -> AudioFilePlaybackId {
        self.playback_id
    }
}

impl AudioSource for FunDspSynthSource {
    fn write(&mut self, output: &mut [f32], _time: &AudioSourceTime) -> usize {
        // receive playback events
        let mut stop_playing = false;
        if let Ok(msg) = self.recv.try_recv() {
            match msg {
                SynthPlaybackMessage::Stop => {
                    if let Some(duration) = self.fade_out_duration {
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

        // run unit in MAX_BUFFER_SIZE blocks
        let mut temp_buffer = [0f64; MAX_BUFFER_SIZE];
        let mut exhausted_test_sum = 0.0;
        let mut written = 0;
        while written < output.len() {
            let frames_left = output.len() - written;
            let to_write = std::cmp::Ord::min(frames_left, MAX_BUFFER_SIZE as usize);
            self.unit.process(to_write, &[], &mut [&mut temp_buffer]);
            let out = &mut output[written..written + to_write];
            for (o, i) in out.iter_mut().zip(temp_buffer) {
                *o = i as f32;
                exhausted_test_sum += i;
            }
            written += to_write;
        }

        // check if output is exhausted (produced silence for longer than half a second)
        let mut is_exhausted = false;
        if (exhausted_test_sum / written as f64).abs() < 0.0000001 {
            self.playback_silence_count += written as u64;
            if self.playback_silence_count > self.sample_rate as u64 / 2 {
                is_exhausted = true;
            }
        } else {
            self.playback_silence_count = 0;
        }

        // apply volume when <> 1
        if (1.0 - self.volume).abs() > 0.0001 {
            for o in output[0..written].as_mut() {
                *o *= self.volume;
            }
        }
        // apply volume fader
        self.volume_fader.process(&mut output[0..written]);

        // update playback pos
        self.playback_pos += written as u64;

        // send Position change Event
        if let Some(event_send) = &self.event_send {
            if self.should_report_pos() {
                self.playback_pos_report_instant = Instant::now();
                // NB: try_send: we want to ignore full channels on playback pos events and don't want to block
                if let Err(err) = event_send.try_send(AudioFilePlaybackStatusEvent::Position {
                    id: self.playback_id,
                    path: self.playback_name.clone(),
                    position: self.samples_to_duration(self.playback_pos),
                }) {
                    log::warn!("Failed to send playback event: {}", err)
                }
            }
        }

        // check if the signal is exhausted and send Stopped event
        let is_exhausted = is_exhausted || written == 0;
        let fade_out_finished = self.volume_fader.state() == FaderState::Finished
            && self.volume_fader.target_volume() == 0.0;
        if stop_playing || is_exhausted || fade_out_finished {
            self.playback_finished = true;
            if let Some(event_send) = &self.event_send {
                if let Err(err) = event_send.send(AudioFilePlaybackStatusEvent::Stopped {
                    id: self.playback_id,
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

unsafe impl Sync for FunDspSynthSource {}
