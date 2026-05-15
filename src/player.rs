use std::{
    collections::HashMap,
    fmt,
    sync::{
        atomic::{self, AtomicBool, AtomicUsize},
        mpsc::{sync_channel, RecvTimeoutError, SyncSender},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

use basedrop::{Collector, Handle, Owned};
use crossbeam_queue::ArrayQueue;
use dashmap::DashMap;

use crate::{
    effect::Effect,
    error::Error,
    generator::sequencer::Sequencer,
    output::OutputDevice,
    source::{
        amplified::AmplifiedSource,
        converted::ConvertedSource,
        file::FileSource,
        guarded::GuardedSource,
        mapped::ChannelMappedSource,
        measured::{CpuLoad, MeasuredSource, SharedCpuLoadState},
        metered::{AudioLevel, MeteredSource, SharedAudioLevelState},
        mixed::{
            EffectProcessor, MixedSource, MixerMessage, SubMixerProcessor, SubMixerThreadPool,
        },
        panned::PannedSource,
        playback::PlaybackMessageQueue,
        resampled::ResamplingQuality,
        status::{PlaybackStatusContext, PlaybackStatusEvent},
        synth::SynthSource,
        Source,
    },
    Generator, Transport,
};

// -------------------------------------------------------------------------------------------------

mod handles;

// -------------------------------------------------------------------------------------------------

/// Unique source ID for played back file, synth or generator sources.
pub type PlaybackId = usize;

/// Unique mixer ID for newly added mixers.
pub type MixerId = usize;

/// Unique ID for newly added effects.
pub type EffectId = usize;

/// Unique ID for individual sounds played in a generator.
pub type NotePlaybackId = usize;

/// Unique ID for a sequencer registered with the player.
pub type SequencerId = usize;

// Playback handles for sources.
pub use handles::{
    EffectHandle, FilePlaybackHandle, GeneratorPlaybackHandle, MixerHandle, SequencerHandle,
    SourcePlaybackHandle, SynthPlaybackHandle,
};

/// A callback function to handle panics occurring within the player's main mixer.
///
/// Will be called once only. The player is silent afterwards and should be shut down
/// as soon as possible.
pub type PanicHandler = crate::source::guarded::PanicHandler;

// -------------------------------------------------------------------------------------------------

/// How to move an effect within a mixer.
pub enum EffectMovement {
    /// Negative value shift the effect towards the start, positive ones towards the end.
    Direction(i32),
    /// Move effect to the start of the effect chain.
    Start,
    /// Move effect to the end of the effect chain.
    End,
}

// -------------------------------------------------------------------------------------------------

/// Player internal info about a currently playing source.
pub(super) struct PlayingSource {
    is_playing: Arc<AtomicBool>,
    is_transient: bool,
    playback_message_queue: PlaybackMessageQueue,
    mixer_id: MixerId,
    source_name: String,
}

impl Drop for PlayingSource {
    fn drop(&mut self) {
        // NB: this only works when Self is not clone or copy, so we can ensure that an object
        // isn't created temporarily and then dropped again!!
        self.is_playing.store(false, atomic::Ordering::Relaxed);
    }
}

// -------------------------------------------------------------------------------------------------

/// Player internal info about a registered sequencer.
#[derive(Debug, Clone)]
pub(super) struct PlayerSequencerInfo {
    mixer_id: MixerId,
}

// -------------------------------------------------------------------------------------------------

/// Player internal info about an instantiated mixer.
#[derive(Debug, Clone)]
pub(super) struct PlayerMixerInfo {
    parent_id: MixerId,
    event_queue: Arc<ArrayQueue<MixerMessage>>,
}

// -------------------------------------------------------------------------------------------------

/// Player internal info about an instantiated effect.
#[derive(Debug, Copy, Clone)]
pub(super) struct PlayerEffectInfo {
    mixer_id: MixerId,
    effect_name: &'static str,
}

// -------------------------------------------------------------------------------------------------

/// Configuration for creating a Player with custom settings.
///
/// This allows configuring optional features like parallel mixer processing.
#[derive(Debug, Clone)]
pub struct PlayerConfig {
    /// Whether the player's mixer runs in stereo, regardless of the output device's channel layout.
    /// The final stereo mix is then remapped to the device's channel count (e.g. expanded to
    /// surround by duplicating the first two channels, or mixed down to mono).
    ///
    /// Enabled by default, so effects and generators can assume a stereo layout without needing
    /// to handle arbitrary channel counts.
    pub enforce_stereo_playback: bool,

    /// Whether concurrent mixer graph processing is enabled.
    ///
    /// Even when enabled, the player will automatically fall back to sequential processing
    /// when thresholds are not met, avoiding unnecessary overhead for simple mixer graphs.
    ///
    /// Note that only the main mixer's sub mixers's are processed in parallel.
    ///
    /// Default: `true` (enabled)
    pub concurrent_processing: bool,

    /// Number of mixer processing worker threads to spawn.
    ///
    /// `None` will auto-detect based on available CPU cores.
    /// Default: `None` (auto)
    pub concurrent_worker_threads: Option<usize>,

    /// How often mixer CPU loads are updated.
    ///
    /// `None` disables CPU tracking entirely.
    /// `Some` values enable tracking with the given update rate.
    ///
    /// Default: `Some(250_ms)`
    pub measuring_interval: Option<Duration>,

    /// How often mixer audio levels (peak/RMS) are updated.
    ///
    /// `None` disables metering entirely.
    /// `Some` values enable metering with the given meter update rate.
    ///
    /// Default: `None` (avoid processing overhead)
    pub metering_interval: Option<Duration>,
}

impl Default for PlayerConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl PlayerConfig {
    /// Create a new default player configuration.
    pub fn new() -> Self {
        Self {
            enforce_stereo_playback: true,
            concurrent_processing: true,
            concurrent_worker_threads: None,
            measuring_interval: Some(Duration::from_millis(250)),
            metering_interval: None,
        }
    }

    /// Set if stereo playback is enforced.
    pub fn enforce_stereo_playback(mut self, enabled: bool) -> Self {
        self.enforce_stereo_playback = enabled;
        self
    }

    /// Set if parallel mixing is enabled.
    pub fn concurrent_processing(mut self, enabled: bool) -> Self {
        self.concurrent_processing = enabled;
        self
    }

    /// Set parallel mixer thread count.
    pub fn concurrent_worker_threads(mut self, count: usize) -> Self {
        self.concurrent_worker_threads = Some(count);
        self
    }

    /// Set the audio CPU load update interval for all mixers.
    ///
    /// Pass `None` to disable measuring entirely (with zero overhead).
    pub fn measuring_interval(mut self, interval: Option<Duration>) -> Self {
        self.measuring_interval = interval;
        self
    }

    /// Set the audio metering update interval for all mixers.
    ///
    /// Pass `None` to disable metering entirely (with zero overhead).
    pub fn metering_interval(mut self, interval: Option<Duration>) -> Self {
        self.metering_interval = interval;
        self
    }

    /// Applied worker thread count, using system's available threads when
    /// `concurrent_worker_threads` is `None`.
    pub fn effective_concurrent_worker_threads(&self) -> usize {
        self.concurrent_worker_threads.unwrap_or(num_cpus::get())
    }
}

// -------------------------------------------------------------------------------------------------

/// Playback controller, which continuously fills an [`OutputDevice`]s stream with audio data
/// generated by one or more [`Source`](crate::Source)s.
///
/// It can play an unlimited number of [`FileSource`] or [`SynthSource`] sources and allows
/// monitoring playback position via an optional [`PlaybackStatusEvent`] [`sync_channel`].
///
/// When starting to play a source a [`FilePlaybackHandle`] or [`SynthPlaybackHandle`] is returned
/// which allows checking if the source is still playing, or to stop it, or to change playback runtime
/// properties such as volume/pan and pitch.
///
/// The player also supports creating complex DSP graphs by adding sub-mixers and [`Effect`]s.
/// Initially, a `Player` contains a single main mixer only. You can add effects to this mixer
/// using [`add_effect`](Self::add_effect). Audio sources played without specifying a target mixer
/// will be routed through the main mixer and its effects.
///
/// To create more advanced routing, you can add new mixers as sub-mixers to existing ones
/// using [`add_mixer`](Self::add_mixer). Each mixer can have its own chain of effects. When
/// playing a source, you can specify a `target_mixer` in the playback options to direct its output
/// to a specific sub-mixer. This allows for parallel processing paths, such as applying different
/// effects to different groups of sounds.
pub struct Player {
    config: PlayerConfig,
    output_device: Box<dyn OutputDevice>,
    playing_sources: Arc<DashMap<PlaybackId, PlayingSource>>,
    playback_status_running: Arc<AtomicBool>,
    playback_status_sender: SyncSender<PlaybackStatusEvent>,
    playback_status_thread: Option<thread::JoinHandle<()>>,
    collector_handle: Handle,
    collector_running: Arc<AtomicBool>,
    collector_thread: Option<thread::JoinHandle<()>>,
    transport: Transport,
    sequencers: Arc<DashMap<SequencerId, PlayerSequencerInfo>>,
    mixers: DashMap<MixerId, PlayerMixerInfo>,
    effects: DashMap<EffectId, PlayerEffectInfo>,
    main_mixer_measurement_state: Option<SharedCpuLoadState>,
    main_mixer_metering_state: Option<SharedAudioLevelState>,
    main_mixer_panic_handler: Arc<Mutex<Option<PanicHandler>>>,
    main_mixer_dropped: Arc<atomic::AtomicBool>,
}

impl Player {
    /// The ID of the main mixer, which is always present.
    const MAIN_MIXER_ID: MixerId = 0;

    /// Create a new player for the given [`OutputDevice`]. Param `playback_status_sender` is an optional
    /// channel which can be used to receive playback status events for the currently playing sources.
    pub fn new<S: Into<Option<SyncSender<PlaybackStatusEvent>>>>(
        output_device: impl OutputDevice + 'static,
        playback_status_sender: S,
    ) -> Self {
        Self::new_with_config(
            output_device,
            playback_status_sender,
            PlayerConfig::default(),
        )
    }

    /// Create a new player with custom [`PlayerConfig`].
    /// This allows enabling optional features like parallel mixer processing.
    ///
    /// See [Self::new] for descriptions of the other parameters.
    pub fn new_with_config<S: Into<Option<SyncSender<PlaybackStatusEvent>>>>(
        output_device: impl OutputDevice + 'static,
        playback_status_sender: S,
        config: PlayerConfig,
    ) -> Self {
        log::info!("Creating a new player...");

        // Memorize the sink
        let mut output_device = Box::new(output_device);

        // Create a playback status proxy channel and thread, so we can intercept stop messages
        let playing_sources = Arc::new(DashMap::with_capacity(1024));
        let playback_status_running = Arc::new(AtomicBool::new(true));
        let playback_status_sender = playback_status_sender.into();
        let (playback_status_sender, playback_status_thread) = Self::handle_playback_events(
            playback_status_sender,
            Arc::clone(&playing_sources),
            Arc::clone(&playback_status_running),
        );
        let playback_status_thread = Some(playback_status_thread);

        // Create audio garbage collector and thread
        let collector = Collector::new();
        let collector_handle = collector.handle();
        let collector_running = Arc::new(AtomicBool::new(true));
        let collector_thread = Some(Self::handle_drop_collects(
            collector,
            Arc::clone(&collector_running),
        ));

        // Create a mixer source and add it to the audio sink
        let mut main_mixer = MixedSource::new(
            if config.enforce_stereo_playback {
                2
            } else {
                output_device.channel_count()
            },
            output_device.sample_rate(),
        );

        // Create thread pool main mixer
        let thread_pool = (config.concurrent_processing
            && config.effective_concurrent_worker_threads() > 1)
            .then(|| {
                log::info!(
                    "Creating mixer thread pool with {} threads...",
                    config.effective_concurrent_worker_threads()
                );
                SubMixerThreadPool::new(
                    config.effective_concurrent_worker_threads(),
                    output_device.sample_rate(),
                )
            });
        main_mixer.set_thread_pool(thread_pool);

        let mixer_event_queue = main_mixer.message_queue();

        // Wrap main mixer in MeteredSource for audio level tracking
        let metered_main_mixer = MeteredSource::new(main_mixer, config.metering_interval);
        let main_mixer_metering_state = metered_main_mixer.state();

        // Wrap in MeasuredSource for CPU load tracking
        let measured_main_mixer =
            MeasuredSource::new(metered_main_mixer, config.measuring_interval);
        let main_mixer_measurement_state = measured_main_mixer.state();

        let mixers = DashMap::new();
        mixers.insert(
            Player::MAIN_MIXER_ID,
            PlayerMixerInfo {
                parent_id: Player::MAIN_MIXER_ID,
                event_queue: mixer_event_queue,
            },
        );
        let effects = DashMap::new();

        // wrap main mixer into a GuardedSource
        let main_mixer_panic_handler = Arc::new(Mutex::new(None));
        let main_mixer_dropped = Arc::new(atomic::AtomicBool::new(false));

        let guarded_main_mixer = GuardedSource::new(
            measured_main_mixer,
            "Player Main-Mixer",
            Arc::clone(&main_mixer_panic_handler),
        )
        .with_drop_signal(Arc::clone(&main_mixer_dropped));

        // Assign the wrapped main mixer as sink source
        if config.enforce_stereo_playback && output_device.channel_count() != 2 {
            // Map the main mixer's enforced stereo output to the output device's channel layout
            let channel_mapped_source =
                ChannelMappedSource::new(guarded_main_mixer, output_device.channel_count());
            output_device.play(channel_mapped_source.into_box());
        } else {
            output_device.play(guarded_main_mixer.into_box());
        }

        let transport = Transport::new(output_device.sample_rate(), 120.0, 4);
        let sequencers = Arc::new(DashMap::new());

        Self {
            config,
            output_device,
            playing_sources,
            playback_status_running,
            playback_status_sender,
            playback_status_thread,
            collector_handle,
            collector_running,
            collector_thread,
            transport,
            mixers,
            sequencers,
            effects,
            main_mixer_dropped,
            main_mixer_panic_handler,
            main_mixer_measurement_state,
            main_mixer_metering_state,
        }
    }

    /// True when the output device is currently suspended,
    /// e.g. because the app which drives the audio stream is hidden.
    pub fn output_suspended(&self) -> bool {
        self.output_device.is_suspended()
    }

    /// Our main mixers sample rate.
    pub fn output_sample_rate(&self) -> u32 {
        self.output_device.sample_rate()
    }
    /// Our main mixer's sample channel count.
    pub fn output_channel_count(&self) -> usize {
        if self.config.enforce_stereo_playback {
            2
        } else {
            self.output_device.channel_count()
        }
    }

    /// Our actual playhead pos in sample frames
    pub fn output_sample_frame_position(&self) -> u64 {
        let channel_count = self.output_device.channel_count();
        if channel_count > 0 {
            self.output_device.sample_position() / channel_count as u64
        } else {
            0
        }
    }

    /// Our output's global volume factor
    pub fn output_volume(&self) -> f32 {
        self.output_device.volume()
    }
    /// Set a new global volume factor
    pub fn set_output_volume(&mut self, volume: f32) {
        assert!(volume >= 0.0);
        self.output_device.set_volume(volume);
    }

    /// Get the current CPU load for the player's main mixer.
    ///
    /// Only available when CPU measurement is enabled in the player's [`PlayerConfig`].
    pub fn cpu_load(&self) -> Option<CpuLoad> {
        self.main_mixer_measurement_state
            .as_ref()
            .and_then(|s| s.try_lock().ok())
            .map(|state| state.cpu_load())
    }

    /// Get the shared CPU load data for the player's main mixer.
    ///
    /// Only available when CPU measurement is enabled in the player's [`PlayerConfig`].
    pub fn cpu_load_state(&self) -> Option<SharedCpuLoadState> {
        self.main_mixer_measurement_state.as_ref().map(Arc::clone)
    }

    /// Get the current audio level for the player's main mixer.
    ///
    /// Only available when audio metering is enabled in the player's [`PlayerConfig`].
    pub fn audio_level(&self) -> Option<AudioLevel> {
        self.main_mixer_metering_state
            .as_ref()
            .and_then(|s| s.try_lock().ok())
            .map(|state| state.audio_level().clone())
    }

    /// Get the shared audio level data for the player's main mixer, if metering is enabled.
    ///
    /// Only available when audio metering is enabled in the player's [`PlayerConfig`].
    pub fn audio_level_state(&self) -> Option<SharedAudioLevelState> {
        self.main_mixer_metering_state.as_ref().map(Arc::clone)
    }

    /// Sets or replaces a panic handler for the player's main mixer.
    ///
    /// The provided handler will be called once when the main mixer panics during audio processing.
    /// Should be used for diagnostic and logging purposes only.
    ///
    /// Setting `None` will disable panic handling and just log panics instead.
    ///
    /// Use `panic::set_hook` to override default panic behavior of external threads in order to
    /// e.g. shut down the process after a panic in the audio threads.
    pub fn set_panic_handler(&mut self, handler: Option<PanicHandler>) {
        *self
            .main_mixer_panic_handler
            .lock()
            .expect("Failed access panic handler lock") = handler;
    }

    /// Start audio playback.
    pub fn is_running(&self) -> bool {
        self.output_device.is_running()
    }

    /// Start audio playback.
    pub fn start(&mut self) {
        self.output_device.resume();
    }

    /// Stop audio playback. This will only pause and thus not drop any playing sources. Use the
    /// `start` function to start it again. Use function `stop_all_sources` to drop all sources.
    pub fn stop(&mut self) {
        self.output_device.pause();
    }

    /// Get the current global transport (BPM, time signature, sample rate) as applied to all
    /// sequencers that get added to the player.
    pub fn transport(&self) -> Transport {
        self.transport
    }

    /// Set the global BPM tempo. This will change BPMs in all currently playing mixer-driven
    /// sequencers as well.
    ///
    /// Panics when `bpm` is <=0.
    pub fn set_transport_bpm(&mut self, bpm: f64) {
        assert!(bpm > 0.0, "Invalid BPM in player: {bpm}");
        self.transport = Transport::new(
            self.transport.sample_rate(),
            bpm,
            self.transport.beats_per_bar(),
        );
        self.send_transport_change();
    }

    /// Set the global time signature as beats per bar. This will change signatures in all
    /// currently playing mixer-driven sequencers as well.
    ///
    /// Panic when `beats_per_bar` is 0.
    pub fn set_transport_beats_per_bar(&mut self, beats_per_bar: usize) {
        assert!(
            beats_per_bar > 0,
            "Invalid beats/bar count in player: {beats_per_bar}"
        );
        self.transport = Transport::new(
            self.transport.sample_rate(),
            self.transport.beats_per_minute(),
            beats_per_bar,
        );
        self.send_transport_change();
    }

    /// Play a newly created or cloned file source.
    pub fn play_file_source<F: FileSource, T: Into<Option<u64>>>(
        &mut self,
        file_source: F,
        start_time: T,
    ) -> Result<FilePlaybackHandle, Error> {
        self.play_file_source_with_context(file_source, start_time, None)
    }
    /// Play a newly created or cloned file source with the given playback status context.
    pub fn play_file_source_with_context<F: FileSource, T: Into<Option<u64>>>(
        &mut self,
        file_source: F,
        start_time: T,
        context: Option<PlaybackStatusContext>,
    ) -> Result<FilePlaybackHandle, Error> {
        // validate and get options
        let playback_options = *file_source.playback_options();
        playback_options.validate()?;
        // validate and get target mixer
        let mixer_id = playback_options.target_mixer.unwrap_or(Self::MAIN_MIXER_ID);
        let mixer_event_queue = self.mixer_event_queue(mixer_id)?;
        // redirect source's playback status channel to us and set context
        let mut file_source = file_source;
        file_source.set_playback_status_sender(Some(self.playback_status_sender.clone()));
        file_source.set_playback_status_context(context);
        // memorize source in playing sources map
        let playback_id = file_source.playback_id();
        let playback_message_queue = file_source.playback_message_queue();
        let source_name = format!("File: '{}'", file_source.file_name());
        // convert file to mixer's rate and channel layout
        let converted_source = ConvertedSource::new(
            file_source,
            self.output_channel_count(),
            self.output_sample_rate(),
            ResamplingQuality::Default,
        );
        // apply volume options
        let amplified_source = AmplifiedSource::new(converted_source, playback_options.volume);
        let volume_message_queue = amplified_source.message_queue();
        // apply panning options
        let panned_source = PannedSource::new(amplified_source, playback_options.panning);
        let panning_message_queue = panned_source.message_queue();
        // apply measure options
        let measure_interval = if playback_options.measure_cpu_load {
            self.config.measuring_interval
        } else {
            None
        };
        let measured_source = MeasuredSource::new(panned_source, measure_interval);
        let measurement_state = measured_source.state();
        // add to playing sources
        let is_playing = Arc::new(AtomicBool::new(true));
        let playback_message_queue = PlaybackMessageQueue::File {
            playback: playback_message_queue,
            volume: volume_message_queue,
            panning: panning_message_queue,
        };
        self.playing_sources.insert(
            playback_id,
            PlayingSource {
                is_playing: Arc::clone(&is_playing),
                is_transient: true,
                playback_message_queue: playback_message_queue.clone(),
                mixer_id,
                source_name,
            },
        );
        // send the source to the mixer
        let source = Owned::new(&self.collector_handle, measured_source.into_box());
        let sample_time = start_time.into().unwrap_or(0);
        if mixer_event_queue
            .push(MixerMessage::AddSource {
                is_transient: true,
                playback_id,
                playback_message_queue: playback_message_queue.clone(),
                source,
                sample_time,
            })
            .is_err()
        {
            self.playing_sources.remove(&playback_id);
            Err(Self::mixer_event_queue_error("play_file"))
        } else {
            // Create and return handle
            Ok(FilePlaybackHandle::new(
                is_playing,
                playback_id,
                playback_message_queue,
                mixer_event_queue,
                measurement_state,
            ))
        }
    }

    /// Play a newly created or cloned synth source with the given playback options.
    pub fn play_synth_source<S: SynthSource, T: Into<Option<u64>>>(
        &mut self,
        synth_source: S,
        start_time: T,
    ) -> Result<SynthPlaybackHandle, Error> {
        self.play_synth_source_with_context(synth_source, start_time, None)
    }
    /// Play a newly created or cloned synth source with the given playback options and
    /// playback status context.
    pub fn play_synth_source_with_context<S: SynthSource, T: Into<Option<u64>>>(
        &mut self,
        synth_source: S,
        start_time: T,
        context: Option<PlaybackStatusContext>,
    ) -> Result<SynthPlaybackHandle, Error> {
        // validate and get options
        let playback_options = *synth_source.playback_options();
        playback_options.validate()?;
        // validate and get target mixer
        let mixer_id = playback_options.target_mixer.unwrap_or(Self::MAIN_MIXER_ID);
        let mixer_event_queue = self.mixer_event_queue(mixer_id)?;
        // redirect source's playback status channel to us and set context
        let mut synth_source = synth_source;
        synth_source.set_playback_status_sender(Some(self.playback_status_sender.clone()));
        synth_source.set_playback_status_context(context);
        // memorize source in playing sources map
        let playback_id = synth_source.playback_id();
        let playback_message_queue = synth_source.playback_message_queue();
        let source_name = format!("Synth: '{}'", synth_source.synth_name());
        // convert synth to mixer's rate and channel layout
        let converted_source = ConvertedSource::new(
            synth_source,
            self.output_channel_count(),
            self.output_sample_rate(),
            ResamplingQuality::Default, // usually unused
        );
        // apply volume options
        let amplified_source = AmplifiedSource::new(converted_source, playback_options.volume);
        let volume_message_queue = amplified_source.message_queue();
        // apply panning options
        let panned_source = PannedSource::new(amplified_source, playback_options.panning);
        let panning_message_queue = panned_source.message_queue();
        // apply measure options
        let measure_interval = if playback_options.measure_cpu_load {
            self.config.measuring_interval
        } else {
            None
        };
        let measured_source = MeasuredSource::new(panned_source, measure_interval);
        let measurement_state = measured_source.state();
        // add to playing sources
        let is_playing = Arc::new(AtomicBool::new(true));
        let playback_message_queue = PlaybackMessageQueue::Synth {
            playback: playback_message_queue,
            volume: volume_message_queue,
            panning: panning_message_queue,
        };
        self.playing_sources.insert(
            playback_id,
            PlayingSource {
                is_playing: Arc::clone(&is_playing),
                is_transient: true,
                playback_message_queue: playback_message_queue.clone(),
                mixer_id,
                source_name,
            },
        );
        // send the source to the mixer
        let source = Owned::new(&self.collector_handle, measured_source.into_box());
        let sample_time = start_time.into().unwrap_or(0);
        if mixer_event_queue
            .push(MixerMessage::AddSource {
                is_transient: true,
                playback_id,
                playback_message_queue: playback_message_queue.clone(),
                source,
                sample_time,
            })
            .is_err()
        {
            self.playing_sources.remove(&playback_id);
            Err(Self::mixer_event_queue_error("play_synth"))
        } else {
            // Create and return handle
            Ok(SynthPlaybackHandle::new(
                is_playing,
                playback_id,
                playback_message_queue,
                mixer_event_queue,
                measurement_state,
            ))
        }
    }

    /// Play a generator source with the given options. *Played* generators will be removed
    /// when stopping all sources or when stopping it like a regular source. To keep a generator
    /// running until it gets explicitly removed use [Self::add_generator] instead.
    ///
    /// Returns a handle that can be used to control the generator, e.g. to stop it or to send
    /// events to trigger or stop individual notes.
    ///
    /// Note that boxed `dyn Generator` can be passed here as well as there's a generator impl
    /// defined for `Box<dyn Generator>` in the Generator trait definition.
    pub fn play_generator<G: Generator + 'static, T: Into<Option<u64>>>(
        &mut self,
        generator: G,
        start_time: T,
    ) -> Result<GeneratorPlaybackHandle, Error> {
        let is_transient = true;
        let mixer_id = generator
            .playback_options()
            .target_mixer
            .unwrap_or(Self::MAIN_MIXER_ID);
        self.add_or_play_generator(generator, is_transient, mixer_id, start_time)
    }

    /// Add a generator source with the given options. *Added* generators will not be removed
    /// when stopping it or when stopping all sources. Use [Self::play_generator] if the generator
    /// source should be automatically removed when stopping like a regular source.
    ///
    /// Returns a handle that can be used to control the generator, e.g. to stop it or to send
    /// events to trigger or stop individual notes.
    ///
    /// Note that boxed `dyn Generator` can be passed here as well as there's a generator impl
    /// defined for `Box<dyn Generator>` in the Generator trait definition.
    pub fn add_generator<G: Generator + 'static, M: Into<Option<MixerId>>>(
        &mut self,
        generator: G,
        mixer_id: M,
    ) -> Result<GeneratorPlaybackHandle, Error> {
        let is_transient = false;
        let mixer_id = mixer_id.into().unwrap_or(Self::MAIN_MIXER_ID);
        if let Some(target_mixer_id) = generator.playback_options().target_mixer {
            if target_mixer_id != mixer_id {
                log::warn!("Ignoring target mixer id from playback options when adding instead of playing a generator");
            }
        }
        self.add_or_play_generator(generator, is_transient, mixer_id, None)
    }

    /// Remove a generator which was added via [Self::add_generator].
    /// This will not stop all playing sounds in the generator, but simply remove it.
    pub fn remove_generator(&self, playback_id: PlaybackId) -> Result<(), Error> {
        // remove from mixer
        match self.playing_sources.get(&playback_id) {
            Some(playing_source) => {
                debug_assert!(
                    !playing_source.is_transient,
                    "Expected a non transient generator here, which was added via 'add_generator'"
                );
                // Send the remove message to parent
                if self
                    .mixer_event_queue(playing_source.mixer_id)?
                    .push(MixerMessage::RemoveSource { playback_id })
                    .is_err()
                {
                    return Err(Self::mixer_event_queue_error("remove_generator"));
                }
            }
            None => return Err(Error::GeneratorNotFoundError(playback_id)),
        }
        // remove from playing sources (outside of the `playing_sources.get` dashmap lock!)
        self.playing_sources.remove(&playback_id);
        Ok(())
    }

    /// Play a sequencer driven automatically by the same mixer that owns `generator`.
    ///
    /// The mixer calls [`run_until`](crate::generators::Sequencer::run_until) every audio block
    /// and propagates transport changes. The player's current [`Transport`] is enforced
    /// immediately on registration - any transport previously set on the sequencer is overridden.
    ///
    /// `start_time` controls when the sequencer activates. Pass `None` to start immediately,
    /// or a specific sample-frame position to defer the first [`reset`](crate::generators::Sequencer::reset)
    /// and event emission until the mixer reaches that position.
    ///
    /// Returns a [`SequencerHandle`] that can be used to query exhaustion status or stop the
    /// sequencer early via [`SequencerHandle::stop`] or [`stop_sequencer`](Self::stop_sequencer).
    pub fn play_sequencer<S: Sequencer + 'static, T: Into<Option<u64>>>(
        &mut self,
        sequencer: S,
        generator: GeneratorPlaybackHandle,
        start_time: T,
    ) -> Result<SequencerHandle, Error> {
        let start_time = start_time.into();
        let playback_id = generator.id();
        let mixer_id = self
            .playing_sources
            .get(&playback_id)
            .map(|s| s.mixer_id)
            .ok_or(Error::GeneratorNotFoundError(playback_id))?;
        let mixer_event_queue = self.mixer_event_queue(mixer_id)?;
        let sequencer_id = Self::unique_sequencer_id();
        let is_playing = Arc::new(AtomicBool::new(true));
        let sequencer = Owned::new(&self.collector_handle, sequencer.into_box());
        if mixer_event_queue
            .push(MixerMessage::AddSequencer {
                sequencer_id,
                sequencer,
                playback_id,
                is_playing: Arc::clone(&is_playing),
                transport: self.transport,
                start_time,
            })
            .is_err()
        {
            return Err(Self::mixer_event_queue_error("play_sequencer"));
        }
        self.sequencers
            .insert(sequencer_id, PlayerSequencerInfo { mixer_id });
        Ok(SequencerHandle::new(
            is_playing,
            sequencer_id,
            mixer_id,
            Arc::clone(&self.sequencers),
            mixer_event_queue,
        ))
    }

    /// Stop and eject a sequencer that was previously registered via [`play_sequencer`](Self::play_sequencer).
    ///
    /// Pass `None` to stop immediately, or `Some(sample_time)` to schedule the stop at a
    /// specific audio frame.
    ///
    /// Prefer [`SequencerHandle::stop`] when you have the handle - this variant is for cases
    /// where only the [`SequencerId`] is available. Returns `Err` if the ID is not found.
    pub fn stop_sequencer(
        &mut self,
        sequencer_id: SequencerId,
        sample_time: impl Into<Option<u64>>,
    ) -> Result<(), Error> {
        let sample_time = sample_time.into();
        let mixer_id = self
            .sequencers
            .remove(&sequencer_id)
            .map(|(_, info)| info.mixer_id)
            .ok_or(Error::SequencerNotFoundError(sequencer_id))?;
        let queue = self.mixer_event_queue(mixer_id)?;
        if queue
            .push(MixerMessage::StopSequencer {
                sequencer_id,
                sample_time,
            })
            .is_err()
        {
            return Err(Self::mixer_event_queue_error("stop_sequencer"));
        }
        Ok(())
    }

    /// Add a new mixer to an existing mixer.
    /// Use `None` as mixer id to add it to the main mixer.
    pub fn add_mixer<M: Into<Option<MixerId>>>(
        &mut self,
        parent_mixer_id: M,
    ) -> Result<MixerHandle, Error> {
        let parent_mixer_id = parent_mixer_id.into().unwrap_or(Self::MAIN_MIXER_ID);
        let parent_mixer_event_queue = self.mixer_event_queue(parent_mixer_id)?;

        let mixer = MixedSource::new(self.output_channel_count(), self.output_sample_rate());
        let mixer_queue = mixer.message_queue();
        let mixer_id = Self::unique_mixer_id();

        // Wrap in MeteredSource for audio level tracking
        let metered_mixer = MeteredSource::new(mixer, self.config.metering_interval);
        let metering_state = metered_mixer.state();

        // Wrap in MeasuredSource for CPU load tracking
        let measured_mixer = MeasuredSource::new(metered_mixer, self.config.measuring_interval);
        let measurement_state = measured_mixer.state();

        // Wrap into an owned processor
        let mixer_processor = Owned::new(
            &self.collector_handle,
            SubMixerProcessor::new(Box::new(measured_mixer)),
        );

        // Send message to parent mixer
        if parent_mixer_event_queue
            .push(MixerMessage::AddMixer {
                mixer_id,
                mixer_processor,
            })
            .is_err()
        {
            Err(Self::mixer_event_queue_error("add_mixer"))
        } else {
            self.mixers.insert(
                mixer_id,
                PlayerMixerInfo {
                    parent_id: parent_mixer_id,
                    event_queue: mixer_queue,
                },
            );

            Ok(MixerHandle::new(
                mixer_id,
                measurement_state,
                metering_state,
            ))
        }
    }

    /// Remove a mixer and all its effects from its parent mixer.
    pub fn remove_mixer(&mut self, mixer_id: MixerId) -> Result<(), Error> {
        // Can't remove the main mixer
        if mixer_id == Self::MAIN_MIXER_ID {
            return Err(Error::ParameterError(
                "Cannot remove the main mixer".to_string(),
            ));
        }

        // Find the parent mixer
        let parent_mixer_id = self.mixer_parent_id(mixer_id)?;

        let parent_mixer_event_queue = self.mixer_event_queue(parent_mixer_id)?;

        // Send the remove message to parent
        if parent_mixer_event_queue
            .push(MixerMessage::RemoveMixer { mixer_id })
            .is_err()
        {
            Err(Self::mixer_event_queue_error("remove_mixer"))
        } else {
            // Remove all effects that belong to this mixer
            let effects_to_remove: Vec<EffectId> = self
                .effects
                .iter()
                .filter_map(|entry| {
                    let (effect_id, effect_info) = (entry.key(), entry.value());
                    if effect_info.mixer_id == mixer_id {
                        Some(*effect_id)
                    } else {
                        None
                    }
                })
                .collect();

            for effect_id in effects_to_remove {
                self.effects.remove(&effect_id);
            }

            // Remove the mixer from tracking maps
            self.mixers.remove(&mixer_id);
            Ok(())
        }
    }

    /// Remove all sub-mixers from the given mixer.
    /// Use `None` as mixer_id to remove all sub-mixers from the main mixer.
    pub fn remove_all_mixers<M: Into<Option<MixerId>>>(
        &mut self,
        mixer_id: M,
    ) -> Result<(), Error> {
        let mixer_id = mixer_id.into().unwrap_or(Self::MAIN_MIXER_ID);

        // Find all sub-mixers that have this mixer as their parent
        let sub_mixers_to_remove: Vec<MixerId> = self.sub_mixers_of(mixer_id);

        // Remove each sub-mixer
        for sub_mixer_id in sub_mixers_to_remove {
            self.remove_mixer(sub_mixer_id)?;
        }

        Ok(())
    }

    /// Add an effect to the given mixer's output.
    /// Use `None` as mixer_id to add the effect to the main mixer.
    ///
    /// Note that boxed `dyn Effect` can be passed here as well as there's a effect impl
    /// defined for `Box<dyn Effect>` in the Effect trait definition.
    pub fn add_effect<E: Effect, M: Into<Option<MixerId>>>(
        &mut self,
        effect: E,
        mixer_id: M,
    ) -> Result<EffectHandle, Error> {
        let mixer_id = mixer_id.into().unwrap_or(Self::MAIN_MIXER_ID);
        let mixer_event_queue = self.mixer_event_queue(mixer_id)?;

        let channel_count = self.output_channel_count();
        // The effect's parent mixer uses a temp buffer of size:
        let max_frames = MixedSource::MAX_MIX_BUFFER_SAMPLES / channel_count;

        let mut effect = effect.into_box();
        let effect_name = effect.name();
        effect.initialize(self.output_sample_rate(), channel_count, max_frames)?;

        // Wrap into a processor
        let effect_processor = Owned::new(&self.collector_handle, EffectProcessor::new(effect));

        let effect_id = Self::unique_effect_id();
        if mixer_event_queue
            .push(MixerMessage::AddEffect {
                effect_id,
                effect_processor,
            })
            .is_err()
        {
            Err(Self::mixer_event_queue_error("add_effect"))
        } else {
            self.effects.insert(
                effect_id,
                PlayerEffectInfo {
                    mixer_id,
                    effect_name,
                },
            );

            // Create and return handle
            Ok(EffectHandle::new(
                effect_id,
                mixer_id,
                effect_name,
                mixer_event_queue,
                self.collector_handle.clone(),
            ))
        }
    }

    /// Move an effect within the given mixer's effect list to reorder the processing chain.
    pub fn move_effect<M: Into<Option<MixerId>>>(
        &mut self,
        movement: EffectMovement,
        effect_id: EffectId,
        mixer_id: M,
    ) -> Result<(), Error> {
        let mixer_id = mixer_id.into().unwrap_or(Self::MAIN_MIXER_ID);

        // Verify the effect exists and belongs to the specified mixer
        let effect_mixer_id = self.effect_parent_mixer_id(effect_id)?;

        if effect_mixer_id != mixer_id {
            return Err(Error::ParameterError(format!(
                "Effect {} does not belong to mixer {}",
                effect_id, mixer_id
            )));
        }

        let mixer_event_queue = self.mixer_event_queue(mixer_id)?;

        // Send the move message to the mixer
        if mixer_event_queue
            .push(MixerMessage::MoveEffect {
                effect_id,
                movement,
            })
            .is_err()
        {
            Err(Self::mixer_event_queue_error("move_effect"))
        } else {
            Ok(())
        }
    }

    /// Remove an effect from the given mixer.
    pub fn remove_effect(&mut self, effect_id: EffectId) -> Result<(), Error> {
        // Send the remove message
        if self
            .effect_mixer_event_queue(effect_id)?
            .push(MixerMessage::RemoveEffect { effect_id })
            .is_err()
        {
            Err(Self::mixer_event_queue_error("remove_effect"))
        } else {
            // Remove from tracking map
            self.effects.remove(&effect_id);
            Ok(())
        }
    }

    /// Remove all effects from the given mixer.
    /// Use `None` as mixer_id to remove all effects from the main mixer.
    pub fn remove_all_effects<M: Into<Option<MixerId>>>(
        &mut self,
        mixer_id: M,
    ) -> Result<(), Error> {
        let mixer_id = mixer_id.into().unwrap_or(Self::MAIN_MIXER_ID);

        // Find all effects belonging to this mixer
        let effects_to_remove = self.effects_of(mixer_id);

        // Remove each effect
        for effect_id in effects_to_remove {
            self.remove_effect(effect_id)?;
        }

        Ok(())
    }

    /// Immediately stop all playing and possibly scheduled sources.
    pub fn stop_all_sources(&mut self) -> Result<(), Error> {
        // Collect IDs of transient sources to stop (avoids holding iterator across modifications)
        let transient_source_ids: Vec<PlaybackId> = self
            .playing_sources
            .iter()
            .filter_map(|entry| {
                if entry.value().is_transient {
                    Some(*entry.key())
                } else {
                    None
                }
            })
            .collect();

        // Stop all transient sources
        for playback_id in transient_source_ids {
            if let Some((_, source)) = self.playing_sources.remove(&playback_id) {
                let _ = source.playback_message_queue.send_stop();
            }
        }

        // remove all upcoming, scheduled sources in all mixers too (force push stop events!)
        for entry in self.mixers.iter() {
            if entry
                .value()
                .event_queue
                .force_push(MixerMessage::RemoveAllPendingEvents)
                .is_some()
            {
                log::warn!("Mixer's event queue is full.");
                log::warn!("Increase the mixer event queue to prevent this from happening...");
            }
        }
        Ok(())
    }

    fn send_transport_change(&self) {
        let transport = self.transport;
        for entry in self.mixers.iter() {
            if entry
                .value()
                .event_queue
                .push(MixerMessage::SetTransport { transport })
                .is_err()
            {
                log::warn!("Mixer's event queue is full. Failed to send SetTransport event.");
            }
        }
    }

    fn add_or_play_generator<G: Generator + 'static, T: Into<Option<u64>>>(
        &mut self,
        generator: G,
        is_transient: bool,
        mixer_id: MixerId,
        start_time: T,
    ) -> Result<GeneratorPlaybackHandle, Error> {
        // validate and get options
        let playback_options = *generator.playback_options();
        playback_options.validate()?;
        // validate and get target mixer
        let mixer_event_queue = self.mixer_event_queue(mixer_id)?;
        // set generator's transient flag
        let mut generator = generator;
        generator.set_is_transient(is_transient);
        // redirect source's playback status channel to us
        generator.set_playback_status_sender(Some(self.playback_status_sender.clone()));
        // get source in playback id and message channel
        let playback_id = generator.playback_id();
        let playback_message_queue = generator.playback_message_queue();
        let source_name = format!("Generator '{}'", generator.generator_name());
        // convert generator to mixer's rate and channel layout
        let converted_source = ConvertedSource::new(
            generator,
            self.output_channel_count(),
            self.output_sample_rate(),
            ResamplingQuality::Default,
        );
        // apply volume options
        let amplified_source = AmplifiedSource::new(converted_source, playback_options.volume);
        let volume_message_queue = amplified_source.message_queue();
        // apply panning options
        let panned_source = PannedSource::new(amplified_source, playback_options.panning);
        let panning_message_queue = panned_source.message_queue();
        // apply measure options
        let measure_interval = if playback_options.measure_cpu_load {
            self.config.measuring_interval
        } else {
            None
        };
        let measured_source = MeasuredSource::new(panned_source, measure_interval);
        let measurement_state = measured_source.state();
        // add to playing sources
        let is_playing = Arc::new(AtomicBool::new(true));
        let playback_message_queue = PlaybackMessageQueue::Generator {
            playback: playback_message_queue,
            volume: volume_message_queue,
            panning: panning_message_queue,
        };
        self.playing_sources.insert(
            playback_id,
            PlayingSource {
                is_playing: Arc::clone(&is_playing),
                is_transient,
                playback_message_queue: playback_message_queue.clone(),
                mixer_id,
                source_name,
            },
        );
        // send the source to the mixer
        let source = Owned::new(&self.collector_handle, measured_source.into_box());
        let sample_time = start_time.into().unwrap_or(0);
        if mixer_event_queue
            .push(MixerMessage::AddSource {
                is_transient,
                playback_id,
                playback_message_queue: playback_message_queue.clone(),
                source,
                sample_time,
            })
            .is_err()
        {
            self.playing_sources.remove(&playback_id);
            Err(Self::mixer_event_queue_error("play_generator"))
        } else {
            // Create and return handle
            Ok(GeneratorPlaybackHandle::new(
                is_playing,
                playback_id,
                playback_message_queue,
                mixer_event_queue,
                self.collector_handle.clone(),
                measurement_state,
            ))
        }
    }

    fn handle_playback_events(
        playback_sender: Option<SyncSender<PlaybackStatusEvent>>,
        playing_sources: Arc<DashMap<PlaybackId, PlayingSource>>,
        running: Arc<AtomicBool>,
    ) -> (SyncSender<PlaybackStatusEvent>, thread::JoinHandle<()>) {
        // use a relatively big bounded channel for playback status tracking
        const DEFAULT_PLAYBACK_EVENTS_CAPACITY: usize = 2048;
        let (playback_sender_proxy, playback_receiver_proxy) =
            sync_channel(DEFAULT_PLAYBACK_EVENTS_CAPACITY);

        let handle = std::thread::Builder::new()
            .name("audio_player_messages".to_string())
            .spawn(move || {
                while running.load(atomic::Ordering::Acquire) {
                    match playback_receiver_proxy.recv_timeout(Duration::from_millis(100)) {
                        Ok(event) => {
                            if let PlaybackStatusEvent::Stopped { id, .. } = event {
                                playing_sources.remove(&id);
                            }
                            if let Some(sender) = &playback_sender {
                                // NB: send and not try_send: block until sender queue is free
                                if let Err(err) = sender.send(event) {
                                    log::warn!("Failed to send file status message: {err}");
                                }
                            }
                        }
                        Err(RecvTimeoutError::Timeout) => {
                            // Check if we're still running
                            continue;
                        }
                        Err(RecvTimeoutError::Disconnected) => {
                            // Stop
                            break;
                        }
                    };
                }
                log::info!("Playback event loop stopped");
            })
            .expect("Failed to spawn audio message thread");

        (playback_sender_proxy, handle)
    }

    fn handle_drop_collects(
        mut collector: Collector,
        running: Arc<AtomicBool>,
    ) -> thread::JoinHandle<()> {
        std::thread::Builder::new()
            .name("audio_player_drops".to_string())
            .spawn(move || {
                while running.load(atomic::Ordering::Acquire) {
                    collector.collect();
                    thread::sleep(Duration::from_millis(100));
                }
                collector.collect();
                if collector.try_cleanup().is_err() {
                    log::warn!("Failed to cleanup collector. Some handes will be leaked...");
                }
                log::info!("Audio collector loop stopped");
            })
            .expect("Failed to spawn audio message thread")
    }

    fn mixer_event_queue(&self, mixer_id: MixerId) -> Result<Arc<ArrayQueue<MixerMessage>>, Error> {
        Ok(self
            .mixers
            .get(&mixer_id)
            .ok_or(Error::MixerNotFoundError(mixer_id))?
            .event_queue
            .clone())
    }

    fn mixer_event_queue_error(event_name: &str) -> Error {
        log::warn!("Mixer's event queue is full. Failed to send a {event_name} event.");
        log::warn!("Increase the mixer event queue to prevent this from happening...");

        Error::SendError("Mixer queue is full".to_string())
    }

    fn effect_mixer_event_queue(
        &self,
        effect_id: EffectId,
    ) -> Result<Arc<ArrayQueue<MixerMessage>>, Error> {
        let effect_info = *self
            .effects
            .get(&effect_id)
            .ok_or(Error::EffectNotFoundError(effect_id))?
            .value();
        self.mixer_event_queue(effect_info.mixer_id)
    }

    fn mixer_parent_id(&self, mixer_id: MixerId) -> Result<MixerId, Error> {
        self.mixers
            .get(&mixer_id)
            .map(|entry| entry.value().parent_id)
            .ok_or(Error::MixerNotFoundError(mixer_id))
    }

    fn sub_mixers_of(&self, mixer_id: MixerId) -> Vec<MixerId> {
        self.mixers
            .iter()
            .filter_map(|entry| {
                let (child_id, info) = (entry.key(), entry.value());
                if info.parent_id == mixer_id && *child_id != Player::MAIN_MIXER_ID {
                    Some(*child_id)
                } else {
                    None
                }
            })
            .collect()
    }

    fn effect_parent_mixer_id(&self, effect_id: EffectId) -> Result<MixerId, Error> {
        self.effects
            .get(&effect_id)
            .map(|entry| entry.value().mixer_id)
            .ok_or(Error::EffectNotFoundError(effect_id))
    }

    fn effects_of(&self, mixer_id: MixerId) -> Vec<EffectId> {
        self.effects
            .iter()
            .filter_map(|entry| {
                let (effect_id, effect_info) = (entry.key(), entry.value());
                if effect_info.mixer_id == mixer_id {
                    Some(*effect_id)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
    }

    fn unique_id() -> usize {
        static ID_COUNTER: AtomicUsize = AtomicUsize::new(1);
        ID_COUNTER.fetch_add(1, atomic::Ordering::Relaxed)
    }

    fn unique_mixer_id() -> MixerId {
        Self::unique_id()
    }

    fn unique_effect_id() -> EffectId {
        Self::unique_id()
    }

    fn unique_sequencer_id() -> SequencerId {
        Self::unique_id()
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        // Replace mixer source in output with an empty source to drop it
        log::info!("Releasing player's main mixer...");
        self.output_device.stop();

        // Wait for the main mixer to be fully dropped
        let mut waited_ms = 0_usize;
        while !self.main_mixer_dropped.load(atomic::Ordering::Acquire) {
            thread::sleep(Duration::from_millis(100));
            waited_ms += 100;
            if waited_ms >= 5000 {
                log::warn!("Timed out waiting for player's main mixer to drop");
                break;
            }
        }

        // Stop playback status thread
        log::info!("Stopping player's playback status thread...");
        self.playback_status_running
            .store(false, atomic::Ordering::Release);
        if let Some(handle) = self.playback_status_thread.take() {
            let _ = handle.join();
        }

        // Stop collector thread (drop handle, collect all remaining objects)
        log::info!("Stopping player's collector thread...");
        self.collector_handle = Collector::new().handle();
        self.collector_running
            .store(false, atomic::Ordering::Release);
        if let Some(handle) = self.collector_thread.take() {
            let _ = handle.join();
        }

        // Close/pause the stream, if supported by the output
        log::info!("Closing outout device...");
        self.output_device.close();
    }
}

impl fmt::Display for Player {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.display_mixer(f, Self::MAIN_MIXER_ID, 0)
    }
}

impl Player {
    fn display_mixer(
        &self,
        f: &mut fmt::Formatter<'_>,
        mixer_id: MixerId,
        indent_level: usize,
    ) -> fmt::Result {
        let indent = "  ".repeat(indent_level);
        let child_indent = "  ".repeat(indent_level + 1);

        // Mixer name
        if mixer_id == Self::MAIN_MIXER_ID {
            writeln!(f, "{}- Main Mixer (ID: {})", indent, mixer_id)?;
        } else {
            writeln!(f, "{}- Sub-Mixer (ID: {})", indent, mixer_id)?;
        }

        // Sub-mixers
        let mut sub_mixers = self.sub_mixers_of(mixer_id);
        sub_mixers.sort();

        for sub_mixer_id in sub_mixers {
            self.display_mixer(f, sub_mixer_id, indent_level + 1)?;
        }

        // Sources
        let sources_on_mixer: Vec<_> = self
            .playing_sources
            .iter()
            .filter(|entry| entry.value().mixer_id == mixer_id)
            .collect();

        if !sources_on_mixer.is_empty() {
            writeln!(f, "{}> Sources:", child_indent)?;
            let item_indent = "  ".repeat(indent_level + 2);

            let mut grouped_sources: HashMap<String, Vec<PlaybackId>> = HashMap::new();
            for source_entry in sources_on_mixer {
                let source_id = *source_entry.key();
                let source_info = source_entry.value();
                grouped_sources
                    .entry(source_info.source_name.clone())
                    .or_default()
                    .push(source_id);
            }

            let mut sorted_grouped_sources: Vec<_> = grouped_sources.into_iter().collect();
            sorted_grouped_sources.sort_by(|(name_a, _), (name_b, _)| name_a.cmp(name_b));

            for (source_name, mut ids) in sorted_grouped_sources {
                ids.sort();
                let ids_str = ids
                    .iter()
                    .map(|id| id.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                writeln!(f, "{}- {} (ID: {})", item_indent, source_name, ids_str)?;
            }
        }

        // Effects
        let mut effects_on_mixer: Vec<_> = self
            .effects
            .iter()
            .filter(|entry| entry.value().mixer_id == mixer_id)
            .collect();
        effects_on_mixer.sort_by_key(|e| *e.key());

        if !effects_on_mixer.is_empty() {
            writeln!(f, "{}^ Effects:", child_indent)?;
            let item_indent = "  ".repeat(indent_level + 2);
            for effect_entry in effects_on_mixer {
                let effect_id = effect_entry.key();
                let effect_info = effect_entry.value();
                writeln!(
                    f,
                    "{}- {} (ID: {})",
                    item_indent, effect_info.effect_name, effect_id
                )?;
            }
        }

        Ok(())
    }
}
