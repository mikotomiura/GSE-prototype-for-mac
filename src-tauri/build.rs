fn main() {
    // macOS: set minimum deployment target to 10.15 (Catalina).
    // Catalina is the first macOS version that requires explicit Input Monitoring
    // permission for CGEventTap — which is what GSE uses for keyboard monitoring.
    #[cfg(target_os = "macos")]
    println!("cargo:rustc-env=MACOSX_DEPLOYMENT_TARGET=10.15");

    tauri_build::build()
}
