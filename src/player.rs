use crossbeam_channel::{unbounded, Sender};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use crate::{
    converted::ConvertedSource,
    error::Error,
    file::FilePlaybackOptions,
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
    AudioSource,
};

#[cfg(feature = "dasp")]
use dasp::Signal;

#[cfg(feature = "dasp")]
use crate::source::synth::{dasp::DaspSynthSource, SynthPlaybackOptions};

// -------------------------------------------------------------------------------------------------

enum PlaybackMessageSender {
    File(Sender<FilePlaybackMessage>),
    Synth(Sender<SynthPlaybackMessage>),
}

pub struct AudioFilePlayer {
    sink: DefaultAudioSink,
    playing_sources: Arc<Mutex<HashMap<PlaybackId, PlaybackMessageSender>>>,
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

    /// Play a new file with default playback options. See [`play_file_with_options`] for more info
    /// on which options can be applied.
    pub fn play_file(&mut self, file_path: &str) -> Result<PlaybackId, Error> {
        self.play_file_with_options(file_path, FilePlaybackOptions::default())
    }
    /// Play a new file with the given file path and options. See [`FilePlaybackOptions`] for more info
    /// on which options can be applied.
    ///
    /// Newly played sources are always added to the final mix and won't stop other playing sources.
    pub fn play_file_with_options(
        &mut self,
        file_path: &str,
        options: FilePlaybackOptions,
    ) -> Result<PlaybackId, Error> {
        // create new preloaded or streamed source and convert it to our output specs
        let source_playback_id: PlaybackId;
        let playback_message_sender: Sender<FilePlaybackMessage>;
        let source: ConvertedSource = if options.stream {
            let streamed_source = StreamedFileSource::new(
                file_path,
                Some(self.playback_status_sender.clone()),
                options.volume,
            )?;
            source_playback_id = streamed_source.playback_id();
            playback_message_sender = streamed_source.playback_message_sender();
            // convert file to mixer's rate and channel layout
            streamed_source.converted(
                self.sink.channel_count(),
                self.sink.sample_rate(),
                DEFAULT_RESAMPLING_QUALITY,
            )
        } else {
            let preloaded_source = PreloadedFileSource::new(
                file_path,
                Some(self.playback_status_sender.clone()),
                options.volume,
            )?;
            source_playback_id = preloaded_source.playback_id();
            playback_message_sender = preloaded_source.playback_message_sender();
            // convert file to mixer's rate and channel layout
            preloaded_source.converted(
                self.sink.channel_count(),
                self.sink.sample_rate(),
                DEFAULT_RESAMPLING_QUALITY,
            )
        };
        // subscribe to playback envets in the newly created source
        let mut playing_sources = self.playing_sources.lock().unwrap();
        playing_sources.insert(
            source_playback_id,
            PlaybackMessageSender::File(playback_message_sender),
        );
        // play the source by adding it to the mixer
        if let Err(err) = self.mixer_event_sender.send(MixedSourceMsg::AddSource {
            source: Box::new(source),
        }) {
            log::error!("failed to send mixer event: {}", err);
            return Err(Error::SendError);
        }
        // return new file's id on success
        Ok(source_playback_id)
    }

    /// Play a mono f64 dasp signal with default playback options. See [`play_dasp_synth_with_options`]
    /// for more info.
    #[cfg(feature = "dasp")]
    pub fn play_dasp_synth<SignalType>(
        &mut self,
        signal: SignalType,
        signal_name: &str,
    ) -> Result<PlaybackId, Error>
    where
        SignalType: Signal<Frame = f64> + Send + 'static,
    {
        self.play_dasp_synth_with_options(signal, signal_name, SynthPlaybackOptions::default())
    }
    /// Play a mono dasp signal with the given options. See [`SynthPlaybackOptions`] for more info
    /// about available options.
    ///
    /// The signal will be wrapped into a dasp::signal::UntilExhausted so it can be used to play
    /// create one-shots.
    ///
    /// Example one-shot signal:
    /// `dasp::signal::from_iter(
    ///     dasp::signal::rate(sample_rate as f64)
    ///         .const_hz(440.0)
    ///         .sine()
    ///         .take(sample_rate as usize * 2),
    /// )`
    /// which plays a sine wave at 440 hz for 2 seconds.
    #[cfg(feature = "dasp")]
    pub fn play_dasp_synth_with_options<SignalType>(
        &mut self,
        signal: SignalType,
        signal_name: &str,
        options: SynthPlaybackOptions,
    ) -> Result<PlaybackId, Error>
    where
        SignalType: Signal<Frame = f64> + Send + 'static,
    {
        // create new source and subscribe to playback envets
        let source = DaspSynthSource::new(
            signal,
            signal_name,
            options.volume,
            self.sink.sample_rate(),
            Some(self.playback_status_sender.clone()),
        );
        self.play_synth(source)
    }

    #[allow(dead_code)]
    fn play_synth<S: SynthSource>(&mut self, source: S) -> Result<PlaybackId, Error> {
        let source_synth_id = source.playback_id();
        let mut playing_sources = self.playing_sources.lock().unwrap();
        playing_sources.insert(
            source_synth_id,
            PlaybackMessageSender::Synth(source.playback_message_sender()),
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
        let playing_sources = self.playing_sources.lock().unwrap();
        if let Some(msg_sender) = playing_sources.get(&playback_id) {
            if let PlaybackMessageSender::File(sender) = msg_sender {
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
        let mut playing_sources = self.playing_sources.lock().unwrap();
        if let Some(msg_sender) = playing_sources.get(&playback_id) {
            match msg_sender {
                PlaybackMessageSender::File(file_sender) => {
                    if let Err(err) = file_sender.send(FilePlaybackMessage::Stop) {
                        log::warn!(
                            "failed to send stop command to file source: {}",
                            err.to_string()
                        );
                    }
                }
                PlaybackMessageSender::Synth(synth_sender) => {
                    if let Err(err) = synth_sender.send(SynthPlaybackMessage::Stop) {
                        log::warn!(
                            "failed to send stop command to synth source: {}",
                            err.to_string()
                        );
                    }
                }
            }
            // we shortly will receive an Exhaused event which removes the source, but neverthless
            // remove it now, to force all following attempts to stop this source to fail
            playing_sources.remove(&playback_id);
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
        playing_sources: Arc<Mutex<HashMap<PlaybackId, PlaybackMessageSender>>>,
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
