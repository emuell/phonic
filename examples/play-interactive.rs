use dasp::{signal, Frame, Signal};
use device_query::{DeviceEvents, DeviceState, Keycode};
use lazy_static::lazy_static;
use std::{
    collections::HashMap,
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};

use afplay::{
    source::{
        file::{preloaded::PreloadedFileSource, FilePlaybackOptions},
        resampled::ResamplingQuality,
        synth::SynthPlaybackOptions,
    },
    utils::{pitch_from_note, speed_from_note},
    AudioFilePlaybackId, AudioFilePlayer, AudioOutput, DefaultAudioOutput, Error,
};

// -------------------------------------------------------------------------------------------------

fn main() -> Result<(), Error> {
    // open default audio output
    let audio_output = DefaultAudioOutput::open()?;

    // create player and move audio device
    let player = Arc::new(Mutex::new(AudioFilePlayer::new(audio_output.sink(), None)));

    // create condvar to block the main thread
    let wait_mutex_cond = Arc::new((Mutex::new(()), Condvar::new()));

    // create global playback state
    let current_playmode = Arc::new(Mutex::new(PlayMode::Synth));
    let current_octave = Arc::new(Mutex::new(5));
    let curent_loop_seek_start = Arc::new(Mutex::new(Duration::ZERO));
    let playing_synth_ids = Arc::new(Mutex::new(HashMap::<Keycode, usize>::new()));

    // start playing the background loop and memorize playback_id
    let loop_playback_id = player.lock().unwrap().play_file(
        "assets/YuaiLoop.wav",
        FilePlaybackOptions::default()
            .streamed()
            .repeat(usize::MAX)
            .volume_db(-3.0)
            .speed(0.9)
            .resampling_quality(ResamplingQuality::HighQuality),
    )?;

    // print header
    println!("*** afplay interactive playback example:");
    println!("  Use keys 'A, S, D, F, G, H,J' to play notes 'C, D, E, F, G, A, H'.");
    println!("  Arrow 'up/down' keys change the current octave.");
    println!("  Arrow 'left/right' to seek through the loop sample");
    println!("  To play a dasp signal synth, hit key '1'. For a sample based synth hit key '2'.");
    println!();
    println!("  NB: this example uses a HighQuality resampler for the loop. ");
    println!("  In debug builds this may be very slow and may thus cause crackles...");
    println!();
    println!("  To quit press 'Esc' or 'Control/Cmd-C'.");
    println!();

    // run key event handlers to play, stop and modify sounds interactively
    let device_state = DeviceState::new();

    ctrlc::set_handler({
        let wait_mutex_cond = Arc::clone(&wait_mutex_cond);
        move || {
            println!("Shutting down...");
            wait_mutex_cond.1.notify_all();
        }
    })
    .map_err(|_err| Error::SendError)?;

    // key down handler
    let _key_down_guard = device_state.on_key_down({
        let wait_mutex_cond = Arc::clone(&wait_mutex_cond);
        let player = Arc::clone(&player);
        let playing_synth_ids = Arc::clone(&playing_synth_ids);

        let current_playmode = Arc::clone(&current_playmode);
        let current_octave = Arc::clone(&current_octave);

        move |key: &Keycode| match key {
            Keycode::Escape => {
                println!("Shutting down...");
                wait_mutex_cond.1.notify_all();
            }
            Keycode::Key1 => {
                let mut playmode = current_playmode.lock().unwrap();
                *playmode = PlayMode::Synth;
                println!("Changed playmode to 'Synth'");
            }
            Keycode::Key2 => {
                let mut playmode = current_playmode.lock().unwrap();
                *playmode = PlayMode::Sample;
                println!("Changed playmode to 'Sample'");
            }
            Keycode::Up => {
                let mut current = current_octave.lock().unwrap();
                if *current < 8 {
                    *current += 1;
                    println!("Changed octave to '{}'", *current);
                }
            }
            Keycode::Down => {
                let mut current = current_octave.lock().unwrap();
                if *current > 1 {
                    *current -= 1;
                    println!("Changed octave to '{}'", *current);
                }
            }
            Keycode::Left => {
                let mut current = curent_loop_seek_start.lock().unwrap();
                *current = Duration::from_secs_f32(0_f32.max(current.as_secs_f32() - 0.5));
                let mut player = player.lock().unwrap();
                player
                    .seek_source(loop_playback_id, *current)
                    .unwrap_or_default();
                println!("Seeked loop to pos: {} sec", current.as_secs_f32());
            }
            Keycode::Right => {
                let mut current = curent_loop_seek_start.lock().unwrap();
                *current = Duration::from_secs_f32(4_f32.min(current.as_secs_f32() + 0.5));
                let mut player = player.lock().unwrap();
                player
                    .seek_source(loop_playback_id, *current)
                    .unwrap_or_default();
                println!("Seeked loop to pos: {} sec", current.as_secs_f32())
            }
            keycode => {
                if let Some(relative_note) = key_to_note(keycode) {
                    let playmode = *current_playmode.lock().unwrap();
                    let octave = *current_octave.lock().unwrap();
                    let final_note = (relative_note + 12 * octave) as u8;

                    let mut player = player.lock().unwrap();
                    let mut playing_synth_ids = playing_synth_ids.lock().unwrap();

                    let playback_id = handle_note_on(&mut player, final_note, playmode);
                    playing_synth_ids.insert(*keycode, playback_id);
                }
            }
        }
    });

    // key up handler
    let _key_up_guard = device_state.on_key_up({
        let player = Arc::clone(&player);
        let playing_synth_ids = Arc::clone(&playing_synth_ids);

        move |key: &Keycode| {
            if key_to_note(key).is_some() {
                let mut player = player.lock().unwrap();
                let mut playing_synth_ids = playing_synth_ids.lock().unwrap();
                if let Some(playback_id) = playing_synth_ids.get(key) {
                    handle_note_off(&mut player, *playback_id);
                    playing_synth_ids.remove(key);
                }
            };
        }
    });

    // block main thread until condvar gets triggered in key loop
    let _guard = wait_mutex_cond
        .1
        .wait(wait_mutex_cond.0.lock().unwrap())
        .unwrap();

    Ok(())
}

// -------------------------------------------------------------------------------------------------

fn key_to_note(keycode: &Keycode) -> Option<u32> {
    match keycode {
        Keycode::A | Keycode::Q => Some(0), // C
        Keycode::W => Some(1),              // C#
        Keycode::S => Some(2),              // D
        Keycode::E => Some(3),              // D#
        Keycode::D => Some(4),              // E
        Keycode::F => Some(5),              // F
        Keycode::T => Some(6),              // F#
        Keycode::G => Some(7),              // G
        Keycode::Z | Keycode::Y => Some(8), // G#
        Keycode::H => Some(9),              // A
        Keycode::U => Some(10),             // A#
        Keycode::J => Some(11),             // H
        Keycode::K => Some(12),             // C'
        _ => None,
    }
}

// -------------------------------------------------------------------------------------------------

#[derive(PartialEq, Copy, Clone)]
enum PlayMode {
    Sample,
    Synth,
}

fn handle_note_on(
    player: &mut AudioFilePlayer,
    note: u8,
    playmode: PlayMode,
) -> AudioFilePlaybackId {
    // create, then play a synth or sample source and return the playback_id
    if playmode == PlayMode::Synth {
        player
            .play_dasp_synth(
                create_synth_source(player.output_sample_rate() as f64, pitch_from_note(note)),
                format!("Synth Note #{}", note).as_str(),
                SynthPlaybackOptions::default()
                    .volume_db(-12.0)
                    .fade_out(Duration::from_secs(1)),
            )
            .expect("failed to play synth note")
    } else {
        player
            .play_file_source(
                create_sample_source(
                    FilePlaybackOptions::default()
                        .volume_db(-6.0)
                        .speed(speed_from_note(note))
                        .fade_out(Duration::from_secs(1)),
                    player.output_sample_rate(),
                )
                .expect("failed to clone a sample file"),
                None,
            )
            .expect("failed to play sample")
    }
}

fn handle_note_off(player: &mut AudioFilePlayer, playback_id: AudioFilePlaybackId) {
    // stop playing source with the given playback_id
    player.stop_source(playback_id).unwrap_or_default();
}

// -------------------------------------------------------------------------------------------------

fn create_synth_source(sample_rate: f64, pitch: f64) -> impl signal::Signal<Frame = f64> {
    let duration_in_ms = 1000;
    let duration_in_samples = (sample_rate / duration_in_ms as f64 * 1000.0) as usize;
    // stack up slightly detuned sine waves
    let fundamental = signal::rate(sample_rate as f64).const_hz(pitch);
    let harmonic_l1 = signal::rate(sample_rate as f64).const_hz(pitch * 2.01);
    let harmonic_h1 = signal::rate(sample_rate as f64).const_hz(pitch / 2.02);
    let harmonic_h2 = signal::rate(sample_rate as f64).const_hz(pitch / 4.04);
    // combine them, limit duration and apply a fade-out envelope
    signal::from_iter(
        fundamental
            .sine()
            .add_amp(harmonic_l1.sine().scale_amp(0.5))
            .add_amp(harmonic_h1.sine().scale_amp(0.5))
            .add_amp(harmonic_h2.sine().scale_amp(0.5))
            .take(duration_in_samples as usize)
            .zip(0..duration_in_samples)
            .map(move |(s, index)| {
                let env: f64 = (1.0 - (index as f64) / (duration_in_samples as f64)).powf(2.0);
                (s * env).to_float_frame()
            }),
    )
}

// -------------------------------------------------------------------------------------------------

fn create_sample_source(
    options: FilePlaybackOptions,
    sample_rate: u32,
) -> Result<PreloadedFileSource, Error> {
    // load and decode sample once - lazily
    lazy_static! {
        static ref SAMPLE_SOURCE: PreloadedFileSource = PreloadedFileSource::new(
            "assets/pad-ambient.wav",
            None,
            FilePlaybackOptions::default()
                .volume_db(-12.0)
                .fade_out(Duration::from_secs(1)),
            44100,
        )
        .expect("failed to load synth sample file");
    }
    // then clone the buffer'd source to avoid decoding again
    SAMPLE_SOURCE.clone(options, sample_rate)
}
