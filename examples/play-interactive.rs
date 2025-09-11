//! An example showcasing interactive audio playback, with real-time control over effects and sources.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    time::Duration,
};

use dasp::{signal, Signal};
use device_query::{DeviceEvents, DeviceEventsHandler, Keycode};
use lazy_static::lazy_static;

use phonic::{
    effects,
    sources::{DaspSynthSource, PreloadedFileSource},
    utils::{pitch_from_note, speed_from_note},
    DefaultOutputDevice, Error, FilePlaybackOptions, MixerId, OutputDevice, PlaybackId, Player,
    ResamplingQuality, SynthPlaybackOptions,
};

const FILTER_TYPES: [effects::FilterEffectType; 4] = [
    effects::FilterEffectType::Allpass,
    effects::FilterEffectType::Lowpass,
    effects::FilterEffectType::Highpass,
    effects::FilterEffectType::Bandpass,
];

// -------------------------------------------------------------------------------------------------

#[cfg(all(debug_assertions, feature = "assert_no_alloc"))]
#[global_allocator]
static A: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

// -------------------------------------------------------------------------------------------------

fn main() -> Result<(), Error> {
    // open default audio output
    let audio_output = DefaultOutputDevice::open()?;

    // create player and move audio device
    let mut player = Player::new(audio_output.sink(), None);

    let loop_mixer_id;
    let loop_filter_effect_id;
    {
        // create a new mixer
        loop_mixer_id = player.add_mixer(None)?;

        // add a filter effect
        const DEFAULT_FILTER_TYPE: effects::FilterEffectType = effects::FilterEffectType::Allpass;
        const DEFAULT_FILTER_CUTOFF: f32 = 8000.0;
        const DEFAULT_FILTER_Q: f32 = 0.707;
        const DEFAULT_FILTER_GAIN: f32 = 1.0;

        loop_filter_effect_id = player.add_effect(
            effects::FilterEffect::with_parameters(
                DEFAULT_FILTER_TYPE,
                DEFAULT_FILTER_CUTOFF,
                DEFAULT_FILTER_Q,
                DEFAULT_FILTER_GAIN,
            ),
            loop_mixer_id,
        )?;
    }

    // tone mixer
    let tone_mixer_id;
    {
        // create a new mixer
        tone_mixer_id = player.add_mixer(None)?;

        // // add a chorus effect
        // player.add_effect(
        //    Box::new(effects::ChorusEffect::default()), tone_mixer_id,
        // )?;

        // add a reverb effect
        player.add_effect(
            effects::ReverbEffect::with_parameters(0.6, 0.8),
            tone_mixer_id,
        )?;
    }

    // add a dc filter effect to the main mixer
    player.add_effect(effects::DcFilterEffect::default(), None)?;

    // start playing the background loop
    let loop_playback_id = player.play_file(
        "assets/YuaiLoop.wav",
        FilePlaybackOptions::default()
            .streamed()
            .repeat_forever()
            .volume_db(-3.0)
            .speed(0.9)
            .resampling_quality(ResamplingQuality::HighQuality)
            .target_mixer(loop_mixer_id),
    )?;

    // wrap configured player into a mutex
    let player = Arc::new(Mutex::new(player));

    // create condvar to block the main thread
    let wait_mutex_cond = Arc::new((Mutex::new(()), Condvar::new()));

    // create global playback state
    let playing_synth_ids = Arc::new(Mutex::new(HashMap::<Keycode, usize>::new()));
    let current_playmode = Arc::new(Mutex::new(PlayMode::Synth));
    let current_octave = Arc::new(Mutex::new(5));
    let current_loop_seek_start = Arc::new(Mutex::new(Duration::ZERO));
    let current_filter_cutoff = Arc::new(Mutex::new(8000.0));
    let current_filter_type_idx = Arc::new(Mutex::new(0));

    // global key state
    let alt_key_pressed = Arc::new(AtomicBool::new(false));

    // print header
    println!("*** phonic interactive playback example:");
    println!("  Use keys 'A, S, D, F, G, H,J' to play notes 'C, D, E, F, G, A, H'.");
    println!("  Arrow 'up/down' keys change the current octave.");
    println!("  Arrow 'left/right' to seek through the loop sample");
    println!();
    println!("  To play a dasp signal synth, hit key '1'. For a sample based synth hit key '2'.");
    println!();
    println!("  Alt + Arrow 'left/right' to change filter cutoff frequency.");
    println!("  Alt + 0,1,2,3 to change filter type (OFF,LP,HP,BP).");
    println!();
    println!("  NB: this example uses a HighQuality resampler for the loop. ");
    println!("  In debug builds this may be very slow and may thus cause crackles...");
    println!();
    println!("  To quit press 'Esc' or 'Control/Cmd-C'.");
    println!();

    // run key event handlers to play, stop and modify sounds interactively
    let event_handler = DeviceEventsHandler::new(Duration::from_millis(10))
        .expect("Could not initialize event loop");

    ctrlc::set_handler({
        let wait_mutex_cond = Arc::clone(&wait_mutex_cond);
        move || {
            println!("Shutting down...");
            wait_mutex_cond.1.notify_all();
        }
    })
    .map_err(|_err| Error::SendError)?;

    // key down handler
    let _key_down_guard = event_handler.on_key_down({
        let wait_mutex_cond = Arc::clone(&wait_mutex_cond);
        let player = Arc::clone(&player);
        let playing_synth_ids = Arc::clone(&playing_synth_ids);

        let current_playmode = Arc::clone(&current_playmode);
        let current_octave = Arc::clone(&current_octave);
        let current_filter_cutoff = Arc::clone(&current_filter_cutoff);
        let current_filter_type_idx = Arc::clone(&current_filter_type_idx);

        let alt_key_pressed = Arc::clone(&alt_key_pressed);

        move |key: &Keycode| {
            let alt_key = alt_key_pressed.load(Ordering::Relaxed);
            match key {
                Keycode::RAlt | Keycode::LAlt => {
                    alt_key_pressed.store(true, Ordering::Relaxed);
                }
                Keycode::Escape => {
                    println!("Shutting down...");
                    wait_mutex_cond.1.notify_all();
                }
                Keycode::Key0 | Keycode::Key1 | Keycode::Key2 | Keycode::Key3 if alt_key => {
                    let new_type_idx = match key {
                        Keycode::Key0 => 0,
                        Keycode::Key1 => 1,
                        Keycode::Key2 => 2,
                        Keycode::Key3 => 3,
                        _ => unreachable!(),
                    };
                    let mut type_idx = current_filter_type_idx.lock().unwrap();
                    *type_idx = new_type_idx;
                    let filter_type = FILTER_TYPES[*type_idx];
                    println!("Filter type: {filter_type:?}");
                    let mut player = player.lock().unwrap();
                    player
                        .send_effect_message(
                            loop_filter_effect_id,
                            effects::FilterEffectMessage::SetFilterType(filter_type),
                            None,
                        )
                        .unwrap_or_default();
                    let cutoff = if filter_type == effects::FilterEffectType::Allpass {
                        player.output_sample_rate() as f32 / 2.0
                    } else {
                        *current_filter_cutoff.lock().unwrap()
                    };
                    player
                        .send_effect_message(
                            loop_filter_effect_id,
                            effects::FilterEffectMessage::SetCutoff(cutoff),
                            None,
                        )
                        .unwrap_or_default();
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
                Keycode::Right if alt_key => {
                    let mut player = player.lock().unwrap();
                    let mut cutoff = current_filter_cutoff.lock().unwrap();
                    *cutoff = (*cutoff * 1.1_f32).min(player.output_sample_rate() as f32 / 2.0);
                    println!("Filter cutoff: {:.0} Hz", *cutoff);
                    player
                        .send_effect_message(
                            loop_filter_effect_id,
                            effects::FilterEffectMessage::SetCutoff(*cutoff),
                            None,
                        )
                        .unwrap_or_default();
                }
                Keycode::Left if alt_key => {
                    let mut cutoff = current_filter_cutoff.lock().unwrap();
                    *cutoff = (*cutoff / 1.1_f32).max(20.0);
                    println!("Filter cutoff: {:.0} Hz", *cutoff);
                    let mut player = player.lock().unwrap();
                    player
                        .send_effect_message(
                            loop_filter_effect_id,
                            effects::FilterEffectMessage::SetCutoff(*cutoff),
                            None,
                        )
                        .unwrap_or_default();
                }
                Keycode::Left => {
                    let mut current = current_loop_seek_start.lock().unwrap();
                    *current = Duration::from_secs_f32(0_f32.max(current.as_secs_f32() - 0.5));
                    let mut player = player.lock().unwrap();
                    let _ = player.seek_source(loop_playback_id, *current, None);
                    println!("Seeked loop to pos: {pos} sec", pos = current.as_secs_f32());
                }
                Keycode::Right => {
                    let mut current = current_loop_seek_start.lock().unwrap();
                    *current = Duration::from_secs_f32(4_f32.min(current.as_secs_f32() + 0.5));
                    let mut player = player.lock().unwrap();
                    let _ = player.seek_source(loop_playback_id, *current, None);
                    println!("Seeked loop to pos: {pos} sec", pos = current.as_secs_f32())
                }
                _ => {
                    if let Some(relative_note) = key_to_note(key) {
                        let playmode = *current_playmode.lock().unwrap();
                        let octave = *current_octave.lock().unwrap();
                        let final_note = (relative_note + 12 * octave) as u8;

                        let mut player = player.lock().unwrap();
                        let mut playing_synth_ids = playing_synth_ids.lock().unwrap();

                        let playback_id =
                            handle_note_on(&mut player, final_note, playmode, tone_mixer_id);
                        playing_synth_ids.insert(*key, playback_id);
                    }
                }
            }
        }
    });

    // key up handler
    let _key_up_guard = event_handler.on_key_up({
        let player = Arc::clone(&player);
        let playing_synth_ids = Arc::clone(&playing_synth_ids);

        let alt_key_pressed = Arc::clone(&alt_key_pressed);

        move |key: &Keycode| match key {
            Keycode::LAlt | Keycode::RAlt => {
                alt_key_pressed.store(false, Ordering::Relaxed);
            }
            _ => {
                if key_to_note(key).is_some() {
                    let mut player = player.lock().unwrap();
                    let mut playing_synth_ids = playing_synth_ids.lock().unwrap();
                    if let Some(playback_id) = playing_synth_ids.get(key) {
                        handle_note_off(&mut player, *playback_id);
                        playing_synth_ids.remove(key);
                    }
                }
            }
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
    player: &mut Player,
    note: u8,
    playmode: PlayMode,
    mixer_id: MixerId,
) -> PlaybackId {
    // create, then play a synth or sample source and return the playback_id
    if playmode == PlayMode::Synth {
        player
            .play_synth_source(
                create_synth_source(
                    note,
                    SynthPlaybackOptions::default()
                        .volume_db(-12.0)
                        .fade_out(Duration::from_secs(1))
                        .target_mixer(mixer_id),
                    player.output_sample_rate(),
                )
                .expect("failed to create a new synth source"),
                None,
            )
            .expect("failed to play synth")
    } else {
        player
            .play_file_source(
                create_sample_source(
                    FilePlaybackOptions::default()
                        .volume_db(-6.0)
                        .speed(speed_from_note(note))
                        .fade_out(Duration::from_secs(3))
                        .target_mixer(mixer_id),
                    player.output_sample_rate(),
                )
                .expect("failed to create a new sample file"),
                None,
            )
            .expect("failed to play sample")
    }
}

fn handle_note_off(player: &mut Player, playback_id: PlaybackId) {
    // stop playing source with the given playback_id
    player.stop_source(playback_id, None).unwrap_or_default();
}

// -------------------------------------------------------------------------------------------------

fn create_synth_source(
    note: u8,
    options: SynthPlaybackOptions,
    sample_rate: u32,
) -> Result<DaspSynthSource<impl Signal<Frame = f64>>, Error> {
    let pitch = pitch_from_note(note);
    let duration_in_ms = 1000;
    let duration_in_samples = (sample_rate as f64 / duration_in_ms as f64 * 1000.0) as usize;
    // stack up slightly detuned sine waves
    let fundamental = signal::rate(sample_rate as f64).const_hz(pitch);
    let harmonic_l1 = signal::rate(sample_rate as f64).const_hz(pitch * 2.01);
    let harmonic_h1 = signal::rate(sample_rate as f64).const_hz(pitch / 2.02);
    let harmonic_h2 = signal::rate(sample_rate as f64).const_hz(pitch / 4.04);
    // combine them, limit duration and apply a fade-out envelope
    let signal = signal::from_iter(
        fundamental
            .sine()
            .add_amp(harmonic_l1.sine().scale_amp(0.5))
            .add_amp(harmonic_h1.sine().scale_amp(0.5))
            .add_amp(harmonic_h2.sine().scale_amp(0.5))
            .take(duration_in_samples)
            .zip(0..duration_in_samples)
            .map(move |(s, index)| {
                let env: f64 = (1.0 - (index as f64) / (duration_in_samples as f64)).powf(2.0);
                s * env
            }),
    );
    DaspSynthSource::new(
        signal,
        format!("Synth Note #{note}").as_str(),
        options,
        sample_rate,
        None,
    )
}

// -------------------------------------------------------------------------------------------------

fn create_sample_source(
    options: FilePlaybackOptions,
    sample_rate: u32,
) -> Result<PreloadedFileSource, Error> {
    // load and decode sample once - lazily
    lazy_static! {
        static ref SAMPLE_SOURCE: PreloadedFileSource = PreloadedFileSource::from_file(
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
