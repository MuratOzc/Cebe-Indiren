use crate::binaries::{bin_dir, is_newer_ytdlp, write_binary, ytdlp_version_path, YTDLP_NAME};
use crate::config::{log_error, log_info};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;
use tauri::plugin::{Builder, PluginHandle, TauriPlugin};
use tauri::Wry;
use tokio::process::Command;

const BUNDLED_YTDLP: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/binaries/yt-dlp"));

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ToolDirs {
    native_lib_dir: PathBuf,
    no_backup_dir: PathBuf,
    cache_dir: PathBuf,
}

// Must match the layout youtubedl-android's YoutubeDL.init and FFmpeg.init unpack into.
impl ToolDirs {
    fn packages(&self, name: &str) -> PathBuf {
        self.no_backup_dir.join("youtubedl-android/packages").join(name)
    }

    fn native(&self, name: &str) -> PathBuf {
        self.native_lib_dir.join(name)
    }

    fn apply_env(&self, cmd: &mut Command) {
        let python_home = self.packages("python").join("usr");
        let lib_path = format!(
            "{}:{}",
            python_home.join("lib").display(),
            self.packages("ffmpeg").join("usr/lib").display()
        );
        let path = match std::env::var("PATH") {
            Ok(path) => format!("{path}:{}", self.native_lib_dir.display()),
            Err(_) => self.native_lib_dir.display().to_string(),
        };
        cmd.env("LD_LIBRARY_PATH", lib_path)
            .env("SSL_CERT_FILE", python_home.join("etc/tls/cert.pem"))
            .env("PATH", path)
            .env("PYTHONHOME", &python_home)
            .env("HOME", &python_home)
            .env("TMPDIR", &self.cache_dir);
    }
}

static TOOLS: OnceLock<Option<ToolDirs>> = OnceLock::new();

fn tools() -> Option<&'static ToolDirs> {
    TOOLS.wait().as_ref()
}

static PLUGIN: OnceLock<PluginHandle<Wry>> = OnceLock::new();

pub fn plugin() -> TauriPlugin<Wry> {
    Builder::new("android")
        .setup(|_app, api| {
            let handle = api.register_android_plugin("com.videoindiren.app", "AndroidPlugin")?;
            let _ = PLUGIN.set(handle);
            Ok(())
        })
        .build()
}

// Kotlin answers on the main thread, so this must never be called from it.
pub fn call<T: DeserializeOwned>(command: &str, payload: impl Serialize) -> Result<T, String> {
    PLUGIN
        .get()
        .ok_or("Android plugin not registered")?
        .run_mobile_plugin(command, payload)
        .map_err(|e| e.to_string())
}

pub fn public_download_dir() -> PathBuf {
    let root = std::env::var("EXTERNAL_STORAGE").unwrap_or_else(|_| "/storage/emulated/0".into());
    PathBuf::from(root).join("Download")
}

// Needs the app data dir set.
pub fn prepare() {
    std::thread::spawn(|| {
        ensure_bundled_ytdlp();
        let dirs = call::<ToolDirs>("prepare", serde_json::json!({}));
        match dirs {
            Ok(dirs) => {
                log_info(&format!("Tools ready in {}", dirs.native_lib_dir.display()));
                let _ = TOOLS.set(Some(dirs));
            }
            Err(e) => {
                log_error(&format!("Failed to prepare tools: {e}"));
                let _ = TOOLS.set(None);
            }
        }
    });
}

fn ensure_bundled_ytdlp() {
    let bundled = env!("BUNDLED_YTDLP_VERSION");
    let dest = bin_dir().join(YTDLP_NAME);
    let installed = fs::read_to_string(ytdlp_version_path()).ok();
    if dest.is_file() && installed.is_some_and(|v| !is_newer_ytdlp(bundled, &v)) {
        return;
    }
    let written = fs::create_dir_all(bin_dir()).and_then(|_| write_binary(&dest, BUNDLED_YTDLP));
    match written {
        Ok(()) => {
            let _ = fs::write(ytdlp_version_path(), bundled);
            log_info(&format!("Installed bundled yt-dlp {bundled}"));
        }
        Err(e) => log_error(&format!("Failed to install bundled yt-dlp: {e}")),
    }
}

fn resolve_ytdlp() -> PathBuf {
    bin_dir().join(YTDLP_NAME)
}

pub fn resolve_ffmpeg() -> Option<PathBuf> {
    tools().map(|t| t.native("libffmpeg.so"))
}

pub fn resolve_ffprobe() -> Option<PathBuf> {
    tools().map(|t| t.native("libffprobe.so"))
}

pub fn tool_command(program: impl AsRef<OsStr>) -> Command {
    let mut cmd = Command::new(program);
    if let Some(t) = tools() {
        t.apply_env(&mut cmd);
    }
    cmd
}

pub fn ytdlp_command() -> Command {
    let Some(t) = tools() else {
        return Command::new(resolve_ytdlp());
    };
    let mut cmd = Command::new(t.native("libpython.so"));
    t.apply_env(&mut cmd);
    cmd.arg(resolve_ytdlp())
        .arg("--js-runtimes")
        .arg(format!("quickjs:{}", t.native("libqjs.so").display()))
        .arg("--cache-dir")
        .arg(t.cache_dir.join("yt-dlp"));
    cmd
}
