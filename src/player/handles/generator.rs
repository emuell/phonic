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
        amplified::AmplifiedSourceMessage, generator::GeneratorPlaybackMessage,
        mixed::MixerMessage, panned::PannedSourceMessage, playback::PlaybackMessageQueue,
        unique_source_id,
    },
};

// -------------------------------------------------------------------------------------------------

/// A handle to control a playing generator source.
#[derive(Clone)]
pub struct GeneratorPlaybackHandle {
    is_playing: Arc<AtomicBool>,
    playback_id: PlaybackId,
    mixer_id: MixerId,
    playback_message_queue: PlaybackMessageQueue,
    mixer_event_queues: Arc<DashMap<MixerId, Arc<ArrayQueue<MixerMessage>>>>,
}

impl GeneratorPlaybackHandle {
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
            playback_message_queue,
            mixer_id,
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
        let stop_time = stop_time.into();
        if !self.is_playing() {
            return Err(Error::SourceNotPlaying);
        }

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
            if let PlaybackMessageQueue::Generator { playback, .. } = &self.playback_message_queue {
                if playback
                    .force_push(GeneratorPlaybackMessage::Stop)
                    .is_some()
                {
                    return Err(Self::generator_message_queue_error("stop"));
                }
            } else {
                unreachable!("Expecting a generator message queue for a generator playback handle");
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
        let sample_time = sample_time.into();
        if !self.is_playing() {
            return Err(Error::SourceNotPlaying);
        }

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
                .push(AmplifiedSourceMessage::SetVolume(volume))
                .is_err()
            {
                return Err(Self::generator_message_queue_error("set_volume"));
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
        let sample_time = sample_time.into();
        if !self.is_playing() {
            return Err(Error::SourceNotPlaying);
        }

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
                .push(PannedSourceMessage::SetPanning(panning))
                .is_err()
            {
                return Err(Self::generator_message_queue_error("set_panning"));
            }
        }
        Ok(())
    }

    /// Trigger a note on event at the given sample time or immediately.
    /// Returns the note playback ID that can be used to control this specific note instance.
    pub fn note_on<T: Into<Option<u64>>>(
        &self,
        note: u8,
        volume: Option<f32>,
        panning: Option<f32>,
        sample_time: T,
    ) -> Result<PlaybackId, Error> {
        let sample_time = sample_time.into();
        if !self.is_playing() {
            return Err(Error::SourceNotPlaying);
        }

        // Generate a unique note playback ID
        let note_playback_id = unique_source_id();

        if let Some(sample_time) = sample_time {
            // Schedule with mixer
            if self
                .mixer_event_queue()?
                .push(MixerMessage::TriggerGeneratorNoteOn {
                    playback_id: self.playback_id,
                    note_playback_id,
                    note,
                    volume,
                    panning,
                    sample_time,
                })
                .is_err()
            {
                return Err(Self::mixer_event_queue_error("note_on"));
            }
        } else {
            // Apply immediately - send directly to generator
            if let PlaybackMessageQueue::Generator { playback, .. } = &self.playback_message_queue {
                if playback
                    .push(GeneratorPlaybackMessage::NoteOn {
                        note_playback_id,
                        note,
                        volume,
                        panning,
                    })
                    .is_err()
                {
                    return Err(Self::generator_message_queue_error("note_on"));
                }
            } else {
                unreachable!("Expecting a generator message queue for a generator playback handle");
            }
        }

        Ok(note_playback_id)
    }

    /// Trigger a note off event for a specific note instance at the given sample time or immediately.
    pub fn note_off<T: Into<Option<u64>>>(
        &self,
        note_playback_id: PlaybackId,
        sample_time: T,
    ) -> Result<(), Error> {
        let sample_time = sample_time.into();
        if !self.is_playing() {
            return Err(Error::SourceNotPlaying);
        }

        if let Some(sample_time) = sample_time {
            // Schedule with mixer
            if self
                .mixer_event_queue()?
                .push(MixerMessage::TriggerGeneratorNoteOff {
                    playback_id: self.playback_id,
                    note_playback_id,
                    sample_time,
                })
                .is_err()
            {
                return Err(Self::mixer_event_queue_error("note_off"));
            }
        } else {
            // Apply immediately
            if let PlaybackMessageQueue::Generator { playback, .. } = &self.playback_message_queue {
                if playback
                    .push(GeneratorPlaybackMessage::NoteOff { note_playback_id })
                    .is_err()
                {
                    return Err(Self::generator_message_queue_error("note_off"));
                }
            } else {
                unreachable!("Expecting a generator message queue for a generator playback handle");
            }
        }

        Ok(())
    }

    /// Set playback speed (pitch) for a specific note instance at the given sample time or immediately.
    pub fn set_note_speed<T: Into<Option<u64>>>(
        &self,
        note_playback_id: PlaybackId,
        speed: f64,
        glide: Option<f32>,
        sample_time: T,
    ) -> Result<(), Error> {
        let sample_time = sample_time.into();
        if !self.is_playing() {
            return Err(Error::SourceNotPlaying);
        }

        if let Some(sample_time) = sample_time {
            // Schedule with mixer
            if self
                .mixer_event_queue()?
                .push(MixerMessage::SetGeneratorNoteSpeed {
                    playback_id: self.playback_id,
                    note_playback_id,
                    speed,
                    glide,
                    sample_time,
                })
                .is_err()
            {
                return Err(Self::mixer_event_queue_error("set_note_speed"));
            }
        } else {
            // Apply immediately
            if let PlaybackMessageQueue::Generator { playback, .. } = &self.playback_message_queue {
                if playback
                    .push(GeneratorPlaybackMessage::SetSpeed {
                        note_playback_id,
                        speed,
                        glide,
                    })
                    .is_err()
                {
                    return Err(Self::generator_message_queue_error("set_note_speed"));
                }
            } else {
                unreachable!("Expecting a generator message queue for a generator playback handle");
            }
        }

        Ok(())
    }

    /// Set volume for a specific note instance at the given sample time or immediately.
    pub fn set_note_volume<T: Into<Option<u64>>>(
        &self,
        note_playback_id: PlaybackId,
        volume: f32,
        sample_time: T,
    ) -> Result<(), Error> {
        let sample_time = sample_time.into();
        if !self.is_playing() {
            return Err(Error::SourceNotPlaying);
        }

        if let Some(sample_time) = sample_time {
            // Schedule with mixer
            if self
                .mixer_event_queue()?
                .push(MixerMessage::SetGeneratorNoteVolume {
                    playback_id: self.playback_id,
                    note_playback_id,
                    volume,
                    sample_time,
                })
                .is_err()
            {
                return Err(Self::mixer_event_queue_error("set_note_volume"));
            }
        } else {
            // Apply immediately
            if let PlaybackMessageQueue::Generator { playback, .. } = &self.playback_message_queue {
                if playback
                    .push(GeneratorPlaybackMessage::SetVolume {
                        note_playback_id,
                        volume,
                    })
                    .is_err()
                {
                    return Err(Self::generator_message_queue_error("set_note_volume"));
                }
            } else {
                unreachable!("Expecting a generator message queue for a generator playback handle");
            }
        }

        Ok(())
    }

    /// Set panning for a specific note instance at the given sample time or immediately.
    pub fn set_note_panning<T: Into<Option<u64>>>(
        &self,
        note_playback_id: PlaybackId,
        panning: f32,
        sample_time: T,
    ) -> Result<(), Error> {
        let sample_time = sample_time.into();
        if !self.is_playing() {
            return Err(Error::SourceNotPlaying);
        }

        if let Some(sample_time) = sample_time {
            // Schedule with mixer
            if self
                .mixer_event_queue()?
                .push(MixerMessage::SetGeneratorNotePanning {
                    playback_id: self.playback_id,
                    note_playback_id,
                    panning,
                    sample_time,
                })
                .is_err()
            {
                return Err(Self::mixer_event_queue_error("set_note_panning"));
            }
        } else {
            // Apply immediately
            if let PlaybackMessageQueue::Generator { playback, .. } = &self.playback_message_queue {
                if playback
                    .push(GeneratorPlaybackMessage::SetPanning {
                        note_playback_id,
                        panning,
                    })
                    .is_err()
                {
                    return Err(Self::generator_message_queue_error("set_note_panning"));
                }
            } else {
                unreachable!("Expecting a generator message queue for a generator playback handle");
            }
        }

        Ok(())
    }

    /// Trigger note off for all currently playing notes immediately or at the given sample time.
    /// This is useful for panic/reset scenarios.
    pub fn all_notes_off<T: Into<Option<u64>>>(&self, sample_time: T) -> Result<(), Error> {
        let sample_time = sample_time.into();
        if !self.is_playing() {
            return Err(Error::SourceNotPlaying);
        }

        if let Some(sample_time) = sample_time {
            // Schedule with mixer
            if self
                .mixer_event_queue()?
                .push(MixerMessage::TriggerGeneratorAllNotesOff {
                    playback_id: self.playback_id,
                    sample_time,
                })
                .is_err()
            {
                return Err(Self::mixer_event_queue_error("all_notes_off"));
            }
        } else {
            // Apply immediately
            if let PlaybackMessageQueue::Generator { playback, .. } = &self.playback_message_queue {
                if playback
                    .push(GeneratorPlaybackMessage::AllNotesOff)
                    .is_err()
                {
                    return Err(Self::generator_message_queue_error("all_notes_off"));
                }
            } else {
                unreachable!("Expecting a generator message queue for a generator playback handle");
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

    fn generator_message_queue_error(event_name: &str) -> Error {
        log::warn!("Generator playback event queue is full. Failed to send a {event_name} event.");
        Error::SendError("Generator playback queue is full".to_string())
    }
}
