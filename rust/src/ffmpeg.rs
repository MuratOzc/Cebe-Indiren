use crate::binaries::resolve_ffmpeg;
use crate::config::{log_error, log_info, log_warn};
use crate::state::{AppState, ProgressPayload};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tauri::{AppHandle, Emitter};
use tokio::process::Command;

#[cfg(windows)]
use crate::state::CREATE_NO_WINDOW;
#[cfg(windows)]
#[allow(unused_imports)]
use std::os::windows::process::CommandExt;

pub fn time_to_seconds(t_str: Option<&str>) -> Option<f64> {
    let raw = t_str?.trim();
    if raw.is_empty() {
        return None;
    }
    let parts: Vec<&str> = raw.split(':').collect();
    match parts.len() {
        3 => {
            let h: f64 = parts[0].parse().ok()?;
            let m: f64 = parts[1].parse().ok()?;
            let s: f64 = parts[2].parse().ok()?;
            Some(h * 3600.0 + m * 60.0 + s)
        }
        2 => {
            let m: f64 = parts[0].parse().ok()?;
            let s: f64 = parts[1].parse().ok()?;
            Some(m * 60.0 + s)
        }
        _ => raw.parse::<f64>().ok(),
    }
}

fn safe_replace_file(src: &Path, dst: &Path) -> std::io::Result<()> {
    if fs::rename(src, dst).is_ok() {
        return Ok(());
    }
    // Windows rename fails if dst exists: move dst to a temp backup first so we never lose the original.
    let backup = dst.with_extension("bak_trim");
    let _ = fs::remove_file(&backup);
    fs::rename(dst, &backup)?;
    match fs::rename(src, dst) {
        Ok(()) => {
            let _ = fs::remove_file(&backup);
            Ok(())
        }
        Err(e) => {
            let _ = fs::rename(&backup, dst);
            Err(e)
        }
    }
}

pub async fn trim_video_ffmpeg(
    app: &AppHandle,
    state: &AppState,
    input_path: &str,
    start_sec: f64,
    end_sec: Option<f64>,
) -> String {
    if state.is_cancelled() {
        return input_path.to_string();
    }

    let in_path = Path::new(input_path);
    let stem = in_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("video");
    let ext = in_path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("mp4");
    let trimmed_path: PathBuf = in_path.with_file_name(format!("{stem}_trimmed.{ext}"));

    let ffmpeg_bin = resolve_ffmpeg(state.resource_dir.as_deref())
        .unwrap_or_else(|| PathBuf::from("ffmpeg"));

    let mut cmd = Command::new(&ffmpeg_bin);
    cmd.arg("-y");

    if start_sec > 0.0 {
        cmd.args(["-ss", &start_sec.to_string()]);
    }

    cmd.args(["-i", input_path]);

    if let Some(end) = end_sec {
        let duration = end - start_sec;
        if duration > 0.0 {
            cmd.args(["-t", &duration.to_string()]);
        }
    }

    let ext_lower = ext.to_lowercase();
    let is_audio = matches!(
        ext_lower.as_str(),
        "mp3" | "m4a" | "wav" | "aac" | "ogg" | "flac" | "wma"
    );

    cmd.args(["-map", "0:v?", "-map", "0:a?", "-map_metadata", "0"]);

    if is_audio {
        cmd.args(["-c:v", "copy"]);
        if ext_lower == "mp3" {
            cmd.args(["-c:a", "libmp3lame", "-b:a", "192k"]);
        } else {
            cmd.args(["-c:a", "aac", "-b:a", "192k"]);
        }
    } else {
        cmd.args([
            "-c:v", "libx264",
            "-preset", "veryfast",
            "-crf", "18",
            "-pix_fmt", "yuv420p",
            "-c:a", "aac",
            "-b:a", "192k",
            "-movflags", "+faststart",
        ]);
    }

    cmd.args([
        "-reset_timestamps",
        "1",
        "-avoid_negative_ts",
        "make_zero",
        &trimmed_path.to_string_lossy(),
    ]);

    cmd.stdout(Stdio::null()).stderr(Stdio::piped());

    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    log_info(&format!("Trimming video: {input_path}"));
    let _ = app.emit(
        "download-progress",
        ProgressPayload {
            percent: "100%".to_string(),
            speed: "N/A".to_string(),
            size_info: "Video kesiliyor...".to_string(),
            playlist_info: String::new(),
            status: "trimming".to_string(),
        },
    );

    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            log_error(&format!("Failed to start FFmpeg trim: {e}"));
            return input_path.to_string();
        }
    };

    state.set_active_pid(child.id());
    if state.is_cancelled() {
        state.kill_active_child();
    }
    let output = child.wait_with_output().await;
    state.set_active_pid(None);

    match output {
        Ok(out) if out.status.success() && trimmed_path.exists() => {
            match safe_replace_file(&trimmed_path, in_path) {
                Ok(()) => {
                    log_info("Trim successful, replaced original file.");
                    input_path.to_string()
                }
                Err(e) => {
                    log_warn(&format!("Could not replace original file with trimmed file: {e}"));
                    trimmed_path.to_string_lossy().into_owned()
                }
            }
        }
        Ok(out) => {
            let stderr_text = String::from_utf8_lossy(&out.stderr);
            log_error(&format!(
                "FFmpeg trim failed ({:?}): {}",
                out.status.code(),
                stderr_text
            ));
            let _ = fs::remove_file(&trimmed_path);
            input_path.to_string()
        }
        Err(e) => {
            log_error(&format!("Trim error: {e}"));
            let _ = fs::remove_file(&trimmed_path);
            input_path.to_string()
        }
    }
}
