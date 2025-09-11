//! An example showcasing how to play a `dasp` signal as a synth source.

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use dasp::{signal, Frame, Signal};

use phonic::{DefaultOutputDevice, Error, PlaybackStatusEvent, Player, SynthPlaybackOptions};

// -------------------------------------------------------------------------------------------------

#[cfg(all(debug_assertions, feature = "assert_no_alloc"))]
#[global_allocator]
static A: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

// -------------------------------------------------------------------------------------------------

fn main() -> Result<(), Error> {
    // Create channel for playback status events
    let (status_sender, status_receiver) = crossbeam_channel::bounded(32);

    // Open default audio device and create a new player
    let mut player = Player::new(DefaultOutputDevice::open()?, Some(status_sender));
    let sample_rate = player.output_sample_rate();

    // Creates a signal of a detuned sines using dasp.
    let generate_chord_note = |pitch: f64, amplitude: f64, duration: u32| {
        let fundamental = signal::rate(sample_rate as f64).const_hz(pitch);
        let harmonic_l1 = signal::rate(sample_rate as f64).const_hz(pitch * 2.01);
        let harmonic_h1 = signal::rate(sample_rate as f64).const_hz(pitch / 2.02);
        let harmonic_h2 = signal::rate(sample_rate as f64).const_hz(pitch / 4.04);

        signal::from_iter(
            fundamental
                .sine()
                .add_amp(harmonic_l1.sine().scale_amp(0.5))
                .add_amp(harmonic_h1.sine().scale_amp(0.5))
                .add_amp(harmonic_h2.sine().scale_amp(0.5))
                .scale_amp(amplitude)
                .take(duration as usize)
                .zip(0..duration)
                .map(move |(s, index)| {
                    let env: f64 = (1.0 - (index as f64) / (duration as f64)).powf(2.0);
                    s * env
                }),
        )
    };

    // Combine 3 notes to a chord
    let chord_note_amp = 0.5_f64;
    let chord_note_duration = 4 * sample_rate;
    let chord = // chord
        generate_chord_note(440_f64, chord_note_amp, chord_note_duration)
        .add_amp(generate_chord_note(
            440_f64 * (4.0 / 3.0),
            chord_note_amp,
            chord_note_duration,
        ))
        .add_amp(generate_chord_note(
            440_f64 * (6.0 / 3.0),
            chord_note_amp,
            chord_note_duration,
        ));

    // Creates a FM synth signal with dasp.
    let generate_synth_note = move |pitch: f64, amplitude: f64| {
        let duration_frames = (4.0 * sample_rate as f64) as u64;
        // Modulator signal.
        let modulator = signal::rate(sample_rate as f64).const_hz(pitch).sine();
        // Modulation index envelope.
        let mod_index_env = (0..duration_frames).map(move |i| {
            let time_secs = i as f64 / sample_rate as f64;
            pitch * (-time_secs).exp()
        });
        // Modulated frequency for carrier.
        let carrier_freq = signal::from_iter(
            modulator
                .take(duration_frames as usize)
                .zip(mod_index_env)
                .map(move |(m, i)| {
                    pitch + m * i // m is stereo, take one channel. i is scalar.
                }),
        );
        // Carrier signal (FM).
        let fm_signal = signal::rate(sample_rate as f64).hz(carrier_freq).sine();
        // Overall envelope.
        let envelope = (0..duration_frames).map(move |i| {
            let time_secs = i as f64 / sample_rate as f64;
            if time_secs < 4.0 {
                (1.0 - time_secs / 4.0).powi(2)
            } else {
                0.0
            }
        });
        // Apply envelope and amplitude.
        signal::from_iter(
            fm_signal
                .take(duration_frames as usize)
                .zip(envelope)
                .map(move |(s, e)| s.map(|smp| smp * e * amplitude)),
        )
    };

    // play all synth sources and memorize ids for the playback status
    let mut playing_synth_ids = vec![
        player.play_dasp_synth(
            chord,
            "synth_chord",
            SynthPlaybackOptions::default().fade_out(Duration::from_secs(2)),
        )?,
        player.play_dasp_synth(
            generate_synth_note(220.0, 1.0),
            "synth_note1",
            SynthPlaybackOptions::default()
                .volume_db(-3.0)
                .start_at_time(sample_rate as u64 * 2),
        )?,
        player.play_dasp_synth(
            generate_synth_note(220.0 * 2.0, 1.0),
            "synth_note2",
            SynthPlaybackOptions::default()
                .volume_db(-3.0)
                .start_at_time(sample_rate as u64 * 3),
        )?,
        player.play_dasp_synth(
            generate_synth_note(220.0 * 3.0, 1.0),
            "synth_note3",
            SynthPlaybackOptions::default()
                .volume_db(-3.0)
                .start_at_time(sample_rate as u64 * 4),
        )?,
    ];

    // handle playback events from the player
    let is_running = Arc::new(AtomicBool::new(true));
    let playback_event_thread = std::thread::spawn({
        let is_running = is_running.clone();
        move || {
            while let Ok(event) = status_receiver.recv() {
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
                        playing_synth_ids.retain(|v| *v != id);
                        if playing_synth_ids.is_empty() {
                            // stop this example when all synths finished
                            is_running.store(false, Ordering::Relaxed);
                            break;
                        }
                    }
                }
            }
        }
    });

    // Wait until all sounds finished playing
    playback_event_thread.join().unwrap();

    Ok(())
}
