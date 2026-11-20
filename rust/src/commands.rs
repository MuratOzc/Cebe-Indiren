use crate::config::{load_config, log_error, save_config};
use crate::downloader::{get_video_info as fetch_video_info, run_download, DownloadRequest, DownloadResult, VideoInfoResult};
use crate::state::{AppState, UpdateState};
use crate::updater::install_update;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{AppHandle, State};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[derive(Debug, Serialize)]
pub struct ConfigResponse {
    pub default_dir: String,
    pub quality: String,
}

#[tauri::command]
pub fn get_config(state: State<'_, Arc<AppState>>) -> ConfigResponse {
    let cfg = load_config();
    let default_dir = state
        .download_dir
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    ConfigResponse {
        default_dir,
        quality: cfg.quality,
    }
}

#[tauri::command]
pub fn set_quality(quality: String) {
    save_config(None, Some(quality));
}

#[tauri::command]
pub async fn select_folder(state: State<'_, Arc<AppState>>) -> Result<Option<String>, String> {
    let initial = state
        .download_dir
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();

    let picked = tokio::task::spawn_blocking(move || {
        let mut dialog = rfd::FileDialog::new();
        if !initial.is_empty() && Path::new(&initial).exists() {
            dialog = dialog.set_directory(&initial);
        }
        dialog
            .pick_folder()
            .map(|p| p.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| e.to_string())?;

    if let Some(ref folder) = picked {
        if let Ok(mut guard) = state.download_dir.lock() {
            *guard = folder.clone();
        }
        save_config(Some(folder.clone()), None);
    }

    Ok(picked)
}

fn normalize_for_compare(path: &Path) -> PathBuf {
    let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let s = resolved.to_string_lossy();
    let stripped = s.strip_prefix(r"\\?\").unwrap_or(&s);
    #[cfg(windows)]
    {
        PathBuf::from(stripped.to_lowercase())
    }
    #[cfg(not(windows))]
    {
        PathBuf::from(stripped)
    }
}

fn is_allowed_open_path(state: &AppState, path: &Path) -> bool {
    let target = normalize_for_compare(path);

    if let Ok(last) = state.last_downloaded_path.lock() {
        if !last.is_empty() && normalize_for_compare(Path::new(&*last)) == target {
            return true;
        }
    }

    if let Ok(dir) = state.download_dir.lock() {
        if !dir.is_empty() {
            let base = normalize_for_compare(Path::new(&*dir));
            if target.starts_with(&base) {
                return true;
            }
        }
    }

    false
}

#[tauri::command]
pub fn open_folder(state: State<'_, Arc<AppState>>, file_path: String) -> Result<(), String> {
    let path = PathBuf::from(&file_path);
    if !is_allowed_open_path(&state, &path) {
        return Err("Access denied".to_string());
    }

    if path.exists() {
        #[cfg(windows)]
        {
            if path.is_file() {
                let clean = path.to_string_lossy().replace('/', "\\");
                Command::new("explorer")
                    .raw_arg(format!("/select,\"{clean}\""))
                    .spawn()
                    .map_err(|e| e.to_string())?;
            } else {
                Command::new("explorer")
                    .arg(&path)
                    .spawn()
                    .map_err(|e| e.to_string())?;
            }
            return Ok(());
        }
        #[cfg(not(windows))]
        {
            let target_dir = if path.is_dir() {
                path.as_path()
            } else {
                path.parent().unwrap_or(path.as_path())
            };
            let _ = Command::new("xdg-open").arg(target_dir).spawn();
            return Ok(());
        }
    }

    if let Some(parent) = path.parent() {
        if parent.exists() && is_allowed_open_path(&state, parent) {
            #[cfg(windows)]
            {
                Command::new("explorer")
                    .arg(parent)
                    .spawn()
                    .map_err(|e| e.to_string())?;
            }
            return Ok(());
        }
    }

    Err("File not found".to_string())
}

#[tauri::command]
pub async fn get_video_info(
    state: State<'_, Arc<AppState>>,
    url: String,
) -> Result<VideoInfoResult, String> {
    Ok(fetch_video_info(&state, &url).await)
}

#[tauri::command]
pub async fn start_download(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    request: DownloadRequest,
) -> Result<DownloadResult, String> {
    if state
        .download_active
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("Zaten aktif bir indirme var.".to_string());
    }

    let mut download_dir = state
        .download_dir
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();

    if let Some(ref req_dir) = request.download_dir {
        if !req_dir.is_empty() && Path::new(req_dir).exists() {
            download_dir = req_dir.clone();
        }
    }

    let res = run_download(&app, &state, request, download_dir).await;
    state.download_active.store(false, Ordering::SeqCst);
    res
}

#[tauri::command]
pub fn cancel_download(state: State<'_, Arc<AppState>>) {
    state.cancel_requested.store(true, Ordering::SeqCst);
    state.kill_active_child();
}

#[tauri::command]
pub fn get_update_status(state: State<'_, Arc<AppState>>) -> UpdateState {
    state
        .update_state
        .lock()
        .map(|g| g.clone())
        .unwrap_or(UpdateState {
            status: "failed".to_string(),
            current: String::new(),
            latest: String::new(),
        })
}

#[tauri::command]
pub async fn update_ytdlp(state: State<'_, Arc<AppState>>) -> Result<UpdateState, String> {
    if state.download_active.load(Ordering::SeqCst) {
        return Err("İndirme sürerken güncelleme yapılamaz.".to_string());
    }

    let (status, latest) = {
        let guard = state.update_state.lock().map_err(|e| e.to_string())?;
        (guard.status.clone(), guard.latest.clone())
    };

    if status != "available" || latest.is_empty() {
        return Err("Güncelleme yok.".to_string());
    }

    if let Err(e) = install_update(&latest).await {
        log_error(&format!("yt-dlp update failed: {e}"));
        return Err(format!("Güncelleme başarısız: {e}"));
    }

    let new_state = UpdateState {
        status: "up_to_date".to_string(),
        current: latest.clone(),
        latest,
    };

    if let Ok(mut guard) = state.update_state.lock() {
        *guard = new_state.clone();
    }

    Ok(new_state)
}
