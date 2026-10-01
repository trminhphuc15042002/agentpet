fn main() {
    tauri_build::build();
    link_windows_resources_into_tests();
}

// cargo test --lib builds a test exe without the bin's application manifest.
// tauri-build emits a Common Controls v6 resource (libresource.a / resource.lib)
// in OUT_DIR. Catch-all rustc-link-arg (not cfg(test)-only) is so the test
// process loads ComCtl32 v6 at runtime; without it the exe links but the v6
// activation-context / loader path fails. Not an unresolved-symbol link error.
fn link_windows_resources_into_tests() {
    if std::env::var("CARGO_CFG_TARGET_OS").ok().as_deref() != Some("windows") {
        return;
    }
    let out_dir = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    let gnu = out_dir.join("libresource.a");
    let msvc = out_dir.join("resource.lib");
    let archive = if gnu.is_file() {
        gnu
    } else if msvc.is_file() {
        msvc
    } else {
        panic!(
            "tauri-build did not emit libresource.a or resource.lib in {}",
            out_dir.display()
        );
    };
    println!("cargo:rustc-link-arg={}", archive.display());
}
