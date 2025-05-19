use std::sync::Arc;

use crossbeam_channel::Sender;
use crossbeam_queue::ArrayQueue;
use dasp::{signal::UntilExhausted, Signal};

use crate::{
    player::{PlaybackId, PlaybackStatusContext, PlaybackStatusEvent},
    source::{synth::SynthPlaybackOptions, Source, SourceTime},
    Error, Player,
};

use super::{
    common::{SynthSourceGenerator, SynthSourceImpl},
    SynthPlaybackMessage, SynthSource,
};

// -------------------------------------------------------------------------------------------------

/// A synth generator which runs a dasp `Signal` until it is exhausted.
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

/// A [`SynthSource`] which runs a dasp `Signal` until it is exhausted.
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
        event_send: Option<Sender<PlaybackStatusEvent>>,
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
    fn playback_id(&self) -> PlaybackId {
        self.0.playback_id()
    }

    fn playback_message_queue(&self) -> Arc<ArrayQueue<SynthPlaybackMessage>> {
        self.0.playback_message_queue()
    }

    fn playback_status_sender(&self) -> Option<Sender<PlaybackStatusEvent>> {
        self.0.playback_status_sender()
    }
    fn set_playback_status_sender(&mut self, sender: Option<Sender<PlaybackStatusEvent>>) {
        self.0.set_playback_status_sender(sender);
    }

    fn playback_status_context(&self) -> Option<PlaybackStatusContext> {
        self.0.playback_status_context()
    }
    fn set_playback_status_context(&mut self, context: Option<PlaybackStatusContext>) {
        self.0.set_playback_status_context(context);
    }
}

impl<SignalType> Source for DaspSynthSource<SignalType>
where
    SignalType: dasp::Signal<Frame = f64> + Send + Sync + 'static,
{
    fn write(&mut self, output: &mut [f32], time: &SourceTime) -> usize {
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
    /// Play a mono [dasp](https://github.com/RustAudio/dasp) signal with the given options.
    /// See [`SynthPlaybackOptions`] for more info about available options.
    ///
    /// The signal will be wrapped into a dasp::signal::UntilExhausted so it can be used to play
    /// create one-shots.
    ///
    /// Example one-shot signal:
    /// ```ignore
    /// dasp::signal::from_iter(
    ///     dasp::signal::rate(sample_rate as f64)
    ///         .const_hz(440.0)
    ///         .sine()
    ///         .take(sample_rate as usize * 2),
    /// )
    /// ```
    /// which plays a sine wave at 440 hz for 2 seconds.
    #[cfg(feature = "dasp")]
    pub fn play_dasp_synth<SignalType>(
        &mut self,
        signal: SignalType,
        signal_name: &str,
        options: SynthPlaybackOptions,
    ) -> Result<PlaybackId, Error>
    where
        SignalType: Signal<Frame = f64> + Send + Sync + 'static,
    {
        self.play_dasp_synth_with_context(signal, signal_name, options, None)
    }
    /// Play a mono [dasp](https://github.com/RustAudio/dasp) signal with the given options
    /// and playback status context.
    #[cfg(feature = "dasp")]
    pub fn play_dasp_synth_with_context<SignalType>(
        &mut self,
        signal: SignalType,
        signal_name: &str,
        options: SynthPlaybackOptions,
        context: Option<PlaybackStatusContext>,
    ) -> Result<PlaybackId, Error>
    where
        SignalType: Signal<Frame = f64> + Send + Sync + 'static,
    {
        // create synth source
        let source = DaspSynthSource::new(
            signal,
            signal_name,
            options,
            self.output_sample_rate(),
            Some(self.playback_status_sender()),
        )?;
        // and play it
        self.play_synth_source_with_context(source, options.start_time, context)
    }
}
