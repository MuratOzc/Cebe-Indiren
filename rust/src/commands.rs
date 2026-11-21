use crate::app_updater::install_app_update;
use crate::config::{load_config, log_error, update_config};
use crate::downloader::{
    clear_taskbar_progress, extract_youtube_id, fetch_video_info, kill_info_lookup, run_download,
    DownloadRequest, DownloadResult, VideoInfo,
};
use crate::history::{self, HistoryItem};
use crate::platform;
use crate::sponsorblock;
use crate::state::{lock, AppState, AppUpdateState, UpdateState};
use crate::updater::{install_update, start_update_check};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use crate::theme::{self, ThemeMode};
use tauri::{AppHandle, State, WebviewWindow};
#[cfg(desktop)]
use tauri::{Manager, UserAttentionType};

type AppStateRef<'a> = State<'a, Arc<AppState>>;

const MAX_CLIPBOARD_LINKS: usize = 50;

struct Claim<'a>(&'a AtomicBool);

impl<'a> Claim<'a> {
    fn acquire(flag: &'a AtomicBool) -> Option<Self> {
        flag.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| Self(flag))
    }
}

impl Drop for Claim<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

#[derive(Debug, Serialize)]
pub struct ConfigResponse {
    pub default_dir: String,
    pub quality: String,
    pub audio_only: bool,
    pub theme: ThemeMode,
    pub sponsorblock: bool,
}

#[tauri::command]
pub fn get_config(state: AppStateRef<'_>) -> ConfigResponse {
    let cfg = load_config();
    ConfigResponse {
        default_dir: lock(&state.download_dir).clone(),
        quality: cfg.quality,
        audio_only: cfg.audio_only,
        theme: cfg.theme,
        sponsorblock: cfg.sponsorblock,
    }
}

#[tauri::command]
pub fn set_quality(quality: String) {
    update_config(|cfg| cfg.quality = quality);
}

#[tauri::command]
pub fn set_audio_only(audio_only: bool) {
    update_config(|cfg| cfg.audio_only = audio_only);
}

#[tauri::command]
pub fn set_sponsorblock(sponsorblock: bool) {
    update_config(|cfg| cfg.sponsorblock = sponsorblock);
}

#[tauri::command]
pub fn clipboard_links() -> Vec<String> {
    platform::clipboard_text()
        .map(|text| links_in(&text))
        .unwrap_or_default()
}

// The page only ever learns about clipboard text that is nothing but links.
fn links_in(text: &str) -> Vec<String> {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let is_link = |token: &&str| {
        let rest = token
            .strip_prefix("https://")
            .or_else(|| token.strip_prefix("http://"));
        rest.is_some_and(|rest| rest.contains('.') && !rest.starts_with('.'))
            && !token.chars().any(char::is_control)
    };
    if tokens.is_empty() || tokens.len() > MAX_CLIPBOARD_LINKS || !tokens.iter().all(is_link) {
        return Vec::new();
    }
    let mut links: Vec<String> = Vec::new();
    for token in tokens {
        if !links.iter().any(|known| known == token) {
            links.push(token.to_string());
        }
    }
    links
}

#[tauri::command]
pub fn set_theme(window: WebviewWindow, theme: ThemeMode) {
    update_config(|cfg| cfg.theme = theme);
    theme::apply_to_window(&window, theme);
}

#[tauri::command]
pub async fn select_folder(state: AppStateRef<'_>) -> Result<Option<String>, String> {
    let initial = lock(&state.download_dir).clone();
    let picked = platform::pick_folder(initial).await?;

    if let Some(folder) = &picked {
        *lock(&state.download_dir) = folder.clone();
        update_config(|cfg| cfg.download_dir = Some(folder.clone()));
    }

    Ok(picked)
}

// Paths go to the shell's default handler: only ones this app saved itself are accepted.
fn saved_download(state: &AppState, file_path: &str) -> Result<PathBuf, String> {
    let saved = lock(&state.history)
        .iter()
        .any(|entry| history::same_file(&entry.path, file_path));
    if !saved {
        return Err("Access denied".to_string());
    }
    let path = PathBuf::from(file_path);
    if !path.exists() {
        return Err("File not found".to_string());
    }
    Ok(path)
}

#[tauri::command]
pub fn open_folder(state: AppStateRef<'_>, file_path: String) -> Result<(), String> {
    let path = saved_download(&state, &file_path)?;
    platform::show_in_folder(&path, &file_path)
}

#[tauri::command]
pub fn open_file(state: AppStateRef<'_>, file_path: String) -> Result<(), String> {
    let path = saved_download(&state, &file_path)?;
    platform::open_with_default_app(&path)
}

#[tauri::command]
pub fn get_history(state: AppStateRef<'_>) -> Vec<HistoryItem> {
    history::items(&lock(&state.history))
}

#[tauri::command]
pub fn remove_history_entry(state: AppStateRef<'_>, file_path: String) -> Vec<HistoryItem> {
    let mut entries = lock(&state.history);
    if history::remove(&mut entries, &file_path) {
        history::save_history(&entries);
    }
    history::items(&entries)
}

#[tauri::command]
pub fn clear_history(state: AppStateRef<'_>) {
    let mut entries = lock(&state.history);
    entries.clear();
    history::save_history(&entries);
}

#[tauri::command]
pub async fn get_video_info(
    state: AppStateRef<'_>,
    url: String,
    playlist: bool,
) -> Result<VideoInfo, String> {
    fetch_video_info(&state, &url, playlist).await
}

#[tauri::command]
pub async fn get_sponsor_segments(url: String) -> Option<sponsorblock::Overview> {
    let video_id = extract_youtube_id(&url)?;
    sponsorblock::fetch_overview(&video_id).await
}

#[tauri::command]
pub async fn start_download(
    app: AppHandle,
    state: AppStateRef<'_>,
    request: DownloadRequest,
) -> Result<DownloadResult, String> {
    let Some(claim) = Claim::acquire(&state.download_active) else {
        return Err("Zaten aktif bir indirme var.".to_string());
    };
    if state.ytdlp_update_active.load(Ordering::SeqCst) {
        return Err("yt-dlp güncelleniyor. Birkaç saniye sonra tekrar deneyin.".to_string());
    }
    if state.app_update_active.load(Ordering::SeqCst) {
        return Err("Uygulama güncelleniyor. Lütfen bekleyin.".to_string());
    }

    let download_dir = lock(&state.download_dir).clone();
    let res = run_download(&app, &state, request, download_dir).await;
    drop(claim);

    if res.is_err() {
        retry_failed_update_check(&state);
    }

    clear_taskbar_progress(&app);
    #[cfg(desktop)]
    if let Some(window) = app.get_webview_window("main") {
        if !window.is_focused().unwrap_or(true) {
            let _ = window.request_user_attention(Some(UserAttentionType::Informational));
        }
    }
    res
}

fn retry_failed_update_check(state: &Arc<AppState>) {
    let mut update = lock(&state.update_state);
    if update.status == "failed" {
        update.status = "checking".to_string();
        start_update_check(Arc::clone(state));
    }
}

#[tauri::command]
pub async fn cancel_download(state: AppStateRef<'_>) -> Result<(), String> {
    state.cancel_requested.store(true, Ordering::SeqCst);
    let state = Arc::clone(&state);
    tokio::task::spawn_blocking(move || state.kill_active_child())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_update_status(state: AppStateRef<'_>) -> UpdateState {
    lock(&state.update_state).clone()
}

#[tauri::command]
pub async fn update_ytdlp(state: AppStateRef<'_>) -> Result<UpdateState, String> {
    let Some(_claim) = Claim::acquire(&state.ytdlp_update_active) else {
        return Err("Güncelleme zaten sürüyor.".to_string());
    };
    if state.download_active.load(Ordering::SeqCst) {
        return Err("İndirme sürerken güncelleme yapılamaz.".to_string());
    }

    let UpdateState { status, latest, .. } = lock(&state.update_state).clone();
    if status != "available" || latest.is_empty() {
        return Err("Güncelleme yok.".to_string());
    }

    // Windows will not replace yt-dlp.exe while a lookup still has it running.
    kill_info_lookup(&state).await;

    if let Err(e) = install_update(&latest).await {
        log_error(&format!("yt-dlp update failed: {e}"));
        return Err(format!("Güncelleme başarısız: {e}"));
    }

    let new_state = UpdateState {
        status: "up_to_date".to_string(),
        current: latest.clone(),
        latest,
    };
    *lock(&state.update_state) = new_state.clone();
    Ok(new_state)
}

#[tauri::command]
pub fn get_app_update_status(state: AppStateRef<'_>) -> AppUpdateState {
    lock(&state.app_update_state).clone()
}

#[tauri::command]
pub async fn update_app(app: AppHandle, state: AppStateRef<'_>) -> Result<(), String> {
    let Some(_claim) = Claim::acquire(&state.app_update_active) else {
        return Err("Güncelleme zaten sürüyor.".to_string());
    };
    if state.download_active.load(Ordering::SeqCst) {
        return Err("İndirme sürerken güncelleme yapılamaz.".to_string());
    }

    let update = lock(&state.app_update_state).clone();
    if update.status != "available" || update.download_url.is_empty() {
        return Err("Güncelleme yok.".to_string());
    }

    let res = install_app_update(
        &app,
        &update.latest,
        &update.download_url,
        update.sha256.as_deref(),
        &update.release_notes,
    )
    .await;
    if let Err(e) = &res {
        log_error(&format!("App update failed: {e}"));
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_text_is_offered_only_when_it_is_all_links() {
        assert_eq!(links_in(" https://a.b/c \r\n"), ["https://a.b/c"]);
        assert_eq!(
            links_in("https://a.b/1\nhttp://c.d/2 https://a.b/1"),
            ["https://a.b/1", "http://c.d/2"]
        );
        assert!(links_in("").is_empty());
        assert!(links_in("parolam: hunter2").is_empty());
        assert!(links_in("bak https://a.b/c").is_empty());
        assert!(links_in("https://localhost").is_empty());
        assert!(links_in("ftp://a.b/c").is_empty());
        let many = vec!["https://a.b/c"; MAX_CLIPBOARD_LINKS + 1].join("\n");
        assert!(links_in(&many).is_empty());
    }
}
