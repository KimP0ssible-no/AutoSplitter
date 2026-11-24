include!("packaging/windows/resources.rs");

fn main() {
    let config = slint_build::CompilerConfiguration::new().with_style("fluent-dark".into());
    slint_build::compile_with_config("ui/app.slint", config).expect("Slint UI failed to compile");

    // The prebuilt ONNX Runtime carries the DirectML provider; OCR runs on the CPU, so
    // DirectML.dll is never loaded, and the program does not need it to start.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg=/DELAYLOAD:DirectML.dll");
        println!("cargo:rustc-link-lib=delayimp");
    }

    // Icon, version and manifest for Explorer, the taskbar and the installer. The
    // resource compiler is on Windows build machines only, not on a Linux type-check.
    println!("cargo:rerun-if-changed=packaging/windows");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") && cfg!(windows) {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        windows_resources(
            env!("CARGO_PKG_VERSION"),
            "AutoSplitter",
            &dir.join("packaging/icons/autosplitter.ico"),
            &dir.join("packaging/windows/app.manifest"),
        );
    }
}
