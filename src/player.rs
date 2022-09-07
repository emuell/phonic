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
        synth::{SynthId, SynthPlaybackMsg, SynthPlaybackStatusMsg, SynthSource},
    },
    utils::resampler::DEFAULT_RESAMPLING_QUALITY,
};

#[cfg(feature = "dasp")]
use dasp::Signal;

#[cfg(feature = "dasp")]
use crate::source::synth::dasp::DaspSynthSource;

// -------------------------------------------------------------------------------------------------

pub struct AudioFilePlayer {
    sink: DefaultAudioSink,
    playing_files: HashMap<FileId, Sender<FilePlaybackMsg>>,
    playing_synths: HashMap<SynthId, Sender<SynthPlaybackMsg>>,
    file_event_send: Option<Sender<FilePlaybackStatusMsg>>,
    #[allow(dead_code)]
    synth_event_send: Option<Sender<SynthPlaybackStatusMsg>>,
    mixer_event_send: Sender<MixedSourceMsg>,
}

impl AudioFilePlayer {
    pub fn new(
        sink: DefaultAudioSink,
        file_event_send: Option<Sender<FilePlaybackStatusMsg>>,
        synth_event_send: Option<Sender<SynthPlaybackStatusMsg>>,
    ) -> Self {
        // Create a mixer source, add it to the audio sink and start running
        let mixer_source = MixedSource::new(sink.channel_count(), sink.sample_rate());
        let mixer_event_sender = mixer_source.event_sender();
        sink.play(mixer_source);
        sink.resume();
        Self {
            sink,
            playing_files: HashMap::new(),
            playing_synths: HashMap::new(),
            file_event_send,
            synth_event_send,
            mixer_event_send: mixer_event_sender,
        }
    }

    /// Start audio playback.
    pub fn start(&self) {
        self.sink.resume()
    }

    /// Stop audio playback. This will only pause and thus not drop any playing sources. Use the
    /// [`start`] function to start it again. Use function [`stop_all_sources`] to drop all sources.
    pub fn stop(&self) {
        self.sink.pause()
    }

    pub fn stop_all_sources(&mut self) -> Result<(), Error> {
        self.stop_all_files()?;
        self.stop_all_synths()?;
        Ok(())
    }

    pub fn play_streamed_file(&mut self, file_path: String) -> Result<FileId, Error> {
        let source = StreamedFileSource::new(file_path, self.file_event_send.clone())?;
        self.play_file(source)
    }

    pub fn play_preloaded_file(&mut self, file_path: String) -> Result<FileId, Error> {
        let source = PreloadedFileSource::new(file_path, self.file_event_send.clone())?;
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
        // return new file's id
        Ok(source_file_id)
    }

    pub fn seek_file(&mut self, file_id: FileId, position: Duration) -> Result<(), Error> {
        if let Some(worker) = self.playing_files.get(&file_id) {
            if let Err(err) = worker.send(FilePlaybackMsg::Seek(position)) {
                log::warn!("failed to send seek command to file: {}", err.to_string());
            }
            return Ok(());
        }
        Err(Error::MediaFileNotFound)
    }

    pub fn stop_file(&mut self, file_id: FileId) -> Result<(), Error> {
        if let Some(worker) = self.playing_files.get(&file_id) {
            if let Err(err) = worker.send(FilePlaybackMsg::Stop) {
                log::warn!("failed to send stop command to file: {}", err.to_string());
            }
            self.playing_files.remove(&file_id);
            return Ok(());
        }
        Err(Error::MediaFileNotFound)
    }

    pub fn stop_all_files(&mut self) -> Result<(), Error> {
        let file_ids: Vec<FileId> = self.playing_files.keys().map(|x| *x).collect();
        for file_id in file_ids {
            self.stop_file(file_id)?;
        }
        Ok(())
    }

    #[cfg(feature = "dasp")]
    pub fn play_dasp_synth<SignalType>(&mut self, signal: SignalType) -> Result<SynthId, Error>
    where
        SignalType: Signal<Frame = f64> + Send + 'static,
    {
        // create new source and subscribe to playback envets
        let source = DaspSynthSource::new(
            signal,
            self.sink.sample_rate(),
            self.synth_event_send.clone(),
        );
        self.play_synth(source)
    }

    fn play_synth<S: SynthSource>(&mut self, source: S) -> Result<SynthId, Error> {
        let source_synth_id = source.synth_id();
        self.playing_synths.insert(source_synth_id, source.sender());
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
        // return new synth's id
        Ok(source_synth_id)
    }

    pub fn stop_synth(&mut self, synth_id: SynthId) -> Result<(), Error> {
        if let Some(worker) = self.playing_synths.get(&synth_id) {
            if let Err(err) = worker.send(SynthPlaybackMsg::Stop) {
                log::warn!("failed to send stop command to synth: {}", err.to_string());
            }
            self.playing_synths.remove(&synth_id);
            return Ok(());
        }
        Err(Error::MediaFileNotFound)
    }

    pub fn stop_all_synths(&mut self) -> Result<(), Error> {
        let synth_ids: Vec<SynthId> = self.playing_synths.keys().map(|x| *x).collect();
        for synth_id in synth_ids {
            self.stop_synth(synth_id)?;
        }
        Ok(())
    }
}
