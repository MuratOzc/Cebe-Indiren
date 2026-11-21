use crate::theme::ThemeMode;
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

static LOG_LOCK: Mutex<()> = Mutex::new(());
static CONFIG_LOCK: Mutex<()> = Mutex::new(());
const MAX_LOG_BYTES: u64 = 1_000_000;

pub fn app_data_dir() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("VideoIndiren");
    let _ = fs::create_dir_all(&dir);
    dir
}

fn config_file() -> PathBuf {
    app_data_dir().join("config.json")
}

fn write_log(level: &str, msg: &str) {
    let _guard = LOG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = app_data_dir().join("app.log");

    if fs::metadata(&path).is_ok_and(|meta| meta.len() >= MAX_LOG_BYTES) {
        let _ = fs::rename(&path, path.with_extension("log.1"));
    }

    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let ts = Local::now().format("%Y-%m-%d %H:%M:%S");
        let _ = writeln!(file, "{ts} - {level} - {msg}");
    }
}

pub fn log_info(msg: &str) {
    write_log("INFO", msg);
}

pub fn log_warn(msg: &str) {
    write_log("WARNING", msg);
}

pub fn log_error(msg: &str) {
    write_log("ERROR", msg);
}

pub fn write_json<T: Serialize + ?Sized>(path: &Path, value: &T) {
    let tmp = path.with_extension("json.tmp");
    let saved = serde_json::to_string(value)
        .map_err(std::io::Error::other)
        .and_then(|json| fs::write(&tmp, json))
        .and_then(|()| fs::rename(&tmp, path));
    if let Err(e) = saved {
        let _ = fs::remove_file(&tmp);
        log_error(&format!("Failed to save {}: {e}", path.display()));
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedConfig {
    pub download_dir: Option<String>,
    #[serde(default = "default_quality")]
    pub quality: String,
    #[serde(default)]
    pub audio_only: bool,
    #[serde(default)]
    pub theme: ThemeMode,
    #[serde(default)]
    pub sponsorblock: bool,
}

pub fn default_quality() -> String {
    "best".to_string()
}

pub fn load_config() -> SavedConfig {
    fs::read_to_string(config_file())
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_else(|| SavedConfig {
            download_dir: None,
            quality: default_quality(),
            audio_only: false,
            theme: ThemeMode::default(),
            sponsorblock: false,
        })
}

pub fn update_config(apply: impl FnOnce(&mut SavedConfig)) {
    let _guard = CONFIG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut current = load_config();
    apply(&mut current);
    write_json(&config_file(), &current);
}

pub fn initialize_download_dir(saved_dir: Option<String>) -> String {
    if let Some(dir) = saved_dir.filter(|dir| !dir.is_empty() && Path::new(dir).exists()) {
        return dir;
    }
    if let Some(dl) = dirs::download_dir().filter(|dl| dl.exists()) {
        return dl.to_string_lossy().into_owned();
    }
    let local = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("downloads");
    let _ = fs::create_dir_all(&local);
    local.to_string_lossy().into_owned()
}
