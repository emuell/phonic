#[cfg(feature = "dasp")]
pub mod dasp;

use crossbeam_channel::Sender;

use super::{playback::PlaybackId, AudioSource};

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
