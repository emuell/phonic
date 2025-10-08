use rustc_version::{version_meta, Channel};

fn main() {
    let target = std::env::var("TARGET").expect("No TARGET env variable set");
    let version = version_meta().unwrap();

    // inject emscripten build options
    if target.contains("emscripten") {
        println!("cargo::rustc-link-arg=-fexceptions");
        println!("cargo::rustc-link-arg=--no-entry");
    }

    // enable docsrs cfg flag in nightly channel builds:
    // used for #![cfg_attr(all(doc, docsrs), feature(doc_auto_cfg))]
    if version.channel == Channel::Nightly {
        println!("cargo:rustc-cfg=docsrs")
    }
}
