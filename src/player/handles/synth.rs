use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use crossbeam_queue::ArrayQueue;
use dashmap::DashMap;

use crate::{
    error::Error,
    player::{MixerId, PlaybackId},
    source::{
        amplified::AmplifiedSourceMessage, mixed::MixerMessage, panned::PannedSourceMessage,
        playback::PlaybackMessageQueue,
    },
    SynthPlaybackMessage,
};

// -------------------------------------------------------------------------------------------------

/// Change runtime playback properties of a playing [`SynthSource`](crate::SynthSource) and test
/// if a source is still playing.
///
/// Handles are `Send` and `Sync` so they can be sent across threads.
///
/// To track detailed playback status use a [`PlaybackStatusEvent`](crate::PlaybackStatusEvent)
/// [`sync_channel`](std::sync::mpsc::sync_channel) in the [`Player`](crate::Player).
#[derive(Clone)]
pub struct SynthPlaybackHandle {
    is_playing: Arc<AtomicBool>,
    playback_id: PlaybackId,
    mixer_id: MixerId,
    playback_message_queue: PlaybackMessageQueue,
    mixer_event_queues: Arc<DashMap<MixerId, Arc<ArrayQueue<MixerMessage>>>>,
}

impl SynthPlaybackHandle {
    pub(crate) fn new(
        is_playing: Arc<AtomicBool>,
        playback_id: PlaybackId,
        mixer_id: MixerId,
        playback_message_queue: PlaybackMessageQueue,
        mixer_event_queues: Arc<DashMap<MixerId, Arc<ArrayQueue<MixerMessage>>>>,
    ) -> Self {
        Self {
            is_playing,
            playback_id,
            mixer_id,
            playback_message_queue,
            mixer_event_queues,
        }
    }

    /// Get the playback ID of this source.
    pub fn id(&self) -> PlaybackId {
        self.playback_id
    }

    /// Check if this source is still playing.
    pub fn is_playing(&self) -> bool {
        self.is_playing.load(Ordering::Relaxed)
    }

    /// Stop this source at the given sample time or immediately.
    pub fn stop<T: Into<Option<u64>>>(&self, stop_time: T) -> Result<(), Error> {
        if !self.is_playing() {
            return Err(Error::SourceNotPlaying);
        }

        let stop_time = stop_time.into();
        if let Some(sample_time) = stop_time {
            // Schedule stop with mixer. Force push stop commands to avoid hanging notes...
            if self
                .mixer_event_queue()?
                .force_push(MixerMessage::StopSource {
                    playback_id: self.playback_id,
                    sample_time,
                })
                .is_some()
            {
                log::warn!("Mixer's event queue is full.");
                log::warn!("Increase the mixer event queue to prevent this from happening...");
            }
        } else {
            // Stop immediately
            if let PlaybackMessageQueue::Synth { playback, .. } = &self.playback_message_queue {
                if playback.force_push(SynthPlaybackMessage::Stop).is_some() {
                    return Err(Self::synth_playback_queue_error("stop"));
                }
            } else {
                unreachable!("Expecting a synth message queue for a synth playback handle");
            }
        }

        Ok(())
    }

    /// Set source's volume at a given sample time in future or immediately.
    pub fn set_volume<T: Into<Option<u64>>>(
        &self,
        volume: f32,
        sample_time: T,
    ) -> Result<(), Error> {
        if !self.is_playing() {
            return Err(Error::SourceNotPlaying);
        }

        let sample_time = sample_time.into();
        if let Some(sample_time) = sample_time {
            // Schedule with mixer
            if self
                .mixer_event_queue()?
                .push(MixerMessage::SetSourceVolume {
                    playback_id: self.playback_id,
                    volume,
                    sample_time,
                })
                .is_err()
            {
                return Err(Self::mixer_event_queue_error("set_volume"));
            }
        } else {
            // Apply immediately
            if self
                .playback_message_queue
                .volume()
                .force_push(AmplifiedSourceMessage::SetVolume(volume))
                .is_some()
            {
                // expected: simply overwrite previous values, if any
            }
        }

        Ok(())
    }

    /// Set source's panning at a given sample time in future or immediately.
    pub fn set_panning<T: Into<Option<u64>>>(
        &self,
        panning: f32,
        sample_time: T,
    ) -> Result<(), Error> {
        if !self.is_playing() {
            return Err(Error::SourceNotPlaying);
        }

        let sample_time = sample_time.into();
        if let Some(sample_time) = sample_time {
            // Schedule with mixer
            if self
                .mixer_event_queue()?
                .push(MixerMessage::SetSourcePanning {
                    playback_id: self.playback_id,
                    panning,
                    sample_time,
                })
                .is_err()
            {
                return Err(Self::mixer_event_queue_error("set_panning"));
            }
        } else {
            // Apply immediately
            if self
                .playback_message_queue
                .panning()
                .force_push(PannedSourceMessage::SetPanning(panning))
                .is_some()
            {
                // expected: simply overwrite previous values, if any
            }
        }

        Ok(())
    }

    fn mixer_event_queue(&self) -> Result<Arc<ArrayQueue<MixerMessage>>, Error> {
        Ok(Arc::clone(
            self.mixer_event_queues
                .get(&self.mixer_id)
                .ok_or(Error::MixerNotFoundError(self.mixer_id))?
                .value(),
        ))
    }

    fn mixer_event_queue_error(event_name: &str) -> Error {
        log::warn!("Mixer's event queue is full. Failed to send a {event_name} event.");
        log::warn!("Increase the mixer event queue to prevent this from happening...");
        Error::SendError("Mixer queue is full".to_string())
    }

    fn synth_playback_queue_error(event_name: &str) -> Error {
        log::warn!("Synth playback event queue is full. Failed to send a {event_name} event.");
        Error::SendError("Synth playback queue is full".to_string())
    }
}
