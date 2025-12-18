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
            "_synth_note_on",
            "_synth_note_off",
            "_get_available_effects",
            "_add_effect",
            "_remove_effect",
            "_get_effect_parameter_string",
            "_set_effect_parameter_value",
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
