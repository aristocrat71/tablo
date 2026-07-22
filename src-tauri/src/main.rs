// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // The Aptabase telemetry plugin calls raw `tokio::spawn` from its Tauri setup
    // hook, which panics under Tauri v2 because that hook isn't run inside a Tokio
    // runtime. Build one, hand its handle to Tauri so both share a single runtime,
    // and enter it on this thread for the app's lifetime so the plugin has a reactor.
    let rt = tokio::runtime::Runtime::new().expect("failed to build Tokio runtime");
    tauri::async_runtime::set(rt.handle().clone());
    let _guard = rt.enter();
    tablo_lib::run()
}
