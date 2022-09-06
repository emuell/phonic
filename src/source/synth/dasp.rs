use std::sync::atomic::AtomicUsize;

use crossbeam_channel::{unbounded, Receiver, Sender};
use dasp::{signal::UntilExhausted, Signal};

use super::{AudioSource, SynthId, SynthPlaybackMsg, SynthPlaybackStatusMsg, SynthSource};

// -------------------------------------------------------------------------------------------------

/// A synth source which runs a dasp Signal until it is exhausted
pub struct DaspSynthSource<SignalType>
where
    SignalType: Signal<Frame = f32>,
{
    signal: UntilExhausted<SignalType>,
    sample_rate: u32,
    send: Sender<SynthPlaybackMsg>,
    recv: Receiver<SynthPlaybackMsg>,
    event_send: Option<Sender<SynthPlaybackStatusMsg>>,
    synth_id: SynthId,
    is_exhausted: bool,
}

impl<SignalType> DaspSynthSource<SignalType>
where
    SignalType: Signal<Frame = f32>,
{
    pub fn new(
        signal: SignalType,
        sample_rate: u32,
        event_send: Option<Sender<SynthPlaybackStatusMsg>>,
    ) -> Self {
        static SYNTH_ID_COUNTER: AtomicUsize = AtomicUsize::new(1);
        let (send, recv) = unbounded::<SynthPlaybackMsg>();
        let is_exhausted = false;
        Self {
            signal: signal.until_exhausted(),
            sample_rate,
            send,
            recv,
            event_send,
            synth_id: SYNTH_ID_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            is_exhausted,
        }
    }
}

impl<SignalType> SynthSource for DaspSynthSource<SignalType>
where
    SignalType: Signal<Frame = f32> + Send + 'static,
{
    /// Channel to control playback
    fn sender(&self) -> Sender<SynthPlaybackMsg> {
        self.send.clone()
    }

    /// The unique synth ID, can be used to identify files in SynthPlaybackStatusMsg events
    fn synth_id(&self) -> SynthId {
        self.synth_id
    }
}

impl<SignalType> AudioSource for DaspSynthSource<SignalType>
where
    SignalType: Signal<Frame = f32> + Send + 'static,
{
    fn write(&mut self, output: &mut [f32]) -> usize {
        // receive playback events
        let mut send_exhausted_event = false;
        if let Ok(msg) = self.recv.try_recv() {
            match msg {
                SynthPlaybackMsg::Stop => {
                    self.is_exhausted = true;
                    send_exhausted_event = true;
                }
            }
        }
        if !send_exhausted_event && self.is_exhausted {
            return 0;
        }
        // run signal on output until exhausted
        let mut written = 0;
        for (o, i) in output.iter_mut().zip(&mut self.signal) {
            *o = i;
            written += 1;
        }
        if written == 0 && !self.is_exhausted {
            self.is_exhausted = true;
            send_exhausted_event = true;
        }
        // send status messages
        if send_exhausted_event {
            if let Some(event_send) = &self.event_send {
                if let Err(err) = event_send.send(SynthPlaybackStatusMsg::Exhausted {
                    synth_id: self.synth_id,
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
}
