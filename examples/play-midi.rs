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
    generators::{AhdsrParameters, MidiFile, Sampler},
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

    // Stop playback until all events are pre-scheduled
    player.stop();

    // Start 1 second after player starts
    let start_time =
        player.output_sample_frame_position() + player.transport().seconds_to_samples(1.0) as u64;

    // Create a sampler with enough voices for typical polyphonic MIDI playback
    let sampler_handle = player.add_generator(
        Sampler::from_file(
            &sample_path,
            GeneratorPlaybackOptions::default()
                .volume_db(-6.0)
                .voices(24),
            player.output_channel_count(),
            player.output_sample_rate(),
        )?
        .with_ahdsr(AhdsrParameters::new(
            Duration::from_millis(1),
            Duration::ZERO,
            Duration::from_millis(100),
            0.7,
            Duration::from_millis(500),
        )?)?,
        None,
    )?;

    // Create a new midi file sequencer and set player's tempo from the file's initial tempo
    let midi_file = MidiFile::from_path(&midi_path)?;
    if let Some(bpm) = midi_file.bpm() {
        player.set_transport_bpm(bpm);
    }

    // Add sequencer to the player, using the sampler as event sink and start playing it
    let midi_file_handle = player.play_sequencer(midi_file, sampler_handle.clone(), start_time)?;

    // Add a reverb effect to the player's main mixer
    let _reverb_handle = player.add_effect(ReverbEffect::with_parameters(0.5, 0.2), None)?;

    // Print player graph
    println!("\nPlaying '{}' with sample '{}'...", midi_path, sample_path);
    println!("\nPlayer Graph:\n{}", player);

    // Start playback
    player.start();

    // Wait for sequencer playback to finish
    while player.is_running() && midi_file_handle.is_playing() {
        std::thread::sleep(Duration::from_millis(100));
    }

    // Wait until sampler's AHDSR fades out
    std::thread::sleep(Duration::from_millis(1000));

    Ok(())
}
