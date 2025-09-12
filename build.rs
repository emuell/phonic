use rustc_version::{version_meta, Channel};

fn main() {
    // inject emscripten build options
    if std::env::var("TARGET").is_ok_and(|v| v.contains("emscripten")) {
        println!("cargo::rustc-link-arg=-fexceptions");
        println!("cargo::rustc-link-arg=--no-entry");
    }

    // enable docsrs cfg flag in nightly channel builds
    // used for #![cfg_attr(all(doc, docsrs), feature(doc_auto_cfg))]
    if version_meta().unwrap().channel == Channel::Nightly {
        println!("cargo:rustc-cfg=docsrs")
    }
}
