fn main() {
    let target = std::env::var("TARGET").expect("No TARGET env variable set");
    let profile = std::env::var("PROFILE").expect("No PROFILE env variable set");
    // inject emscripten build options
    if target.contains("emscripten") {
        // debug options
        if profile == "debug" {
            println!("cargo::rustc-link-arg=-sASSERTIONS=2");
            println!("cargo::rustc-link-arg=-sSAFE_HEAP=1");
        }
        // compile options
        println!("cargo::rustc-link-arg=-fexceptions");
        println!("cargo::rustc-link-arg=-sUSE_PTHREADS=1");
        println!("cargo::rustc-link-arg=-sPTHREAD_POOL_SIZE=8");
        // memory options
        println!("cargo::rustc-link-arg=-sSTACK_SIZE=2MB");
        println!("cargo::rustc-link-arg=-sINITIAL_MEMORY=100MB");
        println!("cargo::rustc-link-arg=-sALLOW_MEMORY_GROWTH=1");
        // audio worklets
        println!("cargo::rustc-link-arg=-sWASM_WORKERS");
        println!("cargo::rustc-link-arg=-sAUDIO_WORKLET");
        if profile == "debug" {
            println!("cargo::rustc-link-arg=-sWEBAUDIO_DEBUG=1");
        }
        // exports
        println!("cargo::rustc-link-arg=--no-entry");
        let exports = ["_start", "_stop", "_synth_note_on", "_synth_note_off"];
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
