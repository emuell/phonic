//! Generator trait for sources that can be driven by sequencers.

use std::sync::Arc;

use crossbeam_queue::ArrayQueue;

use crate::{source::Source, PlaybackId};

// -------------------------------------------------------------------------------------------------

pub mod sampler;

// -------------------------------------------------------------------------------------------------

/// Messages that can be sent to a generator for playback control.
#[derive(Debug, Clone, Copy)]
pub enum GeneratorPlaybackMessage {
    /// Stop the generator and remove it from the mixer. This will abruptly kill all notes.
    Stop,
    /// Trigger note off for all currently playing notes and keep the generator running.
    AllNotesOff,
    /// Trigger a note on event.
    NoteOn {
        note_playback_id: PlaybackId,
        note: u8,
        volume: Option<f32>,
        panning: Option<f32>,
    },
    /// Trigger a note off event for a specific note instance.
    NoteOff { note_playback_id: PlaybackId },
    /// Set playback speed (pitch) for a specific note instance.
    SetSpeed {
        note_playback_id: PlaybackId,
        speed: f64,
        glide: Option<f32>,
    },
    /// Set volume for a specific note instance.
    SetVolume {
        note_playback_id: PlaybackId,
        volume: f32,
    },
    /// Set panning for a specific note instance.
    SetPanning {
        note_playback_id: PlaybackId,
        panning: f32,
    },
}

// -------------------------------------------------------------------------------------------------

/// A generator is a source that is driven by note events.
/// It acts as a `Source` (producing audio) and receives note events via its playback message queue.
pub trait Generator: Source {
    /// Get the playback message queue for this generator.
    fn playback_message_queue(&self) -> Arc<ArrayQueue<GeneratorPlaybackMessage>>;
}
