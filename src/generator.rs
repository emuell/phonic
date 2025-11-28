//! Generator trait for sources that can be driven by sequencers.

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    mpsc::SyncSender,
    Arc,
};

use basedrop::Owned;
use crossbeam_queue::ArrayQueue;
use four_cc::FourCC;

use crate::{
    parameter::{ClonableParameter, ParameterValueUpdate},
    source::Source,
    Error, NotePlaybackId, PlaybackId, PlaybackStatusEvent,
};

// -------------------------------------------------------------------------------------------------

pub mod sampler;

#[cfg(feature = "fundsp")]
pub mod fundsp;

// -------------------------------------------------------------------------------------------------

/// Generates a unique source id for a triggered note in a generator.
pub(crate) fn unique_note_id() -> usize {
    static ID_COUNTER: AtomicUsize = AtomicUsize::new(1);
    ID_COUNTER.fetch_add(1, Ordering::Relaxed)
}

// -------------------------------------------------------------------------------------------------

/// Events to start/stop, change playback properties or parameters **within** a [`Generator`].
pub enum GeneratorPlaybackEvent {
    /// Trigger a note on event.
    NoteOn {
        note_id: NotePlaybackId,
        note: u8,
        volume: Option<f32>,
        panning: Option<f32>,
    },
    /// Trigger a note off event for a specific note playback.
    NoteOff { note_id: NotePlaybackId },
    /// Stop all currently playing notes.
    AllNotesOff,

    /// Set the speed/pitch of a specific note playback.
    SetSpeed {
        note_id: NotePlaybackId,
        speed: f64,
        glide: Option<f32>,
    },
    /// Set the volume of a specific note playback.
    SetVolume {
        note_id: NotePlaybackId,
        volume: f32,
    },
    /// Set the panning of a specific note playback.
    SetPanning {
        note_id: NotePlaybackId,
        panning: f32,
    },

    /// Update a generator automation parameter.
    SetParameter {
        id: FourCC,
        value: Owned<ParameterValueUpdate>,
    },
}

// -------------------------------------------------------------------------------------------------

/// Messages to control playback of and within a [`Generator`].
pub enum GeneratorPlaybackMessage {
    /// Stop the generator and remove it from the mixer. This stops playback, waits until the
    /// source is exhausted and then finally removes the source from the mixer.
    Stop,
    /// Trigger a playback event. All playback events keep the generator running in the mixer.
    Trigger { event: GeneratorPlaybackEvent },
}

// -------------------------------------------------------------------------------------------------

/// A [`Source`] that is driven by note events.
///
/// It supports the usual volume and panning events and additional note trigger events via
/// its playback message queue.
///
/// A generator is active as long as it get's actively stopped. Stopping will remove the
/// generator from it's parent mixer, so to keep it running stop all playing notes only instead.
///
/// Generator parameters work similarly to [`Effect`](crate::Effect) parameters: they provide
/// automation capabilities and can be queried via [`parameters()`](Self::parameters).
pub trait Generator: Source {
    /// A unique ID, which can be used to identify sources in `PlaybackStatusEvent`s.
    fn playback_id(&self) -> PlaybackId;

    /// Get the playback message queue for this generator.
    fn playback_message_queue(&self) -> Arc<ArrayQueue<GeneratorPlaybackMessage>>;

    /// Channel to receive playback status from the generator.
    fn playback_status_sender(&self) -> Option<SyncSender<PlaybackStatusEvent>>;
    /// Set the playback status sender for this generator.
    fn set_playback_status_sender(&mut self, sender: Option<SyncSender<PlaybackStatusEvent>>);

    /// Optional parameter descriptors for the generator.
    ///
    /// When returning parameters here, implement `process_parameter_update` too.
    fn parameters(&self) -> Vec<&dyn ClonableParameter> {
        vec![]
    }

    /// Process a parameter update for this generator in the audio thread.
    fn process_parameter_update(
        &mut self,
        _id: FourCC,
        _value: &ParameterValueUpdate,
    ) -> Result<(), Error> {
        debug_assert!(
            self.parameters().is_empty(),
            "When providing parameters, implement 'process_parameter_update' too!"
        );
        Ok(())
    }
}
