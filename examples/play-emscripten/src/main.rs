//! An example showcasing how to use phonic with emscripten to create a web-based audio application.

use std::{cell::RefCell, collections::HashMap, ffi};

use dasp::{signal, Frame, Signal};

use emscripten_rs_sys::emscripten_request_animation_frame_loop;

use phonic::{
    effects::ReverbEffect,
    sources::{DaspSynthSource, PreloadedFileSource},
    utils::{db_to_linear, pitch_from_note, speed_from_note},
    DefaultOutputDevice, Error, FilePlaybackOptions, MixerId, PlaybackId, Player,
    SynthPlaybackOptions,
};

// -------------------------------------------------------------------------------------------------

// Hold the data structures statically so we can bind the Emscripten C method callbacks.
thread_local!(static PLAYER: RefCell<Option<EmscriptenPlayer>> = const { RefCell::new(None) });

struct EmscriptenPlayer {
    player: Player,
    playback_beat_counter: u32,
    playback_start_time: u64,
    playing_synth_notes: HashMap<u8, PlaybackId>,
    samples: Vec<PreloadedFileSource>,
    synth_mixer_id: MixerId,
}

impl EmscriptenPlayer {
    // Create a new player and preload samples
    pub fn new() -> Result<Self, Error> {
        println!("Initialize audio output...");
        let output = DefaultOutputDevice::open()?;

        println!("Creating audio file player...");
        let mut player = Player::new(output, None);
        let sample_rate = player.output_sample_rate();

        // lower master volume a bit
        player.set_output_volume(db_to_linear(-3.0));

        // create a new mixer for the synth with a reverb effect
        let synth_mixer_id = player.add_mixer(None)?;
        player.add_effect(ReverbEffect::with_parameters(0.6, 0.5), synth_mixer_id)?;

        println!("Preloading sample files...");
        let mut samples = Vec::new();
        for sample in ["./assets/cowbell.wav", "./assets/bass.wav"] {
            match PreloadedFileSource::from_file(
                sample,
                None,
                FilePlaybackOptions::default(),
                sample_rate,
            ) {
                Ok(sample) => samples.push(sample),
                Err(err) => return Err(err),
            }
        }

        println!("Start running...");
        unsafe {
            emscripten_request_animation_frame_loop(Some(Self::run_frame), std::ptr::null_mut())
        };

        // start playback in a second from now
        let playback_start_time =
            player.output_sample_frame_position() + player.output_sample_rate() as u64;
        let playback_beat_counter = 0;

        let playing_synth_notes = HashMap::new();

        Ok(Self {
            player,
            playback_start_time,
            playback_beat_counter,
            playing_synth_notes,
            samples,
            synth_mixer_id,
        })
    }

    // Animation frame callback which drives the player
    extern "C" fn run_frame(_time: f64, _user_data: *mut ffi::c_void) -> bool {
        PLAYER.with_borrow_mut(|player| {
            // is a player running?
            if let Some(launcher) = player {
                launcher.run();
                true // continue running
            } else {
                false // stop running
            }
        })
    }

    // Create a new synth source for the given note
    fn create_synth_source(
        &self,
        note: u8,
    ) -> Result<DaspSynthSource<impl Signal<Frame = f64>>, Error> {
        let sample_rate = self.player.output_sample_rate();
        let pitch = pitch_from_note(note);
        let duration_in_ms = 1000;
        let duration_in_samples = (sample_rate as f64 / duration_in_ms as f64 * 1000.0) as usize;
        // stack up slightly detuned sine waves
        let fundamental = signal::rate(sample_rate as f64).const_hz(pitch);
        let harmonic_l1 = signal::rate(sample_rate as f64).const_hz(pitch * 2.01);
        let harmonic_h1 = signal::rate(sample_rate as f64).const_hz(pitch / 2.02);
        let harmonic_h2 = signal::rate(sample_rate as f64).const_hz(pitch / 4.04);
        // combine them, limit duration and apply a fade-out envelope
        let signal = signal::from_iter(
            fundamental
                .sine()
                .add_amp(harmonic_l1.sine().scale_amp(0.5))
                .add_amp(harmonic_h1.sine().scale_amp(0.5))
                .add_amp(harmonic_h2.sine().scale_amp(0.5))
                .take(duration_in_samples)
                .zip(0..duration_in_samples)
                .map(move |(s, index)| {
                    let env: f64 = (1.0 - (index as f64) / (duration_in_samples as f64)).powf(2.0);
                    (s * env).to_float_frame()
                }),
        );
        let options = SynthPlaybackOptions::default()
            .volume_db(-6.0)
            .target_mixer(self.synth_mixer_id);
        DaspSynthSource::new(
            signal,
            format!("Synth Note #{}", note).as_str(),
            options,
            sample_rate,
            None,
        )
    }

    // Schedule synth note on for playback
    fn synth_note_on(&mut self, note: u8) {
        if let Some(playback_id) = self.playing_synth_notes.get(&note) {
            let _ = self.player.stop_source(*playback_id, None);
            self.playing_synth_notes.remove(&note);
        }
        let playback_id = self
            .player
            .play_synth_source(self.create_synth_source(note).unwrap(), None)
            .unwrap();
        self.playing_synth_notes.insert(note, playback_id);
    }

    // Stop a scheduled synth note on
    fn synth_note_off(&mut self, note: u8) {
        if let Some(playback_id) = self.playing_synth_notes.get(&note) {
            let _ = self.player.stop_source(*playback_id, None);
            self.playing_synth_notes.remove(&note);
        }
    }

    // Schedule samples for playback
    fn run(&mut self) {
        // time consts
        const BEATS_PER_MIN: f64 = 120.0;
        const BEATS_PER_BAR: u32 = 4;

        // calculate metronome speed and signature
        let sample_rate = self.player.output_sample_rate();
        let samples_per_sec = self.player.output_sample_rate();
        let samples_per_beat = samples_per_sec as f64 * 60.0 / BEATS_PER_MIN;

        // schedule playback events one second ahead of the players current time
        let preroll_time = samples_per_sec as u64;

        // when is the next beat playback due?
        let next_beats_sample_time = (self.playback_start_time as f64
            + self.playback_beat_counter as f64 * samples_per_beat)
            as u64;
        let output_sample_time = self.player.output_sample_frame_position();

        // schedule next sample when it's due within the preroll time, else do nothing
        if next_beats_sample_time > output_sample_time + preroll_time
            || self.playback_beat_counter == 0
        {
            // play an octave higher every new bar start
            let sample_speed = speed_from_note(
                if self.playback_beat_counter.is_multiple_of(BEATS_PER_BAR) {
                    72
                } else {
                    60
                },
            );
            // select a new sample every 2 bars
            let sample_index =
                (self.playback_beat_counter / (2 * BEATS_PER_BAR)) as usize % self.samples.len();
            // clone the preloaded sample
            let sample = self.samples[sample_index]
                .clone(
                    FilePlaybackOptions::default().speed(sample_speed),
                    sample_rate,
                )
                .unwrap();

            // play it at the new beat's time
            let playback_id = self
                .player
                .play_file_source(sample, Some(next_beats_sample_time))
                .unwrap();
            // and stop it again (fade out) before the next beat starts
            self.player
                .stop_source(
                    playback_id,
                    next_beats_sample_time + samples_per_beat as u64,
                )
                .unwrap();

            // advance beat counter
            self.playback_beat_counter += 1;
        }
    }
}

// -------------------------------------------------------------------------------------------------

fn main() {
    // Disabled build.rs via `cargo::rustc-link-arg=--no-entry`
    panic!("The main function is not exposed and should never be called");
}

/// Creates a new `EmscriptenPlayer`
/// Exported as `_start` function in the WASM.
#[no_mangle]
pub extern "C" fn start() {
    // create or recreate the player instance
    println!("Creating new player instance...");
    match EmscriptenPlayer::new() {
        Err(err) => {
            eprintln!("Failed to create player instance: {}", err);
            PLAYER.replace(None)
        }
        Ok(player) => {
            println!("Successfully created a new player instance");
            PLAYER.replace(Some(player))
        }
    };
}

/// Destroys `EmscriptenPlayer` when its running.
/// Exported as `_stop` function in the WASM.
#[no_mangle]
pub extern "C" fn stop() {
    // drop the player instance
    println!("Dropping player instance...");
    PLAYER.replace(None);
}

/// Play a single synth note when the player is running.
/// Exported as `_synth_note_on` function in the WASM.
#[no_mangle]
pub extern "C" fn synth_note_on(key: ffi::c_int) {
    PLAYER.with_borrow_mut(|player| {
        // is a player running?
        if let Some(player) = player {
            let note = (60 + key).min(127) as u8;
            player.synth_note_on(note);
        }
    });
}

/// Stop a previously played synth note when the player is running.
/// Exported as `_synth_note_off` function in the WASM.
#[no_mangle]
pub extern "C" fn synth_note_off(key: ffi::c_int) {
    PLAYER.with_borrow_mut(|player| {
        // is a player running?
        if let Some(player) = player {
            let note = (60 + key).min(127) as u8;
            player.synth_note_off(note);
        }
    });
}

// Note: when adding new functions that should be exported in the WASM,
// adjust `cargo::rustc-link-arg=-sEXPORTED_FUNCTIONS` print in `build.rs`
