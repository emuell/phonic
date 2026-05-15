//! Metronome sequencer implementation.

use crate::{NotePlaybackId, Transport};

use super::{Sequencer, SequencerEventSink};

// -------------------------------------------------------------------------------------------------

/// A simple metronome sequencer.
///
/// Plays an accented note on each bar downbeat and a softer note on all other beats.
pub struct Metronome {
    repeat_count: usize,
    start_time: u64,
    transport: Option<Transport>,
    current_bar: usize,
    current_beat: usize,
    finished: bool,
    note_id: Option<NotePlaybackId>,
}

impl Metronome {
    const ACCENT_NOTE: u8 = 72;
    const ACCENT_VOLUME: f32 = 1.0;
    const BEAT_NOTE: u8 = 60;
    const BEAT_VOLUME: f32 = 0.7;

    /// Create a new metronome sequencer.
    ///
    /// `repeat_count = 0` plays one bar; `repeat_count = N` plays N + 1 bars.
    /// `repeat_count = usize::MAX` repeats infinitely.
    ///
    /// Call [`with_transport`](Self::with_transport) before running if not added to a player.
    pub fn new(repeat_count: usize) -> Self {
        Self {
            repeat_count,
            start_time: 0,
            transport: None,
            current_bar: 0,
            current_beat: 0,
            finished: false,
            note_id: None,
        }
    }

    /// Set the transport (BPM, sample rate, time signature) for external use.
    ///
    /// Not needed when the metronome is driven by a player as the player enforces its own transport.
    pub fn with_transport(mut self, transport: Transport) -> Self {
        self.transport = Some(transport);
        self
    }

    /// Set the absolute sample-time at which playback should start.
    ///
    /// Defaults to `0`. Not needed when the metronome is driven by a player.
    pub fn with_start_time(mut self, start_time: u64) -> Self {
        self.start_time = start_time;
        self
    }
}

impl Sequencer for Metronome {
    fn is_playing(&self) -> bool {
        !self.finished
    }

    fn set_transport(&mut self, transport: Transport, current_sample_time: u64) {
        if let Some(old_transport) = self.transport {
            // Compute total beats elapsed under the old transport and map them to the new tempo
            // so the beat position stays continuous...
            let total_beats = self.current_bar * old_transport.beats_per_bar() + self.current_beat;
            let new_offset = transport.beats_to_samples(total_beats as f64);
            self.start_time = current_sample_time.saturating_sub(new_offset);
            // When the bar length changes, the current beat index is meaningless:
            // Restart from the current position rather than drift to a wrong downbeat.
            if old_transport.beats_per_bar() != transport.beats_per_bar() {
                self.current_bar = 0;
                self.current_beat = 0;
                self.start_time = current_sample_time;
            }
        } else {
            // First call: simply use the activation time.
            self.start_time = current_sample_time;
        }
        self.transport = Some(transport);
    }

    fn run_until(&mut self, sample_time: u64, event_sink: &mut dyn SequencerEventSink) {
        let transport = self.transport.expect(
            "Sequencers which are not added/player via the player must have a custom transport set",
        );
        if self.finished || sample_time < self.start_time {
            return;
        }

        while self.repeat_count == usize::MAX || self.current_bar <= self.repeat_count {
            let next_beats_sample_time = self.start_time
                + transport.bars_to_samples(self.current_bar as f64)
                + transport.beats_to_samples(self.current_beat as f64);

            if next_beats_sample_time > sample_time {
                return;
            }

            let is_accent = self.current_beat == 0;
            let note = if is_accent {
                Self::ACCENT_NOTE
            } else {
                Self::BEAT_NOTE
            };
            let volume = if is_accent {
                Self::ACCENT_VOLUME
            } else {
                Self::BEAT_VOLUME
            };

            let note_id = event_sink.note_on(note, Some(volume), None, next_beats_sample_time);
            if let Some(prev_note_id) = self.note_id {
                event_sink.note_off(prev_note_id, next_beats_sample_time);
            }
            self.note_id = note_id;

            self.current_beat += 1;
            if self.current_beat >= transport.beats_per_bar() {
                self.current_beat = 0;
                self.current_bar += 1;
            }
        }

        self.finished = true;
    }

    fn reset(&mut self, start_time: u64) {
        self.start_time = start_time;
        self.current_bar = 0;
        self.current_beat = 0;
        self.finished = false;
        self.note_id = None;
    }

    fn stop(&mut self, sample_time: u64, event_sink: &mut dyn SequencerEventSink) {
        if let Some(note_id) = self.note_id.take() {
            event_sink.note_off(note_id, sample_time);
        }
        self.finished = true;
    }
}
