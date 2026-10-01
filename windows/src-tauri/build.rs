fn main() {
    tauri_build::build();
    if std::env::var("CARGO_CFG_TARGET_OS").ok().as_deref() == Some("windows") {
        // The cfg(test) native link in lib.rs needs the generated archive's
        // directory. Production binaries remain linked solely by tauri-build.
        println!("cargo:rustc-link-search=native={}", std::env::var("OUT_DIR").expect("OUT_DIR"));
    }
}
