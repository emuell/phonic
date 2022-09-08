pub mod preloaded;
pub mod streamed;

use crossbeam_channel::Sender;
use std::time::Duration;

use super::{
    playback::{PlaybackId, PlaybackStatusEvent},
    AudioSource,
};
use crate::error::Error;

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
}

impl Default for FilePlaybackOptions {
    fn default() -> Self {
        Self {
            stream: false,
            volume: 1.0f32,
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

    pub fn with_volume(mut self, volume: f32) -> Self {
        self.volume = volume;
        self
    }
}

// -------------------------------------------------------------------------------------------------

/// Events to control playback of a FileSource
pub enum FilePlaybackMessage {
    /// Seek the file source to a new position
    Seek(Duration),
    /// Start reading streamed sources (internally used only)
    Read,
    /// Stop the source
    Stop,
}

// -------------------------------------------------------------------------------------------------

/// A source which decodes an audio file
pub trait FileSource: AudioSource + Sized {
    /// Create a new file source with an optional FilePlaybackStatusMsg channel sender
    /// to retrieve playback status events, while the source is running
    fn new(
        file_path: &str,
        status_sender: Option<Sender<PlaybackStatusEvent>>,
        volume: f32,
    ) -> Result<Self, Error>;

    /// Channel to control file playback.
    fn playback_message_sender(&self) -> Sender<FilePlaybackMessage>;
    /// A unique ID, which can be used to identify sources in `PlaybackStatusEvent`s.
    fn playback_id(&self) -> PlaybackId;

    /// Total number of sample frames in the decoded file: may not be known before playback finished.
    fn total_frames(&self) -> Option<u64>;
    /// Current playback pos in frames
    fn current_frame_position(&self) -> u64;

    /// True when the source played through the entire file, else false.
    fn end_of_track(&self) -> bool;
}
