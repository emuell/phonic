use core::time;
use std::{
    any::Any,
    sync::{
        atomic::{self, AtomicBool},
        Arc,
    },
    thread,
    time::Duration,
};

use basedrop::{Collector, Handle, Owned};
use crossbeam_channel::Sender;
use crossbeam_queue::ArrayQueue;
use dashmap::DashMap;

use crate::{
    error::Error,
    output::{DefaultOutputSink, OutputSink},
    source::{
        amplified::AmplifiedSource,
        converted::ConvertedSource,
        file::{FilePlaybackMessage, FileSource},
        mixed::{MixedSource, MixedSourceMsg},
        panned::PannedSource,
        resampled::ResamplingQuality,
        synth::{SynthPlaybackMessage, SynthSource},
    },
};

// -------------------------------------------------------------------------------------------------

/// A unique ID for a newly created File or Synth Sources.
pub type PlaybackId = usize;

// -------------------------------------------------------------------------------------------------

/// Custom context type for playback status events.
pub type PlaybackStatusContext = Arc<dyn Any + Send + Sync>;

/// Events send back from File or Synth sources via the player to the user.
pub enum PlaybackStatusEvent {
    Position {
        /// Unique id to resolve played back sources.
        id: PlaybackId,
        /// The file path for file based sources, else a name to somewhat identify the source.
        path: Arc<String>,
        /// Custom, optional context, passed along when starting playback.
        context: Option<PlaybackStatusContext>,
        /// Source's actual playback position in wallclock-time.
        position: Duration,
    },
    Stopped {
        /// Unique id to resolve played back sources
        id: PlaybackId,
        /// the file path for file based sources, else a name to somewhat identify the source
        path: Arc<String>,
        /// Custom, optional context, passed along when starting playback.
        context: Option<PlaybackStatusContext>,
        /// true when the source finished playing (e.g. reaching EOF), false when manually stopped
        exhausted: bool,
    },
}

// -------------------------------------------------------------------------------------------------

/// Wraps File and Synth Playback messages together into one object, allowing to easily stop them.
#[derive(Clone)]
pub(crate) enum PlaybackMessageSender {
    File(Arc<ArrayQueue<FilePlaybackMessage>>),
    Synth(Arc<ArrayQueue<SynthPlaybackMessage>>),
}

impl PlaybackMessageSender {
    pub fn send_stop(&self) -> Result<(), Error> {
        match self {
            PlaybackMessageSender::File(sender) => sender
                .push(FilePlaybackMessage::Stop)
                .map_err(|_err| Error::SendError),
            PlaybackMessageSender::Synth(sender) => sender
                .push(SynthPlaybackMessage::Stop)
                .map_err(|_err| Error::SendError),
        }
    }
}

// -------------------------------------------------------------------------------------------------

/// Playback controller, which drives an [`OutputSink`] and runs a [`MixedSource`] which
/// can play an unlimited number of [`FileSource`] or [`SynthSource`] at the same time.
///
/// Playback status of all sources can be tracked via an optional event channel.
/// New sources can be added any time, and can be stopped and seeked (seeking works for file
/// based sources only).
///
/// NB: For playback of [`SynthSource`]s, the `dasp-synth` feature needs to be enabled.
pub struct Player {
    sink: DefaultOutputSink,
    playing_sources: Arc<DashMap<PlaybackId, PlaybackMessageSender>>,
    playback_status_sender: Sender<PlaybackStatusEvent>,
    collector_handle: Handle,
    collector_running: Arc<AtomicBool>,
    mixer_event_queue: Arc<ArrayQueue<MixedSourceMsg>>,
}

impl Player {
    /// Create a new Player for the given DefaultOutputSink.
    /// Param `playback_status_sender` is an optional channel which can be used to receive
    /// playback status events for the currently playing sources.
    pub fn new(
        sink: DefaultOutputSink,
        playback_status_sender: Option<Sender<PlaybackStatusEvent>>,
    ) -> Self {
        // Create a proxy for the playback status channel, so we can trap stop messages
        let playing_sources = Arc::new(DashMap::with_capacity(1024));
        let playback_status_sender_proxy =
            Self::handle_playback_events(playback_status_sender, playing_sources.clone());

        // Create audio garbage collector and thread
        let collector = Collector::new();
        let collector_handle = collector.handle();
        let collector_running = Arc::new(AtomicBool::new(true));
        Self::handle_drop_collects(collector, collector_running.clone());

        // Create a mixer source, add it to the audio sink and start running
        let mixer_source = MixedSource::new(sink.channel_count(), sink.sample_rate());
        let mixer_event_queue = mixer_source.event_queue();
        let mut sink = sink;
        sink.play(mixer_source);
        sink.resume();

        Self {
            sink,
            playing_sources,
            playback_status_sender: playback_status_sender_proxy,
            collector_handle,
            collector_running,
            mixer_event_queue,
        }
    }

    /// Our audio device's suspended state.
    pub fn output_suspended(&self) -> bool {
        self.sink.suspended()
    }

    /// Our audio device's actual sample rate.
    pub fn output_sample_rate(&self) -> u32 {
        self.sink.sample_rate()
    }
    /// Our audio device's actual sample channel count.
    pub fn output_channel_count(&self) -> usize {
        self.sink.channel_count()
    }
    /// Our actual playhead pos in samples (NOT sample frames)
    pub fn output_sample_position(&self) -> u64 {
        self.sink.sample_position()
    }
    /// Our actual playhead pos in sample frames
    pub fn output_sample_frame_position(&self) -> u64 {
        self.output_sample_position() / self.output_channel_count() as u64
    }

    /// Our output's global volume factor
    pub fn output_volume(&self) -> f32 {
        self.sink.volume()
    }
    /// Set a new global volume factor
    pub fn set_output_volume(&mut self, volume: f32) {
        assert!(volume >= 0.0);
        self.sink.set_volume(volume);
    }

    /// Get a copy of our playback status sender channel.
    /// Should be used by custom audio sources only.
    pub fn playback_status_sender(&self) -> Sender<PlaybackStatusEvent> {
        self.playback_status_sender.clone()
    }

    /// Start audio playback.
    pub fn start(&mut self) {
        self.sink.resume();
    }

    /// Stop audio playback. This will only pause and thus not drop any playing sources. Use the
    /// `start` function to start it again. Use function `stop_all_sources` to drop all sources.
    pub fn stop(&mut self) {
        self.sink.pause();
    }

    /// Play a self created or cloned file source.
    pub fn play_file_source<Source: FileSource>(
        &mut self,
        file_source: Source,
        start_time: Option<u64>,
    ) -> Result<PlaybackId, Error> {
        self.play_file_source_with_context(file_source, start_time, None)
    }
    /// Play a self created or cloned file source with the given playback status context.
    pub fn play_file_source_with_context<Source: FileSource>(
        &mut self,
        file_source: Source,
        start_time: Option<u64>,
        context: Option<PlaybackStatusContext>,
    ) -> Result<PlaybackId, Error> {
        // make sure the source has a valid playback status channel
        let mut file_source = file_source;
        if file_source.playback_status_sender().is_none() {
            file_source.set_playback_status_sender(Some(self.playback_status_sender.clone()));
        }
        // set playback context
        file_source.set_playback_status_context(context);
        // memorize source in playing sources map
        let playback_id = file_source.playback_id();
        let playback_volume = file_source.playback_options().volume;
        let playback_panning = file_source.playback_options().panning;
        let playback_message_queue =
            PlaybackMessageSender::File(file_source.playback_message_queue());
        self.playing_sources
            .insert(playback_id, playback_message_queue.clone());
        // convert file to mixer's rate and channel layout
        let converted_source = ConvertedSource::new(
            file_source,
            self.sink.channel_count(),
            self.sink.sample_rate(),
            ResamplingQuality::Default,
        );
        // apply volume options
        let amplified_source = AmplifiedSource::new(converted_source, playback_volume);
        // apply panning options
        let panned_source = PannedSource::new(amplified_source, playback_panning);
        // send the source to the mixer
        if self
            .mixer_event_queue
            .push(MixedSourceMsg::AddSource {
                playback_id,
                playback_message_queue,
                source: Owned::new(&self.collector_handle, Box::new(panned_source)),
                sample_time: start_time.unwrap_or(0),
            })
            .is_err()
        {
            log::warn!("mixer's event queue is full. playback event got skipped!");
            log::warn!("increase the mixer event queue to prevent this from happening...");
        }
        // return new file's id
        Ok(playback_id)
    }

    /// Play a self created synth source with the given playback options.
    pub fn play_synth_source<S: SynthSource>(
        &mut self,
        synth_source: S,
        start_time: Option<u64>,
    ) -> Result<PlaybackId, Error> {
        self.play_synth_source_with_context(synth_source, start_time, None)
    }
    /// Play a self created synth source with the given playback options and
    /// playback status context.
    pub fn play_synth_source_with_context<S: SynthSource>(
        &mut self,
        synth_source: S,
        start_time: Option<u64>,
        context: Option<PlaybackStatusContext>,
    ) -> Result<PlaybackId, Error> {
        // make sure the source has a valid playback status channel
        let mut synth_source = synth_source;
        if synth_source.playback_status_sender().is_none() {
            synth_source.set_playback_status_sender(Some(self.playback_status_sender.clone()));
        }
        synth_source.set_playback_status_context(context);
        // memorize source in playing sources map
        let playback_id = synth_source.playback_id();
        let playback_volume = synth_source.playback_options().volume;
        let playback_panning = synth_source.playback_options().panning;
        let playback_message_queue =
            PlaybackMessageSender::Synth(synth_source.playback_message_queue());
        self.playing_sources
            .insert(playback_id, playback_message_queue.clone());
        // convert file to mixer's rate and channel layout
        let converted_source = ConvertedSource::new(
            synth_source,
            self.sink.channel_count(),
            self.sink.sample_rate(),
            ResamplingQuality::Default, // usually unused
        );
        // apply volume options
        let amplified_source = AmplifiedSource::new(converted_source, playback_volume);
        // apply panning options
        let panned_source = PannedSource::new(amplified_source, playback_panning);
        // send the source to the mixer
        if self
            .mixer_event_queue
            .push(MixedSourceMsg::AddSource {
                playback_id,
                playback_message_queue,
                source: Owned::new(&self.collector_handle, Box::new(panned_source)),
                sample_time: start_time.unwrap_or(0),
            })
            .is_err()
        {
            log::warn!("mixer's event queue is full. playback event got skipped!");
            log::warn!("increase the mixer event queue to prevent this from happening...");
        }
        // return new synth's id
        Ok(playback_id)
    }

    /// Change playback position of the given played back source. This is only supported for files and thus
    /// won't do anything for synths.
    pub fn seek_source(
        &mut self,
        playback_id: PlaybackId,
        position: Duration,
    ) -> Result<(), Error> {
        if let Some(msg_sender) = self.playing_sources.get(&playback_id) {
            if let PlaybackMessageSender::File(queue) = msg_sender.value() {
                if queue.push(FilePlaybackMessage::Seek(position)).is_err() {
                    log::warn!("failed to send seek command to file");
                    return Err(Error::SendError);
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

    /// Set a playing file source's speed with the given optional glide rate in semitones per second.
    /// This is only supported for files and thus won't do anything for synths.
    pub fn set_source_speed(
        &mut self,
        playback_id: PlaybackId,
        speed: f64,
        glide: Option<f32>,
    ) -> Result<(), Error> {
        // check if the given playback id is still know (playing)
        if let Some(source) = self.playing_sources.get(&playback_id) {
            if let PlaybackMessageSender::File(queue) = &*source {
                if queue
                    .push(FilePlaybackMessage::SetSpeed(speed, glide))
                    .is_err()
                {
                    Err(Error::SendError)
                } else {
                    Ok(())
                }
            } else {
                Err(Error::MediaFileNotFound)
            }
        } else {
            Err(Error::MediaFileNotFound)
        }
    }

    /// Set a playing file source's speed at a given sample time in future with the given
    /// optional glide rate in semitones per second.
    /// This is only supported for files and thus won't do anything for synths.
    pub fn set_source_speed_at_sample_time(
        &mut self,
        playback_id: PlaybackId,
        speed: f64,
        glide: Option<f32>,
        sample_time: u64,
    ) -> Result<(), Error> {
        // check if the given playback id is still know (playing)
        if self.playing_sources.contains_key(&playback_id) {
            // pass event to mixer to schedule it
            if self
                .mixer_event_queue
                .push(MixedSourceMsg::SetSpeed {
                    playback_id,
                    speed,
                    glide,
                    sample_time,
                })
                .is_err()
            {
                log::warn!("mixer's event queue is full. playback event got skipped!");
                log::warn!("increase the mixer event queue to prevent this from happening...");
            }
            Ok(())
        } else {
            Err(Error::MediaFileNotFound)
        }
    }

    /// Immediately stop a playing file or synth source. NB: This will fade-out the source when a
    /// stop_fade_out_duration option was set in the playback options it got started with.
    pub fn stop_source(&mut self, playback_id: PlaybackId) -> Result<(), Error> {
        let stopped = match self.playing_sources.get(&playback_id) {
            Some(msg_queue) => {
                if msg_queue.value().send_stop().is_err() {
                    return Err(Error::SendError);
                }
                true
            }
            None => false,
        };
        if stopped {
            // we shortly will receive an exhausted event which removes the source, but nevertheless
            // remove it now, to force all following attempts to stop this source to fail
            self.playing_sources.remove(&playback_id);
            Ok(())
        } else {
            Err(Error::MediaFileNotFound)
        }
    }

    /// Stop a playing file or synth source at a given sample time in future.
    pub fn stop_source_at_sample_time(
        &mut self,
        playback_id: PlaybackId,
        stop_time: u64,
    ) -> Result<(), Error> {
        // check if the given playback id is still know (playing)
        if self.playing_sources.contains_key(&playback_id) {
            // pass stop request to mixer (force push stop events!)
            self.mixer_event_queue
                .force_push(MixedSourceMsg::StopSource {
                    playback_id,
                    sample_time: stop_time,
                });
            // NB: do not remove from playing_sources, as the event may apply in a long time in future.
            Ok(())
        } else {
            Err(Error::MediaFileNotFound)
        }
    }

    /// Immediately stop all playing and possibly scheduled sources.
    pub fn stop_all_sources(&mut self) -> Result<(), Error> {
        // stop everything that is playing now
        let playing_source_ids = {
            self.playing_sources
                .iter()
                .map(|e| *e.key())
                .collect::<Vec<_>>()
        };
        for source_id in playing_source_ids {
            self.stop_source(source_id)?;
        }
        // remove all upcoming, scheduled sources in the mixer too (force push stop events!)
        self.mixer_event_queue
            .force_push(MixedSourceMsg::RemoveAllPendingSources);
        Ok(())
    }
}

/// details
impl Player {
    fn handle_playback_events(
        playback_sender: Option<Sender<PlaybackStatusEvent>>,
        playing_sources: Arc<DashMap<PlaybackId, PlaybackMessageSender>>,
    ) -> Sender<PlaybackStatusEvent> {
        let (playback_send_proxy, playback_recv_proxy) = {
            // use same capacity in proxy as original one
            if let Some(playback_sender) = &playback_sender {
                if let Some(capacity) = playback_sender.capacity() {
                    crossbeam_channel::bounded::<PlaybackStatusEvent>(capacity)
                } else {
                    crossbeam_channel::unbounded::<PlaybackStatusEvent>()
                }
            // use a bounded channel with a default cap for playback tracking, when there's no original channel
            } else {
                const DEFAULT_PLAYBACK_EVENTS_CAPACITY: usize = 1024;
                crossbeam_channel::bounded::<PlaybackStatusEvent>(DEFAULT_PLAYBACK_EVENTS_CAPACITY)
            }
        };

        std::thread::Builder::new()
            .name("audio_player_messages".to_string())
            .spawn(move || loop {
                if let Ok(event) = playback_recv_proxy.recv() {
                    if let PlaybackStatusEvent::Stopped { id, .. } = event {
                        playing_sources.remove(&id);
                    }
                    if let Some(sender) = &playback_sender {
                        // NB: send and not try_send: block until sender queue is free
                        if let Err(err) = sender.send(event) {
                            log::warn!("failed to send file status message: {err}");
                        }
                    }
                } else {
                    log::info!("playback event loop stopped");
                    break;
                }
            })
            .expect("failed to spawn audio message thread");

        playback_send_proxy
    }

    fn handle_drop_collects(mut collector: Collector, running: Arc<AtomicBool>) {
        std::thread::Builder::new()
            .name("audio_player_drops".to_string())
            .spawn(move || {
                while running.load(atomic::Ordering::Relaxed) {
                    collector.collect();
                    thread::sleep(time::Duration::from_millis(100));
                }
                log::info!("audio collector loop stopped");
                collector.collect();
                if collector.try_cleanup().is_err() {
                    log::warn!("Failed to cleanup collector");
                }
            })
            .expect("failed to spawn audio message thread");
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        // stop collector thread
        self.collector_running
            .store(false, atomic::Ordering::Relaxed);
        // stop playback thread and release mixer source
        self.sink.close();
    }
}
