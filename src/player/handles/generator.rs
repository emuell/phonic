use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use basedrop::{Handle, Owned};
use crossbeam_queue::ArrayQueue;
use dashmap::DashMap;
use four_cc::FourCC;

use crate::{
    error::Error,
    generator::{unique_note_id, GeneratorPlaybackEvent, GeneratorPlaybackMessage},
    parameter::ParameterValueUpdate,
    player::{MixerId, PlaybackId},
    source::{
        amplified::AmplifiedSourceMessage, mixed::MixerMessage, panned::PannedSourceMessage,
        playback::PlaybackMessageQueue,
    },
    NotePlaybackId, PlaybackStatusContext,
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
    collector_handle: Handle,
}

impl GeneratorPlaybackHandle {
    pub(crate) fn new(
        is_playing: Arc<AtomicBool>,
        playback_id: PlaybackId,
        mixer_id: MixerId,
        playback_message_queue: PlaybackMessageQueue,
        mixer_event_queues: Arc<DashMap<MixerId, Arc<ArrayQueue<MixerMessage>>>>,
        collector_handle: Handle,
    ) -> Self {
        Self {
            is_playing,
            playback_id,
            playback_message_queue,
            mixer_id,
            mixer_event_queues,
            collector_handle,
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
            let playback_id = self.playback_id;
            if self
                .mixer_event_queue()?
                .force_push(MixerMessage::StopSource {
                    playback_id,
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
            let playback_id = self.playback_id;
            if self
                .mixer_event_queue()?
                .push(MixerMessage::SetSourceVolume {
                    playback_id,
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
        let sample_time = sample_time.into();
        if !self.is_playing() {
            return Err(Error::SourceNotPlaying);
        }

        if let Some(sample_time) = sample_time {
            // Schedule with mixer
            let playback_id = self.playback_id;
            if self
                .mixer_event_queue()?
                .push(MixerMessage::SetSourcePanning {
                    playback_id,
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

    /// Trigger a note on event at the given sample time or immediately.
    /// Returns the note playback ID that can be used to control this specific note instance.
    pub fn note_on<T: Into<Option<u64>>>(
        &self,
        note: u8,
        volume: Option<f32>,
        panning: Option<f32>,
        sample_time: T,
    ) -> Result<NotePlaybackId, Error> {
        let context = None;
        self.note_on_with_context(note, volume, panning, context, sample_time)
    }

    /// Trigger a note on event at the given sample time or immediately and pass along the given
    /// playback context to the playback status channel.
    /// Returns the note playback ID that can be used to control this specific note instance.
    pub fn note_on_with_context<T: Into<Option<u64>>>(
        &self,
        note: u8,
        volume: Option<f32>,
        panning: Option<f32>,
        context: Option<PlaybackStatusContext>,
        sample_time: T,
    ) -> Result<NotePlaybackId, Error> {
        let sample_time = sample_time.into();
        if !self.is_playing() {
            return Err(Error::SourceNotPlaying);
        }
        let note_id = unique_note_id();
        self.send_generator_event(
            sample_time,
            GeneratorPlaybackEvent::NoteOn {
                note_id,
                note,
                volume,
                panning,
                context,
            },
            "note_on",
        )?;
        Ok(note_id)
    }

    /// Trigger a note off event for a specific note instance at the given sample time or immediately.
    pub fn note_off<T: Into<Option<u64>>>(
        &self,
        note_id: NotePlaybackId,
        sample_time: T,
    ) -> Result<(), Error> {
        let sample_time = sample_time.into();
        if !self.is_playing() {
            return Err(Error::SourceNotPlaying);
        }
        self.send_generator_event(
            sample_time,
            GeneratorPlaybackEvent::NoteOff { note_id },
            "note_off",
        )
    }

    /// Set playback speed (pitch) for a specific note instance at the given sample time or immediately.
    pub fn set_note_speed<T: Into<Option<u64>>>(
        &self,
        note_id: NotePlaybackId,
        speed: f64,
        glide: Option<f32>,
        sample_time: T,
    ) -> Result<(), Error> {
        let sample_time = sample_time.into();
        if !self.is_playing() {
            return Err(Error::SourceNotPlaying);
        }
        self.send_generator_event(
            sample_time,
            GeneratorPlaybackEvent::SetSpeed {
                note_id,
                speed,
                glide,
            },
            "set_note_speed",
        )
    }

    /// Trigger note off for all currently playing notes immediately or at the given sample time.
    /// This is useful for panic/reset scenarios.
    pub fn all_notes_off<T: Into<Option<u64>>>(&self, sample_time: T) -> Result<(), Error> {
        let sample_time = sample_time.into();
        if !self.is_playing() {
            return Err(Error::SourceNotPlaying);
        }
        self.send_generator_event(
            sample_time,
            GeneratorPlaybackEvent::AllNotesOff,
            "all_notes_off",
        )
    }

    /// Set volume for a specific note instance at the given sample time or immediately.
    pub fn set_note_volume<T: Into<Option<u64>>>(
        &self,
        note_id: NotePlaybackId,
        volume: f32,
        sample_time: T,
    ) -> Result<(), Error> {
        let sample_time = sample_time.into();
        if !self.is_playing() {
            return Err(Error::SourceNotPlaying);
        }
        self.send_generator_event(
            sample_time,
            GeneratorPlaybackEvent::SetVolume { note_id, volume },
            "set_note_volume",
        )
    }

    /// Set panning for a specific note instance at the given sample time or immediately.
    pub fn set_note_panning<T: Into<Option<u64>>>(
        &self,
        note_id: NotePlaybackId,
        panning: f32,
        sample_time: T,
    ) -> Result<(), Error> {
        let sample_time = sample_time.into();
        if !self.is_playing() {
            return Err(Error::SourceNotPlaying);
        }
        self.send_generator_event(
            sample_time,
            GeneratorPlaybackEvent::SetPanning { note_id, panning },
            "set_note_panning",
        )
    }

    /// Set a generator parameter value at a specific sample time or immediately.
    ///
    /// The `value` must be of the correct type for the parameter: `f32`, `i32`, `bool`,
    /// or the specific enum type used by the parameter.
    pub fn set_parameter<V, T>(
        &self,
        parameter_id: FourCC,
        value: V,
        sample_time: T,
    ) -> Result<(), Error>
    where
        V: std::any::Any + Send + Sync + 'static,
        T: Into<Option<u64>>,
    {
        let sample_time = sample_time.into();
        if !self.is_playing() {
            return Err(Error::SourceNotPlaying);
        }
        let value = Owned::new(
            &self.collector_handle,
            ParameterValueUpdate::Raw(Box::new(value)),
        );
        self.send_generator_event(
            sample_time,
            GeneratorPlaybackEvent::SetParameter {
                id: parameter_id,
                value,
            },
            "set_parameter",
        )
    }

    /// Set a normalized (0.0..=1.0) parameter value either immediately or at a future sample time.
    pub fn set_parameter_normalized<T: Into<Option<u64>>>(
        &self,
        parameter_id: FourCC,
        value: f32,
        sample_time: T,
    ) -> Result<(), Error> {
        let sample_time = sample_time.into();
        if !self.is_playing() {
            return Err(Error::SourceNotPlaying);
        }
        let value = Owned::new(
            &self.collector_handle,
            ParameterValueUpdate::Normalized(value),
        );
        self.send_generator_event(
            sample_time,
            GeneratorPlaybackEvent::SetParameter {
                id: parameter_id,
                value,
            },
            "set_parameter_normalized",
        )
    }

    fn send_generator_event(
        &self,
        sample_time: Option<u64>,
        event: GeneratorPlaybackEvent,
        event_name: &str,
    ) -> Result<(), Error> {
        if let Some(sample_time) = sample_time {
            // Schedule with mixer
            let playback_id = self.playback_id;
            if self
                .mixer_event_queue()?
                .push(MixerMessage::TriggerGeneratorEvent {
                    playback_id,
                    event,
                    sample_time,
                })
                .is_err()
            {
                return Err(Self::mixer_event_queue_error(event_name));
            }
        } else {
            // Apply immediately
            if let PlaybackMessageQueue::Generator { playback, .. } = &self.playback_message_queue {
                if playback
                    .push(GeneratorPlaybackMessage::Trigger { event })
                    .is_err()
                {
                    return Err(Self::generator_message_queue_error(event_name));
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
