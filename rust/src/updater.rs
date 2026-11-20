use crate::binaries::{exe_stamp, resolve_ytdlp, stamp_path, update_bin_dir, updated_ytdlp_path};
use crate::config::{log_info, log_warn};
use crate::state::{AppState, UpdateState};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Command;

#[cfg(windows)]
use crate::state::CREATE_NO_WINDOW;
#[cfg(windows)]
#[allow(unused_imports)]
use std::os::windows::process::CommandExt;

const RELEASES_URL: &str = "https://github.com/yt-dlp/yt-dlp/releases";
const API_LATEST_URL: &str = "https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest";
const USER_AGENT: &str = "VideoIndiren-updater";

#[cfg(windows)]
const ASSET_NAME: &str = "yt-dlp.exe";
#[cfg(not(windows))]
const ASSET_NAME: &str = "yt-dlp";

fn is_valid_tag(tag: &str) -> bool {
    let parts: Vec<&str> = tag.split('.').collect();
    if parts.len() != 3 && parts.len() != 4 {
        return false;
    }
    parts[0].len() == 4
        && parts[1].len() == 2
        && parts[2].len() == 2
        && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

fn version_tuple(version: &str) -> Option<Vec<u64>> {
    version
        .trim()
        .split('.')
        .map(|part| part.parse::<u64>().ok())
        .collect()
}

pub async fn get_active_ytdlp_version(resource_dir: Option<&Path>) -> Option<String> {
    let bin = resolve_ytdlp(resource_dir);
    let mut cmd = Command::new(bin);
    cmd.arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let out = cmd.output().await.ok()?;
    if !out.status.success() {
        return None;
    }
    let ver = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if ver.is_empty() {
        None
    } else {
        Some(ver)
    }
}

async fn latest_from_redirect() -> Result<String, String> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(8))
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .get(format!("{RELEASES_URL}/latest"))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let status = resp.status().as_u16();
    if matches!(status, 301 | 302 | 303 | 307 | 308) {
        if let Some(loc) = resp.headers().get("location").and_then(|v| v.to_str().ok()) {
            if let Some((_, tag)) = loc.rsplit_once("/tag/") {
                return Ok(tag.trim().to_string());
            }
        }
    }
    Err(format!("Unexpected redirect response: {status}"))
}

async fn latest_from_api() -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
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

    let val: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    val.get("tag_name")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .ok_or_else(|| "Missing tag_name in API response".to_string())
}

pub fn start_update_check(state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        let current = match get_active_ytdlp_version(state.resource_dir.as_deref()).await {
            Some(v) => v,
            None => {
                log_warn("Could not determine active yt-dlp version.");
                if let Ok(mut u) = state.update_state.lock() {
                    *u = UpdateState {
                        status: "failed".to_string(),
                        current: String::new(),
                        latest: String::new(),
                    };
                }
                return;
            }
        };

        if let Ok(mut u) = state.update_state.lock() {
            *u = UpdateState {
                status: "checking".to_string(),
                current: current.clone(),
                latest: String::new(),
            };
        }

        for use_api in [false, true] {
            for attempt in 0..3u64 {
                let res = if use_api {
                    latest_from_api().await
                } else {
                    latest_from_redirect().await
                };
                match res {
                    Ok(tag) if is_valid_tag(&tag) => {
                        let newer = match (version_tuple(&tag), version_tuple(&current)) {
                            (Some(l), Some(c)) => l > c,
                            _ => false,
                        };
                        let status = if newer { "available" } else { "up_to_date" };
                        log_info(&format!(
                            "yt-dlp {current} active, latest stable {tag} ({status})."
                        ));
                        if let Ok(mut u) = state.update_state.lock() {
                            *u = UpdateState {
                                status: status.to_string(),
                                current: current.clone(),
                                latest: tag,
                            };
                        }
                        return;
                    }
                    Ok(bad) => {
                        log_warn(&format!("Unexpected yt-dlp version tag: {bad:?}"));
                    }
                    Err(e) => {
                        let source = if use_api { "api" } else { "redirect" };
                        log_warn(&format!(
                            "yt-dlp update check via {source} failed (attempt {}): {e}",
                            attempt + 1
                        ));
                    }
                }
                if attempt < 2 {
                    tokio::time::sleep(Duration::from_secs(attempt + 1)).await;
                }
            }
        }

        if let Ok(mut u) = state.update_state.lock() {
            *u = UpdateState {
                status: "failed".to_string(),
                current,
                latest: String::new(),
            };
        }
    });
}

pub async fn install_update(tag: &str) -> Result<(), String> {
    if !is_valid_tag(tag) {
        return Err("Invalid version tag".to_string());
    }

    let base = format!("{RELEASES_URL}/download/{tag}");
    let dir = update_bin_dir();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let dest = updated_ytdlp_path();
    let tmp = dest.with_extension("exe.tmp");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| e.to_string())?;

    let sums = client
        .get(format!("{base}/SHA2-256SUMS"))
        .send()
        .await
        .map_err(|e| format!("Checksum indirilemedi: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Checksum hatası: {e}"))?
        .text()
        .await
        .map_err(|e| e.to_string())?;

    let expected = sums
        .lines()
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            let hash = parts.next()?;
            let name = parts.next()?;
            if name.trim_start_matches('*') == ASSET_NAME {
                Some(hash.to_lowercase())
            } else {
                None
            }
        })
        .ok_or_else(|| format!("{ASSET_NAME} checksum bulunamadı"))?;

    let bytes = client
        .get(format!("{base}/{ASSET_NAME}"))
        .send()
        .await
        .map_err(|e| format!("yt-dlp indirilemedi: {e}"))?
        .error_for_status()
        .map_err(|e| format!("İndirme hatası: {e}"))?
        .bytes()
        .await
        .map_err(|e| e.to_string())?;

    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let actual = format!("{:x}", hasher.finalize());

    if actual != expected {
        return Err("SHA-256 doğrulaması başarısız".to_string());
    }

    fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;

    if fs::rename(&tmp, &dest).is_err() {
        let _ = fs::remove_file(&dest);
        if let Err(e) = fs::rename(&tmp, &dest) {
            let _ = fs::remove_file(&tmp);
            return Err(format!("Dosya yerleştirilemedi: {e}"));
        }
    }

    if let Some(stamp) = exe_stamp() {
        let _ = fs::write(stamp_path(), stamp);
    }

    log_info(&format!("Installed yt-dlp {tag} to {}", dest.display()));
    Ok(())
}
