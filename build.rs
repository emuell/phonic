fn main() {
    // inject emscripten build options
    if std::env::var("TARGET").is_ok_and(|v| v.contains("emscripten")) {
        println!("cargo::rustc-link-arg=-fexceptions");
        println!("cargo::rustc-link-arg=--no-entry");
    }
}
