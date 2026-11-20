use crate::config::{log_info, log_warn};
use crate::state::{AppState, AppUpdateState};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{BufWriter, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

const API_LATEST_URL: &str =
    "https://api.github.com/repos/anilyildiz8/Video-Indiren/releases/latest";
const ALLOWED_URL_PREFIX: &str =
    "https://github.com/anilyildiz8/Video-Indiren/releases/download/";
const USER_AGENT: &str = "VideoIndiren-updater";
const ASSET_NAME: &str = "VideoIndiren.exe";
const MIN_EXE_BYTES: u64 = 1_000_000;

#[derive(Debug, Clone, Serialize)]
struct AppUpdateProgress {
    percent: u8,
    downloaded: u64,
    total: u64,
}

fn append_ext(path: &Path, suffix: &str) -> PathBuf {
    let mut os = path.as_os_str().to_os_string();
    os.push(suffix);
    PathBuf::from(os)
}

fn parse_semver(raw: &str) -> Option<(u64, u64, u64)> {
    let cleaned = raw.trim().trim_start_matches(['v', 'V', '.']);
    if cleaned.is_empty() {
        return None;
    }
    let parts: Vec<&str> = cleaned.split('.').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return None;
    }
    let major = parts[0].parse::<u64>().ok()?;
    let minor = parts[1].parse::<u64>().ok()?;
    let patch = if parts.len() == 3 {
        parts[2].parse::<u64>().ok()?
    } else {
        0
    };
    Some((major, minor, patch))
}

fn extract_release_version(tag: &str, name: &str) -> Option<(String, (u64, u64, u64))> {
    if let Some(ver) = parse_semver(tag) {
        let display = tag.trim().trim_start_matches(['v', 'V', '.']).to_string();
        return Some((format!("v{display}"), ver));
    }
    if let Some(ver) = parse_semver(name) {
        let display = name.trim().trim_start_matches(['v', 'V', '.']).to_string();
        return Some((format!("v{display}"), ver));
    }
    None
}

/// Removes the leftover `.old` executable from a previous self-update.
pub fn cleanup_old_exe() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let old_exe = append_ext(&exe, ".old");
    if !old_exe.exists() {
        return;
    }

    tauri::async_runtime::spawn(async move {
        for _ in 0..5 {
            if fs::remove_file(&old_exe).is_ok() {
                log_info(&format!("Cleaned up old binary: {}", old_exe.display()));
                return;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        log_warn(&format!(
            "Could not remove old binary: {}",
            old_exe.display()
        ));
    });
}

async fn fetch_latest_release() -> Result<serde_json::Value, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .get(API_LATEST_URL)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !resp.status().is_success() {
        return Err(format!("GitHub API status {}", resp.status()));
    }

    resp.json::<serde_json::Value>()
        .await
        .map_err(|e| e.to_string())
}

pub fn start_app_update_check(state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        let current_ver = match parse_semver(APP_VERSION) {
            Some(v) => v,
            None => {
                log_warn(&format!("Invalid APP_VERSION: {APP_VERSION}"));
                return;
            }
        };

        for attempt in 0..3u64 {
            match fetch_latest_release().await {
                Ok(val) => {
                    let tag = val
                        .get("tag_name")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default();
                    let name = val
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default();

                    let Some((latest_label, latest_ver)) = extract_release_version(tag, name)
                    else {
                        log_warn(&format!(
                            "Could not parse app release version (tag={tag:?}, name={name:?})"
                        ));
                        break;
                    };

                    let assets = val.get("assets").and_then(|v| v.as_array());
                    let download_url = assets
                        .and_then(|list| {
                            list.iter().find_map(|asset| {
                                let asset_name = asset.get("name")?.as_str()?;
                                if asset_name.eq_ignore_ascii_case(ASSET_NAME) {
                                    asset
                                        .get("browser_download_url")?
                                        .as_str()
                                        .map(|s| s.to_string())
                                } else {
                                    None
                                }
                            })
                        })
                        .unwrap_or_default();

                    let sha256_url = assets.and_then(|list| {
                        list.iter().find_map(|asset| {
                            let asset_name = asset.get("name")?.as_str()?;
                            if matches!(
                                asset_name,
                                "SHA256SUMS.txt" | "SHA2-256SUMS" | "VideoIndiren.exe.sha256"
                            ) {
                                asset
                                    .get("browser_download_url")?
                                    .as_str()
                                    .map(|s| s.to_string())
                            } else {
                                None
                            }
                        })
                    });

                    let newer = latest_ver > current_ver && !download_url.is_empty();
                    let status = if newer { "available" } else { "up_to_date" };
                    log_info(&format!(
                        "App v{APP_VERSION} active, latest {latest_label} ({status})."
                    ));

                    if let Ok(mut u) = state.app_update_state.lock() {
                        *u = AppUpdateState {
                            status: status.to_string(),
                            current: format!("v{APP_VERSION}"),
                            latest: latest_label,
                            download_url,
                            sha256_url,
                        };
                    }
                    return;
                }
                Err(e) => {
                    log_warn(&format!(
                        "App update check failed (attempt {}): {e}",
                        attempt + 1
                    ));
                    if attempt < 2 {
                        tokio::time::sleep(Duration::from_secs(attempt + 1)).await;
                    }
                }
            }
        }

        if let Ok(mut u) = state.app_update_state.lock() {
            *u = AppUpdateState {
                status: "failed".to_string(),
                current: format!("v{APP_VERSION}"),
                latest: String::new(),
                download_url: String::new(),
                sha256_url: None,
            };
        }
    });
}

async fn fetch_expected_sha256(
    client: &reqwest::Client,
    sha256_url: &str,
) -> Result<String, String> {
    if !sha256_url.starts_with(ALLOWED_URL_PREFIX) {
        return Err("Geçersiz checksum adresi".to_string());
    }
    let body = client
        .get(sha256_url)
        .send()
        .await
        .map_err(|e| format!("Checksum indirilemedi: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Checksum hatası: {e}"))?
        .text()
        .await
        .map_err(|e| e.to_string())?;

    for line in body.lines() {
        let mut parts = line.split_whitespace();
        let Some(hash) = parts.next() else {
            continue;
        };
        if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        match parts.next() {
            Some(name) if name.trim_start_matches('*').eq_ignore_ascii_case(ASSET_NAME) => {
                return Ok(hash.to_lowercase());
            }
            None => return Ok(hash.to_lowercase()),
            _ => continue,
        }
    }

    Err("SHA-256 checksum bulunamadı".to_string())
}

pub async fn install_app_update(
    app: &AppHandle,
    download_url: &str,
    sha256_url: Option<&str>,
) -> Result<(), String> {
    if cfg!(debug_assertions) {
        return Err("Geliştirme modunda uygulama güncellemesi devre dışı.".to_string());
    }

    if !download_url.starts_with(ALLOWED_URL_PREFIX) {
        return Err("Geçersiz indirme adresi.".to_string());
    }

    let current_exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let new_exe = append_ext(&current_exe, ".new");
    let old_exe = append_ext(&current_exe, ".old");

    let file = File::create(&new_exe).map_err(|e| {
        if e.kind() == ErrorKind::PermissionDenied {
            "Güncelleme için yazma izni yok. Uygulamayı Masaüstü veya İndirilenler klasörüne taşıyın."
                .to_string()
        } else {
            format!("Geçici dosya oluşturulamadı: {e}")
        }
    })?;

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(30))
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| {
            let _ = fs::remove_file(&new_exe);
            e.to_string()
        })?;

    let expected_sha256 = if let Some(url) = sha256_url {
        match fetch_expected_sha256(&client, url).await {
            Ok(h) => Some(h),
            Err(e) => {
                let _ = fs::remove_file(&new_exe);
                return Err(e);
            }
        }
    } else {
        None
    };

    let mut resp = match client.get(download_url).send().await {
        Ok(r) => match r.error_for_status() {
            Ok(ok) => ok,
            Err(e) => {
                let _ = fs::remove_file(&new_exe);
                return Err(format!("İndirme hatası: {e}"));
            }
        },
        Err(e) => {
            let _ = fs::remove_file(&new_exe);
            return Err(format!("Güncelleme indirilemedi: {e}"));
        }
    };

    let total = resp.content_length().unwrap_or(0);
    let mut writer = BufWriter::new(file);
    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;
    let mut magic = [0u8; 2];
    let mut magic_len = 0usize;
    let mut last_percent: i16 = -1;

    let download_result: Result<(), String> = async {
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| format!("İndirme kesildi: {e}"))?
        {
            if magic_len < 2 {
                let take = (2 - magic_len).min(chunk.len());
                magic[magic_len..magic_len + take].copy_from_slice(&chunk[..take]);
                magic_len += take;
            }
            writer
                .write_all(&chunk)
                .map_err(|e| format!("Dosya yazılamadı: {e}"))?;
            hasher.update(&chunk);
            downloaded += chunk.len() as u64;

            if total > 0 {
                let pct = ((downloaded.saturating_mul(100)) / total).min(100) as u8;
                if pct as i16 != last_percent {
                    last_percent = pct as i16;
                    let _ = app.emit(
                        "app-update-progress",
                        AppUpdateProgress {
                            percent: pct,
                            downloaded,
                            total,
                        },
                    );
                }
            }
        }
        writer
            .flush()
            .map_err(|e| format!("Dosya tamamlanamadı: {e}"))?;
        Ok(())
    }
    .await;

    drop(writer);

    if let Err(e) = download_result {
        let _ = fs::remove_file(&new_exe);
        return Err(e);
    }

    if downloaded < MIN_EXE_BYTES || &magic != b"MZ" {
        let _ = fs::remove_file(&new_exe);
        return Err("İndirilen dosya geçerli bir uygulama değil.".to_string());
    }

    if let Some(expected) = expected_sha256 {
        let actual = format!("{:x}", hasher.finalize());
        if actual != expected {
            let _ = fs::remove_file(&new_exe);
            return Err("SHA-256 doğrulaması başarısız.".to_string());
        }
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
