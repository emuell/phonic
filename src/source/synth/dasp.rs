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

// -------------------------------------------------------------------------------------------------

/// A synth source which runs a dasp Signal until it is exhausted.
pub struct DaspSynthSource<SignalType>
where
    SignalType: dasp::Signal<Frame = f64>,
{
    signal: dasp::signal::UntilExhausted<SignalType>,
    sample_rate: u32,
    volume: f32,
    stop_fader: VolumeFader,
    send: Sender<SynthPlaybackMessage>,
    recv: Receiver<SynthPlaybackMessage>,
    event_send: Option<Sender<AudioFilePlaybackStatusEvent>>,
    playback_id: AudioFilePlaybackId,
    playback_name: String,
    playback_finished: bool,
}

impl<SignalType> DaspSynthSource<SignalType>
where
    SignalType: dasp::Signal<Frame = f64>,
{
    pub fn new(
        signal: SignalType,
        signal_name: &str,
        options: SynthPlaybackOptions,
        sample_rate: u32,
        event_send: Option<Sender<AudioFilePlaybackStatusEvent>>,
    ) -> Self {
        let (send, recv) = unbounded::<SynthPlaybackMessage>();
        let channel_count = 1;
        let is_exhausted = false;
        Self {
            signal: signal.until_exhausted(),
            sample_rate,
            volume: options.volume,
            stop_fader: VolumeFader::new(channel_count, sample_rate),
            send,
            recv,
            event_send,
            playback_id: unique_usize_id(),
            playback_name: signal_name.to_string(),
            playback_finished: is_exhausted,
        }
    }
}

impl<SignalType> SynthSource for DaspSynthSource<SignalType>
where
    SignalType: dasp::Signal<Frame = f64> + Send + 'static,
{
    fn playback_message_sender(&self) -> Sender<SynthPlaybackMessage> {
        self.send.clone()
    }

    fn playback_id(&self) -> AudioFilePlaybackId {
        self.playback_id
    }
}

impl<SignalType> AudioSource for DaspSynthSource<SignalType>
where
    SignalType: dasp::Signal<Frame = f64> + Send + 'static,
{
    fn write(&mut self, output: &mut [f32], _time: &AudioSourceTime) -> usize {
        // receive playback events
        let mut stop_playing = false;
        if let Ok(msg) = self.recv.try_recv() {
            match msg {
                SynthPlaybackMessage::Stop(fadeout) => {
                    if fadeout.is_zero() {
                        stop_playing = true;
                    } else {
                        self.stop_fader.start(fadeout);
                    }
                }
            }
        }

        // return empty handed when playback finished
        if self.playback_finished {
            return 0;
        }

        // run signal on output until exhausted
        let mut written = 0;
        for (o, i) in output.iter_mut().zip(&mut self.signal) {
            *o = i as f32;
            written += 1;
        }

        // apply volume when <> 1
        if (1.0f32 - self.volume).abs() > 0.0001 {
            for o in output[0..written].as_mut() {
                *o *= self.volume;
            }
        }
        // apply volume fader
        self.stop_fader.process(&mut output[0..written]);

        // check if the signal is exhausted and send Stopped event
        let is_exhausted = written == 0;
        let fadeout_completed = self.stop_fader.state() == FaderState::Finished;
        if stop_playing || is_exhausted || fadeout_completed {
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
        1
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn is_exhausted(&self) -> bool {
        self.playback_finished
    }
}
