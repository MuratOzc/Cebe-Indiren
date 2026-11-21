#![cfg_attr(not(any(windows, target_os = "android")), allow(dead_code))]

use crate::config::{app_data_dir, log_info, log_warn, write_json};
use crate::state::{lock, AppState, AppUpdateState};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
#[cfg(not(target_os = "android"))]
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
#[cfg(not(target_os = "android"))]
const API_LATEST_URL: &str =
    "https://api.github.com/repos/anilyildiz8/Video-Indiren/releases/latest";
#[cfg(not(target_os = "android"))]
const ALLOWED_URL_PREFIX: &str =
    "https://github.com/anilyildiz8/Video-Indiren/releases/download/";
#[cfg(not(target_os = "android"))]
const ASSET_NAME: &str = "VideoIndiren.exe";
#[cfg(not(target_os = "android"))]
const ASSET_MAGIC: &[u8; 2] = b"MZ";
#[cfg(target_os = "android")]
const API_LATEST_URL: &str =
    "https://api.github.com/repos/MuratOzc/Cebe-Indiren/releases/latest";
#[cfg(target_os = "android")]
const ALLOWED_URL_PREFIX: &str =
    "https://github.com/MuratOzc/Cebe-Indiren/releases/download/";
#[cfg(target_os = "android")]
const ASSET_NAME: &str = "CebeIndiren.apk";
#[cfg(target_os = "android")]
const ASSET_MAGIC: &[u8; 2] = b"PK";
const USER_AGENT: &str = "VideoIndiren-updater";
const MIN_EXE_BYTES: u64 = 1_000_000;

fn append_ext(path: &Path, suffix: &str) -> PathBuf {
    let mut os = path.as_os_str().to_os_string();
    os.push(suffix);
    PathBuf::from(os)
}

fn parse_semver(raw: &str) -> Option<(u64, u64, u64)> {
    let mut parts = raw
        .trim()
        .trim_start_matches(['v', 'V'])
        .split('.')
        .map(|part| part.parse::<u64>().ok());
    let version = (parts.next()??, parts.next()??, parts.next().unwrap_or(Some(0))?);
    parts.next().is_none().then_some(version)
}

// Upstream tags releases as "rust_vN"; the semantic version lives in the release name.
fn release_version<'a>(tag: &'a str, name: &'a str) -> Option<(&'a str, (u64, u64, u64))> {
    [tag, name]
        .into_iter()
        .find_map(|raw| parse_semver(raw).map(|version| (raw, version)))
}

fn parse_sha256_digest(raw: &str) -> Option<String> {
    let hex = raw.trim().strip_prefix("sha256:")?;
    (hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit())).then(|| hex.to_lowercase())
}

#[derive(Debug, Serialize, Deserialize)]
struct PendingNotes {
    version: String,
    notes: String,
}

fn pending_notes_file() -> PathBuf {
    app_data_dir().join("pending_notes.json")
}

fn take_pending_notes() -> Option<String> {
    let path = pending_notes_file();
    let content = fs::read_to_string(&path).ok()?;
    let _ = fs::remove_file(&path);
    let pending: PendingNotes = serde_json::from_str(&content).ok()?;
    let current = format!("v{APP_VERSION}");
    let notes = pending.notes.trim().to_string();
    (parse_semver(&pending.version) == parse_semver(&current) && !notes.is_empty()).then_some(notes)
}

fn save_pending_notes(version: &str, notes: &str) {
    let trimmed = notes.trim();
    if trimmed.is_empty() {
        return;
    }
    write_json(
        &pending_notes_file(),
        &PendingNotes {
            version: version.to_string(),
            notes: trimmed.to_string(),
        },
    );
}

/// Removes the leftover `.old` executable from a previous self-update.
/// Returns `true` when an `.old` binary was present, indicating this launch followed an update.
pub fn cleanup_old_exe() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let old_exe = append_ext(&exe, ".old");
    if !old_exe.exists() {
        return false;
    }

    tauri::async_runtime::spawn(async move {
        for _ in 0..5 {
            if fs::remove_file(&old_exe).is_ok() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        log_warn(&format!("Could not remove old binary: {}", old_exe.display()));
    });
    true
}

async fn fetch_latest_release(just_updated: bool) -> Result<AppUpdateState, String> {
    let release: serde_json::Value = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| e.to_string())?
        .get(API_LATEST_URL)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .and_then(|resp| resp.error_for_status())
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;

    let tag = release["tag_name"].as_str().unwrap_or_default().trim();
    let name = release["name"].as_str().unwrap_or_default().trim();
    let (label, latest) = release_version(tag, name)
        .ok_or_else(|| format!("Unexpected release version (tag={tag:?}, name={name:?})"))?;
    let release_notes = release["body"]
        .as_str()
        .unwrap_or_default()
        .trim()
        .to_string();

    let asset = release["assets"].as_array().and_then(|assets| {
        assets.iter().find(|asset| {
            asset["name"]
                .as_str()
                .is_some_and(|name| name.eq_ignore_ascii_case(ASSET_NAME))
        })
    });
    let download_url = asset
        .and_then(|asset| asset["browser_download_url"].as_str())
        .unwrap_or_default()
        .to_string();
    let sha256 = asset
        .and_then(|asset| asset["digest"].as_str())
        .and_then(parse_sha256_digest);

    let current_ver = parse_semver(APP_VERSION);
    let newer = current_ver.is_some_and(|current| latest > current) && !download_url.is_empty();
    if newer && sha256.is_none() {
        log_warn("Newer app release has no SHA-256 digest on GitHub; not offering it.");
    }
    let newer = newer && sha256.is_some();
    let patch_notes =
        (just_updated && current_ver == Some(latest) && !release_notes.is_empty())
            .then(|| release_notes.clone());
    Ok(AppUpdateState {
        status: if newer { "available" } else { "up_to_date" }.to_string(),
        current: format!("v{APP_VERSION}"),
        latest: format!("v{}", label.trim_start_matches(['v', 'V'])),
        download_url,
        sha256,
        release_notes,
        patch_notes,
    })
}

pub fn start_app_update_check(state: Arc<AppState>, just_updated: bool) {
    if let Some(notes) = take_pending_notes() {
        lock(&state.app_update_state).patch_notes = Some(notes);
    }

    tauri::async_runtime::spawn(async move {
        let mut result = AppUpdateState {
            status: "failed".to_string(),
            current: format!("v{APP_VERSION}"),
            ..AppUpdateState::default()
        };
        for attempt in 1..=3u64 {
            match fetch_latest_release(just_updated).await {
                Ok(found) => {
                    log_info(&format!(
                        "App v{APP_VERSION} active, latest {} ({}).",
                        found.latest, found.status
                    ));
                    result = found;
                    break;
                }
                Err(e) => log_warn(&format!("App update check failed (attempt {attempt}): {e}")),
            }
            if attempt < 3 {
                tokio::time::sleep(Duration::from_secs(attempt)).await;
            }
        }
        let mut guard = lock(&state.app_update_state);
        if result.patch_notes.is_none() {
            result.patch_notes = guard.patch_notes.clone();
        }
        *guard = result;
    });
}

async fn download_exe(
    app: &AppHandle,
    download_url: &str,
    expected_sha256: &str,
    dest: &Path,
) -> Result<(), String> {
    let mut file = File::create(dest).map_err(|e| {
        if e.kind() == ErrorKind::PermissionDenied {
            "Güncelleme için yazma izni yok. Uygulamayı Masaüstü veya İndirilenler klasörüne taşıyın."
                .to_string()
        } else {
            format!("Geçici dosya oluşturulamadı: {e}")
        }
    })?;

    let mut resp = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(30))
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| e.to_string())?
        .get(download_url)
        .send()
        .await
        .and_then(|resp| resp.error_for_status())
        .map_err(|e| format!("Güncelleme indirilemedi: {e}"))?;

    let total = resp.content_length().unwrap_or(0);
    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;
    let mut last_percent = None;

    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| format!("İndirme kesildi: {e}"))?
    {
        file.write_all(&chunk)
            .map_err(|e| format!("Dosya yazılamadı: {e}"))?;
        hasher.update(&chunk);
        downloaded += chunk.len() as u64;

        let percent = (total > 0).then(|| (downloaded.saturating_mul(100) / total).min(100));
        if percent.is_some() && percent != last_percent {
            last_percent = percent;
            let _ = app.emit("app-update-progress", percent);
        }
    }
    drop(file);

    let mut magic = [0u8; 2];
    let is_exe = File::open(dest).and_then(|mut f| f.read_exact(&mut magic)).is_ok() && &magic == ASSET_MAGIC;
    if downloaded < MIN_EXE_BYTES || !is_exe {
        return Err("İndirilen dosya geçerli bir uygulama değil.".to_string());
    }
    if format!("{:x}", hasher.finalize()) != expected_sha256 {
        return Err("SHA-256 doğrulaması başarısız.".to_string());
    }
    Ok(())
}

#[cfg(target_os = "android")]
pub async fn install_app_update(
    app: &AppHandle,
    latest: &str,
    download_url: &str,
    expected_sha256: Option<&str>,
    release_notes: &str,
) -> Result<(), String> {
    if cfg!(debug_assertions) {
        return Err("Geliştirme modunda uygulama güncellemesi devre dışı.".to_string());
    }
    if !download_url.starts_with(ALLOWED_URL_PREFIX) {
        return Err("Geçersiz indirme adresi.".to_string());
    }
    let expected_sha256 =
        expected_sha256.ok_or_else(|| "Güncelleme doğrulanamadı: SHA-256 değeri yok.".to_string())?;
    let apk = crate::android::cache_dir()
        .ok_or("Uygulama klasörü bulunamadı.")?
        .join("update.apk");

    if let Err(e) = download_exe(app, download_url, expected_sha256, &apk).await {
        let _ = fs::remove_file(&apk);
        return Err(e);
    }
    save_pending_notes(latest, release_notes);
    log_info(&format!("Downloaded app update {latest}; opening the installer."));

    let path = apk.to_string_lossy().into_owned();
    tauri::async_runtime::spawn_blocking(move || {
        crate::android::call::<serde_json::Value>("installApk", serde_json::json!({ "path": path }))
    })
    .await
    .map_err(|e| e.to_string())?
    .map(drop)
}

#[cfg(not(target_os = "android"))]
pub async fn install_app_update(
    app: &AppHandle,
    latest: &str,
    download_url: &str,
    expected_sha256: Option<&str>,
    release_notes: &str,
) -> Result<(), String> {
    if cfg!(debug_assertions) {
        return Err("Geliştirme modunda uygulama güncellemesi devre dışı.".to_string());
    }
    if !download_url.starts_with(ALLOWED_URL_PREFIX) {
        return Err("Geçersiz indirme adresi.".to_string());
    }
    let expected_sha256 =
        expected_sha256.ok_or_else(|| "Güncelleme doğrulanamadı: SHA-256 değeri yok.".to_string())?;

    let current_exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let new_exe = append_ext(&current_exe, ".new");
    let old_exe = append_ext(&current_exe, ".old");

    if let Err(e) = download_exe(app, download_url, expected_sha256, &new_exe).await {
        let _ = fs::remove_file(&new_exe);
        return Err(e);
    }

    let _ = fs::remove_file(&old_exe);
    if let Err(e) = fs::rename(&current_exe, &old_exe) {
        let _ = fs::remove_file(&new_exe);
        return Err(format!("Mevcut uygulama değiştirilemedi: {e}"));
    }
    if let Err(e) = fs::rename(&new_exe, &current_exe) {
        let _ = fs::rename(&old_exe, &current_exe);
        let _ = fs::remove_file(&new_exe);
        return Err(format!("Yeni sürüm yerleştirilemedi: {e}"));
    }

    save_pending_notes(latest, release_notes);

    log_info(&format!(
        "App updated successfully at {}. Relaunching...",
        current_exe.display()
    ));

    if let Err(e) = Command::new(&current_exe).spawn() {
        return Err(format!(
            "Güncelleme yüklendi ancak uygulama yeniden başlatılamadı: {e}"
        ));
    }

    app.exit(0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_versions() {
        assert_eq!(parse_semver("v2.0.3"), Some((2, 0, 3)));
        assert_eq!(parse_semver("2.1"), Some((2, 1, 0)));
        assert_eq!(parse_semver("latest"), None);
        assert_eq!(parse_semver("1.2.3.4"), None);
        assert!(parse_semver("2.0.10") > parse_semver("2.0.9"));
    }

    #[test]
    fn release_version_falls_back_to_name() {
        assert_eq!(release_version("rust_v4", "v2.0.4"), Some(("v2.0.4", (2, 0, 4))));
        assert_eq!(release_version("v2.1.0", "Big release"), Some(("v2.1.0", (2, 1, 0))));
        assert_eq!(release_version("rust_v4", "Spring update"), None);
    }

    #[test]
    fn asset_digests() {
        let hex = "A".repeat(64);
        assert_eq!(parse_sha256_digest(&format!("sha256:{hex}")), Some("a".repeat(64)));
        assert_eq!(parse_sha256_digest(&hex), None);
        assert_eq!(parse_sha256_digest("sha256:abc"), None);
        assert_eq!(parse_sha256_digest(&format!("sha512:{hex}")), None);
    }
}
