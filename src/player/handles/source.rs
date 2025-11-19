use super::{FilePlaybackHandle, GeneratorPlaybackHandle, SynthPlaybackHandle};
use crate::Error;

// -------------------------------------------------------------------------------------------------

/// A unified [`FilePlaybackHandle`] and [`SynthPlaybackHandle`].
#[derive(Clone)]
pub enum SourcePlaybackHandle {
    File(FilePlaybackHandle),
    Synth(SynthPlaybackHandle),
    Generator(GeneratorPlaybackHandle),
}

impl SourcePlaybackHandle {
    /// Check if this source is still playing.
    pub fn is_playing(&self) -> bool {
        match self {
            SourcePlaybackHandle::File(handle) => handle.is_playing(),
            SourcePlaybackHandle::Synth(handle) => handle.is_playing(),
            SourcePlaybackHandle::Generator(handle) => handle.is_playing(),
        }
    }

    pub fn stop<T: Into<Option<u64>>>(&self, stop_time: T) -> Result<(), Error> {
        match self {
            SourcePlaybackHandle::File(handle) => handle.stop(stop_time),
            SourcePlaybackHandle::Synth(handle) => handle.stop(stop_time),
            SourcePlaybackHandle::Generator(handle) => handle.stop(stop_time),
        }
    }
}
