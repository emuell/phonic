//! An example showcasing interactive audio playback, with real-time control over effects and sources.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    time::Duration,
};

use device_query::{DeviceEvents, DeviceEventsHandler, Keycode};

use phonic::{
    effects::{self, FilterEffectType},
    generators::{FunDspGenerator, Sampler},
    utils::ahdsr::AhdsrParameters,
    Error, FilePlaybackOptions, GeneratorPlaybackHandle, GeneratorPlaybackOptions, NotePlaybackId,
    ResamplingQuality,
};

// -------------------------------------------------------------------------------------------------

#[cfg(all(debug_assertions, feature = "assert-allocs"))]
#[global_allocator]
static A: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

// -------------------------------------------------------------------------------------------------

// Common example code
#[path = "./common/arguments.rs"]
mod arguments;

// FunDSP synth example
#[path = "./common/synths/fm3.rs"]
mod fm3_synth;

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

    let loop_mixer;
    let loop_filter_effect;
    {
        // create a new mixer
        loop_mixer = player.add_mixer(None)?;

        // add a filter effect
        const DEFAULT_FILTER_TYPE: effects::FilterEffectType = effects::FilterEffectType::Lowpass;
        const DEFAULT_FILTER_CUTOFF: f32 = 20000.0;
        const DEFAULT_FILTER_Q: f32 = 0.707;

        loop_filter_effect = player.add_effect(
            effects::FilterEffect::with_parameters(
                DEFAULT_FILTER_TYPE,
                DEFAULT_FILTER_CUTOFF,
                DEFAULT_FILTER_Q,
            ),
            loop_mixer.id(),
        )?;
    }

    // tone mixer
    let tone_mixer;
    {
        // create a new mixer
        tone_mixer = player.add_mixer(None)?;
        // add a reverb effect
        player.add_effect(
            effects::ReverbEffect::with_parameters(0.6, 0.8),
            tone_mixer.id(),
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
            .target_mixer(loop_mixer.id()),
    )?;

    // Create FunDSP synth generator with FM synthesis
    let synth_generator = player.play_generator_source(
        FunDspGenerator::with_parameters(
            "fm_synth",
            &fm3_synth::parameters(),
            None,
            fm3_synth::voice_factory,
            GeneratorPlaybackOptions::default()
                .voices(8)
                .target_mixer(tone_mixer.id()),
            player.output_sample_rate(),
        )?,
        None,
    )?;

    // Create sampler generator for sample-based playback
    let sampler_generator = player.play_generator_source(
        Sampler::from_file(
            "assets/pad-ambient.wav",
            Some(AhdsrParameters::new(
                Duration::ZERO,         // attack
                Duration::ZERO,         // hold
                Duration::ZERO,         // decay
                1.0,                    // sustain
                Duration::from_secs(3), // release
            )?),
            GeneratorPlaybackOptions::default()
                .voices(8)
                .target_mixer(tone_mixer.id()),
            player.output_channel_count(),
            player.output_sample_rate(),
        )?,
        None,
    )?;

    // create condvar to block the main thread
    let wait_mutex_cond = Arc::new((Mutex::new(()), Condvar::new()));

    // create global playback state
    let playing_notes = Arc::new(Mutex::new(
        HashMap::<Keycode, (PlayMode, NotePlaybackId)>::new(),
    ));
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
    println!("  To play a funDSP signal synth, hit key '1'. For a sample based synth hit key '2'.");
    println!();
    println!("  Alt + Arrow 'left/right' to change filter cutoff frequency.");
    println!("  Alt + 1,2,3,4 to change filter type (LP,BP,BR,HP).");
    println!("  Alt + R to randomize the FM synths parameters.");
    println!();
    println!("  NB: this example uses a HighQuality resampler for the loop. ");
    println!("  In debug builds this may be very slow and may thus cause crackles...");
    println!();
    println!("  To quit press 'Esc'.");
    println!();

    // Print DSP graph
    println!("Player Graph:\n{}", player);

    // run key event handlers to play, stop and modify sounds interactively
    let event_handler = DeviceEventsHandler::new(Duration::from_millis(10))
        .expect("Could not initialize event loop");

    // key down handler
    let _key_down_guard = event_handler.on_key_down({
        let wait_mutex_cond = Arc::clone(&wait_mutex_cond);
        let playing_notes = Arc::clone(&playing_notes);
        let synth_generator = synth_generator.clone();
        let sampler_generator = sampler_generator.clone();

        let current_playmode = Arc::clone(&current_playmode);
        let current_octave = Arc::clone(&current_octave);
        let current_filter_cutoff = Arc::clone(&current_filter_cutoff);

        let alt_key_pressed = Arc::clone(&alt_key_pressed);

        move |key: &Keycode| {
            let synth_generator = synth_generator.clone();
            let sampler_generator = sampler_generator.clone();
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
                    loop_filter_effect
                        .set_parameter(effects::FilterEffect::TYPE.id(), filter_type, None)
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
                    let mut cutoff = current_filter_cutoff.lock().unwrap();
                    *cutoff = (*cutoff * 1.1_f32).min(20000.0);
                    println!("Filter cutoff: {:.0} Hz", *cutoff);
                    loop_filter_effect
                        .set_parameter(effects::FilterEffect::CUTOFF.id(), *cutoff, None)
                        .unwrap_or_default();
                }
                Keycode::Left if alt_key => {
                    let mut cutoff = current_filter_cutoff.lock().unwrap();
                    *cutoff = (*cutoff / 1.1_f32).max(20.0);
                    println!("Filter cutoff: {:.0} Hz", *cutoff);
                    loop_filter_effect
                        .set_parameter(effects::FilterEffect::CUTOFF.id(), *cutoff, None)
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
                Keycode::R if alt_key => {
                    fm3_synth::randomize(&synth_generator).unwrap();
                    println!("Randomized FM synth params");
                }
                _ => {
                    if let Some(relative_note) = key_to_note(key) {
                        let playmode = *current_playmode.lock().unwrap();
                        let octave = *current_octave.lock().unwrap();
                        let final_note = (relative_note + 12 * octave) as u8;

                        let mut playing_notes = playing_notes.lock().unwrap();

                        let note_id = handle_note_on(
                            &synth_generator,
                            &sampler_generator,
                            final_note,
                            playmode,
                        );
                        playing_notes.insert(*key, (playmode, note_id));
                    }
                }
            }
        }
    });

    // key up handler
    let _key_up_guard = event_handler.on_key_up({
        let playing_notes = Arc::clone(&playing_notes);
        let synth_generator = synth_generator.clone();
        let sampler_generator = sampler_generator.clone();
        let alt_key_pressed = Arc::clone(&alt_key_pressed);

        move |key: &Keycode| match key {
            Keycode::LAlt | Keycode::RAlt => {
                alt_key_pressed.store(false, Ordering::Relaxed);
            }
            _ => {
                if key_to_note(key).is_some() {
                    let mut playing_notes = playing_notes.lock().unwrap();
                    if let Some((playmode, note_id)) = playing_notes.remove(key) {
                        handle_note_off(&synth_generator, &sampler_generator, playmode, note_id);
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
    synth_generator: &GeneratorPlaybackHandle,
    sampler_generator: &GeneratorPlaybackHandle,
    note: u8,
    playmode: PlayMode,
) -> NotePlaybackId {
    if playmode == PlayMode::Synth {
        synth_generator
            .note_on(note, Some(0.5), None, None)
            .expect("failed to trigger synth note")
    } else {
        sampler_generator
            .note_on(note, Some(0.5), None, None)
            .expect("failed to trigger sampler note")
    }
}

fn handle_note_off(
    synth_generator: &GeneratorPlaybackHandle,
    sampler_generator: &GeneratorPlaybackHandle,
    playmode: PlayMode,
    note_id: NotePlaybackId,
) {
    if playmode == PlayMode::Synth {
        let _ = synth_generator.note_off(note_id, None);
    } else {
        let _ = sampler_generator.note_off(note_id, None);
    }
}
