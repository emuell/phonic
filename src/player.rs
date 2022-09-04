use crossbeam_channel::Sender;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::{
    error::Error,
    output::{AudioSink, DefaultAudioSink},
    source::{
        decoded::{DecoderFileId, DecoderPlaybackEvent, DecoderSource, DecoderWorkerMsg},
        mixed::MixedSource,
    },
};

// -------------------------------------------------------------------------------------------------

pub struct AudioFilePlayer {
    sink: DefaultAudioSink,
    event_send: Sender<DecoderPlaybackEvent>,
    playing_files: HashMap<DecoderFileId, Sender<DecoderWorkerMsg>>,
    mixer_source: Arc<Mutex<MixedSource>>,
}

impl AudioFilePlayer {
    pub fn new(sink: DefaultAudioSink, event_send: Sender<DecoderPlaybackEvent>) -> Self {
        let mixer_source = Arc::new(Mutex::new(MixedSource::new(
            sink.channel_count(),
            sink.sample_rate(),
        )));
        sink.play(mixer_source.clone());
        Self {
            sink,
            event_send,
            playing_files: HashMap::new(),
            mixer_source,
        }
    }

    pub fn start(&self) {
        self.sink.resume()
    }

    pub fn stop(&self) {
        self.sink.pause()
    }

    pub fn play_file(&mut self, file_path: String) -> Result<DecoderFileId, Error> {
        // create a decoded source
        let source = DecoderSource::new(file_path, Some(self.event_send.clone()))?;
        let file_id = source.file_id();
        // subscribe to playback envets
        self.playing_files
            .insert(file_id, source.worker_msg_sender());
        // play the source
        self.mixer_source.lock().unwrap().add(source);
        // return file id
        Ok(file_id)
    }

    pub fn seek_file(&self, file_id: usize, position: Duration) -> Result<(), Error> {
        if let Some(worker) = self.playing_files.get(&file_id) {
            if let Err(err) = worker.send(DecoderWorkerMsg::Seek(position)) {
                log::warn!("Failed to send seek command to file: {}", err.to_string())
            }
            return Ok(());
        }
        Err(Error::MediaFileNotFound)
    }

    pub fn stop_file(&self, file_id: usize) -> Result<(), Error> {
        if let Some(worker) = self.playing_files.get(&file_id) {
            if let Err(err) = worker.send(DecoderWorkerMsg::Stop) {
                log::warn!("Failed to send stop command to file: {}", err.to_string())
            }
            return Ok(());
        }
        Err(Error::MediaFileNotFound)
    }
}
