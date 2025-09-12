//! An example showcasing how to build a simple sequencer by scheduling preloaded audio samples
//! and using sample glide playback parameters.

use std::{path::PathBuf, time::Duration};

use phonic::{
    outputs::WavOutputDevice, sources::PreloadedFileSource, utils::speed_from_note,
    DefaultOutputDevice, Error, FilePlaybackOptions, Player,
};

use arg::{parse_args, Args};

// -------------------------------------------------------------------------------------------------

#[cfg(all(debug_assertions, feature = "assert_no_alloc"))]
#[global_allocator]
static A: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

// -------------------------------------------------------------------------------------------------

#[derive(Args, Debug)]
struct Arguments {
    #[arg(short = "o", long = "output")]
    /// Write audio output into the given wav file, instead of using the default audio device.
    output_path: Option<PathBuf>,
}

// -------------------------------------------------------------------------------------------------

fn main() -> Result<(), Error> {
    // Parse optional arguments
    let args = parse_args::<Arguments>();

    // Setup audio output and player
    let mut player = if let Some(output_path) = args.output_path {
        Player::new(WavOutputDevice::open(output_path)?, None)
    } else {
        Player::new(DefaultOutputDevice::open()?, None)
    };

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
        FilePlaybackOptions::default(),
        player.output_sample_rate(),
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

    // Schedule metronome beats
    for beat in 0..(BEATS_PER_BAR * BARS_TO_PLAY) {
        let sample_time = current_time + beat as u64 * samples_per_beat;
        let speed = speed_from_note(60 + if beat % BEATS_PER_BAR == 0 { 12 } else { 0 });
        let playback_id = player.play_file_source(
            cowbell.clone(FilePlaybackOptions::default().speed(speed), samples_per_sec)?,
            sample_time,
        )?;
        player.stop_source(playback_id, sample_time + samples_per_beat)?;
    }

    // Schedule bass line with glides (midi_note, duration_in_beats, glide_rate)
    let bass_line = [
        (60, 4.0, None),
        (56, 1.0, Some(999.0)),
        (58, 1.0, Some(999.0)),
        (65, 2.0, Some(12.0)),
        (56, 4.0, Some(96.0)),
    ];

    // Play first note
    let (first_note, first_duration_beats, _glide) = bass_line[0];
    let bass_playback_id = player.play_file_source(
        bass.clone(
            FilePlaybackOptions::default()
                .speed(speed_from_note(first_note))
                .fade_out(Duration::from_millis(1000)),
            samples_per_sec,
        )?,
        current_time,
    )?;

    // Schedule subsequent speed changes for the bass line
    let first_duration_samples = (first_duration_beats * samples_per_beat as f64) as u64;
    current_time += first_duration_samples;

    for (note, duration_beats, glide) in &bass_line[1..] {
        player.set_source_speed(
            bass_playback_id,
            speed_from_note(*note),
            *glide,
            current_time,
        )?;
        let duration_samples = (duration_beats * samples_per_beat as f64) as u64;
        current_time += duration_samples;
    }

    // Wait for sequence playback to finish
    while player.is_running()
        && player.output_sample_frame_position()
            < output_start_time + BARS_TO_PLAY as u64 * samples_per_bar
    {
        std::thread::sleep(Duration::from_millis(100));
    }

    // Stop all sources (the bass sample will still play)
    player.stop_all_sources()?;

    // Give sources time to fade out
    std::thread::sleep(Duration::from_secs(2));

    Ok(())
}
