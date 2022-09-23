pub mod preloaded;
pub mod streamed;

use crossbeam_channel::Sender;
use std::time::Duration;

use crate::{
    player::{AudioFilePlaybackId, AudioFilePlaybackStatusEvent},
    source::AudioSource,
    utils::db_to_linear,
    Error,
};

// -------------------------------------------------------------------------------------------------

/// Options to control playback of a FileSource
#[derive(Clone, Copy)]
pub struct FilePlaybackOptions {
    /// By default false: when true, the file will be decoded and streamed on the fly.
    /// This should be enabled for very long files only, especiall when a lot of files are
    /// going to be played at once.
    pub stream: bool,
    /// By default 1.0f32. Customize to lower or raise the volume of the file.
    pub volume: f32,
    /// By default 1.0f64. Customize to pitch the playback speed up or down.
    pub speed: f64,
    /// By default 0: when > 0 the number of times the file should be looped.
    /// Set to usize::MAX to repeat forever.
    pub repeat: usize,
    /// By default None: when set, the source should start playing at the given
    /// sample frame time in the audio output stream.
    pub start_time: Option<u64>,
}

impl Default for FilePlaybackOptions {
    fn default() -> Self {
        Self {
            stream: false,
            volume: 1.0,
            speed: 1.0,
            repeat: 0,
            start_time: None,
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

    pub fn speed(mut self, speed: f64) -> Self {
        self.speed = speed;
        self
    }

    pub fn repeat(mut self, count: usize) -> Self {
        self.repeat = count;
        self
    }
    pub fn repeat_forever(mut self) -> Self {
        self.repeat = usize::MAX;
        self
    }

    pub fn start_at_time(mut self, sample_time: u64) -> Self {
        self.start_time = Some(sample_time);
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

/// Events to control playback of a FileSource
pub enum FilePlaybackMessage {
    /// Seek the file source to a new position
    Seek(Duration),
    /// Start reading streamed sources (internally used only)
    Read,
    /// Stop the source with the given fade-out duration
    Stop(Duration),
}

// -------------------------------------------------------------------------------------------------

/// A source which decodes and plays back an audio file.
pub trait FileSource: AudioSource {
    /// Channel to control file playback.
    fn playback_message_sender(&self) -> Sender<FilePlaybackMessage>;
    /// A unique ID, which can be used to identify sources in `PlaybackStatusEvent`s.
    fn playback_id(&self) -> AudioFilePlaybackId;

    /// Total number of sample frames in the decoded file: may not be known before playback finished.
    fn total_frames(&self) -> Option<u64>;
    /// Current playback pos in frames
    fn current_frame_position(&self) -> u64;

    /// True when the source played through the entire file, else false.
    fn end_of_track(&self) -> bool;
}
