pub mod analysis;
pub mod audio_events;
pub mod content;
pub mod export;
pub mod guides;
pub mod media;
pub mod probe;
pub mod proxy;
pub mod text_analysis;
pub mod waveform;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            probe::probe_video,
            probe::get_keyframes,
            analysis::analyze_audio,
            audio_events::analyze_audio_events,
            guides::import_timing_file,
            guides::lookup_content_guide,
            text_analysis::analyze_text,
            proxy::generate_proxy,
            export::export_video,
            waveform::get_waveform,
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|_app, event| {
            if let tauri::RunEvent::Exit = event {
                media::kill_all_children();
            }
        });
}
