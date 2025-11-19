//! Generator trait for sources that can be driven by sequencers.

use std::sync::{mpsc::SyncSender, Arc};

use crossbeam_queue::ArrayQueue;

use crate::{source::Source, PlaybackId, PlaybackStatusEvent};

// -------------------------------------------------------------------------------------------------

pub mod sampler;

// -------------------------------------------------------------------------------------------------

/// Events to start/stop or change playback **within** a [`Generator`].
#[derive(Debug, Clone, Copy)]
pub enum GeneratorPlaybackEvent {
    /// Trigger a note on event.
    NoteOn {
        note_playback_id: PlaybackId,
        note: u8,
        volume: Option<f32>,
        panning: Option<f32>,
    },
    /// Trigger a note off event for a specific note instance.
    NoteOff { note_playback_id: PlaybackId },
    /// Trigger note off for all currently playing notes and keep the generator running.
    AllNotesOff,
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

/// Events to control playback of a [`Generator`].
#[derive(Debug, Clone, Copy)]
pub enum GeneratorPlaybackMessage {
    /// Stop the generator and remove it from the mixer. This will abruptly kill all notes.
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
pub trait Generator: Source {
    /// A unique ID, which can be used to identify sources in `PlaybackStatusEvent`s.
    fn playback_id(&self) -> PlaybackId;

    /// Get the playback message queue for this generator.
    fn playback_message_queue(&self) -> Arc<ArrayQueue<GeneratorPlaybackMessage>>;

    /// Channel to receive playback status from the generator.
    fn playback_status_sender(&self) -> Option<SyncSender<PlaybackStatusEvent>>;
    fn set_playback_status_sender(&mut self, sender: Option<SyncSender<PlaybackStatusEvent>>);
}
