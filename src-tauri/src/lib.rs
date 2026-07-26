mod analysis;
mod export;
mod media;
mod probe;
mod proxy;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            probe::probe_video,
            probe::get_keyframes,
            analysis::analyze_audio,
            proxy::generate_proxy,
            export::export_video,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
