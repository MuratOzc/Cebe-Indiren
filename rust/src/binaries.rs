use crate::config::{app_data_dir, log_warn};
use std::fs;
use std::path::{Path, PathBuf};

pub const YTDLP_NAME: &str = "yt-dlp.exe";
const FFMPEG_NAME: &str = "ffmpeg.exe";
#[cfg(not(debug_assertions))]
const FFPROBE_NAME: &str = "ffprobe.exe";

pub fn bin_dir() -> PathBuf {
    app_data_dir().join("bin")
}

/// Records which yt-dlp version sits in `bin_dir`, so a newer self-updated copy survives the
/// re-extraction that follows an app update.
pub fn ytdlp_version_path() -> PathBuf {
    bin_dir().join("yt-dlp.version")
}

pub fn is_newer_ytdlp(candidate: &str, than: &str) -> bool {
    let parse = |v: &str| -> Option<Vec<u64>> { v.trim().split('.').map(|p| p.parse().ok()).collect() };
    matches!((parse(candidate), parse(than)), (Some(a), Some(b)) if a > b)
}

#[cfg(not(debug_assertions))]
fn stamp_path() -> PathBuf {
    bin_dir().join("exe.stamp")
}

#[cfg(not(debug_assertions))]
fn exe_stamp() -> Option<String> {
    let meta = fs::metadata(std::env::current_exe().ok()?).ok()?;
    let mtime = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos();
    Some(format!("{}-{}", meta.len(), mtime))
}

pub fn write_binary(dest: &Path, bytes: &[u8]) -> std::io::Result<()> {
    replace_binary(dest, |tmp| fs::write(tmp, bytes))
}

fn replace_binary(
    dest: &Path,
    write_tmp: impl FnOnce(&Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let tmp = dest.with_extension("exe.tmp");
    if let Err(e) = write_tmp(&tmp) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    if fs::rename(&tmp, dest).is_err() {
        let _ = fs::remove_file(dest);
        if let Err(e) = fs::rename(&tmp, dest) {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
    }
    Ok(())
}

#[cfg(not(debug_assertions))]
fn write_compressed_binary(dest: &Path, compressed: &[u8]) -> std::io::Result<()> {
    replace_binary(dest, |tmp| {
        let mut decoder = zstd::Decoder::new(compressed)?;
        decoder.window_log_max(31)?;
        let mut out = std::io::BufWriter::new(fs::File::create(tmp)?);
        std::io::copy(&mut decoder, &mut out)?;
        std::io::Write::flush(&mut out)
    })
}

// Release builds embed the tools zstd-compressed (see build.rs) so VideoIndiren.exe is a single
// portable file; they are unpacked on first launch and again whenever the exe itself changes.
// yt-dlp needs ffprobe next to ffmpeg for SponsorBlock and other post-processing.
#[cfg(not(debug_assertions))]
pub fn ensure_binaries_ready() {
    use crate::config::{log_error, log_info};

    let dir = bin_dir();
    if let Err(e) = fs::create_dir_all(&dir) {
        log_error(&format!("Failed to create bin dir: {e}"));
        return;
    }

    let current_stamp = exe_stamp();
    let saved_stamp = fs::read_to_string(stamp_path())
        .ok()
        .map(|s| s.trim().to_string());
    let stamp_changed = current_stamp.is_some() && saved_stamp != current_stamp;

    let embedded: [(&str, &[u8]); 3] = [
        (YTDLP_NAME, include_bytes!(concat!(env!("OUT_DIR"), "/yt-dlp.exe.zst"))),
        (FFMPEG_NAME, include_bytes!(concat!(env!("OUT_DIR"), "/ffmpeg.exe.zst"))),
        (FFPROBE_NAME, include_bytes!(concat!(env!("OUT_DIR"), "/ffprobe.exe.zst"))),
    ];
    for (name, bytes) in embedded {
        let dest = dir.join(name);
        let keep_updated_ytdlp = name == YTDLP_NAME
            && dest.is_file()
            && fs::read_to_string(ytdlp_version_path())
                .is_ok_and(|installed| is_newer_ytdlp(&installed, env!("BUNDLED_YTDLP_VERSION")));
        if (stamp_changed || !dest.is_file()) && !keep_updated_ytdlp {
            log_info(&format!("Extracting embedded {name} to AppData..."));
            if let Err(e) = write_compressed_binary(&dest, bytes) {
                log_error(&format!("Failed to extract {name}: {e}"));
            } else if name == YTDLP_NAME {
                let _ = fs::write(ytdlp_version_path(), env!("BUNDLED_YTDLP_VERSION"));
            }
        }
    }

    if let Some(stamp) = current_stamp {
        if stamp_changed || !stamp_path().is_file() {
            let _ = fs::write(stamp_path(), stamp);
        }
    }
}

fn resolve(name: &str) -> Option<PathBuf> {
    [bin_dir().join(name), Path::new("binaries").join(name)]
        .into_iter()
        .find(|path| path.is_file())
}

pub fn resolve_ytdlp() -> PathBuf {
    resolve(YTDLP_NAME).unwrap_or_else(|| PathBuf::from(YTDLP_NAME))
}

pub fn resolve_ffmpeg() -> Option<PathBuf> {
    let found = resolve(FFMPEG_NAME);
    if found.is_none() {
        log_warn("Bundled ffmpeg not found; falling back to system PATH.");
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ytdlp_version_ordering() {
        assert!(is_newer_ytdlp("2026.11.02", "2026.10.26"));
        assert!(is_newer_ytdlp("2026.10.26.1", "2026.10.26"));
        assert!(!is_newer_ytdlp("2026.10.26", "2026.10.26"));
        assert!(!is_newer_ytdlp("2026.09.01", "2026.10.26"));
        assert!(!is_newer_ytdlp("garbage", "2026.10.26"));
    }
}
