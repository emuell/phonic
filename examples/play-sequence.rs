//! An example showcasing how to build a simple sequencer by scheduling preloaded audio samples
//! and using sample glide playback parameters with a Sampler generator.

use std::time::Duration;

use phonic::{
    sources::{generators::Sampler, PreloadedFileSource},
    utils::speed_from_note,
    Error,
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

fn main() -> Result<(), Error> {
    // Parse optional arguments
    let args = arguments::parse();

    // Create a player instance with the output device as configured via program arguments
    let mut player = arguments::new_player(&args, None)?;

    // Stop until we've scheduled all playback events
    player.stop();

    // Preload samples
    let cowbell = PreloadedFileSource::from_file(
        "assets/cowbell.wav",
        None,
        Default::default(),
        player.output_sample_rate(),
    )?;

    let bass = PreloadedFileSource::from_file(
        "assets/bass.wav",
        None,
        Default::default(),
        player.output_sample_rate(),
    )?;

    // Create samplers
    const VOICE_COUNT: usize = 2;
    let cowbell_sampler = player.play_generator_source(
        Sampler::new(
            cowbell,
            VOICE_COUNT,
            player.output_sample_rate(),
            player.output_channel_count(),
            None,
        )?,
        None,
        None,
    )?;

    let bass_sampler = player.play_generator_source(
        Sampler::new(
            bass,
            VOICE_COUNT,
            player.output_sample_rate(),
            player.output_channel_count(),
            Some(Duration::from_millis(200)),
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
    let samples_per_bar = BEATS_PER_BAR as u64 * samples_per_beat;

    // Start 1 second ahead of the current playback time
    let output_start_time = player.output_sample_frame_position() + samples_per_sec as u64;
    let mut current_time = output_start_time;

    // Schedule metronome beats via the cowbell sampler
    for beat in 0..(BEATS_PER_BAR * BARS_TO_PLAY) {
        let sample_time = current_time + beat as u64 * samples_per_beat;
        let note = if beat % BEATS_PER_BAR == 0 { 72 } else { 60 };
        let note_id = cowbell_sampler.note_on(note, None, None, sample_time)?;
        cowbell_sampler.note_off(note_id, sample_time + samples_per_beat)?;
    }

    // Schedule bass line with glides (midi_note, duration_in_beats, glide, volume, pan)
    let bass_line = [
        (60, 4.0, None, None, Some(0.0)),
        (56, 1.0, Some(999.0), Some(0.75), Some(1.0)),
        (58, 1.0, Some(999.0), Some(0.5), Some(-1.0)),
        (65, 2.0, Some(12.0), Some(0.75), Some(0.0)),
        (56, 4.0, Some(96.0), Some(1.0), None),
    ];

    // Schedule all bass notes via the sampler handle
    current_time = output_start_time;
    let mut bass_note_id = None;

    for (note, duration_beats, glide, volume, panning) in &bass_line {
        // Trigger note on for the first note, or change speed for subsequent notes
        if let Some(note_id) = glide.and(bass_note_id) {
            // Change speed/volume/panning in a playing voice
            bass_sampler.set_note_speed(note_id, speed_from_note(*note), *glide, current_time)?;
            if let Some(volume) = volume {
                bass_sampler.set_note_volume(note_id, *volume, current_time)?;
            }
            if let Some(panning) = panning {
                bass_sampler.set_note_panning(note_id, *panning, current_time)?;
            }
        } else {
            // Stop playing note
            if let Some(note_id) = bass_note_id {
                bass_sampler.note_off(note_id, current_time)?;
            }
            // Trigger note on
            bass_note_id = Some(bass_sampler.note_on(
                *note,
                Some(volume.unwrap_or(1.0)),
                Some(panning.unwrap_or(0.0)),
                current_time,
            )?);
        }

        let duration_samples = (duration_beats * samples_per_beat as f64) as u64;
        current_time += duration_samples;
    }

    // Schedule note off at the end
    bass_sampler.all_notes_off(current_time)?;

    // Start playback
    player.start();

    // Wait for sequence playback to finish
    while player.is_running()
        && player.output_sample_frame_position()
            < output_start_time + BARS_TO_PLAY as u64 * samples_per_bar
    {
        std::thread::sleep(Duration::from_millis(100));
    }

   Ok(())
}
