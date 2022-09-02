pub mod output;

use crossbeam_channel::Sender;
use std::time::Duration;

use crate::utils::resampler::ResamplingQuality;
use crate::{
    error::Error,
    source::{
        decoded::{DecoderPlaybackEvent, DecoderSource, DecoderWorkerMsg},
        mapped::ChannelMappedSource,
        resampled::ResampledSource,
        AudioSource,
    },
};

use self::output::{AudioSink, DefaultAudioSink};

// -------------------------------------------------------------------------------------------------

pub struct PlaybackManager {
    sink: DefaultAudioSink,
    event_send: Sender<DecoderPlaybackEvent>,
    current: Option<(String, Sender<DecoderWorkerMsg>)>,
}

impl PlaybackManager {
    pub fn new(sink: DefaultAudioSink, event_send: Sender<DecoderPlaybackEvent>) -> Self {
        Self {
            sink,
            event_send,
            current: None,
        }
    }

    pub fn playing_file(&self) -> Option<String> {
        if let Some((path, _worker)) = &self.current {
            return Some(path.to_owned());
        }
        None
    }

    pub fn play(&mut self, file_path: String) -> Result<(), Error> {
        let source = DecoderSource::new(file_path.clone(), self.event_send.clone())?;
        self.current = Some((file_path, source.actor.sender()));
        if source.sample_rate() == self.sink.sample_rate()
            && source.channel_count() == self.sink.channel_count()
        {
            // We can start playing the source right away.
            self.sink.play(source);
        } else {
            // Some output streams have different sample rate than the source, so we need to
            // resample before pushing to the sink.
            let source = ResampledSource::new(
                Box::new(source),
                self.sink.sample_rate(),
                ResamplingQuality::SincMediumQuality,
            );
            // Source output streams also have a different channel count. Map the stereo
            // channels and silence the others.
            let source = ChannelMappedSource::new(Box::new(source), self.sink.channel_count());
            self.sink.play(source);
        }
        self.sink.resume();
        Ok(())
    }

    pub fn seek(&self, position: Duration) {
        if let Some((path, worker)) = &self.current {
            let _ = worker.send(DecoderWorkerMsg::Seek(position));

            // Because the position events are sent in the `DecoderSource`, doing this here
            // is slightly hacky. The alternative would be propagating `event_send` into the
            // worker.
            let _ = self.event_send.send(DecoderPlaybackEvent::Position {
                path: path.to_owned(),
                position,
            });
        }
    }

    pub fn stop(&self) {
        self.sink.stop();
    }
}
