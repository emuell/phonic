use crossbeam_channel::Sender;
use std::collections::HashMap;
use std::time::Duration;

use crate::{
    error::Error,
    output::{AudioSink, DefaultAudioSink},
    source::{
        decoded::{DecodedFileId, DecodedFileMsg, DecodedFilePlaybackStatusMsg, DecodedFileSource},
        mixed::{MixedSource, MixedSourceMsg},
        AudioSource,
    },
};

// -------------------------------------------------------------------------------------------------

pub struct AudioFilePlayer {
    sink: DefaultAudioSink,
    playing_files: HashMap<DecodedFileId, Sender<DecodedFileMsg>>,
    decoder_event_send: Sender<DecodedFilePlaybackStatusMsg>,
    mixer_event_send: Sender<MixedSourceMsg>,
}

impl AudioFilePlayer {
    pub fn new(sink: DefaultAudioSink, event_send: Sender<DecodedFilePlaybackStatusMsg>) -> Self {
        // Create a mixer and start playing on the sink
        let mixer_source = MixedSource::new(sink.channel_count(), sink.sample_rate());
        let mixer_event_sender = mixer_source.event_sender();
        sink.play(mixer_source);
        Self {
            sink,
            decoder_event_send: event_send,
            playing_files: HashMap::new(),
            mixer_event_send: mixer_event_sender,
        }
    }

    pub fn start(&self) {
        self.sink.resume()
    }

    pub fn stop(&self) {
        self.sink.pause()
    }

    pub fn play_file(&mut self, file_path: String) -> Result<DecodedFileId, Error> {
        // create a decoded source
        let source = DecodedFileSource::new(file_path, Some(self.decoder_event_send.clone()))?;
        let source_file_id = source.file_id();
        // subscribe to playback envets
        self.playing_files
            .insert(source_file_id, source.worker_msg_sender());
        // convert file to mixer's rate and channel layout
        let converted = source.converted(self.sink.channel_count(), self.sink.sample_rate());
        // play the source
        if let Err(err) = self.mixer_event_send.send(MixedSourceMsg::AddSource {
            source: Box::new(converted),
        }) {
            log::error!("failed to send mixer event: {}", err);
            return Err(Error::SendError);
        }
        // return file id
        Ok(source_file_id)
    }

    pub fn seek_file(&self, file_id: usize, position: Duration) -> Result<(), Error> {
        if let Some(worker) = self.playing_files.get(&file_id) {
            if let Err(err) = worker.send(DecodedFileMsg::Seek(position)) {
                log::error!("failed to send seek command to file: {}", err.to_string());
                return Err(Error::SendError);
            }
            return Ok(());
        }
        Err(Error::MediaFileNotFound)
    }

    pub fn stop_file(&self, file_id: usize) -> Result<(), Error> {
        if let Some(worker) = self.playing_files.get(&file_id) {
            if let Err(err) = worker.send(DecodedFileMsg::Stop) {
                log::error!("failed to send stop command to file: {}", err.to_string());
                return Err(Error::SendError);
            }
            return Ok(());
        }
        Err(Error::MediaFileNotFound)
    }
}
