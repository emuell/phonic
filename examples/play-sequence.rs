//! An example showcasing how to build a simple sequencer by scheduling preloaded audio samples
//! and using sample glide playback parameters.

use std::time::Duration;

use phonic::{
    generators::Sampler,
    utils::{ahdsr::AhdsrParameters, speed_from_note},
    Error, GeneratorPlaybackOptions,
};

// -------------------------------------------------------------------------------------------------

#[cfg(all(debug_assertions, feature = "assert-allocs"))]
#[global_allocator]
static A: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

// -------------------------------------------------------------------------------------------------

// Common example code
#[path = "./common/arguments.rs"]
mod arguments;

// -------------------------------------------------------------------------------------------------

fn main() -> Result<(), Error> {
    // Parse optional arguments
    let args = arguments::parse();

    // Create a player instance with the output device as configured via program arguments
    let mut player = arguments::new_player(&args, None)?;

    // Stop playback until we've scheduled all notes
    player.stop();

    // Create metronome sampler
    let metronome = player.play_generator(
        Sampler::from_file(
            "assets/cowbell.wav",
            None,
            GeneratorPlaybackOptions::default().voices(2),
            player.output_channel_count(),
            player.output_sample_rate(),
        )?,
        None,
    )?;

    // Create bass sampler
    let bass = player.play_generator(
        Sampler::from_file(
            "assets/bass.wav",
            Some(AhdsrParameters::new(
                Duration::from_millis(10),
                Duration::ZERO,
                Duration::ZERO,
                1.0,
                Duration::from_secs_f32(2.0),
            )?),
            GeneratorPlaybackOptions::default(),
            player.output_channel_count(),
            player.output_sample_rate(),
        )?,
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
        (48 + 12, 3.0, None, None, Some(0.0)),
        (36 + 12, 0.5, Some(999.0), None, Some(0.0)),
        (48 + 12, 0.5, Some(999.0), None, Some(0.0)),
        (44 + 12, 1.0, Some(60.0), Some(0.75), Some(0.8)),
        (46 + 12, 1.0, Some(60.0), Some(0.5), Some(-0.8)),
        (53 + 12, 2.0, Some(12.0), None, Some(0.0)),
        (44 + 12, 4.0, Some(60.0), Some(1.0), None),
    ];

    // Start bass with the first metronome beat
    current_time = output_start_time;
    let mut bass_note_id = None;

    for (note, beats, glide, volume, panning) in &bass_line {
        match bass_note_id {
            // Glide existing note
            Some(bass_note_id) if glide.is_some() => {
                bass.set_note_speed(bass_note_id, speed_from_note(*note), *glide, current_time)?;
                if let Some(volume) = volume {
                    bass.set_note_volume(bass_note_id, *volume, current_time)?;
                }
                if let Some(panning) = panning {
                    bass.set_note_panning(bass_note_id, *panning, current_time)?;
                }
            }
            // Play new note
            _ => {
                bass_note_id = Some(bass.note_on(*note, *volume, *panning, current_time)?);
            }
        }
        current_time += (*beats * samples_per_beat as f64) as u64;
    }
    // Stop sampler at the end of the metronome sequence
    bass.stop((BEATS_PER_BAR * BARS_TO_PLAY + 1) as u64 * samples_per_beat)?;

    // Print player graph
    println!("\nPlayer Graph:\n{}", player);

    // Start playback
    player.start();

    // Wait for playback to finish
    while player.is_running() && (bass.is_playing() || metronome.is_playing()) {
        std::thread::sleep(Duration::from_millis(100));
    }

    Ok(())
}
