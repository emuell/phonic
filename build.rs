fn main() {
    let target = std::env::var("TARGET").expect("No TARGET env variable set");

    // inject emscripten build options
    if target.contains("emscripten") {
        println!("cargo::rustc-link-arg=-fexceptions");
        println!("cargo::rustc-link-arg=--no-entry");
    }
}
