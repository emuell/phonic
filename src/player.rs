use crossbeam_channel::Sender;
use std::collections::HashMap;
use std::time::Duration;

use crate::{
    error::Error,
    output::{AudioSink, DefaultAudioSink},
    source::{
        file::{
            preloaded::PreloadedFileSource, streamed::StreamedFileSource, FileId, FilePlaybackMsg,
            FilePlaybackStatusMsg, FileSource,
        },
        mixed::{MixedSource, MixedSourceMsg},
    },
    utils::resampler::DEFAULT_RESAMPLING_QUALITY,
};

// -------------------------------------------------------------------------------------------------

pub struct AudioFilePlayer {
    sink: DefaultAudioSink,
    playing_files: HashMap<FileId, Sender<FilePlaybackMsg>>,
    decoder_event_send: Sender<FilePlaybackStatusMsg>,
    mixer_event_send: Sender<MixedSourceMsg>,
}

impl AudioFilePlayer {
    pub fn new(sink: DefaultAudioSink, event_send: Sender<FilePlaybackStatusMsg>) -> Self {
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

    pub fn play_streamed_file(&mut self, file_path: String) -> Result<FileId, Error> {
        let source =
            StreamedFileSource::new(file_path.clone(), Some(self.decoder_event_send.clone()))?;
        self.play_file(source)
    }

    pub fn play_preloaded_file(&mut self, file_path: String) -> Result<FileId, Error> {
        let source =
            PreloadedFileSource::new(file_path.clone(), Some(self.decoder_event_send.clone()))?;
        self.play_file(source)
    }

    pub fn play_file<F: FileSource>(&mut self, source: F) -> Result<FileId, Error> {
        let source_file_id = source.file_id();
        // subscribe to playback envets
        self.playing_files.insert(source_file_id, source.sender());
        // convert file to mixer's rate and channel layout
        let converted = source.converted(
            self.sink.channel_count(),
            self.sink.sample_rate(),
            DEFAULT_RESAMPLING_QUALITY,
        );
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
            if let Err(err) = worker.send(FilePlaybackMsg::Seek(position)) {
                log::error!("failed to send seek command to file: {}", err.to_string());
                return Err(Error::SendError);
            }
            return Ok(());
        }
        Err(Error::MediaFileNotFound)
    }

    pub fn stop_file(&self, file_id: usize) -> Result<(), Error> {
        if let Some(worker) = self.playing_files.get(&file_id) {
            if let Err(err) = worker.send(FilePlaybackMsg::Stop) {
                log::error!("failed to send stop command to file: {}", err.to_string());
                return Err(Error::SendError);
            }
            return Ok(());
        }
        Err(Error::MediaFileNotFound)
    }
}
