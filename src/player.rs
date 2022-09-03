use crossbeam_channel::Sender;
use std::time::Duration;

use crate::{
    error::Error,
    output::{AudioSink, DefaultAudioSink},
    source::{
        decoded::{DecoderPlaybackEvent, DecoderSource, DecoderWorkerMsg},
        mapped::ChannelMappedSource,
        resampled::ResampledSource,
        AudioSource,
    },
    utils::resampler::ResamplingQuality,
};

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
        let source = DecoderSource::new(file_path.clone(), Some(self.event_send.clone()))?;
        self.current = Some((file_path, source.worker_msg_sender()));
        if source.sample_rate() != self.sink.sample_rate() {
            // convert source sample-rate to ours
            let resampled_source = ResampledSource::new(
                source,
                self.sink.sample_rate(),
                ResamplingQuality::SincMediumQuality,
            );
            if resampled_source.channel_count() != self.sink.channel_count() {
                let channel_mapped_source =
                    ChannelMappedSource::new(resampled_source, self.sink.channel_count());
                self.sink.play(channel_mapped_source);
            } else {
                self.sink.play(resampled_source);
            }
        } else if source.channel_count() != self.sink.channel_count() {
            // convert source channel mapping to ours
            let channel_mapped_source = ChannelMappedSource::new(source, self.sink.channel_count());
            self.sink.play(channel_mapped_source);
        } else {
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
