use crossbeam_channel::{unbounded, Receiver, Sender};

use super::{SynthPlaybackMessage, SynthSource};
use crate::{
    source::{
        playback::{PlaybackId, PlaybackStatusEvent},
        AudioSource,
    },
    utils::id::unique_usize_id,
};

// -------------------------------------------------------------------------------------------------

/// A synth source which runs a dasp Signal until it is exhausted
pub struct DaspSynthSource<SignalType>
where
    SignalType: dasp::Signal<Frame = f64>,
{
    signal: dasp::signal::UntilExhausted<SignalType>,
    sample_rate: u32,
    volume: f32,
    send: Sender<SynthPlaybackMessage>,
    recv: Receiver<SynthPlaybackMessage>,
    event_send: Option<Sender<PlaybackStatusEvent>>,
    playback_id: PlaybackId,
    playback_name: String,
    is_exhausted: bool,
}

impl<SignalType> DaspSynthSource<SignalType>
where
    SignalType: dasp::Signal<Frame = f64>,
{
    pub fn new(
        signal: SignalType,
        signal_name: &str,
        volume: f32,
        sample_rate: u32,
        event_send: Option<Sender<PlaybackStatusEvent>>,
    ) -> Self {
        let (send, recv) = unbounded::<SynthPlaybackMessage>();
        let is_exhausted = false;
        Self {
            signal: signal.until_exhausted(),
            sample_rate,
            volume,
            send,
            recv,
            event_send,
            playback_id: unique_usize_id(),
            playback_name: signal_name.to_string(),
            is_exhausted,
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

    fn playback_id(&self) -> PlaybackId {
        self.playback_id
    }
}

impl<SignalType> AudioSource for DaspSynthSource<SignalType>
where
    SignalType: dasp::Signal<Frame = f64> + Send + 'static,
{
    fn write(&mut self, output: &mut [f32]) -> usize {
        // receive playback events
        let mut keep_playing = true;
        if let Ok(msg) = self.recv.try_recv() {
            match msg {
                SynthPlaybackMessage::Stop => {
                    keep_playing = false;
                }
            }
        }
        if self.is_exhausted {
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
        // check if the signal is exhausted
        if written == 0 && !self.is_exhausted {
            self.is_exhausted = true;
        }
        // send status messages
        if self.is_exhausted || !keep_playing {
            if let Some(event_send) = &self.event_send {
                if let Err(err) = event_send.send(PlaybackStatusEvent::Stopped {
                    id: self.playback_id,
                    path: self.playback_name.clone(),
                    exhausted: self.is_exhausted,
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
        self.is_exhausted
    }
}
