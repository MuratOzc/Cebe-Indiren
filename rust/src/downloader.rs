use crate::binaries::{resolve_ffmpeg, ytdlp_command as base_ytdlp_command};
use crate::config::{default_quality, log_error, log_info, log_warn};
use crate::ffmpeg::{self, time_to_seconds};
use crate::history;
use crate::platform::{hide_path, KeepAwake, NoConsole};
use crate::sponsorblock::{self, Removed};
use crate::state::{
    kill_process_tree, lock, AppState, ItemPayload, ProgressPayload,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
#[cfg(desktop)]
use tauri::window::{ProgressBarState, ProgressBarStatus};
#[cfg(desktop)]
use tauri::Manager;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, BufReader};
use tokio::process::Command;

const INFO_TIMEOUT: Duration = Duration::from_secs(45);
const STALE_TEMP_AGE: Duration = Duration::from_secs(24 * 60 * 60);
const TEMP_DIR_PREFIX: &str = ".videoindiren_tmp_";
const CANCELLED: &str = "DOWNLOAD_CANCELLED";
const TRIM_SOURCE_DIR: &str = "full";
const THUMBNAIL_DIR: &str = "thumbs";
const NOTHING_DOWNLOADED: &str = "İndirilecek video bulunamadı.";
const UPDATE_HINT: &str = "yt-dlp güncellemesi mevcut; güncelleyip tekrar deneyin.";
const MISSING_INDEX_PREFIX: &str = "NA - ";
const TASKBAR_IDLE: i32 = -2;
const TASKBAR_INDETERMINATE: i32 = -1;
const TITLE_MAX_BYTES: usize = 120;
const PLAYLIST_ITEM_TITLE_MAX_BYTES: usize = 100;
const PLAYLIST_TITLE_MAX_BYTES: usize = 60;
const ASSUMED_AUDIO_BYTES_PER_SEC: f64 = 16_000.0;
const SPONSORBLOCK_SKIPPED: &str = "SponsorBlock'a ulaşılamadı; video kesilmeden indirildi.";
const SPONSOR_FREE_SUFFIX: &str = " (sponsorsuz)";
const SPONSOR_FREE_COPY_NOTE: &str = "Klasörde aynı adla bir dosya vardı; ona dokunulmadı, sponsorsuz hali ayrı bir dosya olarak kaydedildi.";
const RANGE_REMOVED: &str = "Seçilen kesitin tamamı çıkarılan sponsor bölümünün içinde kalıyor. Farklı bir aralık seçin veya \"Sponsorları çıkar\" seçeneğini kapatın.";
const NOTHING_LEFT: &str = "Videonun tamamı sponsor bölümü olarak işaretlenmiş; çıkarılınca geriye bir şey kalmıyor. \"Sponsorları çıkar\" seçeneğini kapatıp tekrar deneyin.";
const TRIM_FAILED: &str = "Video indirildi ancak kesilemedi; tam hali kaydedildi.";
const SPONSOR_CUT_FAILED: &str = "Sponsor bölümleri çıkarılamadı; video kesilmeden kaydedildi.";
const WHOLE_FILE_SLACK: f64 = 0.1;
const SECTION_MIN_DURATION: f64 = 20.0 * 60.0;
const SECTION_MAX_SHARE_DIVISOR: f64 = 30.0;
const NOT_SAVED: &str = "Video indirildi ancak indirme klasörüne kaydedilemedi.";
const ITEM_TEMPLATE: &str = "video:ITEM|[%(thumbnail|null)j,%(duration|null)j,%(webpage_url|null)j,%(title|null)j]";

static TASKBAR_PROGRESS: AtomicI32 = AtomicI32::new(TASKBAR_IDLE);

#[derive(Debug, Clone, Deserialize)]
pub struct DownloadRequest {
    pub url: String,
    #[serde(default = "default_quality")]
    pub quality: String,
    #[serde(default)]
    pub audio_only: bool,
    #[serde(default)]
    pub download_playlist: bool,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    #[serde(default)]
    pub save_copy: bool,
    #[serde(default)]
    pub sponsorblock: bool,
    #[serde(default)]
    pub duration: Option<f64>,
}

#[derive(Debug, Default, Serialize)]
pub struct DownloadResult {
    pub status: String,
    pub filename: String,
    pub full_path: String,
    pub is_dir: bool,
    pub already_existed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub removed: Option<String>,
    pub thumbnail: String,
    pub duration: f64,
    pub size: u64,
}

#[derive(Debug, Serialize)]
pub struct VideoInfo {
    pub duration: f64,
    pub title: String,
    pub thumbnail: Option<String>,
    pub playlist_title: Option<String>,
    pub playlist_count: Option<u64>,
}

impl VideoInfo {
    fn new(duration: f64, title: String, thumbnail: Option<String>) -> Self {
        Self {
            duration,
            title,
            thumbnail: thumbnail.filter(|t| t.starts_with("https://")),
            playlist_title: None,
            playlist_count: None,
        }
    }
}

#[cfg_attr(mobile, allow(unused_variables))]
fn set_taskbar_progress(app: &AppHandle, value: i32) {
    if TASKBAR_PROGRESS.swap(value, Ordering::Relaxed) == value {
        return;
    }
    #[cfg(desktop)]
    show_taskbar_progress(app, value);
}

#[cfg(desktop)]
fn show_taskbar_progress(app: &AppHandle, value: i32) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let (status, progress) = match value {
        TASKBAR_IDLE => (ProgressBarStatus::None, None),
        TASKBAR_INDETERMINATE => (ProgressBarStatus::Indeterminate, None),
        percent => (ProgressBarStatus::Normal, Some(percent.clamp(0, 100) as u64)),
    };
    let _ = window.set_progress_bar(ProgressBarState {
        status: Some(status),
        progress,
    });
}

pub fn clear_taskbar_progress(app: &AppHandle) {
    set_taskbar_progress(app, TASKBAR_IDLE);
}

fn taskbar_value(payload: &ProgressPayload) -> i32 {
    if payload.status == "merging" {
        return TASKBAR_INDETERMINATE;
    }
    let shown = if payload.overall.is_empty() {
        &payload.percent
    } else {
        &payload.overall
    };
    parse_percent(shown).map_or(TASKBAR_INDETERMINATE, |p| p as i32)
}

pub fn report_progress(app: &AppHandle, payload: ProgressPayload) {
    set_taskbar_progress(app, taskbar_value(&payload));
    let _ = app.emit("download-progress", payload);
}

fn ytdlp_command() -> Command {
    let mut cmd = base_ytdlp_command();
    // Without this yt-dlp writes piped output in the ANSI code page, which drops or
    // mangles non-ASCII titles and makes the reported final path unusable.
    cmd.args(["--encoding", "utf-8"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .no_console();
    cmd
}

pub async fn next_line_lossy<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    buf: &mut Vec<u8>,
) -> Option<String> {
    buf.clear();
    match reader.read_until(b'\n', buf).await {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(
            String::from_utf8_lossy(buf)
                .trim_end_matches(['\r', '\n'])
                .to_string(),
        ),
    }
}

fn clean_url(raw: &str) -> Option<&str> {
    let url = raw.trim();
    if url.is_empty() || url.starts_with('-') || url.chars().any(char::is_control) {
        None
    } else {
        Some(url)
    }
}

fn last_error_line(stderr: &str) -> Option<&str> {
    let mut fallback = None;
    for line in stderr.lines().rev().map(str::trim).filter(|l| !l.is_empty()) {
        if line.starts_with("ERROR:") {
            return Some(line);
        }
        fallback.get_or_insert(line);
    }
    fallback
}

fn is_definitive_error(err_msg: &str) -> bool {
    let lower = err_msg.to_lowercase();
    const TERMS: [&str; 21] = [
        "unsupported url",
        "ffprobe not found",
        "ffmpeg not found",
        "requested format is not available",
        "has been removed",
        "live event will begin",
        "premieres in",
        "premiere will begin",
        "is not a valid url",
        "getaddrinfo failed",
        "name or service not known",
        "temporary failure in name resolution",
        "failed to resolve",
        "network is unreachable",
        "could not resolve host",
        "failed to connect",
        "could not connect",
        "http error 404",
        "no space left",
        "not enough space",
        "yt-dlp başlatılamadı",
    ];
    err_msg == NOTHING_DOWNLOADED || TERMS.iter().any(|t| lower.contains(t))
}

fn should_try_next_browser(err_msg: &str) -> bool {
    let lower = err_msg.to_lowercase();
    const TERMS: [&str; 14] = [
        "cookie",
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
        "authentication",
        "sign in",
        "login",
    ];
    TERMS.iter().any(|t| lower.contains(t))
}

fn known_error_message(raw: &str) -> Option<&'static str> {
    let lower = raw.to_lowercase();
    let has = |terms: &[&str]| terms.iter().any(|t| lower.contains(t));

    let msg = if has(&["unsupported url"]) {
        "Bu bağlantı desteklenmiyor."
    } else if has(&["is not a valid url"]) {
        "Lütfen geçerli bir bağlantı girin."
    } else if has(&[
        "getaddrinfo failed",
        "name or service not known",
        "temporary failure in name resolution",
        "failed to resolve",
        "network is unreachable",
        "could not resolve host",
        "failed to connect",
        "could not connect",
        "timed out",
        "connection refused",
        "connection reset",
    ]) {
        "İnternet bağlantısı kurulamadı. Bağlantınızı kontrol edip tekrar deneyin."
    } else if has(&["http error 404"]) {
        "Bu adreste bir video bulunamadı (404). Bağlantıyı kontrol edin."
    } else if has(&["confirm your age", "age-restricted", "age restricted"]) {
        "Bu video yaş kısıtlamalı. Tarayıcınızda siteye giriş yapıp tekrar deneyin."
    } else if has(&["not a bot"]) {
        "Site bot doğrulaması istiyor. Tarayıcınızda siteye giriş yapıp tekrar deneyin."
    } else if has(&["private video", "video is private"]) {
        "Bu video gizli."
    } else if has(&["in your country", "geo-restricted", "geo restricted", "from your location"]) {
        "Bu video bulunduğunuz ülkede kullanılamıyor."
    } else if has(&["live event will begin", "premieres in", "premiere will begin"]) {
        "Bu yayın henüz başlamadı."
    } else if has(&["requested format is not available"]) {
        "İstenen kalite bu video için mevcut değil."
    } else if has(&["video unavailable", "is not available", "has been removed", "no longer available"]) {
        "Video kullanılamıyor veya kaldırılmış."
    } else if has(&["no space left", "not enough space"]) {
        "Diskte yeterli alan yok."
    } else if has(&["http error 429", "too many requests"]) {
        "Çok fazla istek gönderildi. Biraz sonra tekrar deneyin."
    } else if has(&["ffmpeg", "ffprobe"]) && has(&["not found", "not installed"]) {
        "FFmpeg bileşenleri bulunamadı. Uygulamayı yeniden başlatın."
    } else if has(&["login required", "log in", "sign in", "logged-in", "members-only", "members only"]) {
        "Bu içerik oturum gerektiriyor. Tarayıcınızda siteye giriş yapıp tekrar deneyin."
    } else if has(&["http error 403"]) {
        "Site erişimi reddetti (403). Tarayıcınızda siteye giriş yapıp tekrar deneyin."
    } else {
        return None;
    };
    Some(msg)
}

/// Whether a failure looks like yt-dlp itself falling behind a site, rather than a problem
/// with the link, the network or the account.
fn update_may_help(raw: &str) -> bool {
    let lower = raw.to_lowercase();
    raw.trim_start().starts_with("ERROR:")
        && (known_error_message(raw).is_none()
            || lower.contains("http error 403")
            || lower.contains("requested format is not available"))
}

pub fn friendly_error(raw: &str) -> String {
    if let Some(msg) = known_error_message(raw) {
        return msg.to_string();
    }

    let trimmed = raw.trim();
    let stripped = trimmed.strip_prefix("ERROR:").unwrap_or(trimmed);
    let stripped = stripped
        .split_once(" (caused by ")
        .map_or(stripped, |(message, _)| message)
        .trim();
    if stripped.is_empty() {
        "Video indirilemedi.".to_string()
    } else {
        stripped.to_string()
    }
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

fn remove_stale_temp_dirs(download_dir: &Path) {
    let Ok(entries) = fs::read_dir(download_dir) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with(TEMP_DIR_PREFIX)
        {
            continue;
        }
        let stale = entry
            .metadata()
            .ok()
            .filter(|m| m.is_dir())
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > STALE_TEMP_AGE);
        if stale {
            log_info(&format!(
                "Removing stale temp directory: {}",
                entry.path().display()
            ));
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

pub fn extract_youtube_id(url: &str) -> Option<String> {
    // ASCII-only lowercasing keeps byte offsets valid for slicing `url` below.
    let lower = url.to_ascii_lowercase();
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

async fn fast_youtube_info(video_id: &str) -> Option<VideoInfo> {
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
        .unwrap_or("Bilinmeyen Başlık")
        .to_string();

    Some(VideoInfo::new(
        duration,
        title,
        Some(format!("https://i.ytimg.com/vi/{video_id}/mqdefault.jpg")),
    ))
}

fn parse_info_output(raw: &str) -> Option<VideoInfo> {
    let mut lines = raw.lines();
    let duration = lines.next()?.trim().parse::<f64>().unwrap_or(0.0);
    let thumbnail = lines.next()?.trim();
    let playlist_count = lines.next()?.trim().parse::<u64>().ok().filter(|n| *n > 0);
    let playlist_title = serde_json::from_str::<String>(lines.next()?.trim())
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty() && t != "NA");
    let title = lines.collect::<Vec<_>>().join(" ");
    let title = title.trim();

    let mut info = VideoInfo::new(
        if duration.is_finite() { duration.max(0.0) } else { 0.0 },
        if title.is_empty() || title == "NA" {
            "Bilinmeyen Başlık".to_string()
        } else {
            title.to_string()
        },
        (thumbnail != "NA").then(|| thumbnail.to_string()),
    );
    info.playlist_title = playlist_title;
    info.playlist_count = playlist_count;
    Some(info)
}

pub async fn kill_info_lookup(state: &AppState) {
    let pid = lock(&state.info_child_pid).take();
    if let Some(pid) = pid {
        let _ = tokio::task::spawn_blocking(move || kill_process_tree(pid)).await;
    }
}

fn has_playlist_param(url: &str) -> bool {
    url.split_once('?').is_some_and(|(_, query)| {
        query
            .split(['&', '#'])
            .any(|pair| pair.strip_prefix("list=").is_some_and(|id| !id.is_empty()))
    })
}

// The playlist switch must match the download's: for a video opened from a playlist, yt-dlp
// otherwise describes the playlist's first entry instead of the linked video.
fn info_args(url: &str, playlist: bool) -> Vec<&str> {
    vec![
        "--flat-playlist",
        "--playlist-items",
        "1",
        if playlist { "--yes-playlist" } else { "--no-playlist" },
        "--no-check-formats",
        "--no-warnings",
        "--no-color",
        "--extractor-args",
        "generic:impersonate=chrome",
        "--extractor-args",
        "youtube:player_skip=js",
        "--print",
        "%(duration)s",
        "--print",
        "%(thumbnail,thumbnails.-1.url)s",
        "--print",
        "%(playlist_count)s",
        "--print",
        "%(playlist_title)j",
        "--print",
        "%(title)s",
        "--",
        url,
    ]
}

async fn run_info_lookup(
    state: &AppState,
    url: &str,
    playlist: bool,
    browser_cookies: Option<&str>,
) -> Result<VideoInfo, (String, bool)> {
    const FAILED: &str = "Video bilgisi alınamadı.";

    let mut args: Vec<&str> = info_args(url, playlist);
    if let Some(browser) = browser_cookies {
        let insert_at = args.len().saturating_sub(2);
        args.splice(insert_at..insert_at, ["--cookies-from-browser", browser]);
    }

    let child = ytdlp_command().args(args).spawn().map_err(|e| {
        log_error(&format!("Failed to run yt-dlp for info: {e}"));
        (FAILED.to_string(), false)
    })?;
    let pid = child.id();
    *lock(&state.info_child_pid) = pid;

    let waited = tokio::time::timeout(INFO_TIMEOUT, child.wait_with_output()).await;
    {
        let mut current = lock(&state.info_child_pid);
        if pid.is_none() || *current != pid {
            return Err((FAILED.to_string(), false));
        }
        *current = None;
    }

    let out = match waited {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => {
            log_error(&format!("Failed to read yt-dlp info output: {e}"));
            return Err((FAILED.to_string(), false));
        }
        Err(_) => {
            // The packaged yt-dlp is a bootloader plus a worker process, so the whole tree must go.
            if let Some(pid) = pid {
                let _ = tokio::task::spawn_blocking(move || kill_process_tree(pid)).await;
            }
            log_warn("yt-dlp info lookup timed out.");
            return Err(("Video bilgisi zaman aşımına uğradı.".to_string(), false));
        }
    };

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let raw = last_error_line(&stderr).unwrap_or_default().to_string();
        let retryable = should_try_next_browser(&raw);
        return Err((raw, retryable));
    }

    parse_info_output(&String::from_utf8_lossy(&out.stdout))
        .ok_or_else(|| ("Video bilgisi ayrıştırılamadı.".to_string(), false))
}

pub async fn fetch_video_info(
    state: &AppState,
    url: &str,
    playlist: bool,
) -> Result<VideoInfo, String> {
    let url = clean_url(url).ok_or("Lütfen geçerli bir bağlantı girin.")?;

    kill_info_lookup(state).await;

    if let Some(yt_id) = extract_youtube_id(url) {
        if !(playlist && has_playlist_param(url)) {
            if let Some(info) = fast_youtube_info(&yt_id).await {
                return Ok(info);
            }
        }
    }

    let (first_err, retryable) = match run_info_lookup(state, url, playlist, None).await {
        Ok(info) => return Ok(info),
        Err(err) => err,
    };

    if retryable {
        for browser in browser_cookie_sources() {
            match run_info_lookup(state, url, playlist, Some(&browser)).await {
                Ok(info) => return Ok(info),
                Err((_, true)) => continue,
                Err((_, false)) => break,
            }
        }
    }

    log_error(&format!("Failed to fetch video info: {first_err}"));
    Err(friendly_error(&first_err))
}

// The cap goes through the sort, where `res` is the smaller dimension: a `height<=` filter
// would hand a 1080x1920 vertical video out as 608x1080.
fn format_sort(quality: &str) -> String {
    let res = match quality {
        "4k" => "res:2160",
        "1440p" => "res:1440",
        "1080p" => "res:1080",
        "720p" => "res:720",
        "480p" => "res:480",
        _ => "res",
    };
    format!("{res},vcodec:h264,acodec:aac,ext:mp4:m4a")
}

fn build_ytdlp_args(
    request: &DownloadRequest,
    url: &str,
    home_dir: &Path,
    temp_dir: &Path,
    ffmpeg_path: Option<&Path>,
    browser_cookies: Option<&str>,
    keep_source: bool,
) -> Vec<String> {
    // Relative output templates allow yt-dlp's `-P home:` and `-P temp:` to isolate all
    // unfinished .part/.ytdl/.temp files inside our own temp directory.
    // Titles are capped in bytes: an uncapped one pushes the temp path past MAX_PATH.
    let outtmpl = if request.download_playlist {
        // playlist_result_path relies on "." and the "NA - " index of a non-playlist link.
        format!(
            "%(playlist_title|.).{PLAYLIST_TITLE_MAX_BYTES}B/%(playlist_index)s - %(title).{PLAYLIST_ITEM_TITLE_MAX_BYTES}B.%(ext)s"
        )
    } else {
        format!("%(title).{TITLE_MAX_BYTES}B [%(id)s].%(ext)s")
    };

    let dir_arg = |kind: &str, dir: &Path| format!("{kind}:{}", dir.to_string_lossy());
    let mut args: Vec<String> = Vec::new();
    let mut add = |more: &[&str]| args.extend(more.iter().map(|arg| arg.to_string()));

    add(&[
        "--no-warnings",
        "--no-color",
        "--no-overwrites",
        "--no-simulate",
        "--newline",
        // --print implies --quiet, which silences the download progress lines without this.
        "--progress",
        "-N",
        "10",
        "--windows-filenames",
        "--js-runtimes",
        "deno",
        "--js-runtimes",
        "node",
        "--extractor-args",
        "generic:impersonate=chrome",
        "--progress-template",
        "download:PROG|%(progress._percent_str)s|%(progress._speed_str)s|%(progress.downloaded_bytes)s|%(progress.total_bytes,progress.total_bytes_estimate)s|%(info.playlist_index)s|%(info.n_entries)s|%(progress._eta_str)s|%(info.vcodec)s|%(info.acodec)s|%(info.duration)s|%(info.title)s",
        "--progress-template",
        "postprocess:POST|%(progress.status)s",
        "--print",
        "post_process:SOURCE_PATH|%(filepath)s",
        "--print",
        "after_move:FINAL_PATH|%(filepath)s",
        "--print",
        ITEM_TEMPLATE,
    ]);
    add(&["-S", &format_sort(&request.quality)]);
    add(&["-P", &dir_arg("home", home_dir)]);
    add(&["-P", &dir_arg("temp", temp_dir)]);
    add(&["-o", &outtmpl]);

    if request.download_playlist {
        add(&["--yes-playlist", "-i"]);
    } else {
        add(&["--no-playlist"]);
    }

    if let Some(ffmpeg) = ffmpeg_path {
        add(&["--ffmpeg-location", &ffmpeg.to_string_lossy()]);
    }

    if request.audio_only {
        add(&[
            "-f",
            "bestaudio/best",
            "-x",
            "--audio-format",
            "mp3",
            "--audio-quality",
            "192K",
            "--embed-metadata",
            "--embed-thumbnail",
            "--convert-thumbnails",
            "jpg",
            // --keep-video keeps the cover image too; it must not land in the download folder.
            "-P",
            &dir_arg("thumbnail", &temp_dir.join(THUMBNAIL_DIR)),
        ]);
        // Extraction deletes its source, and yt-dlp takes a same-named file already in the
        // download folder (a video saved earlier from this link) as that source.
        if keep_source {
            add(&["--keep-video"]);
        }
    } else {
        add(&[
            "-f",
            "bv*+ba/b",
            "--merge-output-format",
            "mp4",
            "--postprocessor-args",
            "Merger:-c copy -movflags +faststart",
        ]);
    }

    if request.sponsorblock {
        add(&[
            "--sponsorblock-mark",
            &sponsorblock::CATEGORIES.join(","),
            "--no-embed-chapters",
            "--print",
            "after_move:SPONSORS|%(sponsorblock_chapters|null)j",
        ]);
    }

    if let Some((start, end)) = section_download(request) {
        let end = end.map_or_else(|| "inf".to_string(), |end| end.to_string());
        add(&[
            "--download-sections",
            &format!("*{start}-{end}"),
            "--force-keyframes-at-cuts",
        ]);
    }

    if let Some(browser) = browser_cookies {
        add(&["--cookies-from-browser", browser]);
    }

    add(&["--", url]);
    args
}

fn wants_trim(request: &DownloadRequest) -> bool {
    let start = time_to_seconds(request.start_time.as_deref()).unwrap_or(0.0);
    let end = time_to_seconds(request.end_time.as_deref());
    !request.download_playlist && (start > 0.0 || end.is_some())
}

/// The range to fetch when downloading only the clip beats downloading the whole video and
/// cutting it locally. Measured on 1080p YouTube: a range costs ~0.6 s per clip second, a full
/// download ~0.02 s per video second, so only clips under ~1/30 of a long video win.
fn section_download(request: &DownloadRequest) -> Option<(f64, Option<f64>)> {
    if !wants_trim(request) || request.sponsorblock || request.audio_only {
        return None;
    }
    let duration = request
        .duration
        .filter(|d| d.is_finite() && *d > SECTION_MIN_DURATION)?;
    let start = time_to_seconds(request.start_time.as_deref()).unwrap_or(0.0);
    let end = time_to_seconds(request.end_time.as_deref());
    let clip_len = end.unwrap_or(duration).min(duration) - start;
    (clip_len > 0.0 && clip_len < duration / SECTION_MAX_SHARE_DIVISOR).then_some((start, end))
}

fn is_sponsorblock_error(err_msg: &str) -> bool {
    let lower = err_msg.to_lowercase();
    lower.contains("sponsorblock")
}

fn only_sponsorblock_errors(stderr: &str) -> bool {
    let mut errors = stderr
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("ERROR:"))
        .peekable();
    errors.peek().is_some() && errors.all(is_sponsorblock_error)
}

fn parse_item_line(rest: &str) -> Option<ItemPayload> {
    let fields: Vec<serde_json::Value> = serde_json::from_str(rest.trim()).ok()?;
    let text = |i: usize| {
        fields
            .get(i)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|t| !t.is_empty() && *t != "NA")
            .unwrap_or_default()
            .to_string()
    };
    Some(ItemPayload {
        thumbnail: Some(text(0))
            .filter(|t| t.starts_with("https://"))
            .unwrap_or_default(),
        duration: fields
            .get(1)
            .and_then(|v| v.as_f64())
            .filter(|d| d.is_finite() && *d > 0.0)
            .unwrap_or(0.0),
        url: Some(text(2))
            .filter(|u| u.starts_with("http://") || u.starts_with("https://"))
            .unwrap_or_default(),
        title: text(3),
    })
}

fn parse_percent(text: &str) -> Option<f64> {
    text.trim()
        .trim_end_matches('%')
        .parse::<f64>()
        .ok()
        .filter(|p| p.is_finite())
        .map(|p| p.clamp(0.0, 100.0))
}

/// Folds the separate video and audio passes of one item into a single 0-100 run.
#[derive(Default)]
struct ProgressTracker {
    item: String,
    video_bytes: f64,
    floor: f64,
}

impl ProgressTracker {
    fn item_percent(&mut self, parts: &[&str], raw: f64) -> f64 {
        let field = |i: usize| parts.get(i).copied().unwrap_or("NA");
        let number = |i: usize| {
            field(i)
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite() && *v > 0.0)
        };
        let present = |codec: &str| !matches!(codec, "" | "NA" | "none");

        if self.item != field(4) {
            *self = Self {
                item: field(4).to_string(),
                ..Self::default()
            };
        }

        let (vcodec, acodec) = (field(7), field(8));
        let combined = if present(vcodec) && acodec == "none" {
            let total = number(3);
            if let Some(done) = number(2) {
                self.video_bytes = done;
            }
            match (total, number(9)) {
                (Some(total), Some(duration)) => {
                    raw * total / (total + duration * ASSUMED_AUDIO_BYTES_PER_SEC)
                }
                _ => raw,
            }
        } else if vcodec == "none" && present(acodec) && self.video_bytes > 0.0 {
            match number(3) {
                Some(total) => {
                    (self.video_bytes * 100.0 + raw * total) / (self.video_bytes + total)
                }
                None => raw,
            }
        } else {
            raw
        };

        self.floor = combined.clamp(self.floor, 100.0);
        self.floor
    }
}

fn parse_progress_line(rest: &str, tracker: &mut ProgressTracker) -> Option<ProgressPayload> {
    let parts: Vec<&str> = rest.split('|').map(str::trim).collect();
    if parts.len() < 6 {
        return None;
    }
    let known = |s: &str| !s.is_empty() && s != "NA" && s != "Unknown";

    let dl = parts[2].parse::<f64>().unwrap_or(0.0);
    let total = parts[3].parse::<f64>().unwrap_or(0.0);
    let size_info = if total > 0.0 {
        format!("{} / {}", format_bytes(dl), format_bytes(total))
    } else {
        format_bytes(dl)
    };
    let playlist_info = if known(parts[4]) && known(parts[5]) {
        format!("{} / {}", parts[4], parts[5])
    } else {
        String::new()
    };

    let item_percent = parse_percent(parts[0]).map(|raw| tracker.item_percent(&parts, raw));
    let overall = match (item_percent, parts[4].parse::<f64>(), parts[5].parse::<f64>()) {
        (Some(item), Ok(index), Ok(count)) if index >= 1.0 && count > 1.0 && index <= count => {
            format!("{:.1}%", (index - 1.0 + item / 100.0) / count * 100.0)
        }
        _ => String::new(),
    };

    Some(ProgressPayload {
        percent: item_percent.map_or_else(|| parts[0].to_string(), |p| format!("{p:.1}%")),
        overall,
        speed: if known(parts[1]) { parts[1].to_string() } else { String::new() },
        size_info,
        playlist_info,
        eta: parts
            .get(6)
            .copied()
            .filter(|e| known(e))
            .unwrap_or_default()
            .to_string(),
        title: parts
            .get(10..)
            .map(|rest| rest.join("|"))
            .filter(|t| known(t))
            .unwrap_or_default(),
        status: "downloading".to_string(),
    })
}

struct Saved {
    path: PathBuf,
    warning: Option<String>,
    already_existed: bool,
    item: Option<ItemPayload>,
    removed: Removed,
    cuts: Vec<(PathBuf, Vec<(f64, f64)>)>,
}

/// `--keep-video` also keeps the sources this run downloaded itself and moves them into
/// `home_dir`; those are removed here. A source outside `temp_dir` was the user's own file.
fn remove_kept_sources(sources: &[String], home_dir: &Path, temp_dir: &Path) {
    for source in sources {
        let source = Path::new(source);
        let converted = source
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| !e.eq_ignore_ascii_case("mp3"));
        let Ok(relative) = source.strip_prefix(temp_dir) else {
            continue;
        };
        let copy = home_dir.join(relative);
        if converted && copy.is_file() {
            if let Err(e) = fs::remove_file(&copy) {
                log_warn(&format!("Could not remove {}: {e}", copy.display()));
            }
        }
    }
}

async fn attempt_download(
    app: &AppHandle,
    state: &AppState,
    request: &DownloadRequest,
    home_dir: &Path,
    temp_dir: &Path,
    browser_cookies: Option<&str>,
) -> Result<Saved, String> {
    if state.is_cancelled() {
        return Err(CANCELLED.to_string());
    }

    let url = clean_url(&request.url).ok_or("is not a valid URL")?;
    let ffmpeg = resolve_ffmpeg();
    let keep_source = request.audio_only && !home_dir.starts_with(temp_dir);
    let args = build_ytdlp_args(
        request,
        url,
        home_dir,
        temp_dir,
        ffmpeg.as_deref(),
        browser_cookies,
        keep_source,
    );

    let mut child = ytdlp_command()
        .args(&args)
        .spawn()
        .map_err(|e| format!("yt-dlp başlatılamadı: {e}"))?;

    *lock(&state.active_child_pid) = child.id();
    if state.is_cancelled() {
        state.kill_active_child();
        let _ = child.wait().await;
        return Err(CANCELLED.to_string());
    }

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let stderr_task = tokio::spawn(async move {
        let mut buf = String::new();
        if let Some(err) = stderr {
            let mut reader = BufReader::new(err);
            let mut raw = Vec::new();
            while let Some(line) = next_line_lossy(&mut reader, &mut raw).await {
                if !buf.is_empty() {
                    buf.push('\n');
                }
                buf.push_str(&line);
            }
        }
        buf
    });

    let mut final_paths: Vec<String> = Vec::new();
    let mut source_paths: Vec<String> = Vec::new();
    let mut tracker = ProgressTracker::default();
    let mut first_item: Option<ItemPayload> = None;
    let mut cuts: Vec<(PathBuf, Vec<(f64, f64)>)> = Vec::new();

    if let Some(out) = stdout {
        let mut reader = BufReader::new(out);
        let mut raw = Vec::new();
        while let Some(line) = next_line_lossy(&mut reader, &mut raw).await {
            if state.is_cancelled() {
                state.kill_active_child();
                break;
            }
            if let Some(rest) = line.strip_prefix("PROG|") {
                if let Some(payload) = parse_progress_line(rest, &mut tracker) {
                    report_progress(app, payload);
                }
            } else if let Some(rest) = line.strip_prefix("POST|") {
                let status = if rest.trim() == "finished" {
                    "finished"
                } else {
                    "merging"
                };
                report_progress(app, ProgressPayload::stage(status, "100%".to_string()));
            } else if let Some(rest) = line.strip_prefix("ITEM|") {
                if let Some(item) = parse_item_line(rest) {
                    let _ = app.emit("download-item", &item);
                    first_item.get_or_insert(item);
                    // yt-dlp reports no progress for a range until it is complete.
                    if section_download(request).is_some() {
                        report_progress(app, ProgressPayload::stage("section", String::new()));
                    }
                }
            } else if let Some(json) = line.strip_prefix("SPONSORS|") {
                let segments = sponsorblock::parse_cuts(json);
                if let (Some(path), false) = (final_paths.last(), segments.is_empty()) {
                    cuts.push((PathBuf::from(path), segments));
                }
            } else if let Some(path) = line.strip_prefix("SOURCE_PATH|") {
                let trimmed = path.trim();
                if !trimmed.is_empty() {
                    source_paths.push(trimmed.to_string());
                }
            } else if let Some(path) = line.strip_prefix("FINAL_PATH|") {
                let trimmed = path.trim();
                if !trimmed.is_empty() {
                    final_paths.push(trimmed.to_string());
                }
            }
        }
    }

    let exit_status = child.wait().await;
    *lock(&state.active_child_pid) = None;
    let stderr_output = stderr_task.await.unwrap_or_default();

    if keep_source {
        remove_kept_sources(&source_paths, home_dir, temp_dir);
    }

    if state.is_cancelled() {
        return Err(CANCELLED.to_string());
    }

    // yt-dlp finishes the download when SponsorBlock cannot be reached and only then
    // reports the failure, so the saved file is complete, just uncut.
    let only_sponsors_failed = request.sponsorblock
        && !final_paths.is_empty()
        && only_sponsorblock_errors(&stderr_output);

    match exit_status {
        Ok(s) if s.success()
            || only_sponsors_failed
            || (request.download_playlist && !final_paths.is_empty()) =>
        {
            let Some(first) = final_paths.first().map(Path::new) else {
                return Err(NOTHING_DOWNLOADED.to_string());
            };
            let path = if request.download_playlist {
                playlist_result_path(first, home_dir)
            } else {
                first.to_path_buf()
            };
            let already_existed = !request.download_playlist
                && Some(first) == source_paths.first().map(Path::new);
            Ok(Saved {
                path,
                warning: if s.success() {
                    None
                } else if only_sponsors_failed {
                    Some(SPONSORBLOCK_SKIPPED.to_string())
                } else {
                    Some("Listedeki bazı videolar indirilemedi.".to_string())
                },
                already_existed,
                item: first_item,
                removed: Removed::default(),
                cuts,
            })
        }
        Ok(_) => Err(last_error_line(&stderr_output)
            .unwrap_or("Video indirilemedi.")
            .to_string()),
        Err(e) => Err(format!("yt-dlp bekleme hatası: {e}")),
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

fn playlist_result_path(first: &Path, home_dir: &Path) -> PathBuf {
    let Some(parent) = first.parent() else {
        return first.to_path_buf();
    };
    if !same_path(parent, home_dir) {
        return parent.to_path_buf();
    }
    let Some(name) = first
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_prefix(MISSING_INDEX_PREFIX))
    else {
        return first.to_path_buf();
    };
    let named = Path::new(name);
    let ext = named.extension().and_then(|e| e.to_str()).unwrap_or("");
    let dest = unique_destination(parent, &file_stem_of(named), ext);
    match fs::rename(first, &dest) {
        Ok(()) => dest,
        Err(_) => first.to_path_buf(),
    }
}

fn clip_time_label(seconds: f64) -> String {
    let total = seconds.max(0.0).round() as u64;
    let (h, m, s) = (total / 3600, (total / 60) % 60, total % 60);
    if h > 0 {
        format!("{h}.{m:02}.{s:02}")
    } else {
        format!("{m:02}.{s:02}")
    }
}

fn clip_file_stem(stem: &str, start_sec: f64, end_sec: Option<f64>) -> String {
    let end = end_sec.map_or_else(|| "son".to_string(), clip_time_label);
    format!("{stem} ({}-{end})", clip_time_label(start_sec))
}

fn unique_destination(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let name = |n: u32| {
        let suffix = if n > 1 { format!(" ({n})") } else { String::new() };
        file_name_with(&format!("{stem}{suffix}"), ext)
    };
    (1..)
        .map(|n| dir.join(name(n)))
        .find(|candidate| !candidate.exists())
        .unwrap_or_else(|| dir.join(name(1)))
}

fn move_into_dir(src: &Path, dir: &Path, stem: &str) -> std::io::Result<PathBuf> {
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("");
    let dest = unique_destination(dir, stem, ext);
    fs::rename(src, &dest)?;
    Ok(dest)
}

fn file_stem_of(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("video")
        .to_string()
}

// Explorer's /select does not accept the \\?\ verbatim prefix canonicalize adds; network
// shares and mapped drives come back as \\?\UNC\server\share.
fn strip_verbatim_prefix(path: &str) -> String {
    if let Some(share) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{share}")
    } else {
        path.strip_prefix(r"\\?\").unwrap_or(path).to_string()
    }
}

fn display_path(path: &Path) -> String {
    let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    strip_verbatim_prefix(&resolved.to_string_lossy())
}

fn firefox_fork_cookie_sources() -> Vec<String> {
    let Some(roaming) = dirs::config_dir() else {
        return Vec::new();
    };
    let mut found: Vec<(SystemTime, PathBuf)> = Vec::new();
    for fork in ["librewolf", "zen", "Waterfox", "Floorp"] {
        let profiles_dir = roaming.join(fork).join("Profiles");
        let Ok(entries) = fs::read_dir(&profiles_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let profile = entry.path();
            if let Ok(meta) = fs::metadata(profile.join("cookies.sqlite")) {
                let mtime = meta.modified().unwrap_or(UNIX_EPOCH);
                found.push((mtime, profile));
            }
        }
    }
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found
        .into_iter()
        .map(|(_, profile)| format!("firefox:{}", profile.to_string_lossy()))
        .collect()
}

fn browser_cookie_sources() -> Vec<String> {
    let mut sources = vec!["firefox".to_string()];
    sources.extend(firefox_fork_cookie_sources());
    sources.extend(
        ["chrome", "brave", "opera", "edge"]
            .into_iter()
            .map(str::to_string),
    );
    sources
}

async fn download_with_cookie_fallback(
    app: &AppHandle,
    state: &AppState,
    request: &DownloadRequest,
    home_dir: &Path,
    temp_dir: &Path,
) -> Result<Saved, String> {
    log_info("Attempting download without browser cookies...");
    let first_err = match attempt_download(app, state, request, home_dir, temp_dir, None).await {
        Ok(done) => return Ok(done),
        Err(e) => e,
    };

    if state.is_cancelled()
        || first_err == CANCELLED
        || is_definitive_error(&first_err)
        || (request.sponsorblock && is_sponsorblock_error(&first_err))
    {
        return Err(first_err);
    }

    log_warn(&format!(
        "Download without browser cookies failed, trying browser cookies: {first_err}"
    ));

    for browser in browser_cookie_sources() {
        if state.is_cancelled() {
            return Err(CANCELLED.to_string());
        }
        log_info(&format!("Attempting download with {browser} cookies..."));
        report_progress(app, ProgressPayload::stage("retrying", String::new()));
        match attempt_download(app, state, request, home_dir, temp_dir, Some(&browser)).await {
            Ok(done) => return Ok(done),
            Err(e) if state.is_cancelled() || e == CANCELLED => return Err(CANCELLED.to_string()),
            Err(e) if should_try_next_browser(&e) => {
                log_warn(&format!(
                    "Browser {browser} failed with retryable cookie/403 error, trying next..."
                ));
            }
            Err(_) => break,
        }
    }

    Err(first_err)
}

/// Moves every file left in staging into `download_dir` under a free name and returns the
/// new path of `first`, or of whichever file made it across.
fn move_staged_files(staging_dir: &Path, download_dir: &Path, first: &Path) -> Option<PathBuf> {
    let mut leftovers: Vec<PathBuf> = fs::read_dir(staging_dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file())
                .collect()
        })
        .unwrap_or_default();
    leftovers.sort_by_key(|p| p != first);

    let mut saved = None;
    for leftover in leftovers {
        match move_into_dir(&leftover, download_dir, &file_stem_of(&leftover)) {
            Ok(dest) => {
                saved.get_or_insert(dest);
            }
            Err(e) => log_error(&format!(
                "Could not move {} into the download folder: {e}",
                leftover.display()
            )),
        }
    }
    saved
}

fn covers_whole_file(intervals: &[(f64, f64)], duration: Option<f64>) -> bool {
    match (intervals, duration) {
        ([(start, end)], Some(duration)) => *start <= 0.0 && *end >= duration - WHOLE_FILE_SLACK,
        _ => false,
    }
}

/// Cuts the requested range, minus any sponsor segments inside it, out of the staged
/// download. Section downloads are already cut at exact timestamps by yt-dlp
/// (`--force-keyframes-at-cuts`), so they are moved straight into place without a
/// second re-encode.
async fn finalize_trim(
    app: &AppHandle,
    state: &AppState,
    request: &DownloadRequest,
    mut done: Saved,
    staging_dir: &Path,
    download_dir: &Path,
) -> Result<Saved, String> {
    let start_sec = time_to_seconds(request.start_time.as_deref()).unwrap_or(0.0);
    let end_sec = time_to_seconds(request.end_time.as_deref());
    let source = done.path.clone();
    if !source.is_file() {
        return Err("Video indirilemedi.".to_string());
    }
    let cuts = done
        .cuts
        .iter()
        .find(|(path, _)| *path == source)
        .map(|(_, cuts)| cuts.clone())
        .unwrap_or_default();
    let media = ffmpeg::probe(&source).await;
    let duration = media.and_then(|m| m.duration);
    let is_section = section_download(request).is_some();

    let clip_end = end_sec.unwrap_or(f64::INFINITY).min(duration.unwrap_or(f64::INFINITY));
    let removed = if is_section {
        Removed::default()
    } else {
        sponsorblock::removed_within(&cuts, start_sec, clip_end)
    };
    let intervals = if is_section {
        vec![(0.0, end_sec.map_or(f64::INFINITY, |end| end - start_sec))]
    } else {
        let led = sponsorblock::lead_cuts(&cuts, media.map_or(0.0, |m| m.frame_seconds));
        sponsorblock::keep_intervals(start_sec, end_sec, duration, &led)
    };
    if intervals.is_empty() && removed.count > 0 {
        return Err(RANGE_REMOVED.to_string());
    }

    let clip = if is_section {
        Some(source.clone())
    } else {
        match media {
            Some(_) if covers_whole_file(&intervals, duration) => Some(source.clone()),
            Some(media) if !intervals.is_empty() => {
                match ffmpeg::cut_media(app, state, &source, &intervals, &media, "trimming").await {
                    Ok(clip) => Some(clip),
                    Err(()) if state.is_cancelled() => return Err(CANCELLED.to_string()),
                    Err(()) => None,
                }
            }
            _ => None,
        }
    };
    if let (Some(_), Some(item), Some(seconds)) = (
        &clip,
        done.item.as_mut(),
        ffmpeg::kept_seconds(&intervals, duration),
    ) {
        item.duration = seconds;
    }
    let stem = clip_file_stem(&file_stem_of(&source), start_sec, end_sec);
    place_clip(done, clip, staging_dir, download_dir, &stem, removed)
}

/// Moves the finished clip into `download_dir` under `stem`. Anything else left in staging
/// (the uncut source when there is no clip, extra videos of a multi-video post) is moved
/// over as-is.
fn place_clip(
    done: Saved,
    clip: Option<PathBuf>,
    staging_dir: &Path,
    download_dir: &Path,
    stem: &str,
    removed: Removed,
) -> Result<Saved, String> {
    let source = done.path.as_path();
    let mut saved = None;
    if let Some(clip) = clip {
        match move_into_dir(&clip, download_dir, stem) {
            Ok(dest) => {
                log_info(&format!("Clip saved to: {}", dest.display()));
                if clip != source {
                    let _ = fs::remove_file(source);
                }
                saved = Some(dest);
            }
            Err(e) => {
                log_error(&format!("Could not move the clip into place: {e}"));
                if clip != source {
                    let _ = fs::remove_file(&clip);
                }
            }
        }
    }

    let (warning, removed) = if saved.is_some() {
        (done.warning, removed)
    } else {
        (Some(TRIM_FAILED.to_string()), Removed::default())
    };
    let moved = move_staged_files(staging_dir, download_dir, source);
    saved
        .or(moved)
        .map(|path| Saved {
            path,
            warning,
            already_existed: false,
            item: done.item,
            removed,
            cuts: Vec::new(),
        })
        .ok_or_else(|| NOT_SAVED.to_string())
}

/// Re-encodes every staged file that has sponsor segments without them, in place of the
/// staged original, and returns the files it cut.
async fn cut_sponsors(
    app: &AppHandle,
    state: &AppState,
    done: &mut Saved,
    playlist: bool,
) -> Result<Vec<PathBuf>, String> {
    let mut cut_files = Vec::new();
    for (path, cuts) in std::mem::take(&mut done.cuts) {
        if state.is_cancelled() {
            return Err(CANCELLED.to_string());
        }
        if !path.is_file() {
            continue;
        }
        let Some(media) = ffmpeg::probe(&path).await else {
            done.warning.get_or_insert_with(|| SPONSOR_CUT_FAILED.to_string());
            continue;
        };
        let removed =
            sponsorblock::removed_within(&cuts, 0.0, media.duration.unwrap_or(f64::INFINITY));
        if removed.count == 0 {
            continue;
        }
        let led = sponsorblock::lead_cuts(&cuts, media.frame_seconds);
        let intervals = sponsorblock::keep_intervals(0.0, None, media.duration, &led);
        if intervals.is_empty() {
            if playlist {
                log_warn(&format!("Nothing but sponsor segments in {}", path.display()));
                continue;
            }
            return Err(NOTHING_LEFT.to_string());
        }
        match ffmpeg::cut_media(app, state, &path, &intervals, &media, "cutting").await {
            Ok(clip) => {
                let ext = clip.extension().and_then(|e| e.to_str()).unwrap_or("mp4");
                let dest = path.with_extension(ext);
                let replaced = fs::remove_file(&path).and_then(|()| fs::rename(&clip, &dest));
                let kept = match replaced {
                    Ok(()) => dest,
                    Err(e) => {
                        log_warn(&format!("Could not rename {}: {e}", clip.display()));
                        clip
                    }
                };
                if done.path == path {
                    done.path = kept.clone();
                }
                done.removed.count += removed.count;
                done.removed.seconds += removed.seconds;
                cut_files.push(kept);
            }
            Err(()) if state.is_cancelled() => return Err(CANCELLED.to_string()),
            Err(()) => {
                done.warning.get_or_insert_with(|| SPONSOR_CUT_FAILED.to_string());
            }
        }
    }
    Ok(cut_files)
}

async fn download_and_finalize(
    app: &AppHandle,
    state: &AppState,
    request: &DownloadRequest,
    download_dir: &Path,
    temp_dir: &Path,
) -> Result<Saved, String> {
    let first = download_once(app, state, request, download_dir, temp_dir).await;
    match first {
        Err(e) if request.sponsorblock && is_sponsorblock_error(&e) && !state.is_cancelled() => {
            log_warn(&format!("SponsorBlock step failed, downloading uncut: {e}"));
            let plain = DownloadRequest {
                sponsorblock: false,
                ..request.clone()
            };
            let mut saved = download_once(app, state, &plain, download_dir, temp_dir).await?;
            saved
                .warning
                .get_or_insert_with(|| SPONSORBLOCK_SKIPPED.to_string());
            Ok(saved)
        }
        other => other,
    }
}

struct Placement {
    path: PathBuf,
    existed: bool,
    renamed: bool,
}

fn same_size(a: &Path, b: &Path) -> bool {
    matches!((fs::metadata(a), fs::metadata(b)), (Ok(a), Ok(b)) if a.len() == b.len())
}

fn file_name_with(stem: &str, ext: &str) -> String {
    if ext.is_empty() {
        stem.to_string()
    } else {
        format!("{stem}.{ext}")
    }
}

/// Moves a staged sponsor-free download into `dest_dir` without replacing anything there.
/// A same-sized file under either of its possible names is the same download and is kept;
/// a different one (the uncut copy) is left alone and the new file gets its own name.
fn place_sponsor_free(src: &Path, dest_dir: &Path, cut: bool) -> std::io::Result<Placement> {
    fs::create_dir_all(dest_dir)?;
    let stem = file_stem_of(src);
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("");
    let plain = dest_dir.join(file_name_with(&stem, ext));
    if !plain.exists() {
        fs::rename(src, &plain)?;
        return Ok(Placement { path: plain, existed: false, renamed: false });
    }

    let own_stem = if cut { format!("{stem}{SPONSOR_FREE_SUFFIX}") } else { stem };
    let own = dest_dir.join(file_name_with(&own_stem, ext));
    for existing in [plain, own] {
        if existing.is_file() && same_size(src, &existing) {
            fs::remove_file(src)?;
            return Ok(Placement { path: existing, existed: true, renamed: false });
        }
    }
    let dest = unique_destination(dest_dir, &own_stem, ext);
    fs::rename(src, &dest)?;
    Ok(Placement { path: dest, existed: false, renamed: true })
}

fn place_sponsor_free_tree(
    staging_dir: &Path,
    dest_dir: &Path,
    cut_files: &[PathBuf],
    placed: &mut Vec<(PathBuf, Placement)>,
) {
    let mut entries: Vec<PathBuf> = fs::read_dir(staging_dir)
        .map(|entries| entries.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    entries.sort();
    for entry in entries {
        if entry.is_dir() {
            let sub = dest_dir.join(entry.file_name().unwrap_or_default());
            place_sponsor_free_tree(&entry, &sub, cut_files, placed);
            continue;
        }
        let cut = cut_files.iter().any(|file| same_path(file, &entry));
        match place_sponsor_free(&entry, dest_dir, cut) {
            Ok(placement) => placed.push((entry, placement)),
            Err(e) => log_error(&format!(
                "Could not move {} into the download folder: {e}",
                entry.display()
            )),
        }
    }
}

/// Sponsor-free downloads are staged in the temp directory and cut there, so a same-named
/// file the user already has in the download folder is never read or replaced.
async fn download_sponsor_free(
    app: &AppHandle,
    state: &AppState,
    request: &DownloadRequest,
    download_dir: &Path,
    temp_dir: &Path,
) -> Result<Saved, String> {
    let staging_dir = temp_dir.join(TRIM_SOURCE_DIR);
    let mut done =
        download_with_cookie_fallback(app, state, request, &staging_dir, temp_dir).await?;
    let cut_files = cut_sponsors(app, state, &mut done, request.download_playlist).await?;

    let mut placed = Vec::new();
    place_sponsor_free_tree(&staging_dir, download_dir, &cut_files, &mut placed);

    let renamed = placed.iter().any(|(_, placement)| placement.renamed);
    let warning = done
        .warning
        .or_else(|| renamed.then(|| SPONSOR_FREE_COPY_NOTE.to_string()));
    let own = placed.into_iter().find(|(src, _)| *src == done.path);
    let (path, already_existed) = match own {
        Some((_, placement)) => (placement.path, placement.existed),
        None => match done.path.strip_prefix(&staging_dir) {
            Ok(relative) if download_dir.join(relative).exists() => {
                (download_dir.join(relative), false)
            }
            _ => return Err(NOT_SAVED.to_string()),
        },
    };
    Ok(Saved {
        path,
        warning,
        already_existed,
        item: done.item,
        removed: if already_existed { Removed::default() } else { done.removed },
        cuts: Vec::new(),
    })
}

async fn download_once(
    app: &AppHandle,
    state: &AppState,
    request: &DownloadRequest,
    download_dir: &Path,
    temp_dir: &Path,
) -> Result<Saved, String> {
    let needs_trim = wants_trim(request);
    let save_copy = request.save_copy && !request.download_playlist;

    if !needs_trim && !save_copy {
        if request.sponsorblock {
            return download_sponsor_free(app, state, request, download_dir, temp_dir).await;
        }
        return download_with_cookie_fallback(app, state, request, download_dir, temp_dir).await;
    }

    // The source is staged inside the temp directory: with the download folder as home,
    // yt-dlp would reuse a same-named file the user already has there and the clip would be
    // cut from (or mistaken for) that file.
    let staging_dir = temp_dir.join(TRIM_SOURCE_DIR);
    let mut done =
        download_with_cookie_fallback(app, state, request, &staging_dir, temp_dir).await?;
    if needs_trim {
        return finalize_trim(app, state, request, done, &staging_dir, download_dir).await;
    }
    cut_sponsors(app, state, &mut done, false).await?;
    let (warning, item, removed) = (done.warning, done.item, done.removed);
    move_staged_files(&staging_dir, download_dir, &done.path)
        .map(|path| Saved {
            path,
            warning,
            already_existed: false,
            item,
            removed,
            cuts: Vec::new(),
        })
        .ok_or_else(|| NOT_SAVED.to_string())
}

pub async fn run_download(
    app: &AppHandle,
    state: &AppState,
    request: DownloadRequest,
    download_dir: String,
) -> Result<DownloadResult, String> {
    state.cancel_requested.store(false, Ordering::SeqCst);

    let download_dir = PathBuf::from(download_dir);
    remove_stale_temp_dirs(&download_dir);

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let temp_dir = download_dir.join(format!("{TEMP_DIR_PREFIX}{stamp}"));
    if let Err(e) = fs::create_dir_all(&temp_dir) {
        log_error(&format!(
            "Cannot write to download directory {}: {e}",
            download_dir.display()
        ));
        return Err("İndirme klasörüne yazılamıyor. Lütfen başka bir klasör seçin.".to_string());
    }
    hide_path(&temp_dir);
    *lock(&state.active_temp_dir) = Some(temp_dir.clone());
    kill_info_lookup(state).await;
    let _keep_awake = KeepAwake::new();

    log_info(&format!(
        "Received download request for URL: {} (Quality: {}, Audio: {}, Playlist: {})",
        request.url, request.quality, request.audio_only, request.download_playlist
    ));

    let outcome = download_and_finalize(app, state, &request, &download_dir, &temp_dir).await;

    let cleanup_target = temp_dir.clone();
    let _ = tokio::task::spawn_blocking(move || cleanup_temp_dir(&cleanup_target)).await;
    *lock(&state.active_temp_dir) = None;

    match outcome {
        Ok(saved) => {
            let is_dir = saved.path.is_dir();
            let full_path = display_path(&saved.path);
            log_info(&format!("Download successful. Saved to: {full_path}"));

            let filename = Path::new(&full_path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&full_path)
                .to_string();

            let item = saved.item.unwrap_or_default();
            let size = if is_dir {
                0
            } else {
                fs::metadata(&saved.path).map(|m| m.len()).unwrap_or(0)
            };
            // A playlist folder is described by its own name, not by its first video.
            let details = history::Details {
                title: if is_dir { String::new() } else { item.title },
                thumbnail: item.thumbnail,
                duration: if is_dir { 0.0 } else { item.duration },
                size,
                url: if is_dir { request.url.trim().to_string() } else { item.url },
                audio: request.audio_only,
            };

            {
                let mut entries = lock(&state.history);
                history::record_with(&mut entries, &full_path, &filename, is_dir, details.clone());
                history::save_history(&entries);
            }

            Ok(DownloadResult {
                status: "success".to_string(),
                filename,
                full_path,
                is_dir,
                already_existed: saved.already_existed,
                warning: saved.warning,
                removed: sponsorblock::removed_note(saved.removed),
                thumbnail: details.thumbnail,
                duration: details.duration,
                size,
            })
        }
        Err(e) => {
            if state.is_cancelled() || e == CANCELLED {
                log_info("Download was cancelled by user.");
                return Ok(DownloadResult {
                    status: "cancelled".to_string(),
                    ..DownloadResult::default()
                });
            }
            log_error(&format!("Download error: {e}"));
            let mut message = friendly_error(&e);
            if lock(&state.update_state).status == "available" && update_may_help(&e) {
                message = format!("{message} {UPDATE_HINT}");
            }
            Err(message)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(audio_only: bool, download_playlist: bool) -> DownloadRequest {
        DownloadRequest {
            url: "https://a.b/c".to_string(),
            quality: "720p".to_string(),
            audio_only,
            download_playlist,
            start_time: None,
            end_time: None,
            save_copy: false,
            sponsorblock: false,
            duration: None,
        }
    }

    #[test]
    fn youtube_ids() {
        let id = Some("dQw4w9WgXcQ".to_string());
        assert_eq!(extract_youtube_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=3"), id);
        assert_eq!(extract_youtube_id("https://youtu.be/dQw4w9WgXcQ?si=abc"), id);
        assert_eq!(extract_youtube_id("https://YouTube.com/shorts/dQw4w9WgXcQ"), id);
        assert_eq!(extract_youtube_id("https://www.youtube.com/watch?list=PL1&v=dQw4w9WgXcQ"), id);
        assert_eq!(extract_youtube_id("https://www.youtube.com/playlist?list=PL1"), None);
        assert_eq!(extract_youtube_id("https://vimeo.com/123"), None);
    }

    #[test]
    fn id_extraction_survives_non_ascii_prefix() {
        assert_eq!(
            extract_youtube_id("İİİ https://youtu.be/dQw4w9WgXcQ"),
            Some("dQw4w9WgXcQ".to_string())
        );
    }

    #[test]
    fn urls_cannot_be_options() {
        assert_eq!(clean_url("  https://a.b/c  "), Some("https://a.b/c"));
        assert_eq!(clean_url("--exec calc"), None);
        assert_eq!(clean_url("   "), None);
        assert_eq!(clean_url("https://a.b/\nc"), None);

        let req = request(false, false);
        let args = build_ytdlp_args(&req, "https://a.b/c", Path::new("D:/dl"), Path::new("D:/dl/tmp"), None, None, false);
        assert_eq!(&args[args.len() - 2..], ["--", "https://a.b/c"]);
        assert!(args.iter().any(|a| a == "--progress"));
        assert!(args.iter().any(|a| a.starts_with("res:720,")));
        assert!(!args.iter().any(|a| a.contains("height<=")));
        assert!(format_sort("best").starts_with("res,"));
        assert!(format_sort("4k").starts_with("res:2160,"));
    }

    #[test]
    fn error_lines_and_messages() {
        let stderr = "WARNING: x\nERROR: [generic] Unsupported URL: https://a.b\ntrailing noise\n";
        let line = last_error_line(stderr).unwrap();
        assert!(line.starts_with("ERROR:"));
        assert!(is_definitive_error(line));
        assert_eq!(friendly_error(line), "Bu bağlantı desteklenmiyor.");

        assert_eq!(last_error_line("a\nb\n\n"), Some("b"));
        assert_eq!(last_error_line(""), None);
        assert_eq!(friendly_error("ERROR: something odd"), "something odd");
        assert_eq!(friendly_error(""), "Video indirilemedi.");
        assert_eq!(
            friendly_error("ERROR: [x] 1: Odd failure (caused by <OddError 5: boom>)"),
            "[x] 1: Odd failure"
        );

        let missing = "ERROR: [generic] nope: Unable to download webpage: HTTP Error 404: File not found (caused by <HTTPError 404: File not found>)";
        assert!(is_definitive_error(missing));
        assert!(friendly_error(missing).contains("(404)"));

        let offline = "ERROR: [generic] x: Unable to download webpage: Failed to perform, curl: (6) Could not resolve host: a.b (caused by TransportError('...'))";
        assert!(is_definitive_error(offline));
        assert!(friendly_error(offline).starts_with("İnternet bağlantısı kurulamadı"));

        let forbidden = "ERROR: unable to download video data: HTTP Error 403: Forbidden";
        assert!(!is_definitive_error(forbidden));
        assert!(friendly_error(forbidden).contains("(403)"));
        assert!(!is_definitive_error("ERROR: Sign in to confirm your age"));
        assert_eq!(
            friendly_error("ERROR: [youtube] x: Requested format is not available. Use --list-formats"),
            "İstenen kalite bu video için mevcut değil."
        );
    }

    #[test]
    fn outdated_ytdlp_symptoms() {
        assert!(update_may_help("ERROR: [youtube] x: Unable to extract initial player response"));
        assert!(update_may_help("ERROR: unable to download video data: HTTP Error 403: Forbidden"));
        assert!(update_may_help("ERROR: [youtube] x: Requested format is not available"));
        assert!(!update_may_help("ERROR: [youtube] x: Private video. Sign in if you've been granted access"));
        assert!(!update_may_help("ERROR: [generic] Unsupported URL: https://a.b"));
        assert!(!update_may_help("ERROR: x: Failed to resolve 'a.b' (caused by TransportError)"));
        assert!(!update_may_help(NOTHING_DOWNLOADED));
        assert!(!update_may_help("yt-dlp başlatılamadı: missing"));

        assert!(is_definitive_error(NOTHING_DOWNLOADED));
        assert_eq!(friendly_error(NOTHING_DOWNLOADED), NOTHING_DOWNLOADED);
    }

    #[test]
    fn info_lookup_follows_the_playlist_switch() {
        let url = "https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=PL1";
        assert!(info_args(url, false).contains(&"--no-playlist"));
        assert!(!info_args(url, false).contains(&"--yes-playlist"));
        assert!(info_args(url, true).contains(&"--yes-playlist"));
        assert_eq!(info_args(url, true).last(), Some(&url));

        assert!(has_playlist_param(url));
        assert!(has_playlist_param("https://www.youtube.com/playlist?list=PL1"));
        assert!(has_playlist_param("https://youtu.be/dQw4w9WgXcQ?si=a&list=PL1#t=3"));
        assert!(!has_playlist_param("https://www.youtube.com/watch?v=dQw4w9WgXcQ&list="));
        assert!(!has_playlist_param("https://www.youtube.com/watch?v=dQw4w9WgXcQ&playlist=PL1"));
        assert!(!has_playlist_param("https://youtu.be/dQw4w9WgXcQ"));
    }

    #[test]
    fn info_output_parsing() {
        let info = parse_info_output("12.5\nhttps://i.example/t.jpg\nNA\nNA\nBaşlık 日本\n").unwrap();
        assert_eq!(info.duration, 12.5);
        assert_eq!(info.title, "Başlık 日本");
        assert_eq!(info.thumbnail.as_deref(), Some("https://i.example/t.jpg"));
        assert_eq!(info.playlist_title, None);
        assert_eq!(info.playlist_count, None);

        let info = parse_info_output("NA\nNA\nNA\n\"NA\"\nNA\n").unwrap();
        assert_eq!(info.duration, 0.0);
        assert_eq!(info.thumbnail, None);
        assert_eq!(info.title, "Bilinmeyen Başlık");
        assert_eq!(info.playlist_title, None);

        let info = parse_info_output("61\nNA\n42\n\"Liste \\\"A\\\"\"\nİlk video\n").unwrap();
        assert_eq!(info.playlist_title.as_deref(), Some("Liste \"A\""));
        assert_eq!(info.playlist_count, Some(42));
        assert_eq!(info.title, "İlk video");

        assert!(parse_info_output("").is_none());
    }

    #[test]
    fn progress_lines() {
        let parse = |line: &str| parse_progress_line(line, &mut ProgressTracker::default());

        let p = parse(" 42.0%| 1.5MiB/s|1048576|10485760|2|7|00:12").unwrap();
        assert_eq!(p.percent, "42.0%");
        assert_eq!(p.overall, "20.3%");
        assert_eq!(p.speed, "1.5MiB/s");
        assert_eq!(p.size_info, "1.0MB / 10.0MB");
        assert_eq!(p.playlist_info, "2 / 7");
        assert_eq!(p.eta, "00:12");
        assert_eq!(taskbar_value(&p), 20);

        let p = parse("  0.0%|Unknown B/s|512|NA|NA|NA|Unknown").unwrap();
        assert_eq!(p.size_info, "512.0B");
        assert_eq!(p.playlist_info, "");
        assert_eq!(p.overall, "");
        assert_eq!(p.eta, "");

        assert_eq!(parse("Unknown%|NA|0|NA|NA|NA|NA").unwrap().percent, "Unknown%");
        assert_eq!(parse("50.0%|NA|5|10|1|1|NA").unwrap().overall, "");

        let titled = parse("1.0%|NA|1|10|NA|NA|NA|avc1|mp4a|100|A | B").unwrap();
        assert_eq!(titled.title, "A|B");
        assert_eq!(parse("1.0%|NA|1|10|NA|NA|NA|avc1|mp4a|100|NA").unwrap().title, "");
        assert!(parse("garbage").is_none());
    }

    #[test]
    fn video_and_audio_passes_share_one_run() {
        let mut tracker = ProgressTracker::default();
        let mut percent = |line: &str| {
            let p = parse_progress_line(line, &mut tracker).unwrap();
            parse_percent(&p.percent).unwrap()
        };

        let half_video = percent("50.0%|NA|4000000|8000000|NA|NA|NA|avc1.64001f|none|100");
        assert!((half_video - 41.7).abs() < 0.1);
        let video_done = percent("100.0%|NA|8000000|8000000|NA|NA|NA|avc1.64001f|none|100");
        assert!((video_done - 83.3).abs() < 0.1);

        let audio_start = percent("0.0%|NA|0|2000000|NA|NA|NA|none|mp4a.40.2|100");
        assert!(audio_start >= video_done);
        let half_audio = percent("50.0%|NA|1000000|2000000|NA|NA|NA|none|mp4a.40.2|100");
        assert!((half_audio - 90.0).abs() < 0.1);
        assert_eq!(percent("100.0%|NA|2000000|2000000|NA|NA|NA|none|mp4a.40.2|100"), 100.0);

        assert_eq!(percent("10.0%|NA|1|10|2|5|NA|none|mp4a.40.2|100"), 10.0);
        assert_eq!(percent("30.0%|NA|3|10|3|5|NA|avc1|mp4a|100"), 30.0);
    }

    #[test]
    fn output_names_are_length_capped() {
        let mut req = request(false, false);
        let template = |req: &DownloadRequest| {
            let args = build_ytdlp_args(req, "https://a.b/c", Path::new("D:/dl"), Path::new("D:/dl/tmp"), None, None, false);
            let at = args.iter().position(|a| a == "-o").unwrap();
            args[at + 1].clone()
        };
        assert_eq!(template(&req), "%(title).120B [%(id)s].%(ext)s");
        req.download_playlist = true;
        assert_eq!(
            template(&req),
            "%(playlist_title|.).60B/%(playlist_index)s - %(title).100B.%(ext)s"
        );
    }

    #[test]
    fn clip_names() {
        assert_eq!(clip_file_stem("a [id]", 90.0, Some(165.4)), "a [id] (01.30-02.45)");
        assert_eq!(clip_file_stem("a", 3690.0, None), "a (1.01.30-son)");

        let dir = std::env::temp_dir().join(format!("videoindiren_test_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(unique_destination(&dir, "clip", "mp4"), dir.join("clip.mp4"));
        fs::write(dir.join("clip.mp4"), b"x").unwrap();
        assert_eq!(unique_destination(&dir, "clip", "mp4"), dir.join("clip (2).mp4"));

        fs::write(dir.join("src.mp4"), b"y").unwrap();
        let moved = move_into_dir(&dir.join("src.mp4"), &dir, "clip").unwrap();
        assert_eq!(moved, dir.join("clip (2).mp4"));
        assert_eq!(fs::read(dir.join("clip.mp4")).unwrap(), b"x");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn staged_copies_never_replace_existing_files() {
        let dir = std::env::temp_dir().join(format!("videoindiren_copy_{}", std::process::id()));
        let staging = dir.join("tmp").join(TRIM_SOURCE_DIR);
        fs::create_dir_all(&staging).unwrap();
        fs::write(dir.join("v [id].mp4"), b"old").unwrap();
        fs::write(staging.join("v [id].mp4"), b"new").unwrap();
        fs::write(staging.join("extra.mp4"), b"extra").unwrap();

        let saved = move_staged_files(&staging, &dir, &staging.join("v [id].mp4"));
        assert_eq!(saved, Some(dir.join("v [id] (2).mp4")));
        assert_eq!(fs::read(dir.join("v [id].mp4")).unwrap(), b"old");
        assert_eq!(fs::read(dir.join("v [id] (2).mp4")).unwrap(), b"new");
        assert!(dir.join("extra.mp4").is_file());
        assert_eq!(move_staged_files(&staging, &dir, &staging.join("v [id].mp4")), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn sponsor_removal_never_touches_existing_files() {
        let dir = std::env::temp_dir().join(format!("videoindiren_sb_{}", std::process::id()));
        let staging = dir.join("tmp").join(TRIM_SOURCE_DIR);
        let stage = |name: &str, data: &[u8]| {
            fs::create_dir_all(staging.join("List")).unwrap();
            fs::write(staging.join(name), data).unwrap();
            staging.join(name)
        };
        fs::create_dir_all(dir.join("List")).unwrap();
        fs::write(dir.join("v [id].mp4"), b"uncut-original").unwrap();
        fs::write(dir.join("List").join("01 - a.mp4"), b"uncut-original").unwrap();

        let placed = place_sponsor_free(&stage("v [id].mp4", b"cut"), &dir, true).unwrap();
        assert_eq!(placed.path, dir.join("v [id] (sponsorsuz).mp4"));
        assert!(placed.renamed && !placed.existed);
        assert_eq!(fs::read(dir.join("v [id].mp4")).unwrap(), b"uncut-original");

        let again = place_sponsor_free(&stage("v [id].mp4", b"cut"), &dir, true).unwrap();
        assert_eq!(again.path, dir.join("v [id] (sponsorsuz).mp4"));
        assert!(again.existed && !again.renamed);
        assert!(!staging.join("v [id].mp4").exists());

        let same = place_sponsor_free(&stage("v [id].mp4", b"uncut-original"), &dir, true).unwrap();
        assert_eq!(same.path, dir.join("v [id].mp4"));
        assert!(same.existed);

        let uncut = place_sponsor_free(&stage("v [id].mp4", b"other"), &dir, false).unwrap();
        assert_eq!(uncut.path, dir.join("v [id] (2).mp4"));

        let fresh = place_sponsor_free(&stage("new.mp4", b"n"), &dir, true).unwrap();
        assert_eq!(fresh.path, dir.join("new.mp4"));
        assert!(!fresh.existed && !fresh.renamed);

        stage("List/01 - a.mp4", b"cut");
        stage("List/02 - b.mp4", b"cut");
        let mut placed = Vec::new();
        let cut_files = [staging.join("List").join("01 - a.mp4")];
        place_sponsor_free_tree(&staging, &dir, &cut_files, &mut placed);
        assert_eq!(placed.len(), 2);
        assert_eq!(fs::read(dir.join("List").join("01 - a.mp4")).unwrap(), b"uncut-original");
        assert!(dir.join("List").join("01 - a (sponsorsuz).mp4").is_file());
        assert!(dir.join("List").join("02 - b.mp4").is_file());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn playlist_results() {
        let home = std::env::temp_dir().join(format!("videoindiren_pl_{}", std::process::id()));
        let list = home.join("My List");
        fs::create_dir_all(&list).unwrap();

        fs::write(list.join("01 - a.mp4"), b"x").unwrap();
        assert_eq!(playlist_result_path(&list.join("01 - a.mp4"), &home), list);

        fs::write(home.join("NA - solo.mp4"), b"x").unwrap();
        assert_eq!(
            playlist_result_path(&home.join("NA - solo.mp4"), &home),
            home.join("solo.mp4")
        );
        assert!(home.join("solo.mp4").is_file());
        assert!(!home.join("NA - solo.mp4").exists());

        fs::write(home.join("NA - solo.mp4"), b"y").unwrap();
        assert_eq!(
            playlist_result_path(&home.join("NA - solo.mp4"), &home),
            home.join("solo (2).mp4")
        );
        assert_eq!(fs::read(home.join("solo.mp4")).unwrap(), b"x");

        let req = request(true, true);
        let args = build_ytdlp_args(&req, "https://a.b/c", &home, &home.join("tmp"), None, None, true);
        assert!(args.iter().any(|a| a.starts_with("%(playlist_title|.).60B/")));
        assert!(args.iter().any(|a| a == "--embed-metadata"));
        assert!(args.iter().any(|a| a == "--embed-thumbnail"));
        let cover_dir = format!(
            "thumbnail:{}",
            home.join("tmp").join(THUMBNAIL_DIR).to_string_lossy()
        );
        assert!(args.contains(&cover_dir));
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn audio_extraction_spares_files_the_user_already_had() {
        let home = std::env::temp_dir().join(format!("videoindiren_keep_{}", std::process::id()));
        let temp = home.join(".tmp");
        fs::create_dir_all(home.join("List")).unwrap();
        fs::create_dir_all(&temp).unwrap();
        for name in ["fresh.m4a", "mine.mp4", "direct.mp3", "List/1 - a.webm"] {
            fs::write(home.join(name), b"x").unwrap();
        }
        let path = |p: PathBuf| p.to_string_lossy().into_owned();
        let sources = [
            path(temp.join("fresh.m4a")),
            path(home.join("mine.mp4")),
            path(temp.join("direct.mp3")),
            path(temp.join("List").join("1 - a.webm")),
            path(temp.join("never-moved.m4a")),
        ];

        remove_kept_sources(&sources, &home, &temp);

        assert!(!home.join("fresh.m4a").exists());
        assert!(!home.join("List/1 - a.webm").exists());
        assert!(home.join("mine.mp4").is_file());
        assert!(home.join("direct.mp3").is_file());

        let has_keep = |audio_only, keep| {
            build_ytdlp_args(&request(audio_only, false), "https://a.b/c", &home, &temp, None, None, keep)
                .iter()
                .any(|a| a == "--keep-video")
        };
        assert!(has_keep(true, true));
        assert!(!has_keep(true, false));
        assert!(!has_keep(false, true));
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn hopeless_errors_skip_the_cookie_retries() {
        assert!(is_definitive_error("ERROR: [youtube] x: Requested format is not available"));
        assert!(is_definitive_error("ERROR: This video has been removed by the uploader"));
        assert!(is_definitive_error(
            "ERROR: Postprocessing: Unable to determine video duration: ffprobe not found."
        ));
        assert!(!is_definitive_error("ERROR: Private video. Sign in if you've been granted access"));
    }

    #[test]
    fn taskbar_values() {
        let payload = |percent: &str, status: &str| ProgressPayload::stage(status, percent.to_string());
        assert_eq!(taskbar_value(&payload(" 42.7%", "downloading")), 42);
        assert_eq!(taskbar_value(&payload("NA", "downloading")), TASKBAR_INDETERMINATE);
        assert_eq!(taskbar_value(&payload("100%", "merging")), TASKBAR_INDETERMINATE);
        assert_eq!(taskbar_value(&payload("", "trimming")), TASKBAR_INDETERMINATE);
    }

    #[test]
    fn verbatim_prefixes_are_removed() {
        assert_eq!(strip_verbatim_prefix(r"\\?\D:\dl\a.mp4"), r"D:\dl\a.mp4");
        assert_eq!(strip_verbatim_prefix(r"\\?\UNC\nas\media\a.mp4"), r"\\nas\media\a.mp4");
        assert_eq!(strip_verbatim_prefix(r"D:\dl\a.mp4"), r"D:\dl\a.mp4");
    }

    #[test]
    fn byte_formatting() {
        assert_eq!(format_bytes(0.0), "0.0B");
        assert_eq!(format_bytes(1536.0), "1.5KB");
        assert_eq!(format_bytes(5.0 * 1024.0 * 1024.0), "5.0MB");
    }

    #[test]
    fn item_lines() {
        let item = parse_item_line(
            r#"["https://i.x/t.jpg?a=1|2",19,"https://a.b/watch?v=1","Me | at the zoo"]"#,
        )
        .unwrap();
        assert_eq!(item.thumbnail, "https://i.x/t.jpg?a=1|2");
        assert_eq!(item.duration, 19.0);
        assert_eq!(item.url, "https://a.b/watch?v=1");
        assert_eq!(item.title, "Me | at the zoo");

        let bare = parse_item_line(r#"["http://i.x/t.jpg",null,"javascript:alert(1)",null]"#).unwrap();
        assert_eq!(bare, ItemPayload::default());
        assert_eq!(parse_item_line("[null]").unwrap(), ItemPayload::default());
        assert!(parse_item_line("NA").is_none());
    }

    #[test]
    fn sponsor_segments_are_only_marked_by_ytdlp() {
        let args = |req: &DownloadRequest| {
            build_ytdlp_args(req, "https://a.b/c", Path::new("D:/dl"), Path::new("D:/dl/tmp"), None, None, false)
        };
        let has = |req: &DownloadRequest| args(req).iter().any(|a| a == "--sponsorblock-mark");

        let mut req = request(false, false);
        assert!(!has(&req));
        assert!(args(&req).iter().any(|a| a == ITEM_TEMPLATE));
        req.sponsorblock = true;
        assert!(has(&req));
        assert!(args(&req).iter().any(|a| a == "--no-embed-chapters"));
        assert!(args(&req).iter().any(|a| a == "after_move:SPONSORS|%(sponsorblock_chapters|null)j"));
        req.start_time = Some("00:00:10".to_string());
        req.duration = Some(7200.0);
        assert!(has(&req));
        assert!(!args(&req).iter().any(|a| a == "--download-sections"));
        req.download_playlist = true;
        assert!(has(&req));
        assert!(!args(&req).iter().any(|a| a == "--sponsorblock-remove"));

        assert!(is_sponsorblock_error("ERROR: Postprocessing: Unable to communicate with SponsorBlock API: timed out"));
        assert!(!is_sponsorblock_error("ERROR: Video unavailable"));
        for message in [RANGE_REMOVED, NOTHING_LEFT] {
            assert!(!is_sponsorblock_error(message));
            assert_eq!(friendly_error(message), message);
            assert!(!update_may_help(message));
        }

        let unreachable = "ERROR: Preprocessing: Unable to communicate with SponsorBlock API: refused";
        assert!(only_sponsorblock_errors(&format!("{unreachable}\n{unreachable}")));
        assert!(!only_sponsorblock_errors(&format!("{unreachable}\nERROR: Video unavailable")));
        assert!(!only_sponsorblock_errors("WARNING: SponsorBlock is slow"));
        assert!(!only_sponsorblock_errors(""));
    }

    #[test]
    fn a_range_spanning_the_whole_file_needs_no_encode() {
        assert!(covers_whole_file(&[(0.0, 750.4)], Some(750.4)));
        assert!(covers_whole_file(&[(0.0, 750.35)], Some(750.4)));
        assert!(!covers_whole_file(&[(0.0, 700.0)], Some(750.4)));
        assert!(!covers_whole_file(&[(3.0, 750.4)], Some(750.4)));
        assert!(!covers_whole_file(&[(0.0, 69.17), (112.5, 750.4)], Some(750.4)));
        assert!(!covers_whole_file(&[(0.0, f64::INFINITY)], None));
        assert!(!covers_whole_file(&[], Some(750.4)));
    }

    #[test]
    fn only_the_clip_is_fetched_from_long_videos() {
        let clip = |start: Option<&str>, end: Option<&str>, duration: Option<f64>| DownloadRequest {
            start_time: start.map(str::to_string),
            end_time: end.map(str::to_string),
            duration,
            ..request(false, false)
        };
        let section = |start, end, duration| section_download(&clip(start, end, duration));
        let args = |req: &DownloadRequest| {
            build_ytdlp_args(req, "https://a.b/c", Path::new("D:/dl"), Path::new("D:/dl/tmp"), None, None, false)
        };
        let hour = Some(3600.0);

        let req = clip(Some("00:01:30"), Some("00:02:45.5"), hour);
        assert_eq!(section_download(&req), Some((90.0, Some(165.5))));
        let built = args(&req);
        let at = built.iter().position(|a| a == "--download-sections").unwrap();
        assert_eq!(built[at + 1], "*90-165.5");
        assert!(built.iter().any(|a| a == "--force-keyframes-at-cuts"));
        assert_eq!(&built[built.len() - 2..], ["--", "https://a.b/c"]);

        assert_eq!(section(None, Some("00:01:59"), hour), Some((0.0, Some(119.0))));
        assert_eq!(section(None, Some("00:02:00"), hour), None);
        assert_eq!(section(None, Some("00:10:00"), hour), None);
        assert_eq!(section(Some("00:58:30"), None, hour), Some((3510.0, None)));
        assert!(args(&clip(Some("00:58:30"), None, hour)).contains(&"*3510-inf".to_string()));
        assert_eq!(section(Some("00:50:00"), None, hour), None);
        assert_eq!(section(Some("00:59:00"), Some("02:00:00"), hour), Some((3540.0, Some(7200.0))));

        assert_eq!(section(Some("00:01:00"), Some("00:01:30"), Some(1200.0)), None);
        assert_eq!(section(Some("00:01:00"), Some("00:01:30"), Some(1201.0)), Some((60.0, Some(90.0))));
        assert_eq!(section(Some("00:01:00"), Some("00:02:00"), None), None);
        assert_eq!(section(Some("00:01:00"), Some("00:02:00"), Some(0.0)), None);
        assert_eq!(section(Some("00:01:00"), Some("00:02:00"), Some(f64::NAN)), None);
        assert_eq!(section(Some("00:02:00"), Some("00:01:00"), hour), None);
        assert_eq!(section(None, None, hour), None);
        assert!(!args(&clip(None, None, hour)).iter().any(|a| a == "--download-sections"));

        let with = |change: fn(&mut DownloadRequest)| {
            let mut req = clip(Some("00:01:30"), Some("00:02:45"), hour);
            change(&mut req);
            section_download(&req)
        };
        assert!(with(|_| {}).is_some());
        assert_eq!(with(|req| req.sponsorblock = true), None);
        assert_eq!(with(|req| req.download_playlist = true), None);
        assert_eq!(with(|req| req.audio_only = true), None);
    }

    #[test]
    fn clips_are_named_after_their_range_and_replace_nothing() {
        let dir = std::env::temp_dir().join(format!("videoindiren_section_{}", std::process::id()));
        let staging = dir.join("tmp").join(TRIM_SOURCE_DIR);
        fs::create_dir_all(&staging).unwrap();
        fs::write(dir.join("v [id].mp4"), b"full").unwrap();
        fs::write(dir.join("v [id] (01.30-02.45).mp4"), b"old clip").unwrap();
        let stage = |clip: Option<&[u8]>| {
            fs::write(staging.join("v [id].mp4"), b"source").unwrap();
            if let Some(clip) = clip {
                fs::write(staging.join("v [id]_cut.mp4"), clip).unwrap();
            }
            Saved {
                path: staging.join("v [id].mp4"),
                warning: None,
                already_existed: false,
                item: None,
                removed: Removed::default(),
                cuts: Vec::new(),
            }
        };
        let stem = clip_file_stem("v [id]", 90.0, Some(165.0));
        let lost = Removed { count: 1, seconds: 12.0 };

        let done = stage(Some(b"clip"));
        let clip = Some(staging.join("v [id]_cut.mp4"));
        let saved = place_clip(done, clip, &staging, &dir, &stem, lost).unwrap();
        assert_eq!(saved.path, dir.join("v [id] (01.30-02.45) (2).mp4"));
        assert!(!saved.already_existed);
        assert_eq!(saved.removed, lost);
        assert_eq!(saved.warning, None);
        assert_eq!(fs::read(&saved.path).unwrap(), b"clip");
        assert_eq!(fs::read(dir.join("v [id].mp4")).unwrap(), b"full");
        assert_eq!(fs::read(dir.join("v [id] (01.30-02.45).mp4")).unwrap(), b"old clip");
        assert!(!staging.join("v [id].mp4").exists());
        assert!(!dir.join("v [id] (2).mp4").exists());

        let done = stage(None);
        let whole = Some(staging.join("v [id].mp4"));
        let saved = place_clip(done, whole, &staging, &dir, &stem, Removed::default()).unwrap();
        assert_eq!(saved.path, dir.join("v [id] (01.30-02.45) (3).mp4"));
        assert_eq!(fs::read(&saved.path).unwrap(), b"source");

        let saved = place_clip(stage(None), None, &staging, &dir, &stem, lost).unwrap();
        assert_eq!(saved.path, dir.join("v [id] (2).mp4"));
        assert_eq!(saved.warning.as_deref(), Some(TRIM_FAILED));
        assert_eq!(saved.removed, Removed::default());
        assert_eq!(fs::read(dir.join("v [id].mp4")).unwrap(), b"full");
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn lossy_lines_tolerate_invalid_utf8() {
        let data: &[u8] = b"ok\r\nbad \xfc line\nlast";
        let mut reader = BufReader::new(data);
        let mut buf = Vec::new();
        assert_eq!(next_line_lossy(&mut reader, &mut buf).await.as_deref(), Some("ok"));
        assert!(next_line_lossy(&mut reader, &mut buf).await.unwrap().starts_with("bad "));
        assert_eq!(next_line_lossy(&mut reader, &mut buf).await.as_deref(), Some("last"));
        assert_eq!(next_line_lossy(&mut reader, &mut buf).await, None);
    }
}
