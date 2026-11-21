#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app_updater;
mod binaries;
mod commands;
mod config;
mod downloader;
mod ffmpeg;
mod history;
mod platform;
mod sponsorblock;
mod state;
mod theme;
mod updater;

use crate::app_updater::{cleanup_old_exe, start_app_update_check};
use crate::config::{initialize_download_dir, load_config, log_info};
use crate::downloader::cleanup_temp_dir;
use crate::state::{kill_process_tree, lock, AppState};
use crate::updater::start_update_check;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{Manager, RunEvent, WebviewWindowBuilder, Window, WindowEvent};

static EXIT_PROMPT_OPEN: AtomicBool = AtomicBool::new(false);

fn confirm_exit_during_download(window: &Window) {
    if EXIT_PROMPT_OPEN.swap(true, Ordering::SeqCst) {
        return;
    }
    let window = window.clone();
    std::thread::spawn(move || {
        let answer = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title("Video İndiren")
            .set_description(
                "Bir indirme sürüyor. Çıkarsanız indirme iptal edilir.

Yine de çıkmak istiyor musunuz?",
            )
            .set_buttons(rfd::MessageButtons::YesNo)
            .set_parent(&window)
            .show();
        EXIT_PROMPT_OPEN.store(false, Ordering::SeqCst);
        if answer == rfd::MessageDialogResult::Yes {
            window.app_handle().exit(0);
        }
    });
}

fn main() {
    let app = tauri::Builder::default()
        .setup(|app| {
            let just_updated = cleanup_old_exe();
            #[cfg(not(debug_assertions))]
            binaries::ensure_binaries_ready();

            let cfg = load_config();
            let download_dir = initialize_download_dir(cfg.download_dir);

            log_info(&format!(
                "Video Indiren (Rust) started. Download dir: {download_dir}"
            ));

            let app_state = Arc::new(AppState::new(download_dir, history::load_history()));
            start_update_check(Arc::clone(&app_state));
            start_app_update_check(Arc::clone(&app_state), just_updated);
            app.manage(app_state);

            let window_config = app
                .config()
                .app
                .windows
                .first()
                .cloned()
                .ok_or("no window configured")?;
            let window = WebviewWindowBuilder::from_config(app.handle(), &window_config)?
                .theme(cfg.theme.native())
                .initialization_script(theme::init_script(cfg.theme))
                .visible(false)
                .build()?;
            theme::apply_to_window(&window, cfg.theme);
            window.show()?;

            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::ThemeChanged(theme) = event {
                let _ = window.set_background_color(Some(theme::background(*theme)));
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                let downloading = window
                    .try_state::<Arc<AppState>>()
                    .is_some_and(|state| state.download_active.load(Ordering::SeqCst));
                if downloading {
                    api.prevent_close();
                    confirm_exit_during_download(window);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::set_quality,
            commands::set_audio_only,
            commands::set_theme,
            commands::set_sponsorblock,
            commands::clipboard_links,
            commands::select_folder,
            commands::open_folder,
            commands::open_file,
            commands::get_history,
            commands::remove_history_entry,
            commands::clear_history,
            commands::get_video_info,
            commands::get_sponsor_segments,
            commands::start_download,
            commands::cancel_download,
            commands::get_update_status,
            commands::update_ytdlp,
            commands::get_app_update_status,
            commands::update_app,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app_handle, event| {
        if let RunEvent::ExitRequested { .. } | RunEvent::Exit = event {
            if let Some(state) = app_handle.try_state::<Arc<AppState>>() {
                if let Some(pid) = lock(&state.info_child_pid).take() {
                    kill_process_tree(pid);
                }
                if state.download_active.load(Ordering::SeqCst) {
                    state.cancel_requested.store(true, Ordering::SeqCst);
                    state.kill_active_child();
                    if let Some(temp_dir) = lock(&state.active_temp_dir).take() {
                        cleanup_temp_dir(&temp_dir);
                    }
                }
            }
        }
    });
}
