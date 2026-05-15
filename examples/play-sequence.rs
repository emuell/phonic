//! An example showcasing how to play a musical sequence using the Pattern and Metronome
//! sequencers with preloaded audio samples and glide playback parameters.

use std::time::Duration;

use phonic::{
    generators::{IntoPatternRow, Metronome, Pattern, PatternEvent, Sampler},
    utils::ahdsr::AhdsrParameters,
    Error, GeneratorPlaybackOptions,
};

// -------------------------------------------------------------------------------------------------

#[cfg(all(debug_assertions, feature = "assert-allocs"))]
#[global_allocator]
static A: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

// -------------------------------------------------------------------------------------------------

// Common example code
mod common;
use common::arguments;

// -------------------------------------------------------------------------------------------------

fn main() -> Result<(), Error> {
    // Parse optional arguments
    let args = arguments::parse();

    // Create a player instance with the output device as configured via program arguments
    let mut player = arguments::new_player(&args, None)?;

    // Stop playback until we've added/scheduled generators and sequencers
    player.stop();

    // Set global sequencer timing
    player.set_transport_bpm(120.0);
    player.set_transport_beats_per_bar(4);
    let start_time =
        player.output_sample_frame_position() + player.transport().seconds_to_samples(1.0);

    // Create a metronome sampler
    let metronome = player.add_generator(
        Sampler::from_file(
            "assets/cowbell.wav",
            GeneratorPlaybackOptions::default().voices(2),
            player.output_channel_count(),
            player.output_sample_rate(),
        )?,
        None,
    )?;

    // Create a bass sampler
    let bass = player.add_generator(
        Sampler::from_file(
            "assets/bass.wav",
            GeneratorPlaybackOptions::default(),
            player.output_channel_count(),
            player.output_sample_rate(),
        )?
        .with_ahdsr(AhdsrParameters::new(
            Duration::from_millis(10),
            Duration::ZERO,
            Duration::ZERO,
            1.0,
            Duration::from_secs_f32(2.0),
        )?)?,
        None,
    )?;

    // Schedule metronome beats via a metronome sequencer
    let metronome_sequencer = player.play_sequencer(Metronome::new(3), metronome, start_time)?;

    // Schedule a simple bass line with glides using the pattern sequencer
    let bass_sequencer = player.play_sequencer(
        Pattern::new(
            vec![
                PatternEvent::note_on(48 + 12).into_row(3.0),
                PatternEvent::note_on(36 + 12).glide(999.0).into_row(0.5),
                PatternEvent::note_on(48 + 12).glide(999.0).into_row(0.5),
                PatternEvent::note_on(44 + 12)
                    .glide(60.0)
                    .volume(0.75)
                    .panning(0.8)
                    .into_row(1.0),
                PatternEvent::note_on(46 + 12)
                    .glide(60.0)
                    .volume(0.5)
                    .panning(-0.8)
                    .into_row(1.0),
                PatternEvent::note_on(53 + 12).glide(12.0).into_row(2.0),
                PatternEvent::note_on(44 + 12).glide(60.0).into_row(4.0),
                PatternEvent::note_off().into_row(8.0),
            ],
            0,
        ),
        bass,
        start_time,
    )?;

    // Print player graph
    println!("\nPlayer Graph:\n{}", player);

    // Start playback
    player.start();

    // Wait for all sequencers to finish
    while player.is_running() && bass_sequencer.is_playing() && metronome_sequencer.is_playing() {
        std::thread::sleep(Duration::from_millis(100));
    }

    // Let samples fade out...
    std::thread::sleep(Duration::from_secs_f64(
        player
            .transport()
            .samples_to_seconds(player.transport().samples_per_bar() as u64),
    ));

    Ok(())
}
