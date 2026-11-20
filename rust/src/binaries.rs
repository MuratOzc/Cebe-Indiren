#[allow(unused_imports)]
use crate::config::{app_data_dir, log_error, log_info, log_warn};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

#[cfg(windows)]
const YTDLP_NAME: &str = "yt-dlp.exe";
#[cfg(not(windows))]
const YTDLP_NAME: &str = "yt-dlp";

#[cfg(windows)]
const FFMPEG_NAME: &str = "ffmpeg.exe";
#[cfg(not(windows))]
const FFMPEG_NAME: &str = "ffmpeg";

// Embedded directly into release builds so VideoIndiren.exe is a single portable file.
// Dev builds keep reading binaries/ from disk so incremental rebuilds stay fast.
#[cfg(all(not(debug_assertions), windows))]
const EMBEDDED_YTDLP: &[u8] = include_bytes!("../binaries/yt-dlp.exe");
#[cfg(all(not(debug_assertions), windows))]
const EMBEDDED_FFMPEG: &[u8] = include_bytes!("../binaries/ffmpeg.exe");

pub fn update_bin_dir() -> PathBuf {
    app_data_dir().join("bin")
}

pub fn updated_ytdlp_path() -> PathBuf {
    update_bin_dir().join(YTDLP_NAME)
}

pub fn extracted_ffmpeg_path() -> PathBuf {
    update_bin_dir().join(FFMPEG_NAME)
}

pub fn stamp_path() -> PathBuf {
    update_bin_dir().join("exe.stamp")
}

pub fn exe_stamp() -> Option<String> {
    if cfg!(debug_assertions) {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let meta = fs::metadata(exe).ok()?;
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some(format!("{}-{}", meta.len(), mtime))
}

#[cfg(all(not(debug_assertions), windows))]
fn write_binary_atomic(dest: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = dest.with_extension("exe.tmp");
    fs::write(&tmp, bytes)?;
    if fs::rename(&tmp, dest).is_err() {
        let _ = fs::remove_file(dest);
        if let Err(e) = fs::rename(&tmp, dest) {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
    }
    Ok(())
}

/// On release (portable exe) launches, unpacks embedded ffmpeg.exe and yt-dlp.exe once into
/// `%APPDATA%\VideoIndiren\bin\` (and refreshes them automatically if a newer portable exe is shipped).
pub fn ensure_binaries_ready() {
    #[cfg(all(not(debug_assertions), windows))]
    {
        let dir = update_bin_dir();
        if let Err(e) = fs::create_dir_all(&dir) {
            log_error(&format!("Failed to create bin dir: {e}"));
            return;
        }

        let current_stamp = exe_stamp();
        let saved_stamp = fs::read_to_string(stamp_path())
            .ok()
            .map(|s| s.trim().to_string());
        let stamp_changed = current_stamp.is_some() && saved_stamp != current_stamp;

        let ytdlp_dest = updated_ytdlp_path();
        if stamp_changed || !ytdlp_dest.is_file() {
            log_info("Extracting embedded yt-dlp.exe to AppData...");
            if let Err(e) = write_binary_atomic(&ytdlp_dest, EMBEDDED_YTDLP) {
                log_error(&format!("Failed to extract yt-dlp.exe: {e}"));
            }
        }

        let ffmpeg_dest = extracted_ffmpeg_path();
        if stamp_changed || !ffmpeg_dest.is_file() {
            log_info("Extracting embedded ffmpeg.exe to AppData...");
            if let Err(e) = write_binary_atomic(&ffmpeg_dest, EMBEDDED_FFMPEG) {
                log_error(&format!("Failed to extract ffmpeg.exe: {e}"));
            }
        }

        if let Some(stamp) = current_stamp {
            if stamp_changed || !stamp_path().is_file() {
                let _ = fs::write(stamp_path(), stamp);
            }
        }
    }
}

pub fn resolve_ytdlp(resource_dir: Option<&Path>) -> PathBuf {
    let appdata_bin = updated_ytdlp_path();
    if appdata_bin.is_file() {
        return appdata_bin;
    }

    if let Some(res) = resource_dir {
        let bundled = res.join("binaries").join(YTDLP_NAME);
        if bundled.is_file() {
            return bundled;
        }
    }

    let dev_local = PathBuf::from("binaries").join(YTDLP_NAME);
    if dev_local.is_file() {
        return dev_local;
    }

    PathBuf::from(YTDLP_NAME)
}

pub fn resolve_ffmpeg(resource_dir: Option<&Path>) -> Option<PathBuf> {
    let appdata_bin = extracted_ffmpeg_path();
    if appdata_bin.is_file() {
        return Some(appdata_bin);
    }

    if let Some(res) = resource_dir {
        let bundled = res.join("binaries").join(FFMPEG_NAME);
        if bundled.is_file() {
            return Some(bundled);
        }
    }

    let dev_local = PathBuf::from("binaries").join(FFMPEG_NAME);
    if dev_local.is_file() {
        return Some(dev_local);
    }

    let parent_ffmpeg = PathBuf::from("..").join("ffmpeg").join("bin").join(FFMPEG_NAME);
    if parent_ffmpeg.is_file() {
        return Some(parent_ffmpeg);
    }

    log_warn("Bundled ffmpeg not found; falling back to system PATH.");
    None
}
