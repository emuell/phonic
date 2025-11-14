use crate::Error;

// -------------------------------------------------------------------------------------------------

mod file;
mod synth;

pub use file::FilePlaybackHandle;
pub use synth::SynthPlaybackHandle;

// -------------------------------------------------------------------------------------------------

/// A unified [`FilePlaybackHandle`] and [`SynthPlaybackHandle`].
#[derive(Clone)]
pub enum PlaybackHandle {
    File(FilePlaybackHandle),
    Synth(SynthPlaybackHandle),
}

impl PlaybackHandle {
    /// Check if this source is still playing.
    pub fn is_playing(&self) -> bool {
        match self {
            PlaybackHandle::File(handle) => handle.is_playing(),
            PlaybackHandle::Synth(handle) => handle.is_playing(),
        }
    }

    pub fn stop<T: Into<Option<u64>>>(&self, stop_time: T) -> Result<(), Error> {
        match self {
            PlaybackHandle::File(handle) => handle.stop(stop_time),
            PlaybackHandle::Synth(handle) => handle.stop(stop_time),
        }
    }
}
