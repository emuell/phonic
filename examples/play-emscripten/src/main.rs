//! An example showcasing how to use phonic with emscripten to create a web-based audio application.

use std::{cell::RefCell, collections::HashMap, ffi};

use emscripten_rs_sys::emscripten_request_animation_frame_loop;
use four_cc::FourCC;
use serde::Serialize;

use phonic::{
    effects,
    sources::PreloadedFileSource,
    utils::{db_to_linear, pitch_from_note, speed_from_note},
    DefaultOutputDevice, Effect, EffectHandle, EffectId, Error, FilePlaybackOptions, MixerId,
    Parameter, ParameterType, Player, SynthPlaybackHandle, SynthPlaybackOptions,
};

// -------------------------------------------------------------------------------------------------

// Serializable parameter metadata for JavaScript

#[derive(Serialize)]
#[serde(tag = "type")]
enum ParamTypeInfo {
    Float,
    Integer,
    Enum { values: Vec<String> },
    Boolean,
}

#[derive(Serialize)]
struct ParameterInfo {
    id: u32,
    name: String,
    #[serde(flatten)]
    param_type: ParamTypeInfo,
    default: f32,
}

#[derive(Serialize)]
struct EffectInfo {
    name: String,
    parameters: Vec<ParameterInfo>,
}

impl From<&dyn Effect> for EffectInfo {
    fn from(effect: &dyn Effect) -> Self {
        let params = effect.parameters();
        let parameters = params
            .iter()
            .map(|p| {
                let id: u32 = p.id().into();
                let name = p.name().to_string();
                let param_type = match p.parameter_type() {
                    ParameterType::Float => ParamTypeInfo::Float,
                    ParameterType::Integer => ParamTypeInfo::Integer,
                    ParameterType::Enum { values } => ParamTypeInfo::Enum { values },
                    ParameterType::Boolean => ParamTypeInfo::Boolean,
                };
                let default = p.default_value();
                ParameterInfo {
                    id,
                    name,
                    param_type,
                    default,
                }
            })
            .collect();

        EffectInfo {
            name: effect.name().to_string(),
            parameters,
        }
    }
}

// -------------------------------------------------------------------------------------------------

// Hold the data structures statically so we can bind the Emscripten C method callbacks.
thread_local!(static APP: RefCell<Option<App>> = const { RefCell::new(None) });

struct App {
    player: Player,
    playback_beat_counter: u32,
    playback_start_time: u64,
    playing_synth_notes: HashMap<u8, SynthPlaybackHandle>,
    samples: Vec<PreloadedFileSource>,
    synth_mixer_id: MixerId,
    active_effects: HashMap<EffectId, (EffectHandle, Vec<Box<dyn Parameter>>)>,
}

impl App {
    // Create a new player and preload samples
    pub fn new() -> Result<Self, Error> {
        println!("Initialize audio output...");
        let output = DefaultOutputDevice::open()?;

        println!("Creating audio file player...");
        let mut player = Player::new(output, None);
        let sample_rate = player.output_sample_rate();

        // lower master volume a bit
        player.set_output_volume(db_to_linear(-3.0));

        // create a new mixer for the synth
        let synth_mixer_id = player.add_mixer(None)?;

        // maintain added effects
        let active_effects = HashMap::new();

        println!("Preloading sample files...");
        let mut samples = Vec::new();
        for sample in ["./assets/cowbell.wav", "./assets/bass.wav"] {
            match PreloadedFileSource::from_file(
                sample,
                None,
                FilePlaybackOptions::default(),
                sample_rate,
            ) {
                Ok(sample) => samples.push(sample),
                Err(err) => return Err(err),
            }
        }

        println!("Start running...");
        unsafe {
            emscripten_request_animation_frame_loop(Some(Self::run_frame), std::ptr::null_mut())
        };

        // start playback in a second from now
        let playback_start_time =
            player.output_sample_frame_position() + player.output_sample_rate() as u64;
        let playback_beat_counter = 0;

        let playing_synth_notes = HashMap::new();

        Ok(Self {
            player,
            playback_start_time,
            playback_beat_counter,
            playing_synth_notes,
            samples,
            synth_mixer_id,
            active_effects,
        })
    }

    // Animation frame callback which drives the player
    extern "C" fn run_frame(_time: f64, _user_data: *mut ffi::c_void) -> bool {
        APP.with_borrow_mut(|app| {
            // is a player running?
            if let Some(app) = app {
                app.run();
                true // continue running
            } else {
                false // stop running
            }
        })
    }

    // Create a new synth source.
    fn create_synth_note(note: u8) -> Box<dyn fundsp::audiounit::AudioUnit> {
        use fundsp::hacker32::*;
        let freq = shared(pitch_from_note(note) as f32);
        let fundamental = var(&freq) >> sine();
        let harmonic_l1 = (var(&freq) * 2.01) >> sine();
        let harmonic_h1 = (var(&freq) * 0.51) >> sine();
        let harmonic_h2 = (var(&freq) * 0.249) >> sine();
        let summed =
            (fundamental + harmonic_l1 * 0.5 + harmonic_h1 * 0.5 + harmonic_h2 * 0.5) * 0.3;
        let envelope = adsr_live(0.001, 0.1, 0.7, 0.5);
        Box::new(envelope * summed)
    }

    // Schedule synth note on for playback
    fn synth_note_on(&mut self, note: u8) {
        if let Some(playing_note) = self.playing_synth_notes.get(&note) {
            let _ = playing_note.stop(None);
            self.playing_synth_notes.remove(&note);
        }
        if let Ok(playing_note) = self.player.play_fundsp_synth(
            "synth_note",
            Self::create_synth_note(note),
            SynthPlaybackOptions::default().target_mixer(self.synth_mixer_id),
        ) {
            self.playing_synth_notes.insert(note, playing_note);
        }
    }

    // Stop a scheduled synth note on
    fn synth_note_off(&mut self, note: u8) {
        if let Some(handle) = self.playing_synth_notes.get(&note) {
            let _ = handle.stop(None);
            self.playing_synth_notes.remove(&note);
        }
    }

    // Get list of available effects
    fn get_available_effects() -> Vec<String> {
        vec![
            "Gain".to_string(),
            "DcFilter".to_string(),
            "Filter".to_string(),
            "Eq5".to_string(),
            "Reverb".to_string(),
            "Chorus".to_string(),
            "Compressor".to_string(),
            "Distortion".to_string(),
        ]
    }

    // Add an effect by name to the synth mixer, returning effect ID and parameter metadata JSON
    fn add_effect_with_name(&mut self, effect_name: &str) -> Result<(EffectId, String), Error> {
        match effect_name {
            "Gain" => self.add_effect(effects::GainEffect::new()),
            "DcFilter" => self.add_effect(effects::DcFilterEffect::new()),
            "Filter" => self.add_effect(effects::FilterEffect::new()),
            "Eq5" => self.add_effect(effects::Eq5Effect::new()),
            "Reverb" => self.add_effect(effects::ReverbEffect::new()),
            "Chorus" => self.add_effect(effects::ChorusEffect::new()),
            "Compressor" => self.add_effect(effects::CompressorEffect::new_compressor()),
            "Distortion" => self.add_effect(effects::DistortionEffect::new()),
            _ => {
                return Err(Error::ParameterError(format!(
                    "Unknown effect: {}",
                    effect_name
                )))
            }
        }
    }

    // Add given effect instance to the synth mixer, returning effect ID and parameter metadata JSON
    fn add_effect<E: Effect>(&mut self, effect: E) -> Result<(EffectId, String), Error> {
        // Store parameter metadata
        let parameters = effect
            .parameters()
            .iter()
            .map(|p| p.dyn_clone())
            .collect::<Vec<_>>();
        let info_json = serde_json::to_string(&EffectInfo::from(&effect as &dyn Effect))
            .unwrap_or_else(|_| "{}".to_string());
        let effect_handle = self.player.add_effect(effect, self.synth_mixer_id)?;
        let effect_id = effect_handle.id();

        self.active_effects
            .insert(effect_id, (effect_handle, parameters));

        Ok((effect_id, info_json))
    }

    // Remove an effect
    fn remove_effect(&mut self, effect_id: EffectId) -> Result<(), Error> {
        if let Some((_, _)) = self.active_effects.remove(&effect_id) {
            self.player.remove_effect(effect_id)?;
            Ok(())
        } else {
            Err(Error::EffectNotFoundError(effect_id))
        }
    }

    // Set an effect parameter value and update our tracking
    fn set_effect_parameter_value(
        &mut self,
        effect_id: EffectId,
        param_id: FourCC,
        normalized_value: f32,
    ) -> Result<(), Error> {
        let (effect_handle, _) = self
            .active_effects
            .get(&effect_id)
            .ok_or(Error::EffectNotFoundError(effect_id))?;

        effect_handle.set_parameter_normalized(param_id, normalized_value.clamp(0.0, 1.0), None)
    }

    // Get an effect parameter's value as a string
    fn get_effect_parameter_string(
        &self,
        effect_id: EffectId,
        param_id: FourCC,
        normalized_value: f32,
    ) -> Result<String, Error> {
        let (_, parameters) = self
            .active_effects
            .get(&effect_id)
            .ok_or(Error::EffectNotFoundError(effect_id))?;

        // Find the parameter
        let parameter =
            parameters
                .iter()
                .find(|p| p.id() == param_id)
                .ok_or(Error::ParameterError(format!(
                    "Parameter {:?} not found in effect {}",
                    param_id, effect_id
                )))?;

        // Convert to string using the parameter's method
        Ok(parameter.value_to_string(normalized_value, true))
    }

    // Schedule samples for playback
    fn run(&mut self) {
        // time consts
        const BEATS_PER_MIN: f64 = 120.0;
        const BEATS_PER_BAR: u32 = 4;

        // calculate metronome speed and signature
        let sample_rate = self.player.output_sample_rate();
        let samples_per_sec = self.player.output_sample_rate();
        let samples_per_beat = samples_per_sec as f64 * 60.0 / BEATS_PER_MIN;

        // schedule playback events one second ahead of the players current time
        let preroll_time = samples_per_sec as u64;

        // when is the next beat playback due?
        let next_beats_sample_time = (self.playback_start_time as f64
            + self.playback_beat_counter as f64 * samples_per_beat)
            as u64;
        let output_sample_time = self.player.output_sample_frame_position();

        // schedule next sample when it's due within the preroll time, else do nothing
        if next_beats_sample_time > output_sample_time + preroll_time
            || self.playback_beat_counter == 0
        {
            // play an octave higher every new bar start
            let sample_speed = speed_from_note(
                if self.playback_beat_counter.is_multiple_of(BEATS_PER_BAR) {
                    72
                } else {
                    60
                },
            );
            // select a new sample every 2 bars
            let sample_index =
                (self.playback_beat_counter / (2 * BEATS_PER_BAR)) as usize % self.samples.len();
            // clone the preloaded sample
            let sample = self.samples[sample_index]
                .clone(
                    FilePlaybackOptions::default().speed(sample_speed),
                    sample_rate,
                )
                .unwrap();

            // play it at the new beat's time
            if let Ok(playback_handle) = self
                .player
                .play_file_source(sample, Some(next_beats_sample_time))
            {
                // and stop it again (fade out) before the next beat starts
                let _ = playback_handle.stop(next_beats_sample_time + samples_per_beat as u64);
            }

            // advance beat counter
            self.playback_beat_counter += 1;
        }
    }
}

// -------------------------------------------------------------------------------------------------

fn main() {
    // Disabled build.rs via `cargo::rustc-link-arg=--no-entry`
    panic!("The main function is not exposed and should never be called");
}

// -------------------------------------------------------------------------------------------------

/// Frees a string ptr which got passed to JS after it got consumed. Exported as `_free_cstring`
/// function in the WASM.
#[no_mangle]
pub unsafe extern "C" fn free_cstring(ptr: *mut ffi::c_char) {
    if !ptr.is_null() {
        drop(ffi::CString::from_raw(ptr as *mut ffi::c_char))
    }
}

/// Creates a new `App` Exported as `_start`
/// function in the WASM.
#[no_mangle]
pub extern "C" fn start() {
    println!("Creating new app instance...");
    match App::new() {
        Err(err) => {
            eprintln!("Failed to create player instance: {}", err);
            APP.replace(None)
        }
        Ok(app) => {
            println!("Successfully created a new app instance");
            APP.replace(Some(app))
        }
    };
}

/// Destroys `App` when its running. Exported as `_stop`
/// function in the WASM.
#[no_mangle]
pub extern "C" fn stop() {
    println!("Dropping app instance...");
    APP.replace(None);
}

/// Play a single synth note when the app is running. Exported as `_synth_note_on`
/// function in the WASM.
#[no_mangle]
pub extern "C" fn synth_note_on(key: ffi::c_int) {
    APP.with_borrow_mut(|app| {
        if let Some(app) = app {
            let note = (60 + key).min(127) as u8;
            app.synth_note_on(note);
        }
    });
}

/// Stop a previously played synth note when the player is running. Exported as `_synth_note_off`
/// function in the WASM.
#[no_mangle]
pub extern "C" fn synth_note_off(key: ffi::c_int) {
    APP.with_borrow_mut(|app| {
        if let Some(app) = app {
            let note = (60 + key).min(127) as u8;
            app.synth_note_off(note);
        }
    });
}

/// Get the list of available effects. Exported as `_get_available_effects` function in the WASM.
///
/// Returns a pointer to a JSON array of effect names, or null on error.
/// The return pointer must be freed with `_free_cstring` after getting consumed!
#[no_mangle]
pub extern "C" fn get_available_effects() -> *const ffi::c_char {
    let effects = App::get_available_effects();
    match serde_json::to_string(&effects) {
        Ok(json) => {
            let c_str = ffi::CString::new(json).unwrap_or_default();
            c_str.into_raw()
        }
        Err(err) => {
            eprintln!("Failed to serialize available effects: {}", err);
            std::ptr::null()
        }
    }
}

/// Add an effect to the synth mixer. Exported as `_add_effect` function in the WASM.
///
/// Returns a pointer to a JSON string containing effect ID and parameter metadata, or null on error.
/// The return pointer must be freed with `_free_cstring` after getting consumed!
#[no_mangle]
pub extern "C" fn add_effect(effect_name: *const ffi::c_char) -> *const ffi::c_char {
    if effect_name.is_null() {
        eprintln!("Effect name is null");
        return std::ptr::null();
    }

    let effect_name_str = unsafe {
        match ffi::CStr::from_ptr(effect_name).to_str() {
            Ok(s) => s,
            Err(err) => {
                eprintln!("Failed to convert effect name to string: {}", err);
                return std::ptr::null();
            }
        }
    };

    APP.with_borrow_mut(|app| {
        if let Some(app) = app {
            match app.add_effect_with_name(effect_name_str) {
                Ok((effect_id, params_json)) => {
                    let result_json =
                        format!(r#"{{"effectId":{},"params":{}}}"#, effect_id, params_json);
                    let c_str = ffi::CString::new(result_json).unwrap_or_default();
                    c_str.into_raw()
                }
                Err(err) => {
                    eprintln!("Failed to add {} effect: {}", effect_name_str, err);
                    std::ptr::null()
                }
            }
        } else {
            std::ptr::null()
        }
    })
}

/// Remove an effect from the synth mixer. Exported as `_remove_effect` function in the WASM.
///
/// Returns 0 on success or -1 on error.
#[no_mangle]
pub extern "C" fn remove_effect(effect_id: ffi::c_int) -> ffi::c_int {
    APP.with_borrow_mut(|app| {
        if let Some(app) = app {
            match app.remove_effect(effect_id as usize) {
                Ok(_) => 0,
                Err(err) => {
                    eprintln!("Failed to remove effect {}: {}", effect_id, err);
                    -1
                }
            }
        } else {
            -1
        }
    })
}

/// Get an effect parameter's value as a string. Exported as `_get_effect_parameter_string`
/// function in the WASM.
///
/// Returns a pointer to a C string containing the parameter value, or null on error.
/// The return pointer must be freed with `_free_cstring` after getting consumed!
#[no_mangle]
pub extern "C" fn get_effect_parameter_string(
    effect_id: ffi::c_int,
    param_id: ffi::c_uint,
    normalized_value: ffi::c_float,
) -> *const ffi::c_char {
    APP.with_borrow_mut(|app| {
        if let Some(app) = app {
            let param_fourcc = FourCC::from(param_id);
            match app.get_effect_parameter_string(
                effect_id as usize,
                param_fourcc,
                normalized_value,
            ) {
                Ok(value_string) => {
                    let c_str = ffi::CString::new(value_string).unwrap_or_default();
                    c_str.into_raw()
                }
                Err(err) => {
                    eprintln!(
                        "Failed to get effect {} parameter {:?} string: {}",
                        effect_id, param_fourcc, err
                    );
                    std::ptr::null()
                }
            }
        } else {
            std::ptr::null()
        }
    })
}

/// Set an effect parameter value (normalized 0.0-1.0). Exported as `_set_effect_parameter_value`
/// function in the WASM.
///
/// Returns 0 on success or -1 on error.
#[no_mangle]
pub extern "C" fn set_effect_parameter_value(
    effect_id: ffi::c_int,
    param_id: ffi::c_uint,
    value: ffi::c_float,
) -> ffi::c_int {
    APP.with_borrow_mut(|app| {
        if let Some(app) = app {
            let param_fourcc = FourCC::from(param_id);
            match app.set_effect_parameter_value(effect_id as usize, param_fourcc, value) {
                Ok(_) => 0,
                Err(err) => {
                    eprintln!(
                        "Failed to set effect {} parameter {:?} to {}: {}",
                        effect_id, param_fourcc, value, err
                    );
                    -1
                }
            }
        } else {
            -1
        }
    })
}

// Note: when adding new functions that should be exported in the WASM,
// adjust `cargo::rustc-link-arg=-sEXPORTED_FUNCTIONS` print in `build.rs`
