use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use dasp::{signal, Frame, Signal};
use fundsp::prelude::*;

use afplay::{
    AudioFilePlaybackStatusEvent, AudioFilePlayer, AudioOutput, DefaultAudioOutput, Error,
    SynthPlaybackOptions,
};

// -------------------------------------------------------------------------------------------------

#[cfg(all(debug_assertions, feature = "assert-no-alloc"))]
#[global_allocator]
static A: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

// -------------------------------------------------------------------------------------------------

fn main() -> Result<(), Error> {
    // Open default device
    let audio_output = DefaultAudioOutput::open()?;

    // create channel for playback status events
    let (status_sender, status_receiver) = crossbeam_channel::unbounded();
    // create a source player
    let mut player = AudioFilePlayer::new(audio_output.sink(), Some(status_sender));

    // Creates a signal of a detuned sine with dasp.
    let sample_rate = player.output_sample_rate();
    let generate_dasp_note = |pitch: f64, amplitude: f64, duration: u32| {
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
                    (s * env).to_float_frame()
                }),
        )
    };

    #[allow(clippy::precedence)]
    let generate_fundsp_note = |pitch: f64, amplitude: f64| {
        let f = pitch;
        let m = lfo(|t: f64| exp(-t));
        let dur_secs = 4.0;
        let signal = sine_hz(f) * f * m + f >> sine();
        let envelope = amplitude
            * envelope(move |t: f64| {
                if t / dur_secs < 1.0 {
                    pow(lerp(1.0, 0.0, t / dur_secs), 2.0)
                } else {
                    0.0
                }
            });
        signal * envelope
    };

    // combine 3 notes to a chord
    let note_amp = 0.5_f64;
    let note_duration = 4 * sample_rate;

    let chord = // chord
        generate_dasp_note(440_f64, note_amp, note_duration)
        .add_amp(generate_dasp_note(
            440_f64 * (4.0 / 3.0),
            note_amp,
            note_duration,
        ))
        .add_amp(generate_dasp_note(
            440_f64 * (6.0 / 3.0),
            note_amp,
            note_duration,
        ));

    // create audio source for the chord and sine and memorize the id for the playback status
    let mut playing_synth_ids = vec![
        player.play_dasp_synth(
            chord,
            "dasp_chord",
            SynthPlaybackOptions::default().fade_out(Duration::from_secs(2)),
        )?,
        player.play_fundsp_synth(
            generate_fundsp_note(220.0, 1.0),
            "fundsp_fm1",
            SynthPlaybackOptions::default()
                .volume_db(-3.0)
                .start_at_time(sample_rate as u64 * 2),
        )?,
        player.play_fundsp_synth(
            generate_fundsp_note(220.0 * 2.0, 1.0),
            "fundsp_fm2",
            SynthPlaybackOptions::default()
                .volume_db(-3.0)
                .start_at_time(sample_rate as u64 * 3),
        )?,
        player.play_fundsp_synth(
            generate_fundsp_note(220.0 * 3.0, 1.0),
            "fundsp_fm3",
            SynthPlaybackOptions::default()
                .volume_db(-3.0)
                .start_at_time(sample_rate as u64 * 4),
        )?,
    ];

    let mut fundsp_synth_id = Some(playing_synth_ids[0]);
    let is_running = Arc::new(AtomicBool::new(true));

    // handle events from the file sources
    let event_thread = std::thread::spawn({
        let is_running = is_running.clone();
        move || {
            while let Ok(event) = status_receiver.recv() {
                match event {
                    AudioFilePlaybackStatusEvent::Position {
                        id,
                        path,
                        context: _,
                        position,
                    } => {
                        println!(
                            "Playback pos of synth #{} '{}': {}",
                            id,
                            path,
                            position.as_secs_f32()
                        );
                    }
                    AudioFilePlaybackStatusEvent::Stopped {
                        id,
                        path,
                        context: _,
                        exhausted,
                    } => {
                        if exhausted {
                            println!("Playback of synth #{} '{}' finished playback", id, path);
                        } else {
                            println!("Playback of synth #{} '{}' stopped", id, path);
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

    // stop (fade-out) the sine after 2 secs
    let play_time = Instant::now();
    while is_running.load(Ordering::Relaxed) {
        if fundsp_synth_id.is_some() && play_time.elapsed() > Duration::from_secs(2) {
            // player.stop_source(fundsp_synth_id.unwrap())?;
            fundsp_synth_id = None;
        }
        std::thread::sleep(Duration::from_secs(1));
    }

    // wait until playback thread finished
    event_thread.join().unwrap();

    Ok(())
}
