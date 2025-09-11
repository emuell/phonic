//! An example showcasing how to glide the playback speed of a source over time.

use phonic::{utils::speed_from_note, DefaultOutputDevice, Error, FilePlaybackOptions, Player};

#[allow(unused_imports)]
use phonic::sources::{PreloadedFileSource, StreamedFileSource};

use std::time::Duration;

// -------------------------------------------------------------------------------------------------

#[cfg(all(debug_assertions, feature = "assert_no_alloc"))]
#[global_allocator]
static A: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

// -------------------------------------------------------------------------------------------------

fn main() -> Result<(), Error> {
    // Setup audio output and player
    let mut player = Player::new(DefaultOutputDevice::open()?, None);
    let sample_rate = player.output_sample_rate();

    // Setup beat times
    const BPM: f64 = 120.0;
    let samples_per_beat = (60.0 / BPM * sample_rate as f64) as u64;
    let output_start_time = player.output_sample_frame_position() + sample_rate as u64; // Start 1 second ahead

    // Bass line: (midi_note, duration_in_beats, glide_rate)
    let bass_line = [
        (60, 8.0, None),
        (65, 4.0, Some(2.0)),
        (60, 2.0, Some(5.0)),
        (56, 4.0, Some(100.0)),
    ];

    // Play note
    let (first_note, first_duration_beats, _glide) = bass_line[0];
    let speed = speed_from_note(first_note);
    // let bass = StreamedFileSource::from_file(
    let bass = PreloadedFileSource::from_file(
        "assets/bass.wav",
        None,
        FilePlaybackOptions::default()
            .speed(speed)
            .repeat_forever()
            .fade_out(Duration::from_millis(100)),
        sample_rate,
    )?;
    let playback_id = player.play_file_source(bass, Some(output_start_time))?;

    // Schedule speed changes
    let mut current_time = output_start_time;
    let first_duration_samples = (first_duration_beats * samples_per_beat as f64) as u64;
    current_time += first_duration_samples;

    for (note, duration_beats, glide) in &bass_line[1..] {
        let speed = speed_from_note(*note);
        player.set_source_speed(playback_id, speed, *glide, current_time)?;
        let duration_samples = (duration_beats * samples_per_beat as f64) as u64;
        current_time += duration_samples;
    }

    // Wait for the playback to finish
    while player.output_sample_frame_position() < current_time {
        std::thread::sleep(Duration::from_millis(100));
    }

    // Stop the source
    player.stop_source(playback_id, None)?;

    // Give it a moment to fade out
    std::thread::sleep(Duration::from_millis(200));

    Ok(())
}
