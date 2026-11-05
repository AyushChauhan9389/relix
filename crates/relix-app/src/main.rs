// Release builds are a GUI app on Windows: no console window next to Relix.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// The app lives in lib.rs, so Android can load it as a library (android_main).
fn main() {
    relix_app::main()
}
