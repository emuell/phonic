use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use crossbeam_queue::ArrayQueue;
use dashmap::DashMap;

use crate::{
    error::Error,
    player::{MixerId, PlayerSequencerInfo, SequencerId},
    source::mixed::MixerMessage,
};

// -------------------------------------------------------------------------------------------------

/// A handle to a sequencer registered with the player via
/// [`play_sequencer`](crate::Player::play_sequencer).
///
/// Handles are `Send` and `Sync` so they can be checked from any thread.
#[derive(Clone)]
pub struct SequencerHandle {
    sequencer_id: SequencerId,
    mixer_id: MixerId,
    is_playing: Arc<AtomicBool>,
    mixer_event_queue: Arc<ArrayQueue<MixerMessage>>,
    sequencers: Arc<DashMap<SequencerId, PlayerSequencerInfo>>,
}

impl SequencerHandle {
    pub(crate) fn new(
        is_playing: Arc<AtomicBool>,
        sequencer_id: SequencerId,
        mixer_id: MixerId,
        sequencers: Arc<DashMap<SequencerId, PlayerSequencerInfo>>,
        mixer_event_queue: Arc<ArrayQueue<MixerMessage>>,
    ) -> Self {
        Self {
            is_playing,
            sequencer_id,
            mixer_id,
            sequencers,
            mixer_event_queue,
        }
    }

    /// The unique ID of this sequencer.
    pub fn id(&self) -> SequencerId {
        self.sequencer_id
    }

    /// The ID of the mixer that drives this sequencer.
    pub fn mixer_id(&self) -> MixerId {
        self.mixer_id
    }

    /// Returns `true` while the sequencer is still active (not yet finished or stopped).
    pub fn is_playing(&self) -> bool {
        self.is_playing.load(Ordering::Relaxed)
    }

    /// Reset the sequencer, restarting playback from the given sample time.
    ///
    /// Returns `Err` if the sequencer has already been stopped or exhausted.
    pub fn reset(&self, sample_time: u64) -> Result<(), Error> {
        if !self.is_playing() {
            return Err(Error::SourceNotPlaying);
        }
        let sequencer_id = self.sequencer_id;
        if self
            .mixer_event_queue
            .force_push(MixerMessage::ResetSequencer {
                sequencer_id,
                sample_time,
            })
            .is_some()
        {
            log::warn!("Mixer's event queue is full. Sequencer reset may be delayed.");
        }
        Ok(())
    }

    /// Stop the sequencer and remove it from the mixer.
    ///
    /// Pass `None` to stop immediately, or `Some(sample_time)` to schedule the stop at a
    /// specific audio frame. For scheduled stops the handle's `is_playing` flag is cleared
    /// by the mixer once the stop time is reached.
    ///
    /// Returns `Err` if the sequencer has already been stopped or exhausted.
    pub fn stop(&self, sample_time: impl Into<Option<u64>>) -> Result<(), Error> {
        if !self.is_playing() {
            return Err(Error::SourceNotPlaying);
        }
        let sample_time = sample_time.into();
        let sequencer_id = self.sequencer_id;
        // schedule or remove the sequencer immedeately
        if self
            .mixer_event_queue
            .force_push(MixerMessage::StopSequencer {
                sequencer_id,
                sample_time,
            })
            .is_some()
        {
            log::warn!("Mixer's event queue is full. Sequencer stop may be delayed.");
        }
        // immediately remove sequence from the player (can't schedule that)
        self.sequencers.remove(&sequencer_id);
        Ok(())
    }
}
