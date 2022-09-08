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
