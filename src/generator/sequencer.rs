//! Sequencer traits and common types for playing musical sequences on generators.

use crate::{GeneratorPlaybackHandle, NotePlaybackId, PlaybackStatusContext, Transport};

// -------------------------------------------------------------------------------------------------

pub mod metronome;
#[cfg(feature = "midi")]
pub mod midi_file;
pub mod pattern;

// -------------------------------------------------------------------------------------------------

/// A musical sequence that emits timed note events into a [`SequencerEventSink`].
///
/// Call [`run_until`](Sequencer::run_until) with the current sample time to fire any events that
/// are due. This can be done periodically from a background thread for live/looping sequences, or
/// called once with [`u64::MAX`] to pre-schedule an entire sequence upfront.
///
/// When added to a [`Player`](crate::Player) via [`play_sequencer`](crate::Player::play_sequencer),
/// the player will automatically manage and run he sequencer in its audio thread.
/// All sequencer trait functions then must be !real-time safe! as they are called on the player's
/// main audio thread.
pub trait Sequencer: Send + Sync {
    /// Convert into a `Box<dyn Sequencer>`.
    ///
    /// The default implementation boxes `self`. `Box<dyn Sequencer>` overrides this to return
    /// itself directly, avoiding double-boxing.
    fn into_box(self) -> Box<dyn Sequencer>
    where
        Self: Sized + 'static,
    {
        Box::new(self)
    }

    /// Returns `true` while the sequencer is still active (not yet finished playback
    /// nor stopped manually).
    fn is_playing(&self) -> bool;

    /// Set initial or update existing transport (BPM, time signature, sample rate).
    ///
    /// Called by the player whenever its global transport is updated, and also immediately when
    /// a sequencer is first registered via `play_sequencer`. Sequencer implementations should
    /// re-anchor their internal beat position to `current_sample_time` using the new transport.
    fn set_transport(&mut self, transport: Transport, current_sample_time: u64);

    /// Process events up to the given sample time, using the given playback interface
    /// to trigger events.
    fn run_until(&mut self, sample_time: u64, event_sink: &mut dyn SequencerEventSink);

    /// Reset the sequencer to (re)start playback, starting at the given sample time.
    fn reset(&mut self, sample_time: u64);

    /// Stop playback and send note-off events for all currently held notes.
    ///
    /// Called by the mixer when the sequencer is stopped manually before it exhausted.
    /// Implementations should release all held notes via `event_sink` and mark
    /// themselves as finished.
    fn stop(&mut self, sample_time: u64, event_sink: &mut dyn SequencerEventSink);
}

// -------------------------------------------------------------------------------------------------

/// Allow using a boxed `dyn Sequencer` anywhere a `Sequencer` is expected.
impl Sequencer for Box<dyn Sequencer> {
    fn into_box(self) -> Box<dyn Sequencer> {
        self
    }

    fn is_playing(&self) -> bool {
        (**self).is_playing()
    }

    fn set_transport(&mut self, transport: Transport, current_sample_time: u64) {
        (**self).set_transport(transport, current_sample_time)
    }

    fn run_until(&mut self, sample_time: u64, event_sink: &mut dyn SequencerEventSink) {
        (**self).run_until(sample_time, event_sink)
    }

    fn reset(&mut self, sample_time: u64) {
        (**self).reset(sample_time)
    }

    fn stop(&mut self, sample_time: u64, event_sink: &mut dyn SequencerEventSink) {
        (**self).stop(sample_time, event_sink)
    }
}

// -------------------------------------------------------------------------------------------------

/// Trait for triggering note events from a sequencer into a generator.
pub trait SequencerEventSink {
    /// Trigger a note on event.
    /// Returns `Some(NotePlaybackId)` on success, or `None` if the note could not be played.
    ///
    /// The default implementation calls `note_on_with_context` without a context.
    fn note_on(
        &mut self,
        note: u8,
        volume: Option<f32>,
        panning: Option<f32>,
        start_time: u64,
    ) -> Option<NotePlaybackId> {
        self.note_on_with_context(note, volume, panning, None, start_time)
    }

    /// Trigger a note on event with optional volume, pan and playback status context.
    ///
    /// The context is forwarded to [`PlaybackStatusEvent`](crate::PlaybackStatusEvent) callbacks
    /// so callers can attach custom metadata (e.g. instrument or voice indices) to each note.
    ///
    /// Returns `Some(NotePlaybackId)` on success, or `None` if the note could not be played.
    fn note_on_with_context(
        &mut self,
        note: u8,
        volume: Option<f32>,
        panning: Option<f32>,
        context: Option<PlaybackStatusContext>,
        start_time: u64,
    ) -> Option<NotePlaybackId>;

    /// Trigger a note off event for a specific note instance.
    fn note_off(&mut self, note_id: NotePlaybackId, stop_time: u64);

    /// Set playback speed (pitch) for a specific note instance, if supported.
    fn set_speed(
        &mut self,
        note_id: NotePlaybackId,
        speed: f64,
        glide: Option<f32>,
        sample_time: u64,
    );

    /// Set volume level for a specific note instance, if supported.
    fn set_volume(&mut self, note_id: NotePlaybackId, volume: f32, sample_time: u64);

    /// Set stereo panning position for a specific note instance, if supported.
    /// (-1.0 = left, 0.0 = center, 1.0 = right)
    fn set_panning(&mut self, note_id: NotePlaybackId, panning: f32, sample_time: u64);
}

// -------------------------------------------------------------------------------------------------

/// A no-op [`SequencerEventSink`] which can be used to drain events in [`Sequencer`]
pub struct SequencerNoopEventSink;

impl SequencerEventSink for SequencerNoopEventSink {
    fn note_on_with_context(
        &mut self,
        _note: u8,
        _volume: Option<f32>,
        _panning: Option<f32>,
        _context: Option<PlaybackStatusContext>,
        _start_time: u64,
    ) -> Option<NotePlaybackId> {
        Some(0)
    }

    fn note_off(&mut self, _note_id: NotePlaybackId, _stop_time: u64) {}

    fn set_speed(
        &mut self,
        _note_id: NotePlaybackId,
        _speed: f64,
        _glide: Option<f32>,
        _sample_time: u64,
    ) {
    }

    fn set_volume(&mut self, _note_id: NotePlaybackId, _volume: f32, _sample_time: u64) {}

    fn set_panning(&mut self, _note_id: NotePlaybackId, _panning: f32, _sample_time: u64) {}
}

// -------------------------------------------------------------------------------------------------

impl SequencerEventSink for GeneratorPlaybackHandle {
    fn note_on(
        &mut self,
        note: u8,
        volume: Option<f32>,
        panning: Option<f32>,
        start_time: u64,
    ) -> Option<NotePlaybackId> {
        self.note_on_with_context(note, volume, panning, None, start_time)
    }

    fn note_on_with_context(
        &mut self,
        note: u8,
        volume: Option<f32>,
        panning: Option<f32>,
        context: Option<PlaybackStatusContext>,
        start_time: u64,
    ) -> Option<NotePlaybackId> {
        GeneratorPlaybackHandle::note_on_with_context(
            self,
            note,
            volume,
            panning,
            context,
            Some(start_time),
        )
        .map_err(|err| {
            log::warn!("Sequencer note_on failed: {err}");
        })
        .ok()
    }

    fn note_off(&mut self, note_id: NotePlaybackId, stop_time: u64) {
        let _ = GeneratorPlaybackHandle::note_off(self, note_id, Some(stop_time));
    }

    fn set_speed(
        &mut self,
        note_id: NotePlaybackId,
        speed: f64,
        glide: Option<f32>,
        sample_time: u64,
    ) {
        let _ =
            GeneratorPlaybackHandle::set_note_speed(self, note_id, speed, glide, Some(sample_time));
    }

    fn set_volume(&mut self, note_id: NotePlaybackId, volume: f32, sample_time: u64) {
        let _ = GeneratorPlaybackHandle::set_note_volume(self, note_id, volume, Some(sample_time));
    }

    fn set_panning(&mut self, note_id: NotePlaybackId, panning: f32, sample_time: u64) {
        let _ =
            GeneratorPlaybackHandle::set_note_panning(self, note_id, panning, Some(sample_time));
    }
}
