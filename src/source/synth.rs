#[cfg(feature = "dasp")]
pub mod dasp;

use crossbeam_channel::Sender;

use super::{playback::PlaybackId, AudioSource};

// -------------------------------------------------------------------------------------------------

/// Options to control playback of a FileSource
#[derive(Clone, Copy)]
pub struct SynthPlaybackOptions {
    /// By default 1.0f32. Customize to lower or raise the volume of the file.
    pub volume: f32,
}

impl Default for SynthPlaybackOptions {
    fn default() -> Self {
        Self { volume: 1.0f32 }
    }
}

impl SynthPlaybackOptions {
    pub fn with_volume(mut self, volume: f32) -> Self {
        self.volume = volume;
        self
    }
}

// -------------------------------------------------------------------------------------------------

/// Events to control playback of a synth source
pub enum SynthPlaybackMessage {
    /// Stop the synth source
    Stop,
}

// -------------------------------------------------------------------------------------------------

pub trait SynthSource: AudioSource + Sized {
    /// Channel sender to control this sources's playback
    fn playback_message_sender(&self) -> Sender<SynthPlaybackMessage>;
    /// A unique ID, which can be used to identify sources in `PlaybackStatusEvent`s
    fn playback_id(&self) -> PlaybackId;
}
