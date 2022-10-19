use crossbeam_channel::Sender;
use dasp::{signal::UntilExhausted, Signal};

use crate::{
    AudioFilePlaybackId, AudioFilePlaybackStatusEvent, AudioSource, AudioSourceTime, Error,
    SynthPlaybackOptions,
};

use super::{
    common::{SynthSourceGenerator, SynthSourceImpl},
    SynthPlaybackMessage, SynthSource,
};

// -------------------------------------------------------------------------------------------------

/// A synth generator which runs a dasp Signal until it is exhausted.
pub struct DaspSynthGenerator<SignalType>
where
    SignalType: Signal<Frame = f64>,
{
    signal: UntilExhausted<SignalType>,
    is_exhausted: bool,
}

impl<SignalType> DaspSynthGenerator<SignalType>
where
    SignalType: dasp::Signal<Frame = f64>,
{
    pub fn new(signal: SignalType, _sample_rate: u32) -> Self {
        Self {
            signal: signal.until_exhausted(),
            is_exhausted: false,
        }
    }
}

impl<SignalType> SynthSourceGenerator for DaspSynthGenerator<SignalType>
where
    SignalType: dasp::Signal<Frame = f64> + Send + Sync + 'static,
{
    fn generate(&mut self, output: &mut [f32]) -> usize {
        // run signal on output until exhausted
        let mut written = 0;
        for (o, i) in output.iter_mut().zip(&mut self.signal) {
            *o = i as f32;
            written += 1;
        }
        self.is_exhausted = written == 0;
        written
    }

    fn is_exhausted(&self) -> bool {
        self.is_exhausted
    }
}

// -------------------------------------------------------------------------------------------------

/// A synth source which runs a dasp Signal until it is exhausted.
pub struct DaspSynthSource<SignalType>(SynthSourceImpl<DaspSynthGenerator<SignalType>>)
where
    SignalType: Signal<Frame = f64> + Send + Sync + 'static;

impl<SignalType> DaspSynthSource<SignalType>
where
    SignalType: dasp::Signal<Frame = f64> + Send + Sync + 'static,
{
    pub fn new(
        signal: SignalType,
        signal_name: &str,
        options: SynthPlaybackOptions,
        sample_rate: u32,
        event_send: Option<Sender<AudioFilePlaybackStatusEvent>>,
    ) -> Result<Self, Error> {
        Ok(Self(SynthSourceImpl::new(
            DaspSynthGenerator::new(signal, sample_rate),
            signal_name,
            options,
            sample_rate,
            event_send,
        )?))
    }
}

impl<SignalType> SynthSource for DaspSynthSource<SignalType>
where
    SignalType: dasp::Signal<Frame = f64> + Send + Sync + 'static,
{
    fn playback_message_sender(&self) -> Sender<SynthPlaybackMessage> {
        self.0.playback_message_sender()
    }

    fn playback_id(&self) -> AudioFilePlaybackId {
        self.0.playback_id()
    }
}

impl<SignalType> AudioSource for DaspSynthSource<SignalType>
where
    SignalType: dasp::Signal<Frame = f64> + Send + Sync + 'static,
{
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
