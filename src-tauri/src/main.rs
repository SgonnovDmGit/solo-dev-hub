// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Git merge driver mode: must run before anything else (no DB, keyring or window).
    if let Some(code) = solo_dev_hub_lib::merge_driver_entry(std::env::args().collect()) {
        std::process::exit(code);
    }
    solo_dev_hub_lib::run()
}
