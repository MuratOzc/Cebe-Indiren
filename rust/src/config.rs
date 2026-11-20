use chrono::Local;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

static LOG_LOCK: Mutex<()> = Mutex::new(());
const MAX_LOG_BYTES: u64 = 1_000_000;

pub fn app_data_dir() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("VideoIndiren");
    let _ = fs::create_dir_all(&dir);
    dir
}

pub fn config_file() -> PathBuf {
    app_data_dir().join("config.json")
}

pub fn log_file() -> PathBuf {
    app_data_dir().join("app.log")
}

fn write_log(level: &str, msg: &str) {
    let _guard = LOG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = log_file();

    if let Ok(meta) = fs::metadata(&path) {
        if meta.len() >= MAX_LOG_BYTES {
            let b2 = app_data_dir().join("app.log.2");
            let b1 = app_data_dir().join("app.log.1");
            let _ = fs::remove_file(&b2);
            let _ = fs::rename(&b1, &b2);
            let _ = fs::rename(&path, &b1);
        }
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedConfig {
    pub download_dir: Option<String>,
    #[serde(default = "default_quality")]
    pub quality: String,
}

fn default_quality() -> String {
    "best".to_string()
}

impl Default for SavedConfig {
    fn default() -> Self {
        Self {
            download_dir: None,
            quality: default_quality(),
        }
    }
}

pub fn load_config() -> SavedConfig {
    let path = config_file();
    if let Ok(content) = fs::read_to_string(&path) {
        if let Ok(cfg) = serde_json::from_str::<SavedConfig>(&content) {
            return cfg;
        }
    }
    SavedConfig::default()
}

pub fn save_config(download_dir: Option<String>, quality: Option<String>) {
    let mut current = load_config();
    if let Some(d) = download_dir {
        current.download_dir = Some(d);
    }
    if let Some(q) = quality {
        current.quality = q;
    }
    if let Ok(json) = serde_json::to_string(&current) {
        if let Err(e) = fs::write(config_file(), json) {
            log_error(&format!("Failed to save config: {e}"));
        }
    }
}

pub fn initialize_download_dir(saved_dir: Option<String>) -> String {
    if let Some(dir) = saved_dir {
        if !dir.is_empty() && PathBuf::from(&dir).exists() {
            return dir;
        }
    }

    if let Some(dl) = dirs::download_dir() {
        if dl.exists() {
            return dl.to_string_lossy().into_owned();
        }
    }

    let local = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("downloads");
    let _ = fs::create_dir_all(&local);
    local.to_string_lossy().into_owned()
}
