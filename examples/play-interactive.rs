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
    effects::{self, FilterEffectType},
    sources::{DaspSynthSource, PreloadedFileSource},
    utils::{pitch_from_note, speed_from_note},
    Error, FilePlaybackOptions, MixerId, PlaybackHandle, Player, ResamplingQuality,
    SynthPlaybackOptions,
};

// -------------------------------------------------------------------------------------------------

#[cfg(all(debug_assertions, feature = "assert_no_alloc"))]
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

    if args.output_path.is_some() {
        return Err(Error::ParameterError(
            "The interactive example only supports real-time playback ".to_owned()
                + "and thus does not support the wav-writer 'output' argument",
        ));
    }

    // Create a player instance with the output device as configured via program arguments
    let mut player = arguments::new_player(&args, None)?;

    let loop_mixer_id;
    let loop_filter_effect_id;
    {
        // create a new mixer
        loop_mixer_id = player.add_mixer(None)?;

        // add a filter effect
        const DEFAULT_FILTER_TYPE: effects::FilterEffectType = effects::FilterEffectType::Lowpass;
        const DEFAULT_FILTER_CUTOFF: f32 = 20000.0;
        const DEFAULT_FILTER_Q: f32 = 0.707;

        loop_filter_effect_id = player.add_effect(
            effects::FilterEffect::with_parameters(
                DEFAULT_FILTER_TYPE,
                DEFAULT_FILTER_CUTOFF,
                DEFAULT_FILTER_Q,
            ),
            loop_mixer_id,
        )?;
    }

    // tone mixer
    let tone_mixer_id;
    {
        // create a new mixer
        tone_mixer_id = player.add_mixer(None)?;

        // add a chorus effect
        // player.add_effect(effects::ChorusEffect::default(), tone_mixer_id)?;

        // add a reverb effect
        player.add_effect(
            effects::ReverbEffect::with_parameters(0.6, 0.8),
            tone_mixer_id,
        )?;
    }

    // add a dc filter effect to the main mixer
    player.add_effect(effects::DcFilterEffect::default(), None)?;

    // start playing the background loop
    let loop_file = player.play_file(
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
    let playing_notes = Arc::new(Mutex::new(HashMap::<Keycode, PlaybackHandle>::new()));
    let current_playmode = Arc::new(Mutex::new(PlayMode::Synth));
    let current_octave = Arc::new(Mutex::new(5));
    let current_loop_seek_start = Arc::new(Mutex::new(Duration::ZERO));
    let current_filter_cutoff = Arc::new(Mutex::new(20000.0));

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
    println!("  Alt + 1,2,3,4 to change filter type (LP,BP,BR,HP).");
    println!();
    println!("  NB: this example uses a HighQuality resampler for the loop. ");
    println!("  In debug builds this may be very slow and may thus cause crackles...");
    println!();
    println!("  To quit press 'Esc'.");
    println!();

    // run key event handlers to play, stop and modify sounds interactively
    let event_handler = DeviceEventsHandler::new(Duration::from_millis(10))
        .expect("Could not initialize event loop");

    // key down handler
    let _key_down_guard = event_handler.on_key_down({
        let wait_mutex_cond = Arc::clone(&wait_mutex_cond);
        let player = Arc::clone(&player);
        let playing_notes = Arc::clone(&playing_notes);

        let current_playmode = Arc::clone(&current_playmode);
        let current_octave = Arc::clone(&current_octave);
        let current_filter_cutoff = Arc::clone(&current_filter_cutoff);

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
                Keycode::Key1 | Keycode::Key2 | Keycode::Key3 | Keycode::Key4 if alt_key => {
                    let filter_type = match key {
                        Keycode::Key1 => FilterEffectType::Lowpass,
                        Keycode::Key2 => FilterEffectType::Bandpass,
                        Keycode::Key3 => FilterEffectType::Bandstop,
                        Keycode::Key4 => FilterEffectType::Highpass,
                        _ => unreachable!(),
                    };
                    println!("Filter type: {filter_type}");
                    let mut player = player.lock().unwrap();
                    player
                        .set_effect_parameter(
                            loop_filter_effect_id,
                            effects::FilterEffect::TYPE_ID,
                            filter_type,
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
                    *cutoff = (*cutoff * 1.1_f32)
                        .min(20000.0)
                        .min(player.output_sample_rate() as f32 / 2.0);
                    println!("Filter cutoff: {:.0} Hz", *cutoff);
                    player
                        .set_effect_parameter(
                            loop_filter_effect_id,
                            effects::FilterEffect::CUTOFF_ID,
                            *cutoff,
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
                        .set_effect_parameter(
                            loop_filter_effect_id,
                            effects::FilterEffect::CUTOFF_ID,
                            *cutoff,
                            None,
                        )
                        .unwrap_or_default();
                }
                Keycode::Left => {
                    let mut current = current_loop_seek_start.lock().unwrap();
                    *current = Duration::from_secs_f32(0_f32.max(current.as_secs_f32() - 0.5));
                    let _ = loop_file.seek(*current, None);
                    println!("Seeked loop to pos: {pos} sec", pos = current.as_secs_f32());
                }
                Keycode::Right => {
                    let mut current = current_loop_seek_start.lock().unwrap();
                    *current = Duration::from_secs_f32(4_f32.min(current.as_secs_f32() + 0.5));
                    let _ = loop_file.seek(*current, None);
                    println!("Seeked loop to pos: {pos} sec", pos = current.as_secs_f32())
                }
                _ => {
                    if let Some(relative_note) = key_to_note(key) {
                        let playmode = *current_playmode.lock().unwrap();
                        let octave = *current_octave.lock().unwrap();
                        let final_note = (relative_note + 12 * octave) as u8;

                        let mut player = player.lock().unwrap();
                        let mut playing_notes = playing_notes.lock().unwrap();

                        let note_handle =
                            handle_note_on(&mut player, final_note, playmode, tone_mixer_id);
                        playing_notes.insert(*key, note_handle);
                    }
                }
            }
        }
    });

    // key up handler
    let _key_up_guard = event_handler.on_key_up({
        let playing_notes = Arc::clone(&playing_notes);
        let alt_key_pressed = Arc::clone(&alt_key_pressed);

        move |key: &Keycode| match key {
            Keycode::LAlt | Keycode::RAlt => {
                alt_key_pressed.store(false, Ordering::Relaxed);
            }
            _ => {
                if key_to_note(key).is_some() {
                    let mut playing_notes = playing_notes.lock().unwrap();
                    if let Some(handle) = playing_notes.remove(key) {
                        handle_note_off(handle);
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
) -> PlaybackHandle {
    // create, then play a synth or sample source and return the handle
    if playmode == PlayMode::Synth {
        PlaybackHandle::Synth(
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
                .expect("failed to play synth"),
        )
    } else {
        PlaybackHandle::File(
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
                .expect("failed to play sample"),
        )
    }
}

fn handle_note_off(handle: PlaybackHandle) {
    // ignore result, source maybe no longer plays
    let _ = handle.stop(None);
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
