//! Pattern sequencer implementation.

use crate::{utils::speed_from_note, NotePlaybackId, Transport};

use super::{Sequencer, SequencerEventSink};

// -------------------------------------------------------------------------------------------------

/// A single note event in a [`PatternRow`].
#[derive(Debug, Clone, Copy)]
pub struct PatternEvent {
    /// MIDI note number.
    pub note: u8,
    /// Optional glide time in semitones per second.
    pub glide: Option<f32>,
    /// Optional volume (0.0 to 1.0)
    pub volume: Option<f32>,
    /// Optional panning (-1.0 to 1.0)
    pub panning: Option<f32>,
}

impl PatternEvent {
    pub const NOTE_OFF: u8 = 0xFF;
    pub const NOTE_CONTINUE: u8 = 0xFE;

    pub fn note_on(note: u8) -> Self {
        assert!((0..127).contains(&note), "Invalid note value");
        Self {
            note,
            glide: None,
            volume: None,
            panning: None,
        }
    }

    /// Creates a continue event. The note at this voice index keeps playing unchanged.
    /// Optional `volume` and `panning` modifiers are still applied if set.
    pub fn note_continue() -> Self {
        Self {
            note: Self::NOTE_CONTINUE,
            glide: None,
            volume: None,
            panning: None,
        }
    }

    /// Creates a note-off event that stops the note at the same voice index.
    pub fn note_off() -> Self {
        Self {
            note: Self::NOTE_OFF,
            glide: None,
            volume: None,
            panning: None,
        }
    }

    pub fn glide(mut self, glide: f32) -> Self {
        self.glide = Some(glide);
        self
    }

    pub fn volume(mut self, volume: f32) -> Self {
        self.volume = Some(volume);
        self
    }

    pub fn panning(mut self, panning: f32) -> Self {
        self.panning = Some(panning);
        self
    }
}

// -------------------------------------------------------------------------------------------------

/// A single step in a [`Pattern`], holding one or more [`PatternEvent`]s and a beat duration.
///
/// Multiple events in a row play simultaneously as a chord.
/// Notes with glide events modify the existing note at the same index instead of starting a new one.
/// Note-on events implicitely send Note-off events for already playing notes on the same index.
#[derive(Debug, Clone)]
pub struct PatternRow {
    /// Events to play at this step.
    pub events: Vec<PatternEvent>,
    /// Duration of this step in beats. Must be > 0.0.
    pub duration_beats: f64,
}

impl PatternRow {
    /// Create a new row with the given events and duration.
    pub fn new(events: Vec<PatternEvent>, duration_beats: f64) -> Self {
        assert!(duration_beats > 0.0, "duration_beats must be > 0.0");
        Self {
            events,
            duration_beats,
        }
    }
}

// -------------------------------------------------------------------------------------------------

/// Converts a [`PatternEvent`] or an array of [`PatternEvent`]s into a [`PatternRow`].
///
/// Implemented for:
/// - [`PatternEvent`] produces a single-voice row.
/// - `[PatternEvent; N]` produces a chord row with `N` simultaneous voices (a chord).
pub trait IntoPatternRow {
    fn into_row(self, duration_beats: f64) -> PatternRow;
}

impl IntoPatternRow for PatternEvent {
    fn into_row(self, duration_beats: f64) -> PatternRow {
        PatternRow::new(vec![self], duration_beats)
    }
}

impl<const N: usize> IntoPatternRow for [PatternEvent; N] {
    fn into_row(self, duration_beats: f64) -> PatternRow {
        PatternRow::new(self.into(), duration_beats)
    }
}

// -------------------------------------------------------------------------------------------------

/// A sequencer that plays notes from a predefined, static pattern, similar to a polyphonic step
/// sequencer but with individual row lengths.
///
/// * Each [`PatternRow`] holds one or more [`PatternEvent`]s and a step duration in beats.
///
/// * Each row's voice slots interacts with voice slots from previous rows:
///   A `note_on` implicitely stops notes that got triggered from the previous row at the same index.
///   `glide` properties avoid that, and update the pitch of the voice without stopping it.
///   A `note_continue` explicitely elongens a previous row's note playback but allows changing
///   playing note properties such as volume and panning.
pub struct Pattern {
    rows: Vec<PatternRow>,
    start_time: u64,
    repeat_count: usize,
    current_repeat: usize,
    current_note_index: usize,
    transport: Option<Transport>,
    finished: bool,
    note_ids: Vec<Option<NotePlaybackId>>,
}

impl Pattern {
    /// Create a new pattern sequencer.
    ///
    /// `repeat_count = 0` plays the pattern once; `repeat_count = N` plays it N + 1 times.
    /// `repeat_count = usize::MAX` repeats infinitely.
    ///
    /// Call [`with_transport`](Self::with_transport) before running, if not added to a player.
    pub fn new(rows: Vec<PatternRow>, repeat_count: usize) -> Self {
        let max_polyphony = rows.iter().map(|row| row.events.len()).max().unwrap_or(0);
        Self {
            rows,
            start_time: 0,
            repeat_count,
            current_repeat: 0,
            current_note_index: 0,
            transport: None,
            finished: false,
            note_ids: vec![None; max_polyphony],
        }
    }

    /// Set the transport (BPM, sample rate, time signature) for external use.
    ///
    /// Not needed when the pattern is driven by a player as the player enforces its own transport.
    pub fn with_transport(mut self, transport: Transport) -> Self {
        self.transport = Some(transport);
        self
    }

    /// Set the absolute sample-time at which the pattern should start playing.
    ///
    /// Defaults to `0`. Not needed when the pattern is driven by a player.
    pub fn with_start_time(mut self, start_time: u64) -> Self {
        self.start_time = start_time;
        self
    }
}

impl Sequencer for Pattern {
    fn is_playing(&self) -> bool {
        !self.finished
    }

    fn set_transport(&mut self, transport: Transport, current_sample_time: u64) {
        if self.transport.is_some() {
            let repeat_beats: f64 = self.rows.iter().map(|row| row.duration_beats).sum();
            let note_beats: f64 = self.rows[..self.current_note_index]
                .iter()
                .map(|row| row.duration_beats)
                .sum();
            let total_beats = self.current_repeat as f64 * repeat_beats + note_beats;
            let new_offset = transport.beats_to_samples(total_beats);
            self.start_time = current_sample_time.saturating_sub(new_offset);
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

        let mut current_time = self.start_time
            + self.current_repeat as u64
                * self
                    .rows
                    .iter()
                    .map(|row| transport.beats_to_samples(row.duration_beats))
                    .sum::<u64>()
            + self.rows[..self.current_note_index]
                .iter()
                .map(|row| transport.beats_to_samples(row.duration_beats))
                .sum::<u64>();

        // Process steps
        while self.repeat_count == usize::MAX || self.current_repeat <= self.repeat_count {
            while self.current_note_index < self.rows.len() {
                if current_time > sample_time {
                    return;
                }

                let row = &self.rows[self.current_note_index];

                for (voice_index, event) in row.events.iter().enumerate() {
                    if event.note == PatternEvent::NOTE_CONTINUE {
                        if let Some(note_id) = self.note_ids[voice_index] {
                            if let Some(volume) = event.volume {
                                event_sink.set_volume(note_id, volume, current_time);
                            }
                            if let Some(panning) = event.panning {
                                event_sink.set_panning(note_id, panning, current_time);
                            }
                        }
                    } else if event.note == PatternEvent::NOTE_OFF {
                        if let Some(note_id) = self.note_ids[voice_index].take() {
                            event_sink.note_off(note_id, current_time);
                        }
                    } else if event.glide.is_none() || self.note_ids[voice_index].is_none() {
                        if let Some(note_id) = self.note_ids[voice_index].take() {
                            event_sink.note_off(note_id, current_time);
                        }
                        self.note_ids[voice_index] = event_sink.note_on(
                            event.note,
                            event.volume,
                            event.panning,
                            current_time,
                        );
                    } else {
                        let note_id = self.note_ids[voice_index].unwrap();
                        event_sink.set_speed(
                            note_id,
                            speed_from_note(event.note),
                            event.glide,
                            current_time,
                        );
                        if let Some(volume) = event.volume {
                            event_sink.set_volume(note_id, volume, current_time);
                        }
                        if let Some(panning) = event.panning {
                            event_sink.set_panning(note_id, panning, current_time);
                        }
                    }
                }

                current_time += transport.beats_to_samples(row.duration_beats);
                self.current_note_index += 1;
            }

            self.current_note_index = 0;
            self.current_repeat += 1;
        }

        self.finished = true;
    }

    fn reset(&mut self, start_time: u64) {
        self.start_time = start_time;
        self.current_repeat = 0;
        self.current_note_index = 0;
        self.finished = false;
        self.note_ids.fill(None);
    }

    fn stop(&mut self, sample_time: u64, event_sink: &mut dyn SequencerEventSink) {
        for slot in &mut self.note_ids {
            if let Some(note_id) = slot.take() {
                event_sink.note_off(note_id, sample_time);
            }
        }
        self.finished = true;
    }
}
