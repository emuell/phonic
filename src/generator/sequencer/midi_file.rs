//! MIDI file sequencer - plays a Standard MIDI File through a [`SequencerEventSink`] target.

use std::{collections::HashMap, path::Path};

use crate::{Error, NotePlaybackId};

use super::{Sequencer, SequencerEventSink};

// -------------------------------------------------------------------------------------------------

struct MidiFileEvent {
    sample_offset: u64,
    kind: MidiFileEventKind,
}

enum MidiFileEventKind {
    NoteOn { channel: u8, note: u8, volume: f32 },
    NoteOff { channel: u8, note: u8 },
}

// -------------------------------------------------------------------------------------------------

/// A sequencer that reads a Standard MIDI File and emits note events into a [`SequencerEventSink`]
/// target.
///
/// Create via [`MidiFile::from_path`] or [`MidiFile::from_bytes`], then drive it the same way as
/// any other [`Sequencer`]: call [`run_until`](Sequencer::run_until) periodically from a background
/// thread, or once with [`u64::MAX`] to pre-schedule all events upfront.
///
/// Tempo change events (`Set Tempo` meta-messages) are fully supported and applied during
/// construction, so the timing of the pre-computed events is always accurate.
///
/// Multi-track (Type 1) files are supported: tracks are merged into a single, sorted event list.
/// Polyphony is tracked per `(channel, note)` pair so the same note number on different channels
/// does not cancel each other.
pub struct MidiFile {
    events: Vec<MidiFileEvent>,
    cursor: usize,
    base_sample_time: u64,
    active_notes: HashMap<(u8, u8), NotePlaybackId>,
    finished: bool,
}

impl MidiFile {
    /// Create a `MidiFile` sequencer by reading a `.mid` file from disk.
    pub fn from_path(
        path: impl AsRef<Path>,
        start_time: u64,
        sample_rate: u32,
    ) -> Result<Self, Error> {
        let bytes = std::fs::read(path)?;
        Self::from_bytes(&bytes, start_time, sample_rate)
    }

    /// Create a `MidiFile` sequencer from raw MIDI bytes already in memory.
    pub fn from_bytes(bytes: &[u8], start_time: u64, sample_rate: u32) -> Result<Self, Error> {
        let events = parse_midi_events(bytes, sample_rate)?;
        Ok(Self {
            events,
            cursor: 0,
            base_sample_time: start_time,
            active_notes: HashMap::new(),
            finished: false,
        })
    }

    /// Total duration of the MIDI file in samples (offset of the last event from t=0).
    pub fn duration_samples(&self) -> u64 {
        self.events.last().map_or(0, |e| e.sample_offset)
    }
}

// -------------------------------------------------------------------------------------------------

impl Sequencer for MidiFile {
    fn run_until(&mut self, sample_time: u64, event_sink: &mut dyn SequencerEventSink) {
        if self.finished {
            return;
        }

        while self.cursor < self.events.len() {
            let event = &self.events[self.cursor];
            let abs_time = self.base_sample_time.saturating_add(event.sample_offset);
            if abs_time > sample_time {
                break;
            }

            match event.kind {
                MidiFileEventKind::NoteOn { channel, note, volume } => {
                    // Stop any currently playing instance of this (channel, note) first
                    if let Some(prev_id) = self.active_notes.remove(&(channel, note)) {
                        event_sink.note_off(prev_id, abs_time);
                    }
                    let note_id = event_sink.note_on(note, Some(volume), None, abs_time);
                    self.active_notes.insert((channel, note), note_id);
                }
                MidiFileEventKind::NoteOff { channel, note } => {
                    if let Some(note_id) = self.active_notes.remove(&(channel, note)) {
                        event_sink.note_off(note_id, abs_time);
                    }
                }
            }

            self.cursor += 1;
        }

        if self.cursor >= self.events.len() {
            // Stop any notes still ringing - MIDI end-of-track implies all notes off
            let last_time = self
                .events
                .last()
                .map_or(self.base_sample_time, |e| {
                    self.base_sample_time.saturating_add(e.sample_offset)
                });
            for (_, note_id) in self.active_notes.drain() {
                event_sink.note_off(note_id, last_time);
            }
            self.finished = true;
        }
    }

    fn is_exhausted(&self) -> bool {
        self.finished
    }

    fn reset(&mut self, start_time: u64) {
        self.base_sample_time = start_time;
        self.cursor = 0;
        self.finished = false;
        self.active_notes.clear();
    }
}

// -------------------------------------------------------------------------------------------------

fn parse_midi_events(bytes: &[u8], sample_rate: u32) -> Result<Vec<MidiFileEvent>, Error> {
    use midly::{MetaMessage, MidiMessage, Smf, Timing, TrackEventKind};

    let smf = Smf::parse(bytes)
        .map_err(|e| Error::AudioDecodingError(format!("MIDI parse error: {e}").into()))?;

    let ticks_per_beat: u64 = match smf.header.timing {
        Timing::Metrical(tpb) => tpb.as_int() as u64,
        Timing::Timecode(fps, sub) => {
            return Err(Error::AudioDecodingError(
                format!(
                    "MIDI timecode timing ({} fps, {} subdivisions) is not supported",
                    fps.as_f32(),
                    sub
                )
                .into(),
            ));
        }
    };

    let mut all_events: Vec<MidiFileEvent> = Vec::new();

    for track in &smf.tracks {
        let mut current_sample: u64 = 0;
        let mut current_tempo: u64 = 500_000; // microseconds per beat (= 120 BPM)
        // Track active notes within this track to emit implicit NoteOffs when needed.
        // Key is (channel, note); presence means the note is currently sounding.
        let mut active: HashMap<(u8, u8), ()> = HashMap::new();

        for event in track {
            let delta_ticks = event.delta.as_int() as u64;

            // Convert delta ticks → samples:
            // delta_samples = delta_ticks * sample_rate * tempo / (ticks_per_beat * 1_000_000)
            if ticks_per_beat > 0 {
                let delta_samples =
                    delta_ticks * sample_rate as u64 * current_tempo / (ticks_per_beat * 1_000_000);
                current_sample = current_sample.saturating_add(delta_samples);
            }

            match event.kind {
                TrackEventKind::Meta(MetaMessage::Tempo(tempo)) => {
                    current_tempo = tempo.as_int() as u64;
                }
                // End-of-track: stop any notes that were never explicitly turned off
                TrackEventKind::Meta(MetaMessage::EndOfTrack) => {
                    for ((ch, note), _) in active.drain() {
                        all_events.push(MidiFileEvent {
                            sample_offset: current_sample,
                            kind: MidiFileEventKind::NoteOff { channel: ch, note },
                        });
                    }
                }
                TrackEventKind::Midi { channel, message } => {
                    let ch = channel.as_int();
                    match message {
                        MidiMessage::NoteOn { key, vel } => {
                            let note = key.as_int();
                            if vel.as_int() == 0 {
                                // NoteOn with vel=0 is the standard "running status" NoteOff
                                active.remove(&(ch, note));
                                all_events.push(MidiFileEvent {
                                    sample_offset: current_sample,
                                    kind: MidiFileEventKind::NoteOff { channel: ch, note },
                                });
                            } else {
                                let volume = vel.as_int() as f32 / 127.0;
                                active.insert((ch, note), ());
                                all_events.push(MidiFileEvent {
                                    sample_offset: current_sample,
                                    kind: MidiFileEventKind::NoteOn { channel: ch, note, volume },
                                });
                            }
                        }
                        MidiMessage::NoteOff { key, .. } => {
                            let note = key.as_int();
                            active.remove(&(ch, note));
                            all_events.push(MidiFileEvent {
                                sample_offset: current_sample,
                                kind: MidiFileEventKind::NoteOff { channel: ch, note },
                            });
                        }
                        // CC 120 (All Sound Off) and CC 123 (All Notes Off) - both used by
                        // many sequencers to stop sounding notes instead of individual NoteOffs
                        MidiMessage::Controller { controller, .. }
                            if controller.as_int() == 120 || controller.as_int() == 123 =>
                        {
                            let keys: Vec<(u8, u8)> = active
                                .keys()
                                .filter(|(c, _)| *c == ch)
                                .copied()
                                .collect();
                            for (c, note) in keys {
                                active.remove(&(c, note));
                                all_events.push(MidiFileEvent {
                                    sample_offset: current_sample,
                                    kind: MidiFileEventKind::NoteOff { channel: c, note },
                                });
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        // Safety net: stop any notes still active if there was no EndOfTrack meta event
        for ((ch, note), _) in active.drain() {
            all_events.push(MidiFileEvent {
                sample_offset: current_sample,
                kind: MidiFileEventKind::NoteOff { channel: ch, note },
            });
        }
    }

    // Merge tracks by stable-sorting on sample_offset so relative event order within a
    // track is preserved when two events share the same absolute time.
    all_events.sort_by_key(|e| e.sample_offset);

    // Verify NoteOn/NoteOff balance - every NoteOn must have a matching NoteOff.
    let mut balance: HashMap<(u8, u8), i32> = HashMap::new();
    for event in &all_events {
        match event.kind {
            MidiFileEventKind::NoteOn { channel, note, .. } => {
                *balance.entry((channel, note)).or_insert(0) += 1;
            }
            MidiFileEventKind::NoteOff { channel, note } => {
                *balance.entry((channel, note)).or_insert(0) -= 1;
            }
        }
    }
    for ((ch, note), delta) in &balance {
        if *delta != 0 {
            log::warn!(
                "MidiFile: unbalanced note events: channel={ch} note={note} delta={delta} \
                (positive=extra NoteOn, negative=extra NoteOff)"
            );
        }
    }

    Ok(all_events)
}
