use crate::history::HistoryEntry;
use serde::Serialize;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct UpdateState {
    pub status: String,
    pub current: String,
    pub latest: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AppUpdateState {
    pub status: String,
    pub current: String,
    pub latest: String,
    #[serde(skip_serializing)]
    pub download_url: String,
    #[serde(skip_serializing)]
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ProgressPayload {
    pub percent: String,
    pub overall: String,
    pub speed: String,
    pub size_info: String,
    pub playlist_info: String,
    pub eta: String,
    pub title: String,
    pub status: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ItemPayload {
    pub thumbnail: String,
    pub duration: f64,
    pub url: String,
    pub title: String,
}

impl ProgressPayload {
    pub fn stage(status: &str, percent: String) -> Self {
        Self {
            percent,
            status: status.to_string(),
            ..Self::default()
        }
    }
}

pub struct AppState {
    pub download_active: AtomicBool,
    pub cancel_requested: AtomicBool,
    pub active_temp_dir: Mutex<Option<PathBuf>>,
    pub active_child_pid: Mutex<Option<u32>>,
    pub info_child_pid: Mutex<Option<u32>>,
    pub ytdlp_update_active: AtomicBool,
    pub download_dir: Mutex<String>,
    pub history: Mutex<Vec<HistoryEntry>>,
    pub update_state: Mutex<UpdateState>,
    pub app_update_state: Mutex<AppUpdateState>,
    pub app_update_active: AtomicBool,
}

impl AppState {
    pub fn new(download_dir: String, history: Vec<HistoryEntry>) -> Self {
        Self {
            download_active: AtomicBool::new(false),
            cancel_requested: AtomicBool::new(false),
            active_temp_dir: Mutex::new(None),
            active_child_pid: Mutex::new(None),
            info_child_pid: Mutex::new(None),
            ytdlp_update_active: AtomicBool::new(false),
            download_dir: Mutex::new(download_dir),
            history: Mutex::new(history),
            update_state: Mutex::new(UpdateState {
                status: "checking".to_string(),
                ..UpdateState::default()
            }),
            app_update_state: Mutex::new(AppUpdateState {
                status: "checking".to_string(),
                current: format!("v{}", env!("CARGO_PKG_VERSION")),
                ..AppUpdateState::default()
            }),
            app_update_active: AtomicBool::new(false),
        }
    }

    pub fn kill_active_child(&self) {
        if let Some(pid) = lock(&self.active_child_pid).take() {
            kill_process_tree(pid);
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel_requested.load(Ordering::SeqCst)
    }
}

pub fn kill_process_tree(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
}
