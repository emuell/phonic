//! Sequencer traits and common types for playing musical sequences on generators.

use crate::{GeneratorPlaybackHandle, NotePlaybackId};

// -------------------------------------------------------------------------------------------------

/// Transport provides timing information for sequencers
#[derive(Debug, Clone, Copy)]
pub struct SequencerTransport {
    pub sample_rate: u32,
    pub beats_per_minute: f64,
    pub beats_per_bar: usize,
}

impl SequencerTransport {
    pub const fn new(sample_rate: u32, beats_per_minute: f64, beats_per_bar: usize) -> Self {
        Self {
            sample_rate,
            beats_per_minute,
            beats_per_bar,
        }
    }

    pub fn samples_per_beat(&self) -> u64 {
        (60.0 / self.beats_per_minute * self.sample_rate as f64) as u64
    }

    pub fn samples_per_bar(&self) -> u64 {
        self.samples_per_beat() * self.beats_per_bar as u64
    }

    pub fn samples_to_seconds(&self, samples: u64) -> f64 {
        samples as f64 / self.sample_rate as f64
    }

    pub fn seconds_to_samples(&self, seconds: f64) -> u64 {
        (seconds * self.sample_rate as f64) as u64
    }
}

// -------------------------------------------------------------------------------------------------

/// Trait for triggering note events from a sequencer into a generator.
pub trait SequencerPlayback {
    /// Trigger a note on event.
    /// Returns a NotePlaybackId that can be used to control this specific note instance.
    fn note_on(
        &mut self,
        note: u8,
        volume: Option<f32>,
        panning: Option<f32>,
        start_time: u64,
    ) -> NotePlaybackId;

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

/// A musical sequence that emits timed note events into a [`SequencerPlayback`] target.
///
/// Call [`run_until`](Sequencer::run_until) with the current sample time to fire any events that
/// are due. This can be done periodically from a background thread for live/looping sequences, or
/// called once with [`u64::MAX`] to pre-schedule an entire sequence upfront.
pub trait Sequencer: Send + Sync {
    /// Check if the sequencer finished playback.
    fn is_exhausted(&self) -> bool;

    /// Process events up to the given sample time, using the given playback interface
    /// to trigger events.
    fn run_until(&mut self, sample_time: u64, context: &mut dyn SequencerPlayback);

    /// Reset the sequencer to (re)start playback, starting at the given sample time.
    fn reset(&mut self, sample_time: u64);
}

// -------------------------------------------------------------------------------------------------

pub mod metronome;
pub mod pattern;
#[cfg(feature = "midi")]
pub mod midi_file;

// -------------------------------------------------------------------------------------------------

impl SequencerPlayback for GeneratorPlaybackHandle {
    fn note_on(
        &mut self,
        note: u8,
        volume: Option<f32>,
        panning: Option<f32>,
        start_time: u64,
    ) -> NotePlaybackId {
        GeneratorPlaybackHandle::note_on(self, note, volume, panning, Some(start_time))
            .unwrap_or_else(|err| {
                log::warn!("Sequencer note_on failed: {err}");
                0
            })
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
