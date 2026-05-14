//! Pattern sequencer implementation.

use crate::{utils::speed_from_note, NotePlaybackId};

use super::{Sequencer, SequencerPlayback, SequencerTransport};

// -------------------------------------------------------------------------------------------------

/// A single event in a pattern sequence
#[derive(Debug, Clone, Copy)]
pub struct PatternEvent {
    /// MIDI note number.
    pub note: u8,
    /// Duration in beats.
    pub duration_beats: f64,
    /// Optional glide time in semitones per second.
    pub glide: Option<f32>,
    /// Optional volume (0.0 to 1.0)
    pub volume: Option<f32>,
    /// Optional panning (-1.0 to 1.0)
    pub panning: Option<f32>,
}

impl PatternEvent {
    const NOTE_OFF_NOTE: u8 = 0xFF;

    pub fn note_on(note: u8, duration_beats: f64) -> Self {
        assert!((0..127).contains(&note), "Invalid note value");
        Self {
            note,
            duration_beats,
            glide: None,
            volume: None,
            panning: None,
        }
    }

    pub fn note_off(duration_beats: f64) -> Self {
        Self {
            note: Self::NOTE_OFF_NOTE,
            duration_beats,
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

/// A pattern sequencer that plays notes or triggers note parameter changes
pub struct Pattern {
    notes: Vec<PatternEvent>,
    start_time: u64,
    repeat_count: usize,
    current_repeat: usize,
    current_note_index: usize,
    transport: SequencerTransport,
    finished: bool,
    note_id: Option<NotePlaybackId>,
}

impl Pattern {
    pub fn new(
        notes: Vec<PatternEvent>,
        start_time: u64,
        repeat_count: usize,
        transport: SequencerTransport,
    ) -> Self {
        Self {
            notes,
            start_time,
            repeat_count,
            current_repeat: 0,
            current_note_index: 0,
            transport,
            finished: false,
            note_id: None,
        }
    }
}

impl Sequencer for Pattern {
    fn run_until(&mut self, sample_time: u64, context: &mut dyn SequencerPlayback) {
        if self.finished || sample_time < self.start_time {
            return;
        }

        let samples_per_beat = self.transport.samples_per_beat();
        let mut current_time = self.start_time
            + (self.current_repeat as u64
                * self
                    .notes
                    .iter()
                    .map(|e| (e.duration_beats * samples_per_beat as f64) as u64)
                    .sum::<u64>())
            + self.notes[..self.current_note_index]
                .iter()
                .map(|e| (e.duration_beats * samples_per_beat as f64) as u64)
                .sum::<u64>();

        // Process notes
        while self.current_repeat < self.repeat_count {
            while self.current_note_index < self.notes.len() {
                if current_time > sample_time {
                    return;
                }

                let event = &self.notes[self.current_note_index];

                if event.note == PatternEvent::NOTE_OFF_NOTE {
                    // Just stop the previous note, if any
                    if let Some(prev_note_id) = self.note_id {
                        context.note_off(prev_note_id, current_time);
                    }
                } else if event.glide.is_none() || self.note_id.is_none() {
                    // Play a new note with all optional parameters
                    let note_id =
                        context.note_on(event.note, event.volume, event.panning, current_time);

                    // Stop previous note if any
                    if let Some(prev_note_id) = self.note_id {
                        context.note_off(prev_note_id, current_time);
                    }

                    self.note_id = Some(note_id);
                } else {
                    // Modify existing playback (glide mode)
                    if let Some(note_id) = self.note_id {
                        context.set_speed(
                            note_id,
                            speed_from_note(event.note),
                            event.glide,
                            current_time,
                        );
                        if let Some(vol) = event.volume {
                            context.set_volume(note_id, vol, current_time);
                        }
                        if let Some(pan) = event.panning {
                            context.set_panning(note_id, pan, current_time);
                        }
                    }
                }

                let duration_samples = (event.duration_beats * samples_per_beat as f64) as u64;
                current_time += duration_samples;
                self.current_note_index += 1;
            }

            self.current_repeat += 1;
            self.current_note_index = 0;
        }

        self.finished = true;
    }

    fn is_exhausted(&self) -> bool {
        self.finished
    }

    fn reset(&mut self, start_time: u64) {
        self.start_time = start_time;
        self.current_repeat = 0;
        self.current_note_index = 0;
        self.finished = false;
        self.note_id = None;
    }
}
