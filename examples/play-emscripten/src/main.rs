//! An example showcasing how to use phonic with emscripten to create a web-based audio application.

use std::ffi;

use four_cc::FourCC;

// -------------------------------------------------------------------------------------------------

mod app;
use app::*;

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

/// Get the player's main mixer CPU load. Exported as `_get_cpu_load` function in the WASM.
///
/// Returns the average CPU load as a float.
#[no_mangle]
pub extern "C" fn get_cpu_load() -> ffi::c_float {
    APP.with_borrow(|app| {
        if let Some(app) = app.as_ref() {
            app.cpu_load()
        } else {
            0.0
        }
    })
}

/// Set the active synth.
#[no_mangle]
pub extern "C" fn set_active_synth(synth_type: ffi::c_int) {
    APP.with_borrow_mut(|app| {
        if let Some(app) = app {
            if let Err(err) = app.set_active_synth(synth_type) {
                eprintln!("Failed to set active synth to {}: {}", synth_type, err);
            }
        }
    });
}

/// Set the voice count for all synths. Exported as `_set_synth_voice_count` function in the WASM.
#[no_mangle]
pub extern "C" fn set_synth_voice_count(voice_count: ffi::c_int) {
    APP.with_borrow_mut(|app| {
        if let Some(app) = app {
            let voice_count = voice_count.max(1) as usize;
            if let Err(err) = app.set_synth_voice_count(voice_count) {
                eprintln!("Failed to set voice count to {}: {}", voice_count, err);
            }
        }
    });
}

/// Set metronome enabled state. Exported as `_set_metronome_enabled` function in the WASM.
#[no_mangle]
pub extern "C" fn set_metronome_enabled(enabled: bool) {
    APP.with_borrow_mut(|app| {
        if let Some(app) = app {
            app.set_metronome_enabled(enabled);
        }
    });
}

/// Get the active synth parameters. Exported as `_get_synth_parameters` function in the WASM.
///
/// Returns a pointer to a JSON string containing parameter metadata, or null on error.
/// The return pointer must be freed with `_free_cstring` after getting consumed!
#[no_mangle]
pub extern "C" fn get_synth_parameters() -> *const ffi::c_char {
    APP.with_borrow(|app| {
        if let Some(app) = app.as_ref() {
            let info = app.get_active_synth_parameters();
            match serde_json::to_string(&info) {
                Ok(json) => {
                    let c_str = ffi::CString::new(json).unwrap_or_default();
                    c_str.into_raw()
                }
                Err(err) => {
                    eprintln!("Failed to serialize synth parameters: {}", err);
                    std::ptr::null()
                }
            }
        } else {
            std::ptr::null()
        }
    })
}

/// Set a synth parameter value (normalized 0.0-1.0). Exported as `_set_synth_parameter_value` function in the WASM.
#[no_mangle]
pub extern "C" fn set_synth_parameter_value(param_id: ffi::c_uint, value: ffi::c_float) {
    APP.with_borrow_mut(|app| {
        if let Some(app) = app {
            let param_fourcc = FourCC::from(param_id);
            app.set_synth_parameter_value(param_fourcc, value);
        }
    });
}

/// Get a synth parameter's value as a string. Exported as `_synth_parameter_value_to_string`
/// function in the WASM.
///
/// Returns a pointer to a C string containing the parameter value, or null on error.
/// The return pointer must be freed with `_free_cstring` after getting consumed!
#[no_mangle]
pub extern "C" fn synth_parameter_value_to_string(
    param_id: ffi::c_uint,
    normalized_value: ffi::c_float,
) -> *const ffi::c_char {
    APP.with_borrow_mut(|app| {
        if let Some(app) = app {
            let param_fourcc = FourCC::from(param_id);
            match app.synth_parameter_value_to_string(param_fourcc, normalized_value) {
                Ok(value_string) => {
                    let c_str = ffi::CString::new(value_string).unwrap_or_default();
                    c_str.into_raw()
                }
                Err(err) => {
                    eprintln!(
                        "Failed to get synth parameter {:?} string: {}",
                        param_fourcc, err
                    );
                    std::ptr::null()
                }
            }
        } else {
            std::ptr::null()
        }
    })
}

/// Convert an synth parameter's string value to a normalized value.
/// Exported as `_synth_parameter_string_to_value` function in the WASM.
///
/// Returns a normalized parameter value, or NaN on error.
#[no_mangle]
pub extern "C" fn synth_parameter_string_to_value(
    param_id: ffi::c_uint,
    string: *const ffi::c_char,
) -> ffi::c_float {
    if string.is_null() {
        eprintln!("Parameter string is null");
        return ffi::c_float::NAN;
    }
    let string = unsafe {
        match ffi::CStr::from_ptr(string).to_str() {
            Ok(s) => s,
            Err(err) => {
                eprintln!("Failed to convert string: {}", err);
                return ffi::c_float::NAN;
            }
        }
    }
    .to_string();

    APP.with_borrow_mut(|app| {
        if let Some(app) = app {
            let param_fourcc = FourCC::from(param_id);
            match app.synth_parameter_string_to_value(param_fourcc, string) {
                Ok(Some(value)) => value,
                Ok(None) => ffi::c_float::NAN,
                Err(err) => {
                    eprintln!(
                        "Failed to get synth parameter {} value: {}",
                        param_fourcc, err
                    );
                    ffi::c_float::NAN
                }
            }
        } else {
            ffi::c_float::NAN
        }
    })
}

/// Play a single synth note when the app is running. Exported as `_synth_note_on`
/// function in the WASM.
#[no_mangle]
pub extern "C" fn synth_note_on(note: ffi::c_int, velocity: ffi::c_float) {
    APP.with_borrow_mut(|app| {
        if let Some(app) = app {
            let note = note.clamp(0, 127) as u8;
            let velocity = velocity.clamp(0.0, 1.0);
            app.synth_note_on(note, velocity);
        }
    });
}

/// Stop a previously played synth note when the player is running. Exported as `_synth_note_off`
/// function in the WASM.
#[no_mangle]
pub extern "C" fn synth_note_off(note: ffi::c_int) {
    APP.with_borrow_mut(|app| {
        if let Some(app) = app {
            let note = note.clamp(0, 127) as u8;
            app.synth_note_off(note);
        }
    });
}

/// Randomize synth parameters. Exported as `_randomize_synth` function in the WASM.
///
/// Returns a pointer to a JSON string containing the updated parameter values and modulation routings, or null on error.
/// The return pointer must be freed with `_free_cstring` after getting consumed!
#[no_mangle]
pub extern "C" fn randomize_synth() -> *const ffi::c_char {
    APP.with_borrow(|app| {
        if let Some(app) = app.as_ref() {
            let (param_updates, mod_updates) = app.randomize_synth();
            let result = serde_json::json!({
                "parameters": param_updates,
                "modulation": mod_updates,
            });
            match serde_json::to_string(&result) {
                Ok(json) => {
                    let c_str = ffi::CString::new(json).unwrap_or_default();
                    c_str.into_raw()
                }
                Err(err) => {
                    eprintln!("Failed to serialize updates: {}", err);
                    std::ptr::null()
                }
            }
        } else {
            std::ptr::null()
        }
    })
}

/// Get modulation sources for the active synth. Exported as `_get_modulation_sources` function in the WASM.
///
/// Returns a pointer to a JSON array of modulation source info, or null on error.
/// The return pointer must be freed with `_free_cstring` after getting consumed!
#[no_mangle]
pub extern "C" fn get_modulation_sources() -> *const ffi::c_char {
    APP.with_borrow(|app| {
        if let Some(app) = app.as_ref() {
            let sources = app.get_modulation_sources();
            match serde_json::to_string(&sources) {
                Ok(json) => {
                    let c_str = ffi::CString::new(json).unwrap_or_default();
                    c_str.into_raw()
                }
                Err(err) => {
                    eprintln!("Failed to serialize modulation sources: {}", err);
                    std::ptr::null()
                }
            }
        } else {
            std::ptr::null()
        }
    })
}

/// Get modulation targets for the active synth. Exported as `_get_modulation_targets` function in the WASM.
///
/// Returns a pointer to a JSON array of modulation target info, or null on error.
/// The return pointer must be freed with `_free_cstring` after getting consumed!
#[no_mangle]
pub extern "C" fn get_modulation_targets() -> *const ffi::c_char {
    APP.with_borrow(|app| {
        if let Some(app) = app.as_ref() {
            let targets = app.get_modulation_targets();
            match serde_json::to_string(&targets) {
                Ok(json) => {
                    let c_str = ffi::CString::new(json).unwrap_or_default();
                    c_str.into_raw()
                }
                Err(err) => {
                    eprintln!("Failed to serialize modulation targets: {}", err);
                    std::ptr::null()
                }
            }
        } else {
            std::ptr::null()
        }
    })
}

/// Set a modulation routing. Exported as `_set_modulation` function in the WASM.
///
/// Returns 0 on success or -1 on error.
#[no_mangle]
pub extern "C" fn set_modulation(
    source_id: ffi::c_uint,
    target_id: ffi::c_uint,
    amount: ffi::c_float,
    bipolar: bool,
) -> ffi::c_int {
    APP.with_borrow_mut(|app| {
        if let Some(app) = app {
            let source = FourCC::from(source_id);
            let target = FourCC::from(target_id);
            match app.set_modulation(source, target, amount, bipolar) {
                Ok(_) => 0,
                Err(err) => {
                    eprintln!(
                        "Failed to set modulation {:?} -> {:?}: {}",
                        source, target, err
                    );
                    -1
                }
            }
        } else {
            -1
        }
    })
}

/// Clear a modulation routing. Exported as `_clear_modulation` function in the WASM.
///
/// Returns 0 on success or -1 on error.
#[no_mangle]
pub extern "C" fn clear_modulation(source_id: ffi::c_uint, target_id: ffi::c_uint) -> ffi::c_int {
    APP.with_borrow_mut(|app| {
        if let Some(app) = app {
            let source = FourCC::from(source_id);
            let target = FourCC::from(target_id);
            match app.clear_modulation(source, target) {
                Ok(_) => 0,
                Err(err) => {
                    eprintln!(
                        "Failed to clear modulation {:?} -> {:?}: {}",
                        source, target, err
                    );
                    -1
                }
            }
        } else {
            -1
        }
    })
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

/// Get an effect parameter's value as a string. Exported as `_effect_parameter_value_to_string`
/// function in the WASM.
///
/// Returns a pointer to a C string containing the parameter value, or null on error.
/// The return pointer must be freed with `_free_cstring` after getting consumed!
#[no_mangle]
pub extern "C" fn effect_parameter_value_to_string(
    effect_id: ffi::c_int,
    param_id: ffi::c_uint,
    normalized_value: ffi::c_float,
) -> *const ffi::c_char {
    APP.with_borrow_mut(|app| {
        if let Some(app) = app {
            let param_id = FourCC::from(param_id);
            match app.effect_parameter_value_to_string(
                effect_id as usize,
                param_id,
                normalized_value,
            ) {
                Ok(value_string) => {
                    let c_str = ffi::CString::new(value_string).unwrap_or_default();
                    c_str.into_raw()
                }
                Err(err) => {
                    eprintln!(
                        "Failed to get effect {} parameter {} string: {}",
                        effect_id, param_id, err
                    );
                    std::ptr::null()
                }
            }
        } else {
            std::ptr::null()
        }
    })
}

/// Convert an effect parameter's string value to a normalized value.
/// Exported as `_effect_parameter_string_to_value` function in the WASM.
///
/// Returns a normalized parameter value, or NaN on error.
#[no_mangle]
pub extern "C" fn effect_parameter_string_to_value(
    effect_id: ffi::c_int,
    param_id: ffi::c_uint,
    string: *const ffi::c_char,
) -> ffi::c_float {
    if string.is_null() {
        eprintln!("Parameter string is null");
        return ffi::c_float::NAN;
    }
    let string = unsafe {
        match ffi::CStr::from_ptr(string).to_str() {
            Ok(s) => s,
            Err(err) => {
                eprintln!("Failed to convert string: {}", err);
                return ffi::c_float::NAN;
            }
        }
    }
    .to_string();

    APP.with_borrow_mut(|app| {
        if let Some(app) = app {
            let param_fourcc = FourCC::from(param_id);
            match app.effect_parameter_string_to_value(effect_id as usize, param_fourcc, string) {
                Ok(Some(value)) => value,
                Ok(None) => ffi::c_float::NAN,
                Err(err) => {
                    eprintln!(
                        "Failed to get effect {} parameter {} value: {}",
                        effect_id, param_fourcc, err
                    );
                    ffi::c_float::NAN
                }
            }
        } else {
            ffi::c_float::NAN
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
