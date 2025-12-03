//! An example showcasing how to build a simple sequencer by scheduling preloaded audio samples
//! and using sample glide playback parameters.

use std::time::Duration;

use phonic::{generators::Sampler, utils::speed_from_note, Error};

// Use a fundsp generator, if available, else a sampler
#[cfg(feature = "fundsp")]
use phonic::generators::FunDspGenerator;
#[cfg(not(feature = "fundsp"))]
use phonic::utils::ahdsr::AhdsrParameters;

// -------------------------------------------------------------------------------------------------

#[cfg(all(debug_assertions, feature = "assert-allocs"))]
#[global_allocator]
static A: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

// -------------------------------------------------------------------------------------------------

// Common example code
#[path = "./common/arguments.rs"]
mod arguments;

#[cfg(feature = "fundsp")]
#[path = "./common/synths/bass.rs"]
mod bass_synth;

// -------------------------------------------------------------------------------------------------

fn main() -> Result<(), Error> {
    // Parse optional arguments
    let args = arguments::parse();

    // Create a player instance with the output device as configured via program arguments
    let mut player = arguments::new_player(&args, None)?;

    // Stop playback until we've scheduled all notes
    player.stop();

    // Create metronome sampler
    let metronome = player.play_generator_source(
        Sampler::from_file(
            "assets/cowbell.wav",
            None,
            None,
            None,
            2,
            player.output_channel_count(),
            player.output_sample_rate(),
        )?,
        None,
        None,
    )?;

    // Create bass sampler
    #[cfg(not(feature = "fundsp"))]
    let bass = player.play_generator_source(
        Sampler::from_file(
            "assets/bass.wav",
            Some(AhdsrParameters::new(
                Duration::from_millis(10),
                Duration::ZERO,
                Duration::ZERO,
                1.0,
                Duration::from_secs_f32(2.0),
            )?),
            None,
            None,
            4,
            player.output_channel_count(),
            player.output_sample_rate(),
        )?,
        None,
        None,
    )?;
    // Create a fundsp bass generator FM synth
    #[cfg(feature = "fundsp")]
    let bass = player.play_generator_source(
        FunDspGenerator::new(
            "super_saw",
            bass_synth::voice_factory,
            8,
            None,
            None,
            player.output_sample_rate(),
        )?,
        None,
        None,
    )?;

    // Sequencer timing
    const BPM: f64 = 120.0;
    const BEATS_PER_BAR: usize = 4;
    const BARS_TO_PLAY: usize = 4;

    let samples_per_sec = player.output_sample_rate();
    let samples_per_beat = (60.0 / BPM * samples_per_sec as f64) as u64;

    let output_start_time = player.output_sample_frame_position() + samples_per_sec as u64;

    // Schedule metronome beats
    let mut current_time = output_start_time;
    for beat in 0..(BEATS_PER_BAR * BARS_TO_PLAY) {
        let note = match beat {
            _ if beat.is_multiple_of(BEATS_PER_BAR) => 72,
            _ => 60,
        };
        metronome.note_on(note, Some(1.0), None, current_time)?;
        current_time += samples_per_beat;
    }
    // Stop sampler at the end of the sequence
    metronome.stop(current_time)?;

    // Schedule bass line with glides (midi_note, duration_in_beats, glide, volume, pan)
    let bass_line = [
        (48, 4.0, None, None, Some(0.0)),
        (44, 1.0, Some(999.0), Some(0.75), Some(0.8)),
        (46, 1.0, Some(999.0), Some(0.5), Some(-0.8)),
        (53, 2.0, Some(12.0), None, Some(0.0)),
        (44, 4.0, Some(96.0), Some(1.0), None),
    ];

    // pitch bass sample file by an octave
    let bass_pitch: u8 = if cfg!(feature = "fundsp") { 0 } else { 12 };

    // Start bass with the first metronome beat
    current_time = output_start_time;
    let mut bass_note_id = None;

    for (note, beats, glide, volume, panning) in &bass_line {
        match bass_note_id {
            // Glide existing note
            Some(bass_note_id) if glide.is_some() => {
                bass.set_note_speed(
                    bass_note_id,
                    speed_from_note(*note + bass_pitch),
                    *glide,
                    current_time,
                )?;
                if let Some(volume) = volume {
                    bass.set_note_volume(bass_note_id, *volume, current_time)?;
                }
                if let Some(panning) = panning {
                    bass.set_note_panning(bass_note_id, *panning, current_time)?;
                }
            }
            // Play new note
            _ => {
                bass_note_id = Some(bass.note_on(
                    *note + bass_pitch, //
                    *volume,
                    *panning,
                    current_time,
                )?);
            }
        }
        current_time += (*beats * samples_per_beat as f64) as u64;
    }
    // Stop sampler at the end of the metronome sequence
    bass.stop((BEATS_PER_BAR * BARS_TO_PLAY + 1) as u64 * samples_per_beat)?;

    // Start playback
    player.start();

    // Wait for playback to finish
    while player.is_running() && (bass.is_playing() || metronome.is_playing()) {
        std::thread::sleep(Duration::from_millis(100));
    }

    Ok(())
}
