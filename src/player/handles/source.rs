use super::{FilePlaybackHandle, GeneratorPlaybackHandle, SynthPlaybackHandle};

use crate::{error::Error, source::measured::CpuLoad};

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

    /// Get the CPU load data for this source.
    ///
    /// Returns `None` if CPU measurement was not enabled for this source, or if the
    /// measurement is not available at this time.
    pub fn cpu_load(&self) -> Option<CpuLoad> {
        match self {
            SourcePlaybackHandle::File(handle) => handle.cpu_load(),
            SourcePlaybackHandle::Synth(handle) => handle.cpu_load(),
            SourcePlaybackHandle::Generator(handle) => handle.cpu_load(),
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
