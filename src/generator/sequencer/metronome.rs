//! Metronome sequencer implementation.

use super::{
    pattern::{Pattern, PatternEvent},
    Sequencer, SequencerPlayback, SequencerTransport,
};

// -------------------------------------------------------------------------------------------------

/// A simple metronome sequencer pattern
pub struct Metronome {
    pattern: Pattern,
}

impl Metronome {
    /// Create a new metronome sequencer that plays for the given number of bars.
    pub fn new(bar_count: usize, start_time: u64, transport: SequencerTransport) -> Self {
        let beats_per_bar = transport.beats_per_bar;

        // Create a pattern with accent on first beat of each bar
        let notes = (0..beats_per_bar)
            .map(|beat| {
                let is_accent = beat % beats_per_bar == 0;
                let volume = if is_accent { 1.0 } else { 0.7 };
                let note = if is_accent { 72 } else { 60 };
                PatternEvent::note_on(note, 1.0).volume(volume)
            })
            .collect::<Vec<_>>();

        Self {
            pattern: Pattern::new(notes, start_time, bar_count, transport),
        }
    }
}

impl Sequencer for Metronome {
    fn run_until(&mut self, sample_time: u64, context: &mut dyn SequencerPlayback) {
        self.pattern.run_until(sample_time, context);
    }

    fn is_exhausted(&self) -> bool {
        self.pattern.is_exhausted()
    }

    fn reset(&mut self, start_time: u64) {
        self.pattern.reset(start_time);
    }
}
