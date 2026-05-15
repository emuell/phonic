//! MIDI file sequencer - plays a Standard MIDI File through a [`SequencerEventSink`] target.

use std::{collections::HashMap, path::Path};

use super::{Sequencer, SequencerEventSink};
use crate::{Error, NotePlaybackId, Transport};

// -------------------------------------------------------------------------------------------------

/// A sequencer that reads a Standard MIDI File and emits note events into a [`SequencerEventSink`]
/// target.
///
/// Multi-track (Type 1) files are supported: tracks are merged into a single event list.
///
/// Polyphony is tracked per `(channel, note)` pair so the same note number on different channels
/// does not cancel each other.
pub struct MidiFile {
    // Parsed tick-domain data (never changes after construction)
    ticks_per_beat: u64,
    raw_events: Vec<MidiFileEvent>,
    tempo_map: Vec<TempoChange>,
    // Timing configuration
    sample_rate: Option<u32>,
    transport: Option<Transport>,
    // Playback state
    start_time: u64,
    cursor: usize,
    active_notes: HashMap<(u8, u8), NotePlaybackId>,
    finished: bool,
}

impl MidiFile {
    /// Create a `MidiFile` sequencer by reading a `.mid` file from disk.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, Error> {
        let bytes = std::fs::read(path)?;
        Self::from_bytes(&bytes)
    }

    /// Create a `MidiFile` sequencer from raw MIDI bytes in memory.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        let (ticks_per_beat, raw_events, tempo_map) = parse_midi_events(bytes)?;
        Ok(Self {
            ticks_per_beat,
            raw_events,
            tempo_map,
            sample_rate: None,
            transport: None,
            start_time: 0,
            cursor: 0,
            active_notes: HashMap::with_capacity(128),
            finished: false,
        })
    }

    /// Set the sample rate for MIDI-native tempo playback.
    ///
    /// BPM is taken from the file's own `Set Tempo` events. This is the default mode when no
    /// transport override is set.
    ///
    /// Not needed when added to a player - the player provides its own transport via
    /// [`set_transport`](Sequencer::set_transport).
    pub fn with_sample_rate(mut self, sample_rate: u32) -> Self {
        debug_assert!(
            self.transport.is_none(),
            "No need to configure sample rate when a transport got set"
        );
        self.sample_rate = Some(sample_rate);
        self.compute_sample_offsets();
        self
    }

    /// Override the BPM with a fixed [`Transport`].
    ///
    /// When set, the file's own `Set Tempo` events are ignored and the transport's
    /// `beats_per_minute` is used uniformly for all events.
    ///
    /// Not needed when added to a player - the player provides its own transport via
    /// [`set_transport`](Sequencer::set_transport).
    pub fn with_transport(mut self, transport: Transport) -> Self {
        self.transport = Some(transport);
        self.sample_rate = None;
        self.compute_sample_offsets();
        self
    }

    /// Set the absolute sample-time at which playback should start.
    ///
    /// Defaults to `0`. Not needed when the sequencer is driven by a player.
    pub fn with_start_time(mut self, start_time: u64) -> Self {
        self.start_time = start_time;
        self
    }

    /// The initial tempo from the file's own `Set Tempo` events
    ///
    /// Note that setting a transport via `with_transport` will override the MIDI files' tempo map.
    pub fn bpm(&self) -> Option<f64> {
        self.tempo_map
            .first()
            .map(|t| 60_000_000.0 / t.micros_per_beat as f64)
    }

    /// Total duration of the MIDI file as a [`std::time::Duration`].
    ///
    /// This is computed directly from tick offsets and the tempo map, so it is valid regardless
    /// of whether a sample rate or transport has been set.
    ///
    /// When a BPM override transport is set via [`with_transport`](Self::with_transport), the
    /// transport's BPM is used. Otherwise the file's own `Set Tempo` events are used.
    pub fn duration(&self) -> std::time::Duration {
        let last_tick = self.raw_events.last().map_or(0, |e| e.tick_offset);
        if last_tick == 0 || self.ticks_per_beat == 0 {
            return std::time::Duration::ZERO;
        }
        let total_micros = if let Some(transport) = self.transport {
            // BPM override: uniform tempo throughout
            let micros_per_beat = (60_000_000.0 / transport.beats_per_minute()) as u64;
            last_tick * micros_per_beat / self.ticks_per_beat
        } else {
            // MIDI-native: walk the tempo map and accumulate microseconds per segment
            let mut total = 0u64;
            let mut prev_tick = 0u64;
            let mut micros_per_beat = 500_000u64; // 120 BPM default
            for change in &self.tempo_map {
                if change.tick_offset >= last_tick {
                    break;
                }
                let segment_ticks = change.tick_offset - prev_tick;
                total += segment_ticks * micros_per_beat / self.ticks_per_beat;
                prev_tick = change.tick_offset;
                micros_per_beat = change.micros_per_beat;
            }
            // Remaining ticks after the last tempo change
            total += (last_tick - prev_tick) * micros_per_beat / self.ticks_per_beat;
            total
        };
        std::time::Duration::from_micros(total_micros)
    }

    /// Total duration of the MIDI file in samples (offset of the last event from t=0).
    ///
    /// Returns `0` if the sample rate / transport has not been configured yet.
    pub fn duration_samples(&self) -> u64 {
        self.raw_events.last().map_or(0, |e| e.sample_offset)
    }

    // Recompute `sample_offset` for every event. Uses the BPM override transport when set,
    // otherwise uses the file's own tempo map together with the stored sample_rate.
    fn compute_sample_offsets(&mut self) {
        if let Some(transport) = self.transport {
            self.compute_offsets_with_bpm(transport.sample_rate(), transport.beats_per_minute());
        } else if let Some(sample_rate) = self.sample_rate {
            self.compute_offsets_midi_tempo(sample_rate);
        }
    }

    fn compute_offsets_with_bpm(&mut self, sample_rate: u32, bpm: f64) {
        if self.ticks_per_beat == 0 {
            return;
        }
        let micros_per_beat = (60_000_000.0 / bpm) as u64;
        for event in &mut self.raw_events {
            event.sample_offset = ticks_to_samples(
                event.tick_offset,
                self.ticks_per_beat,
                micros_per_beat,
                sample_rate,
            );
        }
    }

    fn compute_offsets_midi_tempo(&mut self, sample_rate: u32) {
        if self.ticks_per_beat == 0 {
            return;
        }
        let mut tempo_idx = 0usize;
        let mut current_micros_per_beat = 500_000u64; // 120 BPM default
        let mut tempo_start_tick = 0u64;
        let mut tempo_start_sample = 0u64;

        for event in &mut self.raw_events {
            // Advance tempo map to the latest change at or before this event's tick
            while tempo_idx < self.tempo_map.len()
                && self.tempo_map[tempo_idx].tick_offset <= event.tick_offset
            {
                let change = &self.tempo_map[tempo_idx];
                let delta_ticks = change.tick_offset - tempo_start_tick;
                tempo_start_sample += ticks_to_samples(
                    delta_ticks,
                    self.ticks_per_beat,
                    current_micros_per_beat,
                    sample_rate,
                );
                tempo_start_tick = change.tick_offset;
                current_micros_per_beat = change.micros_per_beat;
                tempo_idx += 1;
            }
            let delta_ticks = event.tick_offset - tempo_start_tick;
            event.sample_offset = tempo_start_sample
                + ticks_to_samples(
                    delta_ticks,
                    self.ticks_per_beat,
                    current_micros_per_beat,
                    sample_rate,
                );
        }
    }

    // Advance the cursor to the first event whose abs_time > current_sample_time.
    fn sync_cursor(&mut self, current_sample_time: u64) {
        if current_sample_time <= self.start_time {
            self.cursor = 0;
            return;
        }
        let elapsed = current_sample_time - self.start_time;
        self.cursor = self
            .raw_events
            .partition_point(|e| e.sample_offset <= elapsed);
    }
}

#[inline]
fn ticks_to_samples(
    ticks: u64,
    ticks_per_beat: u64,
    micros_per_beat: u64,
    sample_rate: u32,
) -> u64 {
    ticks * sample_rate as u64 * micros_per_beat / (ticks_per_beat * 1_000_000)
}

// -------------------------------------------------------------------------------------------------

impl Sequencer for MidiFile {
    fn is_playing(&self) -> bool {
        !self.finished
    }

    fn set_transport(&mut self, transport: Transport, current_sample_time: u64) {
        // Player always drives with BPM override; switch out of MIDI native mode if needed
        self.transport = Some(transport);
        self.sample_rate = None;
        self.compute_sample_offsets();
        self.active_notes.clear();
        self.finished = false;
        self.sync_cursor(current_sample_time);
    }

    fn run_until(&mut self, sample_time: u64, event_sink: &mut dyn SequencerEventSink) {
        if self.finished {
            return;
        }
        // Can't run without timing information
        if self.transport.is_none() && self.sample_rate.is_none() {
            return;
        }

        while self.cursor < self.raw_events.len() {
            let event = &self.raw_events[self.cursor];
            let abs_time = self.start_time.saturating_add(event.sample_offset);
            if abs_time > sample_time {
                break;
            }

            match event.kind {
                MidiFileEventKind::NoteOn {
                    channel,
                    note,
                    volume,
                } => {
                    if let Some(prev_id) = self.active_notes.remove(&(channel, note)) {
                        event_sink.note_off(prev_id, abs_time);
                    }
                    if let Some(note_id) = event_sink.note_on(note, Some(volume), None, abs_time) {
                        self.active_notes.insert((channel, note), note_id);
                    }
                }
                MidiFileEventKind::NoteOff { channel, note } => {
                    if let Some(note_id) = self.active_notes.remove(&(channel, note)) {
                        event_sink.note_off(note_id, abs_time);
                    }
                }
            }

            self.cursor += 1;
        }

        if self.cursor >= self.raw_events.len() {
            let last_time = self.raw_events.last().map_or(self.start_time, |e| {
                self.start_time.saturating_add(e.sample_offset)
            });
            for (_, note_id) in self.active_notes.drain() {
                event_sink.note_off(note_id, last_time);
            }
            self.finished = true;
        }
    }

    fn reset(&mut self, start_time: u64) {
        self.start_time = start_time;
        self.cursor = 0;
        self.finished = false;
        self.active_notes.clear();
    }

    fn stop(&mut self, sample_time: u64, event_sink: &mut dyn SequencerEventSink) {
        for (_, note_id) in self.active_notes.drain() {
            event_sink.note_off(note_id, sample_time);
        }
        self.finished = true;
    }
}

// -------------------------------------------------------------------------------------------------

struct MidiFileEvent {
    tick_offset: u64,
    sample_offset: u64,
    kind: MidiFileEventKind,
}

// -------------------------------------------------------------------------------------------------

enum MidiFileEventKind {
    NoteOn { channel: u8, note: u8, volume: f32 },
    NoteOff { channel: u8, note: u8 },
}

// -------------------------------------------------------------------------------------------------

struct TempoChange {
    tick_offset: u64,
    micros_per_beat: u64,
}

// -------------------------------------------------------------------------------------------------

fn parse_midi_events(bytes: &[u8]) -> Result<(u64, Vec<MidiFileEvent>, Vec<TempoChange>), Error> {
    use midly::{MetaMessage, MidiMessage, Smf, Timing, TrackEventKind};

    let smf = Smf::parse(bytes)
        .map_err(|e| Error::MidiDecodingError(format!("MIDI parse error: {e}").into()))?;

    let ticks_per_beat: u64 = match smf.header.timing {
        Timing::Metrical(tpb) => tpb.as_int() as u64,
        Timing::Timecode(fps, sub) => {
            return Err(Error::MidiDecodingError(
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
    let mut tempo_map: Vec<TempoChange> = Vec::new();

    for track in &smf.tracks {
        let mut current_tick: u64 = 0;
        let mut active: HashMap<(u8, u8), ()> = HashMap::new();

        for event in track {
            current_tick += event.delta.as_int() as u64;

            match event.kind {
                TrackEventKind::Meta(MetaMessage::Tempo(tempo)) => {
                    let micros = tempo.as_int() as u64;
                    // Only record if not already in the map at this tick
                    if tempo_map
                        .last()
                        .is_none_or(|t| t.tick_offset != current_tick)
                    {
                        tempo_map.push(TempoChange {
                            tick_offset: current_tick,
                            micros_per_beat: micros,
                        });
                    } else if let Some(last) = tempo_map.last_mut() {
                        last.micros_per_beat = micros;
                    }
                }
                TrackEventKind::Meta(MetaMessage::EndOfTrack) => {
                    for ((ch, note), _) in active.drain() {
                        all_events.push(MidiFileEvent {
                            tick_offset: current_tick,
                            sample_offset: 0,
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
                                active.remove(&(ch, note));
                                all_events.push(MidiFileEvent {
                                    tick_offset: current_tick,
                                    sample_offset: 0,
                                    kind: MidiFileEventKind::NoteOff { channel: ch, note },
                                });
                            } else {
                                let volume = vel.as_int() as f32 / 127.0;
                                active.insert((ch, note), ());
                                all_events.push(MidiFileEvent {
                                    tick_offset: current_tick,
                                    sample_offset: 0,
                                    kind: MidiFileEventKind::NoteOn {
                                        channel: ch,
                                        note,
                                        volume,
                                    },
                                });
                            }
                        }
                        MidiMessage::NoteOff { key, .. } => {
                            let note = key.as_int();
                            active.remove(&(ch, note));
                            all_events.push(MidiFileEvent {
                                tick_offset: current_tick,
                                sample_offset: 0,
                                kind: MidiFileEventKind::NoteOff { channel: ch, note },
                            });
                        }
                        // CC 120 (All Sound Off) and CC 123 (All Notes Off)
                        MidiMessage::Controller { controller, .. }
                            if controller.as_int() == 120 || controller.as_int() == 123 =>
                        {
                            let keys: Vec<(u8, u8)> =
                                active.keys().filter(|(c, _)| *c == ch).copied().collect();
                            for (c, note) in keys {
                                active.remove(&(c, note));
                                all_events.push(MidiFileEvent {
                                    tick_offset: current_tick,
                                    sample_offset: 0,
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

        for ((ch, note), _) in active.drain() {
            all_events.push(MidiFileEvent {
                tick_offset: current_tick,
                sample_offset: 0,
                kind: MidiFileEventKind::NoteOff { channel: ch, note },
            });
        }
    }

    // Merge tracks by stable-sorting on tick_offset
    all_events.sort_by_key(|e| e.tick_offset);

    // Sort tempo map
    tempo_map.sort_by_key(|t| t.tick_offset);

    // Verify NoteOn/NoteOff balance
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

    Ok((ticks_per_beat, all_events, tempo_map))
}
