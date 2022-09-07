use crossbeam_channel::{select, unbounded, SendError, Sender};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
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
    playing_files: Arc<Mutex<HashMap<FileId, Sender<FilePlaybackMsg>>>>,
    playing_synths: Arc<Mutex<HashMap<SynthId, Sender<SynthPlaybackMsg>>>>,
    file_event_send: Sender<FilePlaybackStatusMsg>,
    #[allow(dead_code)]
    synth_event_send: Sender<SynthPlaybackStatusMsg>,
    mixer_event_send: Sender<MixedSourceMsg>,
}

/// public interface
impl AudioFilePlayer {
    pub fn new(
        sink: DefaultAudioSink,
        file_event_send_arg: Option<Sender<FilePlaybackStatusMsg>>,
        synth_event_send_arg: Option<Sender<SynthPlaybackStatusMsg>>,
    ) -> Self {
        // Create a proxy for file/synth_event_send, so we can trap stop messages
        let playing_files = Arc::new(Mutex::new(HashMap::new()));
        let playing_synths = Arc::new(Mutex::new(HashMap::new()));
        let (file_event_send, synth_event_send) = Self::handle_playback_status_messages(
            file_event_send_arg,
            synth_event_send_arg,
            Arc::clone(&playing_files),
            Arc::clone(&playing_synths),
        );
        // Create a mixer source, add it to the audio sink and start running
        let mixer_source = MixedSource::new(sink.channel_count(), sink.sample_rate());
        let mixer_event_send = mixer_source.event_sender();
        sink.play(mixer_source);
        sink.resume();
        Self {
            sink,
            playing_files,
            playing_synths,
            file_event_send,
            synth_event_send,
            mixer_event_send,
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
        let source = StreamedFileSource::new(file_path, Some(self.file_event_send.clone()))?;
        self.play_file(source)
    }

    pub fn play_preloaded_file(&mut self, file_path: String) -> Result<FileId, Error> {
        let source = PreloadedFileSource::new(file_path, Some(self.file_event_send.clone()))?;
        self.play_file(source)
    }

    pub fn play_file<F: FileSource>(&mut self, source: F) -> Result<FileId, Error> {
        let source_file_id = source.file_id();
        // subscribe to playback envets
        self.playing_files
            .lock()
            .unwrap()
            .insert(source_file_id, source.sender());
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
        if let Some(worker) = self.playing_files.lock().unwrap().get(&file_id) {
            if let Err(err) = worker.send(FilePlaybackMsg::Seek(position)) {
                log::warn!("failed to send seek command to file: {}", err.to_string());
            }
            return Ok(());
        } else {
            log::warn!("trying to seek file #{file_id} which is not or no longer playing");
        }
        Err(Error::MediaFileNotFound)
    }

    pub fn stop_file(&mut self, file_id: FileId) -> Result<(), Error> {
        if let Some(worker) = self.playing_files.lock().unwrap().get(&file_id) {
            if let Err(err) = worker.send(FilePlaybackMsg::Stop) {
                log::warn!("failed to send stop command to file: {}", err.to_string());
            }
            self.playing_files.lock().unwrap().remove(&file_id);
            return Ok(());
        } else {
            log::warn!("trying to stop file #{file_id} which is not or no longer playing");
        }
        Err(Error::MediaFileNotFound)
    }

    pub fn stop_all_files(&mut self) -> Result<(), Error> {
        let file_ids: Vec<FileId>;
        {
            let playing_files = self.playing_files.lock().unwrap();
            file_ids = playing_files.keys().copied().collect();
        }
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
            Some(self.synth_event_send.clone()),
        );
        self.play_synth(source)
    }

    #[allow(dead_code)]
    fn play_synth<S: SynthSource>(&mut self, source: S) -> Result<SynthId, Error> {
        let source_synth_id = source.synth_id();
        self.playing_synths
            .lock()
            .unwrap()
            .insert(source_synth_id, source.sender());
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
        if let Some(worker) = self.playing_synths.lock().unwrap().get(&synth_id) {
            if let Err(err) = worker.send(SynthPlaybackMsg::Stop) {
                log::warn!("failed to send stop command to synth: {}", err.to_string());
            }
            self.playing_synths.lock().unwrap().remove(&synth_id);
            return Ok(());
        } else {
            log::warn!("trying to stop synth #{synth_id} which is not or no longer playing");
        }
        Err(Error::MediaFileNotFound)
    }

    pub fn stop_all_synths(&mut self) -> Result<(), Error> {
        let synth_ids: Vec<SynthId>;
        {
            let playing_synths = self.playing_synths.lock().unwrap();
            synth_ids = playing_synths.keys().copied().collect();
        }
        for synth_id in synth_ids {
            self.stop_synth(synth_id)?;
        }
        Ok(())
    }
}

/// details
impl AudioFilePlayer {
    fn handle_file_events(
        original_sender: &Option<Sender<FilePlaybackStatusMsg>>,
        playing_files: &Arc<Mutex<HashMap<SynthId, Sender<FilePlaybackMsg>>>>,
        msg: FilePlaybackStatusMsg,
    ) -> Result<(), SendError<FilePlaybackStatusMsg>> {
        if let FilePlaybackStatusMsg::Stopped {
            file_id,
            file_path: _,
            end_of_file: _,
        } = msg
        {
            playing_files.lock().unwrap().remove(&file_id);
        }
        if let Some(sender) = original_sender {
            sender.send(msg)
        } else {
            Ok(())
        }
    }

    fn handle_synth_events(
        original_sender: &Option<Sender<SynthPlaybackStatusMsg>>,
        playing_synths: &Arc<Mutex<HashMap<SynthId, Sender<SynthPlaybackMsg>>>>,
        msg: SynthPlaybackStatusMsg,
    ) -> Result<(), SendError<SynthPlaybackStatusMsg>> {
        #[allow(irrefutable_let_patterns)]
        if let SynthPlaybackStatusMsg::Stopped {
            synth_id,
            exhausted: _,
        } = msg
        {
            playing_synths.lock().unwrap().remove(&synth_id);
        }
        if let Some(sender) = original_sender {
            sender.send(msg)
        } else {
            Ok(())
        }
    }

    fn handle_playback_status_messages(
        file_event_send_arg: Option<Sender<FilePlaybackStatusMsg>>,
        synth_event_send_arg: Option<Sender<SynthPlaybackStatusMsg>>,
        playing_files: Arc<Mutex<HashMap<SynthId, Sender<FilePlaybackMsg>>>>,
        playing_synths: Arc<Mutex<HashMap<SynthId, Sender<SynthPlaybackMsg>>>>,
    ) -> (
        Sender<FilePlaybackStatusMsg>,
        Sender<SynthPlaybackStatusMsg>,
    ) {
        let (file_send_proxy, file_recv_proxy) = unbounded::<FilePlaybackStatusMsg>();
        let (synth_send_proxy, synth_recv_proxy) = unbounded::<SynthPlaybackStatusMsg>();

        std::thread::Builder::new()
            .name("audio_player_messages".to_string())
            .spawn(move || loop {
                select! {
                    recv(file_recv_proxy) -> file_recv_proxy => {
                        if let Ok(msg) = file_recv_proxy {
                            if let Err(err) = Self::handle_file_events(&file_event_send_arg, &playing_files, msg) {
                                log::warn!("failed to send file status message: {}", err);
                            }
                        } else {
                            break;
                        }
                    }
                    recv(synth_recv_proxy) -> synth_recv_proxy => {
                        if let Ok(msg) = synth_recv_proxy {
                            if let Err(err) = Self::handle_synth_events(&synth_event_send_arg, &playing_synths, msg) {
                                log::warn!("failed to send synth status message: {}", err);
                            }
                        } else {
                            break;
                        }
                    }
                }
            })
            .expect("failed to spawn audio message thread");

        (file_send_proxy, synth_send_proxy)
    }
}
