// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Started again by the engine as a render worker for heavy pages.
    if edits_core::worker::serve_if_requested() {
        return;
    }
    edits_app::run();
}
