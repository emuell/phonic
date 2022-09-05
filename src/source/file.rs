pub mod preloaded;
pub mod streamed;

use crossbeam_channel::Sender;
use std::time::Duration;

use super::AudioSource;
use crate::error::Error;

// -------------------------------------------------------------------------------------------------

/// A uniquie ID for a newly created DecodedFileSources
pub type FileId = usize;

// -------------------------------------------------------------------------------------------------

/// Events send back from decoder to user
pub enum FilePlaybackStatusMsg {
    Position {
        file_id: FileId,
        file_path: String,
        position: Duration,
    },
    EndOfFile {
        file_id: FileId,
        file_path: String,
    },
}

// -------------------------------------------------------------------------------------------------

/// Events to control playback of a FileSource
pub enum FilePlaybackMsg {
    Seek(Duration),
    Read,
    Stop,
}

// -------------------------------------------------------------------------------------------------

/// A source which decodes an audio file
pub trait FileSource: AudioSource + Sized {
    /// Create a new file source with an optional FilePlaybackStatusMsg channel sender
    /// to retrieve playback status events, while the source ius running
    fn new(
        file_path: String,
        status_sender: Option<Sender<FilePlaybackStatusMsg>>,
    ) -> Result<Self, Error>;

    /// Channel to control playback
    fn sender(&self) -> Sender<FilePlaybackMsg>;

    /// The unique file ID, can be used to identify files in FilePlaybackStatusMsg events
    fn file_id(&self) -> FileId;

    /// Total number of sample frames in the decoded file: may not be known before playback finished
    fn total_frames(&self) -> Option<u64>;
    /// Current playback pos in frames
    fn current_frame_position(&self) -> u64;

    /// True the source played through the entire file, else false
    fn end_of_track(&self) -> bool;
}
