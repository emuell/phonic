fn main() {
    let target = std::env::var("TARGET").expect("No TARGET env variable set");
    let profile = std::env::var("PROFILE").expect("No PROFILE env variable set");
    // inject emscripten build options
    if target.contains("emscripten") {
        // compile options
        println!("cargo::rustc-link-arg=-fexceptions");
        println!("cargo::rustc-link-arg=-sINVOKE_RUN=0");
        println!("cargo::rustc-link-arg=-sUSE_PTHREADS=1");
        println!("cargo::rustc-link-arg=-sPTHREAD_POOL_SIZE=4");
        println!("cargo::rustc-link-arg=-sALLOW_MEMORY_GROWTH=1");
        println!("cargo::rustc-link-arg=-sSTACK_SIZE=2MB");
        println!("cargo::rustc-link-arg=-sINITIAL_MEMORY=100MB");
        if profile == "debug" {
            println!("cargo::rustc-link-arg=-sASSERTIONS=2");
            println!("cargo::rustc-link-arg=-sSAFE_HEAP=1");
            println!("cargo::rustc-link-arg=-sSTACK_OVERFLOW_CHECK=1");
            println!("cargo::rustc-link-arg=-sCHECK_NULL_WRITES=1");
        }
        // exports
        println!("cargo::rustc-link-arg=--no-entry");
        println!("cargo::rustc-link-arg=-sEXPORTED_RUNTIME_METHODS=ccall,cwrap");
        let exports = [
            "_free_cstring",
            "_start",
            "_stop",
            "_get_cpu_load",
            "_set_metronome_enabled",
            "_set_active_synth",
            "_get_synth_parameters",
            "_set_synth_parameter_value",
            "_set_synth_voice_count",
            "_synth_parameter_value_to_string",
            "_synth_parameter_string_to_value",
            "_synth_note_on",
            "_synth_note_off",
            "_get_available_effects",
            "_add_effect",
            "_remove_effect",
            "_effect_parameter_value_to_string",
            "_effect_parameter_string_to_value",
            "_set_effect_parameter_value",
            "_randomize_synth",
        ];
        println!(
            "cargo::rustc-link-arg=-sEXPORTED_FUNCTIONS={}",
            exports.join(",")
        );
        // assets
        println!(
            "cargo::rustc-link-arg=--preload-file={}/assets@/assets",
            std::env::var("CARGO_MANIFEST_DIR").unwrap()
        );
    } else {
        println!("cargo::warning=This examples only works with target 'wasm32-unknown-emscripten'")
    }
}
