use std::{path::Path, sync::mpsc::SyncSender, sync::Arc, time::Duration};

use crossbeam_queue::ArrayQueue;

use crate::{
    player::{FilePlaybackHandle, MixerId, PlaybackId},
    source::{
        status::{PlaybackStatusContext, PlaybackStatusEvent},
        Source,
    },
    utils::db_to_linear,
    Error, Player, ResamplingQuality,
};

use self::{preloaded::PreloadedFileSource, streamed::StreamedFileSource};

// -------------------------------------------------------------------------------------------------

pub mod common;
pub mod preloaded;
pub mod streamed;

pub(crate) mod decoder;

// -------------------------------------------------------------------------------------------------

/// Options to control playback properties of a [`FileSource`].
#[derive(Clone, Copy)]
pub struct FilePlaybackOptions {
    /// By default false: when true, the file will be decoded and streamed on the fly.
    /// This should be enabled for very long files only, especially when a lot of files
    /// are going to be played at once.
    pub stream: bool,

    /// By default 1.0f32. Customize to lower or raise the volume of the file.
    pub volume: f32,

    /// By default 0.0f32. Set in range -1.0..=1.0 to adjust panning position.
    pub panning: f32,

    /// By default 1.0f64. Customize to pitch the playback speed up or down.
    /// See also `resampling_quality` property.
    pub speed: f64,

    /// By default `None`: When `Some(> 0)`, this is the number of times the file should
    /// be looped. When supported by the file reader, embedded file loop points are used,
    /// else the entire sample range is repeated.
    /// When loop points are present and repeat is `None`, repeating is enabled by default,
    /// except when explicitly disabled via `Some(0)`.
    pub repeat: Option<usize>,

    /// By default None: when set, the source should start playing at the given
    /// sample frame time in the audio output stream.
    pub start_time: Option<u64>,

    /// By default None: when set, the source's volume will fade in with the given
    /// amount when starting to play.
    pub fade_in_duration: Option<Duration>,
    /// By default 5ms: volume fade out duration, applied when the the source gets
    /// stopped before it finished playing.
    pub fade_out_duration: Option<Duration>,

    /// By default ResamplingQuality::Default: Quality mode of a applied resampler,
    /// either when the source is getting played back on a stream with a sample rate
    /// which does not match the file's sample rate or when pitching the playback up
    /// or down.
    pub resampling_quality: ResamplingQuality,

    /// By default None, which means play on the main mixer. When set to some specific id,
    /// the source will be played on the given mixer instead of the default one.
    pub target_mixer: Option<MixerId>,

    /// By default false. When true, measure the CPU load of the file source.
    /// CPU load can then be accessed via the source's playback handle.
    pub measure_cpu_load: bool,

    /// Wallclock time rate of playback pos events, emitted via PlaybackStatusEvent
    /// in the player. By default one second to avoid unnecessary overhead.
    /// Set to e.g. Duration::from_secf32(1.0/30.0) to trigger events 30 times per second.
    pub playback_pos_emit_rate: Option<Duration>,
}

impl Default for FilePlaybackOptions {
    fn default() -> Self {
        Self {
            stream: false,
            volume: 1.0,
            panning: 0.0,
            speed: 1.0,
            repeat: None,
            start_time: None,
            fade_in_duration: None,
            fade_out_duration: Some(Duration::from_millis(50)),
            resampling_quality: ResamplingQuality::Default,
            target_mixer: None,
            measure_cpu_load: false,
            playback_pos_emit_rate: Some(Duration::from_secs(1)),
        }
    }
}

impl FilePlaybackOptions {
    pub fn preloaded(mut self) -> Self {
        self.stream = false;
        self
    }
    pub fn streamed(mut self) -> Self {
        self.stream = true;
        self
    }

    pub fn volume(mut self, volume: f32) -> Self {
        self.volume = volume;
        self
    }
    pub fn volume_db(mut self, volume_db: f32) -> Self {
        self.volume = db_to_linear(volume_db);
        self
    }

    pub fn panning(mut self, panning: f32) -> Self {
        self.panning = panning;
        self
    }

    pub fn fade_in(mut self, duration: Duration) -> Self {
        self.fade_in_duration = Some(duration);
        self
    }
    pub fn fade_out(mut self, duration: Duration) -> Self {
        self.fade_out_duration = Some(duration);
        self
    }

    pub fn speed(mut self, speed: f64) -> Self {
        self.speed = speed;
        self
    }

    pub fn repeat(mut self, count: usize) -> Self {
        self.repeat = Some(count);
        self
    }
    pub fn repeat_forever(mut self) -> Self {
        self.repeat = Some(usize::MAX);
        self
    }

    pub fn start_at_time(mut self, sample_time: u64) -> Self {
        self.start_time = Some(sample_time);
        self
    }

    pub fn playback_pos_emit_rate(mut self, duration: std::time::Duration) -> Self {
        self.playback_pos_emit_rate = Some(duration);
        self
    }

    pub fn resampling_quality(mut self, quality: ResamplingQuality) -> Self {
        self.resampling_quality = quality;
        self
    }

    pub fn target_mixer(mut self, mixer_id: MixerId) -> Self {
        self.target_mixer = Some(mixer_id);
        self
    }

    /// Set whether to measure the CPU load of this source.
    pub fn measure_cpu_load(mut self, measure: bool) -> Self {
        self.measure_cpu_load = measure;
        self
    }

    /// Validate all parameters. Returns Error::ParameterError on errors.
    pub fn validate(&self) -> Result<(), Error> {
        if self.volume < 0.0 || self.volume.is_nan() {
            return Err(Error::ParameterError(format!(
                "playback options 'volume' value is '{}'",
                self.volume
            )));
        }
        if !(-1.0..=1.0).contains(&self.panning) || self.panning.is_nan() {
            return Err(Error::ParameterError(format!(
                "playback options 'panning' value is '{}'",
                self.panning
            )));
        }
        if self.speed < 0.0 || self.speed.is_nan() || self.speed.is_infinite() {
            return Err(Error::ParameterError(format!(
                "playback options 'speed' value is '{}'",
                self.speed
            )));
        }
        Ok(())
    }
}

// -------------------------------------------------------------------------------------------------

/// Events to control playback of a [`FileSource`]
pub enum FilePlaybackMessage {
    /// Seek the file source to a new position.
    Seek(Duration),
    /// Update the source's playback speed with the given optional glide rate in semitones per second.
    SetSpeed(f64, Option<f32>),
    /// Stop the source gracefully, applying off actions, fade-outs and stuff.
    Stop,
    /// Stop the source by force, immediately.
    Kill,
}

// -------------------------------------------------------------------------------------------------

/// A source which decodes and plays back an audio file.
pub trait FileSource: Source {
    /// Name or path of the file for display or debugging purposes.
    fn file_name(&self) -> String;

    /// A unique ID, which can be used to identify sources in `PlaybackStatusEvent`s.
    fn playback_id(&self) -> PlaybackId;

    /// The file source's playback options
    fn playback_options(&self) -> &FilePlaybackOptions;

    /// Message queue to control file playback.
    fn playback_message_queue(&self) -> Arc<ArrayQueue<FilePlaybackMessage>>;

    /// Channel to receive playback status from the file.
    fn playback_status_sender(&self) -> Option<SyncSender<PlaybackStatusEvent>>;
    fn set_playback_status_sender(&mut self, sender: Option<SyncSender<PlaybackStatusEvent>>);

    /// Optional context passed along with the playback status.
    fn playback_status_context(&self) -> Option<PlaybackStatusContext>;
    fn set_playback_status_context(&mut self, context: Option<PlaybackStatusContext>);

    /// Total number of sample frames in the decoded file: may not be known before playback finished.
    fn total_frames(&self) -> Option<u64>;
    /// Current playback pos in frames
    fn current_frame_position(&self) -> u64;

    /// True when the source played through the entire file, else false.
    fn end_of_track(&self) -> bool;
}

// -------------------------------------------------------------------------------------------------

impl Player {
    /// Play a new file with the given file path and options. See [`FilePlaybackOptions`]
    /// for more info on which options can be applied.
    pub fn play_file<P: AsRef<Path>>(
        &mut self,
        path: P,
        options: FilePlaybackOptions,
    ) -> Result<FilePlaybackHandle, Error> {
        self.play_file_with_context(path, options, None)
    }

    /// Play a new file with the given file path, options and context.
    /// See [`FilePlaybackOptions`] for more info on which options can be applied.
    pub fn play_file_with_context<P: AsRef<Path>>(
        &mut self,
        path: P,
        options: FilePlaybackOptions,
        context: Option<PlaybackStatusContext>,
    ) -> Result<FilePlaybackHandle, Error> {
        // create a streamed or preloaded source, depending on the options and play it
        if options.stream {
            let streamed_source = StreamedFileSource::from_file(
                path,
                options,
                self.output_sample_rate(),
            )?;
            self.play_file_source_with_context(streamed_source, options.start_time, context)
        } else {
            let preloaded_source = PreloadedFileSource::from_file(
                path,
                options,
                self.output_sample_rate(),
            )?;
            self.play_file_source_with_context(preloaded_source, options.start_time, context)
        }
    }

    /// Play a new file with the give raw encoded file buffer and display path and options.
    /// Param file_path is used as path when reporting file playback status.
    /// See [`FilePlaybackOptions`] for more info on which options can be applied.
    pub fn play_file_buffer(
        &mut self,
        file_buffer: Vec<u8>,
        file_path: &str,
        options: FilePlaybackOptions,
    ) -> Result<FilePlaybackHandle, Error> {
        self.play_file_buffer_with_context(file_buffer, file_path, options, None)
    }

    /// Play a new file with the give raw encoded file buffer, display path, options and context.
    /// Param file_path is used as path when reporting file playback status.
    /// See [`FilePlaybackOptions`] for more info on which options can be applied.
    pub fn play_file_buffer_with_context(
        &mut self,
        file_buffer: Vec<u8>,
        file_path: &str,
        options: FilePlaybackOptions,
        context: Option<PlaybackStatusContext>,
    ) -> Result<FilePlaybackHandle, Error> {
        debug_assert!(!options.stream, "Streaming file buffers is not supported.");
        let preloaded_source = PreloadedFileSource::from_file_buffer(
            file_buffer,
            file_path,
            options,
            self.output_sample_rate(),
        )?;
        self.play_file_source_with_context(preloaded_source, options.start_time, context)
    }
}
