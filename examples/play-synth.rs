use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use dasp::{signal, Frame, Signal};

use afplay::{
    source::resampled::ResamplingQuality, AudioFilePlaybackStatusEvent, AudioFilePlayer,
    AudioOutput, DefaultAudioOutput, Error, SynthPlaybackOptions,
};

fn main() -> Result<(), Error> {
    // Open default device
    let audio_output = DefaultAudioOutput::open()?;

    // create channel for playback status events
    let (status_sender, status_receiver) = crossbeam_channel::unbounded();
    // create a source player
    let mut player = AudioFilePlayer::new(audio_output.sink(), Some(status_sender));

    // Creates a signal of a detuned sine.
    let sample_rate = player.output_sample_rate();
    let generate_note = |pitch: f64, amplitude: f64, duration: u32| {
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

    // combine 3 notes to a chord
    let note_amp = 0.5_f64;
    let note_duration = 4 * sample_rate;

    let chord = // chord
        generate_note(440_f64, note_amp, note_duration)
        .add_amp(generate_note(
            440_f64 * (4.0 / 3.0),
            note_amp,
            note_duration,
        ))
        .add_amp(generate_note(
            440_f64 * (6.0 / 3.0),
            note_amp,
            note_duration,
        ));

    let sine = dasp::signal::from_iter(
        dasp::signal::rate(sample_rate as f64)
            .const_hz(440.0)
            .sine()
            .take(sample_rate as usize * 8),
    );
    // create audio source for the chord and sine and memorize the id for the playback status
    let mut playing_synth_ids = vec![
        player.play_dasp_synth(chord, "my_chord", SynthPlaybackOptions::default())?,
        player.play_dasp_synth(
            sine,
            "sine",
            SynthPlaybackOptions::default()
                .volume_db(-3.0)
                .fade_in(Duration::from_secs(2))
                .fade_out(Duration::from_secs(2)),
        )?,
    ];

    let mut sine_synth_id = Some(playing_synth_ids[1]);
    let is_running = Arc::new(AtomicBool::new(true));

    // handle events from the file sources
    let event_thread = std::thread::spawn({
        let is_running = is_running.clone();
        move || {
            while let Ok(event) = status_receiver.recv() {
                match event {
                    AudioFilePlaybackStatusEvent::Position { id, path, position } => {
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
        if sine_synth_id.is_some() && play_time.elapsed() > Duration::from_secs(2) {
            player.stop_source(sine_synth_id.unwrap())?;
            sine_synth_id = None;
        }
        std::thread::sleep(Duration::from_secs(1));
    }

    // wait until playback thread finished
    event_thread.join().unwrap();

    Ok(())
}
