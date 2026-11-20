#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod binaries;
mod commands;
mod config;
mod downloader;
mod ffmpeg;
mod state;
mod updater;

use crate::binaries::ensure_binaries_ready;
use crate::config::{initialize_download_dir, load_config, log_info};
use crate::downloader::cleanup_temp_dir;
use crate::state::AppState;
use crate::updater::start_update_check;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{Manager, RunEvent};

fn main() {
    let app = tauri::Builder::default()
        .setup(|app| {
            ensure_binaries_ready();

            let cfg = load_config();
            let download_dir = initialize_download_dir(cfg.download_dir);
            let resource_dir = app.path().resource_dir().ok();

            log_info(&format!(
                "Video Indiren (Rust) started. Download dir: {download_dir}"
            ));

            let app_state = Arc::new(AppState::new(download_dir, resource_dir));
            start_update_check(Arc::clone(&app_state));
            app.manage(app_state);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::set_quality,
            commands::select_folder,
            commands::open_folder,
            commands::get_video_info,
            commands::start_download,
            commands::cancel_download,
            commands::get_update_status,
            commands::update_ytdlp,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app_handle, event| {
        if let RunEvent::ExitRequested { .. } | RunEvent::Exit = event {
            if let Some(state) = app_handle.try_state::<Arc<AppState>>() {
                if state.download_active.load(Ordering::SeqCst) {
                    state.cancel_requested.store(true, Ordering::SeqCst);
                    state.kill_active_child();
                    if let Some(temp_dir) = state.active_temp_dir.lock().ok().and_then(|mut g| g.take()) {
                        cleanup_temp_dir(&temp_dir);
                    }
                }
            }
        }
    });
}
