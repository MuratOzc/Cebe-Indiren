use crate::binaries::{
    bin_dir, is_newer_ytdlp, resolve_ytdlp, write_binary, ytdlp_version_path, YTDLP_NAME,
};
use crate::config::{log_info, log_warn};
use crate::state::{lock, AppState, UpdateState, CREATE_NO_WINDOW};
use sha2::{Digest, Sha256};
use std::fs;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Command;

const RELEASES_URL: &str = "https://github.com/yt-dlp/yt-dlp/releases";
const USER_AGENT: &str = "VideoIndiren-updater";

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

async fn active_ytdlp_version() -> Option<String> {
    let out = Command::new(resolve_ytdlp())
        .arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .await
        .ok()?;
    let ver = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !ver.is_empty()).then_some(ver)
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

pub fn start_update_check(state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        let set = |status: &str, current: &str, latest: &str| {
            *lock(&state.update_state) = UpdateState {
                status: status.to_string(),
                current: current.to_string(),
                latest: latest.to_string(),
            };
        };

        let Some(current) = active_ytdlp_version().await else {
            log_warn("Could not determine active yt-dlp version.");
            set("failed", "", "");
            return;
        };
        set("checking", &current, "");

        for attempt in 1..=3u64 {
            match latest_from_redirect().await {
                Ok(tag) if is_valid_tag(&tag) => {
                    let status = if is_newer_ytdlp(&tag, &current) {
                        "available"
                    } else {
                        "up_to_date"
                    };
                    log_info(&format!(
                        "yt-dlp {current} active, latest stable {tag} ({status})."
                    ));
                    set(status, &current, &tag);
                    return;
                }
                Ok(bad) => log_warn(&format!("Unexpected yt-dlp version tag: {bad:?}")),
                Err(e) => log_warn(&format!(
                    "yt-dlp update check failed (attempt {attempt}): {e}"
                )),
            }
            if attempt < 3 {
                tokio::time::sleep(Duration::from_secs(attempt)).await;
            }
        }
        set("failed", &current, "");
    });
}

pub async fn install_update(tag: &str) -> Result<(), String> {
    if !is_valid_tag(tag) {
        return Err("Invalid version tag".to_string());
    }

    let base = format!("{RELEASES_URL}/download/{tag}");
    fs::create_dir_all(bin_dir()).map_err(|e| e.to_string())?;
    let dest = bin_dir().join(YTDLP_NAME);

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(30))
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| e.to_string())?;

    let sums = client
        .get(format!("{base}/SHA2-256SUMS"))
        .send()
        .await
        .and_then(|resp| resp.error_for_status())
        .map_err(|e| format!("Checksum indirilemedi: {e}"))?
        .text()
        .await
        .map_err(|e| e.to_string())?;

    let expected = sums
        .lines()
        .find_map(|line| {
            let (hash, name) = line.split_once(char::is_whitespace)?;
            (name.trim().trim_start_matches('*') == YTDLP_NAME).then(|| hash.to_lowercase())
        })
        .ok_or_else(|| format!("{YTDLP_NAME} checksum bulunamadı"))?;

    let bytes = client
        .get(format!("{base}/{YTDLP_NAME}"))
        .send()
        .await
        .and_then(|resp| resp.error_for_status())
        .map_err(|e| format!("yt-dlp indirilemedi: {e}"))?
        .bytes()
        .await
        .map_err(|e| e.to_string())?;

    if format!("{:x}", Sha256::digest(&bytes)) != expected {
        return Err("SHA-256 doğrulaması başarısız".to_string());
    }

    write_binary(&dest, &bytes).map_err(|e| format!("Dosya yerleştirilemedi: {e}"))?;
    let _ = fs::write(ytdlp_version_path(), tag);

    log_info(&format!("Installed yt-dlp {tag} to {}", dest.display()));
    Ok(())
}
