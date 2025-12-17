use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use crossbeam_queue::ArrayQueue;

use crate::{
    error::Error,
    player::PlaybackId,
    source::{
        amplified::AmplifiedSourceMessage,
        file::FilePlaybackMessage,
        measured::{CpuLoad, SharedMeasurementState},
        mixed::MixerMessage,
        panned::PannedSourceMessage,
        playback::PlaybackMessageQueue,
    },
};
use std::time::Duration;

// -------------------------------------------------------------------------------------------------

/// Query and change runtime playback properties of a playing [`FileSource`](crate::FileSource).
///
/// Handles are `Send` and `Sync` so they can be sent across threads.
///
/// To track detailed playback status use a [`PlaybackStatusEvent`](crate::PlaybackStatusEvent)
/// [`sync_channel`](std::sync::mpsc::sync_channel) in the [`Player`](crate::Player).
#[derive(Clone)]
pub struct FilePlaybackHandle {
    is_playing: Arc<AtomicBool>,
    playback_id: PlaybackId,
    playback_message_queue: PlaybackMessageQueue,
    mixer_event_queue: Arc<ArrayQueue<MixerMessage>>,
    measurement_state: Option<SharedMeasurementState>,
}

impl FilePlaybackHandle {
    pub(crate) fn new(
        is_playing: Arc<AtomicBool>,
        playback_id: PlaybackId,
        playback_message_queue: crate::source::playback::PlaybackMessageQueue,
        mixer_event_queue: Arc<ArrayQueue<MixerMessage>>,
        measurement_state: Option<SharedMeasurementState>,
    ) -> Self {
        Self {
            is_playing,
            playback_id,
            playback_message_queue,
            mixer_event_queue,
            measurement_state,
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

    /// Get the CPU load data for this source.
    ///
    /// Returns `None` if CPU measurement was not enabled for this source, or if the
    /// measurement is not available at this time.
    pub fn cpu_load(&self) -> Option<CpuLoad> {
        self.measurement_state
            .as_ref()
            .and_then(|state| state.try_lock().map(|state| state.cpu_load()).ok())
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
                .mixer_event_queue
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
            if let PlaybackMessageQueue::File { playback, .. } = &self.playback_message_queue {
                if playback.force_push(FilePlaybackMessage::Stop).is_some() {
                    return Err(Self::file_playback_queue_error("stop"));
                }
            } else {
                unreachable!("Expecting a file message queue for a file playback handle");
            }
        }

        Ok(())
    }

    /// Change playback position of the source at a specific sample time or immediately.
    pub fn seek<T: Into<Option<u64>>>(
        &self,
        position: Duration,
        sample_time: T,
    ) -> Result<(), Error> {
        if !self.is_playing() {
            return Err(Error::SourceNotPlaying);
        }

        let sample_time = sample_time.into();
        if let Some(sample_time) = sample_time {
            // Schedule with mixer
            if self
                .mixer_event_queue
                .push(MixerMessage::SeekSource {
                    playback_id: self.playback_id,
                    position,
                    sample_time,
                })
                .is_err()
            {
                return Err(Self::mixer_event_queue_error("seek"));
            }
        } else {
            // Apply immediately
            if let PlaybackMessageQueue::File { playback, .. } = &self.playback_message_queue {
                if playback.push(FilePlaybackMessage::Seek(position)).is_err() {
                    return Err(Self::file_playback_queue_error("seek"));
                }
            } else {
                unreachable!("Expecting a file message queue for a file playback handle");
            }
        }

        Ok(())
    }

    /// Set file source's speed at a given sample time in future or immediately,
    /// with the given optional glide rate in semitones per second.
    pub fn set_speed<T: Into<Option<u64>>>(
        &self,
        speed: f64,
        glide: Option<f32>,
        sample_time: T,
    ) -> Result<(), Error> {
        if !self.is_playing() {
            return Err(Error::SourceNotPlaying);
        }

        let sample_time = sample_time.into();
        if let Some(sample_time) = sample_time {
            // Schedule with mixer
            if self
                .mixer_event_queue
                .push(MixerMessage::SetSourceSpeed {
                    playback_id: self.playback_id,
                    speed,
                    glide,
                    sample_time,
                })
                .is_err()
            {
                return Err(Self::mixer_event_queue_error("set_source_speed"));
            }
        } else {
            // Apply immediately
            if let PlaybackMessageQueue::File { playback, .. } = &self.playback_message_queue {
                if playback
                    .push(FilePlaybackMessage::SetSpeed(speed, glide))
                    .is_err()
                {
                    return Err(Self::file_playback_queue_error("seek"));
                }
            } else {
                unreachable!("Expecting a file message queue for a file playback handle");
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
                .mixer_event_queue
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
                .mixer_event_queue
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

    fn mixer_event_queue_error(event_name: &str) -> Error {
        log::warn!("Mixer's event queue is full. Failed to send a {event_name} event.");
        log::warn!("Increase the mixer event queue to prevent this from happening...");
        Error::SendError("Mixer queue is full".to_string())
    }

    fn file_playback_queue_error(event_name: &str) -> Error {
        log::warn!("File playback event queue is full. Failed to send a {event_name} event.");
        Error::SendError("File playback queue is full".to_string())
    }
}
