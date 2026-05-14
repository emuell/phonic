//! An example that plays a standard MIDI file through a Sampler generator.
//!
//! Usage:
//!   play-midi [midi_file] [sample_file]
//!
//! Defaults:
//!   midi_file   = assets/cnt1.mid
//!   sample_file = assets/wurlie.wav

use std::time::Duration;

use phonic::{
    effects::ReverbEffect,
    generators::{AhdsrParameters, MidiFile, Sampler, Sequencer},
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
    let args = arguments::parse();
    #[allow(clippy::get_first)]
    let midi_path = args
        .positional
        .get(0)
        .cloned()
        .unwrap_or_else(|| "assets/cnt1.mid".into());
    let sample_path = args
        .positional
        .get(1)
        .cloned()
        .unwrap_or_else(|| "assets/wurlie.wav".into());

    // Create player
    let mut player = arguments::new_player(&args, None)?;
    let sample_rate = player.output_sample_rate();

    // Stop playback until all events are pre-scheduled
    player.stop();

    // Start 1 second after player starts
    let start_time = player.output_sample_frame_position() + sample_rate as u64;

    // Create a sampler with enough voices for typical polyphonic MIDI playback
    let mut generator = player.play_generator(
        Sampler::from_file(
            &sample_path,
            GeneratorPlaybackOptions::default()
                .volume_db(-6.0)
                .voices(24),
            player.output_channel_count(),
            sample_rate,
        )?
        .with_ahdsr(AhdsrParameters::new(
            Duration::from_millis(1),
            Duration::ZERO,
            Duration::from_millis(100),
            0.7,
            Duration::from_millis(500),
        )?)?,
        start_time,
    )?;

    // Parse the MIDI file and pre-schedule all events upfront
    let mut sequence = MidiFile::from_path(&midi_path, start_time, sample_rate)?;
    sequence.run_until(u64::MAX, &mut generator);

    // Stop the generator one second after the last MIDI event
    let stop_time = start_time + sequence.duration_samples() + sample_rate as u64;
    generator.stop(stop_time)?;

    // Add a reverb effect to the master output
    let _ = player.add_effect(ReverbEffect::with_parameters(0.5, 0.2), None)?;

    // Print player graph
    println!("\nPlayer Graph:\n{}", player);
    println!(
        "\nPlaying '{}' through '{}' ({:.1}s)...",
        midi_path,
        sample_path,
        sequence.duration_samples() as f64 / sample_rate as f64
    );

    // Start playback
    player.start();

    // Wait for playback to finish
    while player.is_running() && generator.is_playing() {
        std::thread::sleep(Duration::from_millis(100));
    }

    Ok(())
}
