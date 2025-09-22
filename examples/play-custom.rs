//! An example showcasing how to create and use custom effects and synth sources.

use std::{any::Any, f32::consts::PI, time::Duration};

use phonic::{
    effects::{CompressorEffect, ReverbEffect},
    sources::{PreloadedFileSource, SynthSourceGenerator, SynthSourceImpl},
    utils::{pitch_from_note, speed_from_note, InterleavedBufferMut},
    Effect, EffectMessage, EffectMessagePayload, EffectTime, Error, FilePlaybackOptions,
    SynthPlaybackOptions,
};

// -------------------------------------------------------------------------------------------------

#[cfg(all(debug_assertions, feature = "assert_no_alloc"))]
#[global_allocator]
static A: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

// -------------------------------------------------------------------------------------------------

// Common example code
#[path = "./common/arguments.rs"]
mod arguments;

// -------------------------------------------------------------------------------------------------

/// Message enum for our custom TanhDistortion effect.
#[derive(Clone, Debug)]
#[allow(unused)]
enum TanhDistortionMessage {
    /// Set the distortion gain. Range: 0.0 to 1.0.
    SetGain(f32),
}

impl EffectMessage for TanhDistortionMessage {
    fn effect_name(&self) -> &'static str {
        TanhDistortion::name()
    }
    fn payload(&self) -> &dyn Any {
        self
    }
}

// -------------------------------------------------------------------------------------------------

/// A simple distortion [`Effect`] that uses the `tanh` function for waveshaping.
#[derive(Clone)]
struct TanhDistortion {
    channel_count: usize,
    gain: f32,
}

impl TanhDistortion {
    const DEFAULT_GAIN: f32 = 0.7;

    fn with_parameters(gain: f32) -> Self {
        Self {
            channel_count: 0,
            gain,
        }
    }
}

impl Default for TanhDistortion {
    fn default() -> Self {
        Self::with_parameters(Self::DEFAULT_GAIN)
    }
}

impl Effect for TanhDistortion {
    fn name() -> &'static str {
        "TanhDistortion"
    }

    fn initialize(
        &mut self,
        _sample_rate: u32,
        channel_count: usize,
        _max_frames: usize,
    ) -> Result<(), Error> {
        self.channel_count = channel_count;
        Ok(())
    }

    fn process(&mut self, mut output: &mut [f32], _time: &EffectTime) {
        // Map gain (0-1) to a drive amount.
        let drive = 1.0 + self.gain * 15.0;

        for frame in output.frames_mut(self.channel_count) {
            for sample in frame {
                let dry = *sample;
                // Apply drive, waveshape, and apply makeup gain.
                let wet = (dry * drive).tanh() * 0.5;
                // Linearly interpolate between dry and wet signal based on gain.
                *sample = dry * (1.0 - self.gain) + wet * self.gain;
            }
        }
    }

    fn process_message(&mut self, message: &EffectMessagePayload) {
        if let Some(message) = message.payload().downcast_ref::<TanhDistortionMessage>() {
            match message {
                TanhDistortionMessage::SetGain(gain) => {
                    self.gain = (*gain).clamp(0.0, 1.0);
                }
            }
        } else {
            log::error!("TanhDistortion: Invalid/unknown message payload");
        }
    }
}

// -------------------------------------------------------------------------------------------------

/// A simple sine wave oscillator.
struct SineOsc {
    phase: f32,
    phase_inc: f32,
}

impl SineOsc {
    fn new(freq: f32, sample_rate: u32) -> Self {
        Self {
            phase: PI,
            phase_inc: 2.0 * PI * freq / sample_rate as f32,
        }
    }

    fn next(&mut self) -> f32 {
        let value = self.phase.sin();
        self.phase += self.phase_inc;
        if self.phase >= 2.0 * PI {
            self.phase -= 2.0 * PI;
        }
        value
    }
}

// -------------------------------------------------------------------------------------------------

/// A custom synth source, using two detuned sine oscillators.
struct SineSynth {
    osc1: SineOsc,
    osc2: SineOsc,
    samples_left: usize,
    total_samples: usize,
}

impl SineSynth {
    fn new(note: u8, duration_samples: usize, sample_rate: u32) -> Self {
        let freq = pitch_from_note(note) as f32;
        Self {
            osc1: SineOsc::new(freq, sample_rate),
            osc2: SineOsc::new(freq * 1.02, sample_rate), // Slightly detuned
            samples_left: duration_samples,
            total_samples: duration_samples,
        }
    }
}

impl SynthSourceGenerator for SineSynth {
    fn is_exhausted(&self) -> bool {
        self.samples_left == 0
    }
    fn generate(&mut self, output: &mut [f32]) -> usize {
        let num_frames = std::cmp::min(output.len(), self.samples_left);
        for sample in output.iter_mut().take(num_frames) {
            let osc_mix = (self.osc1.next() + self.osc2.next()) * 0.5;
            let envelope = self.samples_left as f32 / self.total_samples as f32;
            *sample = osc_mix * envelope * 0.5; // Adjust volume (simple linear fade-out)
            self.samples_left -= 1;
        }
        num_frames
    }
}

// -------------------------------------------------------------------------------------------------

/// A [`SynthSource`] which runs a custom `SinSynth`` generator until it is exhausted.
type SinSynthSource = SynthSourceImpl<SineSynth>;

// -------------------------------------------------------------------------------------------------

fn main() -> Result<(), Error> {
    // Parse optional arguments
    let args = arguments::parse();

    // Create a player instance with the output device as configured via program arguments
    let mut player = arguments::new_player(&args, None)?;

    // Stop the player until we've scheduled all sources
    player.stop();

    // Create a sub-mixer for the synth, child of the main mixer.
    let bass_mixer_id = player.add_mixer(None)?;
    player.add_effect(TanhDistortion::with_parameters(0.9), bass_mixer_id)?;

    // Create a sub-mixer for the pad, with a high-pass filter.
    let pad_mixer_id = player.add_mixer(None)?;
    player.add_effect(ReverbEffect::with_parameters(0.4, 0.6), pad_mixer_id)?;

    // Add a limiter to the main mixer
    player.add_effect(CompressorEffect::default_limiter(), None)?;

    // Sequencing
    const BPM: f64 = 160.0; // BPM of the loop file
    const BARS_TO_PLAY: usize = 4;
    const NOTE_DURATION_IN_BEATS: f64 = 0.9;

    // A 4-step pad line, one note per bar
    const PAD_LINE: [u8; BARS_TO_PLAY] = [55, 58, 51, 53];
    // A 16-step bass line, one note per beat, 0 is a note off
    const BASS_LINE: [u8; 16] = [36, 0, 36, 34, 31, 0, 34, 31, 29, 0, 29, 24, 31, 36, 34, 31];

    let samples_per_sec = player.output_sample_rate();
    let samples_per_beat = (60.0 / BPM * samples_per_sec as f64) as u64;
    let note_duration_samples = (samples_per_beat as f64 * NOTE_DURATION_IN_BEATS) as usize;

    // Preload sample files
    let drumloop = PreloadedFileSource::from_file(
        "assets/YuaiLoop.wav",
        None,
        FilePlaybackOptions::default(),
        samples_per_sec,
    )?;
    let pad = PreloadedFileSource::from_file(
        "assets/pad-ambient.wav",
        None,
        FilePlaybackOptions::default(),
        samples_per_sec,
    )?;

    // Schedule bassline and pad notes for all bars
    let output_start_time = player.output_sample_frame_position();
    #[allow(clippy::needless_range_loop)]
    for bar in 0..BARS_TO_PLAY {
        // Drum loop (on the main mixer)
        if bar == 0 {
            player.play_file_source(
                drumloop.clone(
                    FilePlaybackOptions::default()
                        .repeat(BARS_TO_PLAY / 2)
                        .volume_db(0.0),
                    samples_per_sec,
                )?,
                Some(output_start_time),
            )?;
        }

        // Pad (on the pad mixer)
        let pad_note = PAD_LINE[bar];
        let pad_start_time = output_start_time + (bar * BASS_LINE.len()) as u64 * samples_per_beat;
        let pad_id = player.play_file_source(
            pad.clone(
                FilePlaybackOptions::default()
                    .speed(speed_from_note(pad_note))
                    .volume_db(0.0)
                    .fade_out(Duration::from_millis(500))
                    .target_mixer(pad_mixer_id),
                samples_per_sec,
            )?,
            Some(pad_start_time),
        )?;
        let pad_stop_time = pad_start_time + 16 * samples_per_beat;
        player.stop_source(pad_id, pad_stop_time)?;

        // Bass line (on the bass line mixer)
        for (beat, note) in BASS_LINE.iter().enumerate().filter(|(_, n)| *n != &0) {
            let beat_in_loop = (bar * BASS_LINE.len() + beat) as u64;
            let sample_time = output_start_time + beat_in_loop * samples_per_beat;

            // Create our custom synth source for the current note
            let bass = SinSynthSource::new(
                SineSynth::new(*note, note_duration_samples, samples_per_sec),
                "sin_synth",
                SynthPlaybackOptions::default()
                    .volume_db(-5.0)
                    .target_mixer(bass_mixer_id),
                samples_per_sec,
                None,
            )?;
            player.play_synth_source(bass, Some(sample_time))?;
        }
    }

    // start playback
    player.start();

    // Print DSP graph
    println!("Playing a {BARS_TO_PLAY} bar bass line loop over a drum loop and pad sequence...");
    println!("DSP Graph:");
    println!("  - Drum Loop -> Main Mixer");
    println!("  - Pad -> Pad Mixer (HP Filter) -> Main Mixer");
    println!("  - SineSynth -> Synth Mixer (TanhDistortion -> Reverb) -> Main Mixer");

    // Wait for playback to finish
    let total_beats = (BASS_LINE.len() * BARS_TO_PLAY) as u64 + 1; // one extra beat as tail;
    let duration_samples = total_beats * samples_per_beat;
    while player.is_running()
        && player.output_sample_frame_position() < output_start_time + duration_samples
    {
        std::thread::sleep(Duration::from_millis(500));
    }

    println!("Playback finished.");
    Ok(())
}
