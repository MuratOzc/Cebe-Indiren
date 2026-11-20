use crate::binaries::{resolve_ffmpeg, resolve_ytdlp};
use crate::config::{log_error, log_info, log_warn};
use crate::ffmpeg::{time_to_seconds, trim_video_ffmpeg};
use crate::state::{AppState, ProgressPayload};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

#[cfg(windows)]
use crate::state::CREATE_NO_WINDOW;
#[cfg(windows)]
#[allow(unused_imports)]
use std::os::windows::process::CommandExt;

#[derive(Debug, Deserialize)]
pub struct DownloadRequest {
    pub url: String,
    pub download_dir: Option<String>,
    #[serde(default = "default_quality")]
    pub quality: String,
    #[serde(default)]
    pub audio_only: bool,
    #[serde(default)]
    pub download_playlist: bool,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
}

fn default_quality() -> String {
    "best".to_string()
}

#[derive(Debug, Serialize)]
pub struct DownloadResult {
    pub status: String,
    pub message: String,
    pub filename: String,
    pub full_path: String,
}

#[derive(Debug, Serialize)]
pub struct VideoInfoResult {
    pub status: String,
    pub duration: f64,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

fn format_bytes(b: f64) -> String {
    if b <= 0.0 || !b.is_finite() {
        return "0.0B".to_string();
    }
    let mut val = b;
    for unit in ["B", "KB", "MB", "GB"] {
        if val.abs() < 1024.0 {
            return format!("{val:.1}{unit}");
        }
        val /= 1024.0;
    }
    format!("{val:.1}TB")
}

fn should_try_next_browser(err_msg: &str) -> bool {
    let lower = err_msg.to_lowercase();
    const TERMS: [&str; 12] = [
        "cookie",
        "cookies",
        "browser",
        "profile",
        "database",
        "keyring",
        "decrypt",
        "permission denied",
        "access is denied",
        "forbidden",
        "403",
        "dpapi",
    ];
    TERMS.iter().any(|t| lower.contains(t))
}

pub fn cleanup_temp_dir(temp_dir: &Path) {
    if !temp_dir.exists() {
        return;
    }
    log_info(&format!(
        "Cleaning up isolated temp directory: {}",
        temp_dir.display()
    ));
    for attempt in 0..5 {
        if fs::remove_dir_all(temp_dir).is_ok() || !temp_dir.exists() {
            return;
        }
        if attempt < 4 {
            std::thread::sleep(Duration::from_millis(400));
        }
    }
    log_warn(&format!(
        "Could not fully remove temp directory: {}",
        temp_dir.display()
    ));
}

fn extract_youtube_id(url: &str) -> Option<String> {
    let lower = url.to_lowercase();
    if !lower.contains("youtube.com") && !lower.contains("youtu.be") {
        return None;
    }
    let clean_id = |raw: &str| -> Option<String> {
        let id: String = raw
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        if id.len() == 11 {
            Some(id)
        } else {
            None
        }
    };

    if let Some(pos) = lower.find("youtu.be/") {
        return clean_id(&url[pos + "youtu.be/".len()..]);
    }
    for prefix in ["/shorts/", "/embed/", "/live/"] {
        if let Some(pos) = lower.find(prefix) {
            return clean_id(&url[pos + prefix.len()..]);
        }
    }
    if let Some(query_pos) = url.find('?') {
        for pair in url[query_pos + 1..].split('&') {
            if let Some(val) = pair.strip_prefix("v=") {
                return clean_id(val);
            }
        }
    }
    None
}

async fn fast_youtube_info(video_id: &str) -> Option<VideoInfoResult> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(4))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .build()
        .ok()?;

    let body = serde_json::json!({
        "videoId": video_id,
        "context": {
            "client": {
                "clientName": "WEB",
                "clientVersion": "2.20261001.00.00"
            }
        }
    });

    let resp = client
        .post("https://www.youtube.com/youtubei/v1/player")
        .json(&body)
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let json: serde_json::Value = resp.json().await.ok()?;
    let details = json.get("videoDetails")?;
    let duration = details
        .get("lengthSeconds")
        .and_then(|v| v.as_str().and_then(|s| s.parse::<f64>().ok()).or_else(|| v.as_f64()))
        .unwrap_or(0.0);
    if duration <= 0.0 {
        return None;
    }
    let title = details
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown Title")
        .to_string();

    Some(VideoInfoResult {
        status: "success".to_string(),
        duration,
        title,
        message: None,
    })
}

fn extract_twitter_id(url: &str) -> Option<String> {
    let lower = url.to_lowercase();
    if !lower.contains("x.com/") && !lower.contains("twitter.com/") {
        return None;
    }
    let pos = lower.find("/status/")?;
    let rest = &url[pos + "/status/".len()..];
    let id: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if id.len() >= 10 {
        Some(id)
    } else {
        None
    }
}

async fn fast_twitter_info(tweet_id: &str) -> Option<VideoInfoResult> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(4))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .build()
        .ok()?;

    let resp = client
        .get(format!(
            "https://cdn.syndication.twimg.com/tweet-result?id={tweet_id}&token=a"
        ))
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let json: serde_json::Value = resp.json().await.ok()?;
    let duration_ms = json
        .get("video")
        .and_then(|v| v.get("durationMs"))
        .and_then(|v| v.as_f64())
        .or_else(|| {
            json.get("mediaDetails")
                .and_then(|m| m.as_array())
                .and_then(|arr| {
                    arr.iter().find_map(|item| {
                        item.get("video_info")
                            .and_then(|vi| vi.get("duration_millis"))
                            .and_then(|d| d.as_f64())
                    })
                })
        })?;

    if duration_ms <= 0.0 {
        return None;
    }

    let author = json
        .get("user")
        .and_then(|u| u.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or("");
    let text = json
        .get("text")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .lines()
        .next()
        .unwrap_or("")
        .trim();

    let title = match (author.is_empty(), text.is_empty()) {
        (false, false) => format!("{author} - {text}"),
        (false, true) => author.to_string(),
        (true, false) => text.to_string(),
        (true, true) => format!("X Video {tweet_id}"),
    };

    Some(VideoInfoResult {
        status: "success".to_string(),
        duration: duration_ms / 1000.0,
        title,
        message: None,
    })
}

pub async fn get_video_info(state: &AppState, url: &str) -> VideoInfoResult {
    // Fast direct paths (~150-300ms, no yt-dlp.exe startup or JS challenge)
    if let Some(yt_id) = extract_youtube_id(url) {
        if let Some(info) = fast_youtube_info(&yt_id).await {
            return info;
        }
    } else if let Some(tw_id) = extract_twitter_id(url) {
        if let Some(info) = fast_twitter_info(&tw_id).await {
            return info;
        }
    }

    // Lightweight yt-dlp fallback for all other sites (IG, TikTok, Facebook, etc.)
    let ytdlp = resolve_ytdlp(state.resource_dir.as_deref());
    let mut cmd = Command::new(&ytdlp);
    cmd.args([
        "--flat-playlist",
        "--playlist-items",
        "1",
        "--no-check-formats",
        "--no-warnings",
        "--no-color",
        "--extractor-args",
        "generic:impersonate=chrome",
        "--extractor-args",
        "youtube:player_skip=js",
        "--print",
        "%(duration)s|%(title)s",
        url,
    ])
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());

    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    match cmd.output().await {
        Ok(out) if out.status.success() => {
            let raw = String::from_utf8_lossy(&out.stdout);
            if let Some(line) = raw.lines().find(|l| l.contains('|')) {
                if let Some((dur_str, title_str)) = line.split_once('|') {
                    let duration = dur_str.trim().parse::<f64>().unwrap_or(0.0);
                    let title = title_str.trim();
                    return VideoInfoResult {
                        status: "success".to_string(),
                        duration,
                        title: if title.is_empty() || title == "NA" {
                            "Unknown Title".to_string()
                        } else {
                            title.to_string()
                        },
                        message: None,
                    };
                }
            }
            VideoInfoResult {
                status: "error".to_string(),
                duration: 0.0,
                title: String::new(),
                message: Some("Video bilgisi ayrıştırılamadı.".to_string()),
            }
        }
        Ok(out) => {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            log_error(&format!("Failed to fetch video info: {err}"));
            VideoInfoResult {
                status: "error".to_string(),
                duration: 0.0,
                title: String::new(),
                message: Some(err),
            }
        }
        Err(e) => {
            log_error(&format!("Failed to run yt-dlp for info: {e}"));
            VideoInfoResult {
                status: "error".to_string(),
                duration: 0.0,
                title: String::new(),
                message: Some(e.to_string()),
            }
        }
    }
}

fn build_ytdlp_args(
    request: &DownloadRequest,
    download_dir: &str,
    temp_dir: &Path,
    ffmpeg_path: Option<&Path>,
    browser_cookies: Option<&str>,
) -> Vec<String> {
    // Relative output templates allow yt-dlp's `-P home:` and `-P temp:` to isolate all
    // unfinished .part/.ytdl/.temp files inside our own temp directory.
    let outtmpl = if request.download_playlist {
        "%(playlist_title)s/%(playlist_index)s - %(title)s.%(ext)s".to_string()
    } else {
        "%(title)s [%(id)s].%(ext)s".to_string()
    };

    let mut args = vec![
        "--no-warnings".to_string(),
        "--no-color".to_string(),
        "--no-overwrites".to_string(),
        "--no-simulate".to_string(),
        "--newline".to_string(),
        "-N".to_string(),
        "10".to_string(),
        "--windows-filenames".to_string(),
        "--js-runtimes".to_string(),
        "deno".to_string(),
        "--js-runtimes".to_string(),
        "node".to_string(),
        "--extractor-args".to_string(),
        "generic:impersonate=chrome".to_string(),
        "-S".to_string(),
        "res,vcodec:h264,acodec:aac,ext:mp4:m4a".to_string(),
        "-P".to_string(),
        format!("home:{download_dir}"),
        "-P".to_string(),
        format!("temp:{}", temp_dir.to_string_lossy()),
        "-o".to_string(),
        outtmpl,
        "--progress-template".to_string(),
        "download:PROG|%(progress._percent_str)s|%(progress._speed_str)s|%(progress.downloaded_bytes)s|%(progress.total_bytes,progress.total_bytes_estimate)s|%(info.playlist_index)s|%(info.n_entries)s".to_string(),
        "--progress-template".to_string(),
        "postprocess:POST|%(progress.status)s".to_string(),
        "--print".to_string(),
        "after_move:FINAL_PATH|%(filepath)s".to_string(),
    ];

    if request.download_playlist {
        args.push("--yes-playlist".to_string());
        args.push("-i".to_string());
    } else {
        args.push("--no-playlist".to_string());
    }

    if let Some(ffmpeg) = ffmpeg_path {
        if let Some(parent) = ffmpeg.parent() {
            args.push("--ffmpeg-location".to_string());
            args.push(parent.to_string_lossy().into_owned());
        }
    }

    if request.audio_only {
        args.extend([
            "-f".to_string(),
            "bestaudio/best".to_string(),
            "-x".to_string(),
            "--audio-format".to_string(),
            "mp3".to_string(),
            "--audio-quality".to_string(),
            "192K".to_string(),
        ]);
    } else {
        let fmt = match request.quality.as_str() {
            "4k" => "bv*[height<=2160]+ba/b[height<=2160]/bv*+ba/b",
            "1080p" => "bv*[height<=1080]+ba/b[height<=1080]/bv*+ba/b",
            "720p" => "bv*[height<=720]+ba/b[height<=720]/bv*+ba/b",
            "480p" => "bv*[height<=480]+ba/b[height<=480]/bv*+ba/b",
            _ => "bv*+ba/b",
        };
        args.extend([
            "-f".to_string(),
            fmt.to_string(),
            "--merge-output-format".to_string(),
            "mp4".to_string(),
            "--postprocessor-args".to_string(),
            "Merger:-c copy -movflags +faststart".to_string(),
        ]);
    }

    if let Some(browser) = browser_cookies {
        args.push("--cookies-from-browser".to_string());
        args.push(browser.to_string());
    }

    args.push(request.url.clone());
    args
}

async fn attempt_download(
    app: &AppHandle,
    state: &AppState,
    request: &DownloadRequest,
    download_dir: &str,
    temp_dir: &Path,
    browser_cookies: Option<&str>,
) -> Result<String, String> {
    if state.is_cancelled() {
        return Err("DOWNLOAD_CANCELLED".to_string());
    }

    let ytdlp = resolve_ytdlp(state.resource_dir.as_deref());
    let ffmpeg = resolve_ffmpeg(state.resource_dir.as_deref());
    let args = build_ytdlp_args(request, download_dir, temp_dir, ffmpeg.as_deref(), browser_cookies);

    let mut cmd = Command::new(&ytdlp);
    cmd.args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("yt-dlp başlatılamadı: {e}"))?;

    state.set_active_pid(child.id());
    if state.is_cancelled() {
        state.kill_active_child();
        let _ = child.wait().await;
        return Err("DOWNLOAD_CANCELLED".to_string());
    }

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let stderr_task = tokio::spawn(async move {
        let mut buf = String::new();
        if let Some(err) = stderr {
            let mut reader = BufReader::new(err).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if !buf.is_empty() {
                    buf.push('\n');
                }
                buf.push_str(&line);
            }
        }
        buf
    });

    let mut final_paths: Vec<String> = Vec::new();

    if let Some(out) = stdout {
        let mut reader = BufReader::new(out).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            if state.is_cancelled() {
                state.kill_active_child();
                break;
            }
            if let Some(rest) = line.strip_prefix("PROG|") {
                let parts: Vec<&str> = rest.split('|').collect();
                if parts.len() >= 6 {
                    let percent = parts[0].trim().to_string();
                    let speed = parts[1].trim().to_string();
                    let dl = parts[2].trim().parse::<f64>().unwrap_or(0.0);
                    let total = parts[3].trim().parse::<f64>().unwrap_or(0.0);
                    let size_info = format!("{} / {}", format_bytes(dl), format_bytes(total));
                    let p_idx = parts[4].trim();
                    let n_ent = parts[5].trim();
                    let playlist_info = if !p_idx.is_empty()
                        && p_idx != "NA"
                        && !n_ent.is_empty()
                        && n_ent != "NA"
                    {
                        format!("{p_idx} / {n_ent}")
                    } else {
                        String::new()
                    };

                    let _ = app.emit(
                        "download-progress",
                        ProgressPayload {
                            percent,
                            speed,
                            size_info,
                            playlist_info,
                            status: "downloading".to_string(),
                        },
                    );
                }
            } else if let Some(rest) = line.strip_prefix("POST|") {
                let status = if rest.trim() == "finished" {
                    "finished"
                } else {
                    "merging"
                };
                let _ = app.emit(
                    "download-progress",
                    ProgressPayload {
                        percent: "100%".to_string(),
                        speed: "N/A".to_string(),
                        size_info: String::new(),
                        playlist_info: String::new(),
                        status: status.to_string(),
                    },
                );
            } else if let Some(path) = line.strip_prefix("FINAL_PATH|") {
                let trimmed = path.trim();
                if !trimmed.is_empty() {
                    final_paths.push(trimmed.to_string());
                }
            }
        }
    }

    let exit_status = child.wait().await;
    state.set_active_pid(None);
    let stderr_output = stderr_task.await.unwrap_or_default();

    if state.is_cancelled() {
        return Err("DOWNLOAD_CANCELLED".to_string());
    }

    match exit_status {
        Ok(s) if s.success() || (request.download_playlist && !final_paths.is_empty()) => {
            if let Some(first) = final_paths.first() {
                if request.download_playlist {
                    if let Some(parent) = Path::new(first).parent() {
                        return Ok(parent.to_string_lossy().into_owned());
                    }
                }
                return Ok(first.clone());
            }
            Ok(download_dir.to_string())
        }
        Ok(_) => {
            let msg = if stderr_output.trim().is_empty() {
                "Video indirilemedi.".to_string()
            } else {
                stderr_output
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("Video indirilemedi.")
                    .to_string()
            };
            Err(msg)
        }
        Err(e) => Err(format!("yt-dlp bekleme hatası: {e}")),
    }
}

pub async fn run_download(
    app: &AppHandle,
    state: &AppState,
    request: DownloadRequest,
    download_dir: String,
) -> Result<DownloadResult, String> {
    state.cancel_requested.store(false, Ordering::SeqCst);

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let temp_dir = Path::new(&download_dir).join(format!(".videoindiren_tmp_{stamp}"));
    let _ = fs::create_dir_all(&temp_dir);
    if let Ok(mut guard) = state.active_temp_dir.lock() {
        *guard = Some(temp_dir.clone());
    }

    log_info(&format!(
        "Received download request for URL: {} (Quality: {}, Audio: {}, Playlist: {})",
        request.url, request.quality, request.audio_only, request.download_playlist
    ));

    let start_sec = time_to_seconds(request.start_time.as_deref()).unwrap_or(0.0);
    let end_sec = time_to_seconds(request.end_time.as_deref());
    let needs_trim = !request.download_playlist && (start_sec > 0.0 || end_sec.is_some());

    log_info("Attempting download without browser cookies...");
    let mut dl_result = attempt_download(app, state, &request, &download_dir, &temp_dir, None).await;

    if let Err(ref no_cookie_err) = dl_result {
        if !state.is_cancelled() && no_cookie_err != "DOWNLOAD_CANCELLED" {
            log_warn(&format!(
                "Download without browser cookies failed, trying browser cookies: {no_cookie_err}"
            ));
            let first_err = no_cookie_err.clone();
            let browsers = ["firefox", "chrome", "brave", "opera", "edge"];
            let mut succeeded = false;

            for browser in browsers {
                if state.is_cancelled() {
                    break;
                }
                log_info(&format!("Attempting download with {browser} cookies..."));
                match attempt_download(app, state, &request, &download_dir, &temp_dir, Some(browser)).await {
                    Ok(path) => {
                        dl_result = Ok(path);
                        succeeded = true;
                        break;
                    }
                    Err(e) => {
                        if state.is_cancelled() || e == "DOWNLOAD_CANCELLED" {
                            dl_result = Err("DOWNLOAD_CANCELLED".to_string());
                            succeeded = true;
                            break;
                        }
                        if should_try_next_browser(&e) {
                            log_warn(&format!(
                                "Browser {browser} failed with retryable cookie/403 error, trying next..."
                            ));
                            continue;
                        }
                        break;
                    }
                }
            }

            if !succeeded {
                dl_result = Err(first_err);
            }
        }
    }

    cleanup_temp_dir(&temp_dir);
    if let Ok(mut guard) = state.active_temp_dir.lock() {
        *guard = None;
    }

    match dl_result {
        Ok(mut full_path) => {
            if let Ok(canon) = PathBuf::from(&full_path).canonicalize() {
                let s = canon.to_string_lossy().into_owned();
                // Strip Windows verbatim prefix \\?\ so explorer /select works cleanly
                full_path = s.strip_prefix(r"\\?\").unwrap_or(&s).to_string();
            }

            log_info(&format!("Download successful. Saved to: {full_path}"));

            if needs_trim && Path::new(&full_path).is_file() && !state.is_cancelled() {
                full_path = trim_video_ffmpeg(app, state, &full_path, start_sec, end_sec).await;
                if state.is_cancelled() {
                    // User cancelled mid-trim: discard the untrimmed source file too.
                    let _ = fs::remove_file(&full_path);
                    log_info("Trim was cancelled by user; removed untrimmed source file.");
                    return Ok(DownloadResult {
                        status: "cancelled".to_string(),
                        message: "İndirme iptal edildi".to_string(),
                        filename: String::new(),
                        full_path: String::new(),
                    });
                }
                log_info(&format!("Trimmed video saved to: {full_path}"));
            }

            if state.is_cancelled() {
                return Ok(DownloadResult {
                    status: "cancelled".to_string(),
                    message: "İndirme iptal edildi".to_string(),
                    filename: String::new(),
                    full_path: String::new(),
                });
            }

            if let Ok(mut last) = state.last_downloaded_path.lock() {
                *last = full_path.clone();
            }

            let filename = Path::new(&full_path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&full_path)
                .to_string();

            Ok(DownloadResult {
                status: "success".to_string(),
                message: "Video downloaded successfully".to_string(),
                filename,
                full_path,
            })
        }
        Err(e) => {
            if state.is_cancelled() || e == "DOWNLOAD_CANCELLED" {
                log_info("Download was cancelled by user.");
                return Ok(DownloadResult {
                    status: "cancelled".to_string(),
                    message: "İndirme iptal edildi".to_string(),
                    filename: String::new(),
                    full_path: String::new(),
                });
            }
            log_error(&format!("Download error: {e}"));
            Err(e)
        }
    }
}
