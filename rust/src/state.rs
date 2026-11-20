use serde::Serialize;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, Serialize)]
pub struct UpdateState {
    pub status: String,
    pub current: String,
    pub latest: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AppUpdateState {
    pub status: String,
    pub current: String,
    pub latest: String,
    #[serde(skip_serializing)]
    pub download_url: String,
    #[serde(skip_serializing)]
    pub sha256_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProgressPayload {
    pub percent: String,
    pub speed: String,
    pub size_info: String,
    pub playlist_info: String,
    pub status: String,
}

pub struct AppState {
    pub download_active: AtomicBool,
    pub cancel_requested: AtomicBool,
    pub active_temp_dir: Mutex<Option<PathBuf>>,
    pub active_child_pid: Mutex<Option<u32>>,
    pub download_dir: Mutex<String>,
    pub last_downloaded_path: Mutex<String>,
    pub update_state: Mutex<UpdateState>,
    pub app_update_state: Mutex<AppUpdateState>,
    pub app_update_active: AtomicBool,
    pub resource_dir: Option<PathBuf>,
}

impl AppState {
    pub fn new(download_dir: String, resource_dir: Option<PathBuf>) -> Self {
        Self {
            download_active: AtomicBool::new(false),
            cancel_requested: AtomicBool::new(false),
            active_temp_dir: Mutex::new(None),
            active_child_pid: Mutex::new(None),
            download_dir: Mutex::new(download_dir),
            last_downloaded_path: Mutex::new(String::new()),
            update_state: Mutex::new(UpdateState {
                status: "checking".to_string(),
                current: String::new(),
                latest: String::new(),
            }),
            app_update_state: Mutex::new(AppUpdateState {
                status: "checking".to_string(),
                current: format!("v{}", env!("CARGO_PKG_VERSION")),
                latest: String::new(),
                download_url: String::new(),
                sha256_url: None,
            }),
            app_update_active: AtomicBool::new(false),
            resource_dir,
        }
    }

    pub fn set_active_pid(&self, pid: Option<u32>) {
        if let Ok(mut guard) = self.active_child_pid.lock() {
            *guard = pid;
        }
    }

    pub fn kill_active_child(&self) {
        let pid = self
            .active_child_pid
            .lock()
            .ok()
            .and_then(|mut guard| guard.take());

        if let Some(pid) = pid {
            kill_process_tree(pid);
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel_requested.load(Ordering::SeqCst)
    }
}

pub fn kill_process_tree(pid: u32) {
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("kill")
            .args(["-9", &pid.to_string()])
            .output();
    }
}
