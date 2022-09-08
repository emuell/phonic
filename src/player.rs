use crossbeam_channel::{unbounded, Sender};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use crate::{
    error::Error,
    output::{AudioSink, DefaultAudioSink},
    source::{
        file::{
            preloaded::PreloadedFileSource, streamed::StreamedFileSource, FilePlaybackMessage,
            FileSource,
        },
        mixed::{MixedSource, MixedSourceMsg},
        playback::{PlaybackId, PlaybackStatusEvent},
        synth::{SynthPlaybackMessage, SynthSource},
    },
    utils::resampler::DEFAULT_RESAMPLING_QUALITY,
};

#[cfg(feature = "dasp")]
use dasp::Signal;

#[cfg(feature = "dasp")]
use crate::source::synth::dasp::DaspSynthSource;

// -------------------------------------------------------------------------------------------------

enum PlaybackMsgSender {
    File(Sender<FilePlaybackMessage>),
    Synth(Sender<SynthPlaybackMessage>),
}

// -------------------------------------------------------------------------------------------------

pub struct AudioFilePlayer {
    sink: DefaultAudioSink,
    playing_sources: Arc<Mutex<HashMap<PlaybackId, PlaybackMsgSender>>>,
    playback_status_sender: Sender<PlaybackStatusEvent>,
    mixer_event_sender: Sender<MixedSourceMsg>,
}

/// public interface
impl AudioFilePlayer {
    pub fn new(
        sink: DefaultAudioSink,
        playback_status_sender_arg: Option<Sender<PlaybackStatusEvent>>,
    ) -> Self {
        // Create a proxy for the playback status channel, so we can trap stop messages
        let playing_sources = Arc::new(Mutex::new(HashMap::new()));
        let playback_status_sender = Self::handle_playback_status_messages(
            playback_status_sender_arg,
            Arc::clone(&playing_sources),
        );
        // Create a mixer source, add it to the audio sink and start running
        let mixer_source = MixedSource::new(sink.channel_count(), sink.sample_rate());
        let mixer_event_sender = mixer_source.event_sender();
        sink.play(mixer_source);
        sink.resume();
        Self {
            sink,
            playing_sources,
            playback_status_sender,
            mixer_event_sender,
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

    pub fn play_streamed_file(&mut self, file_path: &str) -> Result<PlaybackId, Error> {
        let source = StreamedFileSource::new(file_path, Some(self.playback_status_sender.clone()))?;
        self.play_file(source)
    }

    pub fn play_preloaded_file(&mut self, file_path: &str) -> Result<PlaybackId, Error> {
        let source =
            PreloadedFileSource::new(file_path, Some(self.playback_status_sender.clone()))?;
        self.play_file(source)
    }

    pub fn play_file<F: FileSource>(&mut self, source: F) -> Result<PlaybackId, Error> {
        let source_file_id = source.playback_id();
        // subscribe to playback envets
        self.playing_sources.lock().unwrap().insert(
            source_file_id,
            PlaybackMsgSender::File(source.playback_message_sender()),
        );
        // convert file to mixer's rate and channel layout
        let converted = source.converted(
            self.sink.channel_count(),
            self.sink.sample_rate(),
            DEFAULT_RESAMPLING_QUALITY,
        );
        // play the source
        if let Err(err) = self.mixer_event_sender.send(MixedSourceMsg::AddSource {
            source: Box::new(converted),
        }) {
            log::error!("failed to send mixer event: {}", err);
            return Err(Error::SendError);
        }
        // return new file's id
        Ok(source_file_id)
    }

    #[cfg(feature = "dasp")]
    pub fn play_dasp_synth<SignalType>(
        &mut self,
        signal: SignalType,
        signal_name: &str,
    ) -> Result<PlaybackId, Error>
    where
        SignalType: Signal<Frame = f64> + Send + 'static,
    {
        // create new source and subscribe to playback envets
        let source = DaspSynthSource::new(
            signal,
            signal_name,
            self.sink.sample_rate(),
            Some(self.playback_status_sender.clone()),
        );
        self.play_synth(source)
    }

    #[allow(dead_code)]
    fn play_synth<S: SynthSource>(&mut self, source: S) -> Result<PlaybackId, Error> {
        let source_synth_id = source.playback_id();
        self.playing_sources.lock().unwrap().insert(
            source_synth_id,
            PlaybackMsgSender::Synth(source.playback_message_sender()),
        );
        // convert file to mixer's rate and channel layout
        let converted = source.converted(
            self.sink.channel_count(),
            self.sink.sample_rate(),
            DEFAULT_RESAMPLING_QUALITY,
        );
        // play the source
        if let Err(err) = self.mixer_event_sender.send(MixedSourceMsg::AddSource {
            source: Box::new(converted),
        }) {
            log::error!("failed to send mixer event: {}", err);
            return Err(Error::SendError);
        }
        // return new synth's id
        Ok(source_synth_id)
    }

    /// Change playback position of the given played back source. This is only supported for files and thus
    /// won't do anyththing for synths.
    pub fn seek_source(
        &mut self,
        playback_id: PlaybackId,
        position: Duration,
    ) -> Result<(), Error> {
        if let Some(msg_sender) = self.playing_sources.lock().unwrap().get(&playback_id) {
            if let PlaybackMsgSender::File(sender) = msg_sender {
                if let Err(err) = sender.send(FilePlaybackMessage::Seek(position)) {
                    log::warn!("failed to send seek command to file: {}", err.to_string());
                }
            } else {
                log::warn!("trying to seek a synth source, which is not supported");
            }
            return Ok(());
        } else {
            log::warn!("trying to seek source #{playback_id} which is not or no longer playing");
        }
        Err(Error::MediaFileNotFound)
    }

    /// Stop a playing file or synth source.
    pub fn stop_source(&mut self, playback_id: PlaybackId) -> Result<(), Error> {
        if let Some(msg_sender) = self.playing_sources.lock().unwrap().get(&playback_id) {
            match msg_sender {
                PlaybackMsgSender::File(file_sender) => {
                    if let Err(err) = file_sender.send(FilePlaybackMessage::Stop) {
                        log::warn!(
                            "failed to send stop command to file source: {}",
                            err.to_string()
                        );
                    }
                }
                PlaybackMsgSender::Synth(synth_sender) => {
                    if let Err(err) = synth_sender.send(SynthPlaybackMessage::Stop) {
                        log::warn!(
                            "failed to send stop command to synth source: {}",
                            err.to_string()
                        );
                    }
                }
            }
            self.playing_sources.lock().unwrap().remove(&playback_id);
            return Ok(());
        } else {
            log::warn!("trying to stop source #{playback_id} which is not or no longer playing");
        }
        Err(Error::MediaFileNotFound)
    }

    /// Stop all playing sources.
    pub fn stop_all_sources(&mut self) -> Result<(), Error> {
        let playing_ids: Vec<PlaybackId>;
        {
            let playing_sources = self.playing_sources.lock().unwrap();
            playing_ids = playing_sources.keys().copied().collect();
        }
        for source_id in playing_ids {
            self.stop_source(source_id)?;
        }
        Ok(())
    }
}

/// details
impl AudioFilePlayer {
    fn handle_playback_status_messages(
        playback_sender_arg: Option<Sender<PlaybackStatusEvent>>,
        playing_sources: Arc<Mutex<HashMap<PlaybackId, PlaybackMsgSender>>>,
    ) -> Sender<PlaybackStatusEvent> {
        let (send_proxy, recv_proxy) = unbounded::<PlaybackStatusEvent>();

        std::thread::Builder::new()
            .name("audio_player_messages".to_string())
            .spawn(move || {
                while let Ok(msg) = recv_proxy.recv() {
                    if let PlaybackStatusEvent::Stopped {
                        id,
                        path: _,
                        exhausted: _,
                    } = msg
                    {
                        playing_sources.lock().unwrap().remove(&id);
                    }
                    if let Some(sender) = &playback_sender_arg {
                        if let Err(err) = sender.send(msg) {
                            log::warn!("failed to send file status message: {}", err);
                        }
                    }
                }
            })
            .expect("failed to spawn audio message thread");

        send_proxy
    }
}
