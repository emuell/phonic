use crossbeam_channel::Sender;
use std::time::Duration;

use crate::{
    error::Error,
    output::{AudioSink, DefaultAudioSink},
    source::{
        decoded::{DecoderPlaybackEvent, DecoderSource, DecoderWorkerMsg},
        AudioSource,
    },
};

// -------------------------------------------------------------------------------------------------

pub struct AudioFilePlayer {
    sink: DefaultAudioSink,
    event_send: Sender<DecoderPlaybackEvent>,
    current: Option<(String, Sender<DecoderWorkerMsg>)>,
}

impl AudioFilePlayer {
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
        // create a decoded source
        let source = DecoderSource::new(file_path.clone(), Some(self.event_send.clone()))?;
        // subscribe to playback envets
        self.current = Some((file_path, source.worker_msg_sender()));
        // convert channes and rate output, if needed
        let converted = source.converted(self.sink.channel_count(), self.sink.sample_rate());
        // play the source
        self.sink.play(converted);
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
