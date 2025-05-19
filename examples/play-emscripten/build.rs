fn main() {
    // inject emscripten build options
    if std::env::var("TARGET").is_ok_and(|v| v.contains("emscripten")) {
        println!("cargo::rustc-link-arg=-fexceptions");
        println!(
            "cargo::rustc-link-arg=-sEXPORTED_FUNCTIONS=\
_start,_stop,_synth_note_on,_synth_note_off"
        );
        println!("cargo::rustc-link-arg=-sINVOKE_RUN=0");
        println!("cargo::rustc-link-arg=-sUSE_PTHREADS=1");
        println!("cargo::rustc-link-arg=-sPTHREAD_POOL_SIZE=4");
        println!("cargo::rustc-link-arg=-sALLOW_MEMORY_GROWTH=1");
        println!(
            "cargo::rustc-link-arg=--preload-file={}/assets@/assets",
            std::env::var("CARGO_MANIFEST_DIR").unwrap()
        );
        println!("cargo::rustc-link-arg=--no-entry");
    } else {
        println!("cargo::warning=This examples only works with target 'wasm32-unknown-emscripten'")
    }
}
