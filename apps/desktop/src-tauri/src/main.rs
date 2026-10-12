// Prevents an extra console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // `--verify-native` is the CI hook: it exercises the same host plumbing the window uses, but
    // headless, so a runner without a window server still gets a real answer.
    if std::env::args().any(|arg| arg == "--verify-native") {
        std::process::exit(smollm_desktop_lib::verify_native::run());
    }
    smollm_desktop_lib::run();
}
