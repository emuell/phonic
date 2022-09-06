#[cfg(feature = "dasp")]
pub mod dasp;

use super::AudioSource;
use crossbeam_channel::Sender;

// -------------------------------------------------------------------------------------------------

/// A unique ID for a newly created SynthSources
pub type SynthId = usize;

// -------------------------------------------------------------------------------------------------

/// Events send back from synth to user
pub enum SynthPlaybackStatusMsg {
    Exhausted { synth_id: SynthId },
}

// -------------------------------------------------------------------------------------------------

/// Events to control playback of a DaspSource
pub enum SynthPlaybackMsg {
    Stop,
}

// -------------------------------------------------------------------------------------------------

pub trait SynthSource: AudioSource + Sized {
    /// Channel to control playback
    fn sender(&self) -> Sender<SynthPlaybackMsg>;

    /// The unique synth ID, can be used to identify files in SynthPlaybackStatusMsg events
    fn synth_id(&self) -> SynthId;
}
