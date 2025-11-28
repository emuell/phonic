//! An example showcasing how to play a `fundsp` signal as a synth source.

use std::{sync::mpsc::sync_channel, time::Duration};

use fundsp::hacker32::*;

use phonic::{Error, PlaybackStatusEvent, SynthPlaybackHandle, SynthPlaybackOptions};

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

    // Create a player with the default output device and a channel to receive playback events.
    let (playback_status_sender, playback_status_receiver) = sync_channel(32);
    let mut player = arguments::new_player(&args, playback_status_sender.clone())?;

    // Pause playback until we've added all sources.
    player.stop();

    let sample_rate = player.output_sample_rate();

    // Creates a mono FunDSP synth node for a chord note
    let generate_chord_note = |pitch: f32, amplitude: f32, duration_secs: f32| {
        let freq = shared(pitch);
        let amp = shared(amplitude);
        // Duration envelope with smooth fade-out
        let fade_out_time = 0.5; // 0.5 seconds fade-out
        let gate = envelope(move |t| {
            if t < duration_secs {
                1.0
            } else if t < duration_secs + fade_out_time {
                // Linear fade from 1.0 to 0.0 over fade_out_time
                1.0 - (t - duration_secs) / fade_out_time
            } else {
                0.0
            }
        });
        // Detuned oscillators
        let fundamental = var(&freq) >> sine();
        let harmonic_l1 = (var(&freq) * 2.01) >> sine();
        let harmonic_h1 = (var(&freq) * 0.52) >> sine();
        let harmonic_h2 = (var(&freq) * 0.251) >> sine();
        // Combine oscillators with envelope
        Box::new(
            gate * (fundamental + harmonic_l1 * 0.5 + harmonic_h1 * 0.5 + harmonic_h2 * 0.5)
                * var(&amp),
        )
    };

    // Creates a stereo FM synth signal with FunDSP
    let generate_synth_note = |pitch: f32, amplitude: f32, duration_secs: f32| {
        let freq = shared(pitch);
        let amp = shared(amplitude);
        // Duration envelope with smooth exponential fade-out after duration
        let fade_out_time = 1.0; // 1 seconds fade-out
        let gate = envelope(move |t| {
            if t < duration_secs {
                1.0
            } else if t < duration_secs + fade_out_time {
                // Exponential fade from 1.0 to 0.0
                let fade_progress = (t - duration_secs) / fade_out_time;
                (1.0 - fade_progress).powf(2.0) // Quadratic fade for smooth decay
            } else {
                0.0
            }
        });
        // Modulator signal for FM synthesis
        let modulator = var(&freq) >> sine();
        // Modulation index (constant for one-shot)
        let mod_index = var(&freq) * 5.0;
        // Modulated frequency: carrier_freq = pitch + modulator * mod_index
        let carrier_freq = var(&freq) + modulator * mod_index;
        // Carrier signal (FM)
        let fm_signal = carrier_freq >> sine();
        // Apply amplitude and envelope
        let signal = gate * fm_signal * var(&amp);
        // Split to stereo and feed into a chorus
        Box::new(signal >> pan(0.0) >> (chorus(0, 0.0, 0.01, 0.2) | chorus(1, 0.0, 0.01, 0.2)))
    };

    // Play all synth sources and memorize ids for the playback status.
    let playing_synths: [SynthPlaybackHandle; 4] = [
        player.play_fundsp_synth(
            "synth_chord",
            generate_chord_note(440.0, 0.5, 4.0),
            SynthPlaybackOptions::default(),
        )?,
        player.play_fundsp_synth(
            "synth_note1",
            generate_synth_note(110.0, 1.0, 1.0),
            SynthPlaybackOptions::default()
                .volume_db(-3.0)
                .start_at_time(sample_rate as u64 * 2),
        )?,
        player.play_fundsp_synth(
            "synth_note2",
            generate_synth_note(110.0 * 2.0, 1.0, 1.0),
            SynthPlaybackOptions::default()
                .volume_db(-3.0)
                .start_at_time(sample_rate as u64 * 3),
        )?,
        player.play_fundsp_synth(
            "synth_note3",
            generate_synth_note(110.0 * 3.0, 1.0, 1.0),
            SynthPlaybackOptions::default()
                .volume_db(-3.0)
                .start_at_time(sample_rate as u64 * 4),
        )?,
    ];

    // Handle playback events from the player.
    std::thread::spawn(move || {
        while let Ok(event) = playback_status_receiver.recv() {
            match event {
                PlaybackStatusEvent::Position {
                    id,
                    path,
                    context: _,
                    position,
                } => {
                    println!(
                        "Playback pos of synth #{id} '{path}': {pos}",
                        pos = position.as_secs_f32()
                    );
                }
                PlaybackStatusEvent::Stopped {
                    id,
                    path,
                    context: _,
                    exhausted,
                } => {
                    if exhausted {
                        println!("Playback of synth #{id} '{path}' finished playback");
                    } else {
                        println!("Playback of synth #{id} '{path}' stopped");
                    }
                }
            }
        }
    });

    // Start playing.
    player.start();

    // Wait until all synth sources finished playing...
    while playing_synths.iter().any(|synth| synth.is_playing()) {
        std::thread::sleep(Duration::from_millis(100));
    }

    Ok(())
}
