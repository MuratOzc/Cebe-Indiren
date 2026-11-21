use crate::binaries::{resolve_ffmpeg, resolve_ffprobe, tool_command};
use crate::config::{log_error, log_info};
use crate::downloader::{next_line_lossy, report_progress};
use crate::platform::NoConsole;
use crate::state::{lock, AppState, ProgressPayload};
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tauri::AppHandle;
use tokio::io::BufReader;

const STDERR_TAIL_LINES: usize = 40;

pub fn time_to_seconds(t_str: Option<&str>) -> Option<f64> {
    let raw = t_str?.trim();
    if raw.is_empty() {
        return None;
    }
    let parts: Vec<&str> = raw.split(':').collect();
    if parts.len() > 3 {
        return None;
    }
    let mut total = 0.0;
    for part in parts {
        total = total * 60.0 + part.trim().parse::<f64>().ok()?;
    }
    (total.is_finite() && total >= 0.0).then_some(total)
}

fn parse_out_time(line: &str) -> Option<f64> {
    let micros = line
        .strip_prefix("out_time_us=")
        .or_else(|| line.strip_prefix("out_time_ms="))?;
    let micros = micros.trim().parse::<f64>().ok()?;
    (micros >= 0.0).then_some(micros / 1_000_000.0)
}

fn emit_cut_progress(app: &AppHandle, stage: &str, percent: Option<u8>) {
    let percent = percent.map(|p| format!("{p}%")).unwrap_or_default();
    report_progress(app, ProgressPayload::stage(stage, percent));
}

const AUDIO_EXTENSIONS: [&str; 8] = ["mp3", "m4a", "wav", "aac", "ogg", "flac", "wma", "opus"];
const EVEN_SIZE: &str = "scale=trunc(iw/2)*2:trunc(ih/2)*2";

fn is_audio_extension(ext: &str) -> bool {
    AUDIO_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str())
}

fn extension_of(path: &Path) -> String {
    path.extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn cut_output_path(in_path: &Path) -> PathBuf {
    let stem = in_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("video");
    let ext = in_path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("mp4");
    let out_ext = if is_audio_extension(ext) { ext } else { "mp4" };
    in_path.with_file_name(format!("{stem}_cut.{out_ext}"))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Media {
    pub duration: Option<f64>,
    pub has_video: bool,
    pub has_audio: bool,
    pub frame_seconds: f64,
}

fn frame_seconds(rate: &str) -> Option<f64> {
    let (num, den) = rate.split_once('/')?;
    let fps = num.trim().parse::<f64>().ok()? / den.trim().parse::<f64>().ok()?;
    (fps.is_finite() && fps >= 1.0).then(|| 1.0 / fps)
}

fn parse_probe(json: &str) -> Option<Media> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let streams = value.get("streams")?.as_array()?;
    let first = |kind: &str| {
        streams.iter().find(|stream| {
            let cover = stream
                .pointer("/disposition/attached_pic")
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
                != 0;
            stream.get("codec_type").and_then(|v| v.as_str()) == Some(kind) && !cover
        })
    };
    let video = first("video");
    let duration = value
        .pointer("/format/duration")
        .and_then(|v| v.as_str())
        .and_then(|d| d.trim().parse::<f64>().ok())
        .filter(|d| d.is_finite() && *d > 0.0);
    Some(Media {
        duration,
        has_video: video.is_some(),
        has_audio: first("audio").is_some(),
        frame_seconds: video
            .and_then(|stream| stream.get("avg_frame_rate")?.as_str())
            .and_then(frame_seconds)
            .unwrap_or(0.0),
    })
}

pub async fn probe(path: &Path) -> Option<Media> {
    let ffprobe = resolve_ffprobe().unwrap_or_else(|| PathBuf::from("ffprobe"));
    let out = tool_command(ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration:stream=codec_type,avg_frame_rate:stream_disposition=attached_pic",
            "-of",
            "json",
        ])
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .no_console()
        .output()
        .await
        .ok()?;
    parse_probe(&String::from_utf8_lossy(&out.stdout))
}

fn audio_codec_args(ext: &str) -> &'static [&'static str] {
    match ext {
        "mp3" => &["-c:a", "libmp3lame", "-b:a", "192k", "-id3v2_version", "3"],
        "opus" | "ogg" => &["-c:a", "libopus", "-b:a", "160k"],
        "flac" => &["-c:a", "flac"],
        "wav" => &["-c:a", "pcm_s16le"],
        "wma" => &["-c:a", "wmav2", "-b:a", "192k"],
        _ => &["-c:a", "aac", "-b:a", "192k"],
    }
}

fn seconds_arg(seconds: f64) -> String {
    let text = format!("{seconds:.3}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn cut_args(
    in_path: &Path,
    out_path: &Path,
    intervals: &[(f64, f64)],
    media: &Media,
) -> Option<Vec<String>> {
    let ext = extension_of(out_path);
    let audio_file = is_audio_extension(&ext);
    let video = media.has_video && !audio_file;
    let audio = media.has_audio;
    if intervals.is_empty() || !(video || audio) {
        return None;
    }

    let mut args: Vec<String> = Vec::new();
    let mut add = |more: &[&str]| args.extend(more.iter().map(|arg| arg.to_string()));

    add(&["-y", "-nostdin", "-nostats", "-progress", "pipe:1"]);
    for (start, end) in intervals {
        if *start > 0.0 {
            add(&["-ss", &seconds_arg(*start)]);
        }
        if end.is_finite() {
            add(&["-t", &seconds_arg(end - start)]);
        }
        add(&["-i", &in_path.to_string_lossy()]);
    }

    let pieces = intervals.len();
    if pieces > 1 {
        let mut graph = String::new();
        for input in 0..pieces {
            if video {
                graph.push_str(&format!("[{input}:v:0]"));
            }
            if audio {
                graph.push_str(&format!("[{input}:a:0]"));
            }
        }
        graph.push_str(&format!("concat=n={pieces}:v={}:a={}", video as u8, audio as u8));
        if video {
            graph.push_str("[joined]");
        }
        if audio {
            graph.push_str("[a]");
        }
        if video {
            graph.push_str(&format!(";[joined]{EVEN_SIZE}[v]"));
        }
        add(&["-filter_complex", &graph]);
        if video {
            add(&["-map", "[v]"]);
        }
        if audio {
            add(&["-map", "[a]"]);
        }
    } else {
        if video {
            add(&["-map", "0:v:0", "-vf", EVEN_SIZE]);
        }
        if audio {
            add(&["-map", "0:a:0"]);
        }
    }

    if matches!(ext.as_str(), "mp3" | "m4a") {
        add(&["-map", "0:v?", "-c:v", "copy"]);
    }
    add(&["-map_metadata", "0", "-map_chapters", "-1"]);
    if video {
        add(&["-c:v", "libx264", "-preset", "veryfast", "-crf", "18", "-pix_fmt", "yuv420p"]);
    }
    if audio {
        add(audio_codec_args(if audio_file { &ext } else { "mp4" }));
    }
    if ext == "mp4" {
        add(&["-movflags", "+faststart"]);
    }
    add(&[&out_path.to_string_lossy()]);
    Some(args)
}

pub fn kept_seconds(intervals: &[(f64, f64)], duration: Option<f64>) -> Option<f64> {
    let limit = duration.unwrap_or(f64::INFINITY);
    let total: f64 = intervals
        .iter()
        .map(|(start, end)| (end.min(limit) - start).max(0.0))
        .sum();
    (total.is_finite() && total > 0.0).then_some(total)
}

/// Re-encodes the `intervals` of `in_path` (seconds on its own timeline) into one file next
/// to it and returns that file's path; `in_path` is never modified.
pub async fn cut_media(
    app: &AppHandle,
    state: &AppState,
    in_path: &Path,
    intervals: &[(f64, f64)],
    media: &Media,
    stage: &str,
) -> Result<PathBuf, ()> {
    if state.is_cancelled() {
        return Err(());
    }

    let cut_path = cut_output_path(in_path);
    let Some(args) = cut_args(in_path, &cut_path, intervals, media) else {
        log_error(&format!("Nothing to encode from {}", in_path.display()));
        return Err(());
    };
    let total = kept_seconds(intervals, media.duration);
    let ffmpeg_bin = resolve_ffmpeg().unwrap_or_else(|| PathBuf::from("ffmpeg"));

    let mut cmd = tool_command(&ffmpeg_bin);
    cmd.args(args);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .no_console();

    log_info(&format!("Cutting {}: keeping {intervals:?}", in_path.display()));
    emit_cut_progress(app, stage, total.map(|_| 0));

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            log_error(&format!("Failed to start FFmpeg cut: {e}"));
            return Err(());
        }
    };

    *lock(&state.active_child_pid) = child.id();
    if state.is_cancelled() {
        state.kill_active_child();
    }

    let stderr = child.stderr.take();
    let stderr_task = tokio::spawn(async move {
        let mut tail: VecDeque<String> = VecDeque::new();
        if let Some(err) = stderr {
            let mut reader = BufReader::new(err);
            let mut raw = Vec::new();
            while let Some(line) = next_line_lossy(&mut reader, &mut raw).await {
                if tail.len() == STDERR_TAIL_LINES {
                    tail.pop_front();
                }
                tail.push_back(line);
            }
        }
        Vec::from(tail).join("\n")
    });

    if let Some(out) = child.stdout.take() {
        let mut reader = BufReader::new(out);
        let mut raw = Vec::new();
        let mut last_percent = 0u8;
        while let Some(line) = next_line_lossy(&mut reader, &mut raw).await {
            let (Some(done), Some(total)) = (parse_out_time(&line), total) else {
                continue;
            };
            let percent = ((done / total) * 100.0).clamp(0.0, 99.0) as u8;
            if percent > last_percent {
                last_percent = percent;
                emit_cut_progress(app, stage, Some(percent));
            }
        }
    }

    let status = child.wait().await;
    *lock(&state.active_child_pid) = None;
    let stderr_text = stderr_task.await.unwrap_or_default();

    match status {
        Ok(s) if s.success() && cut_path.exists() && !state.is_cancelled() => Ok(cut_path),
        Ok(s) => {
            if !state.is_cancelled() {
                log_error(&format!(
                    "FFmpeg cut failed ({:?}): {}",
                    s.code(),
                    stderr_text
                ));
            }
            let _ = fs::remove_file(&cut_path);
            Err(())
        }
        Err(e) => {
            log_error(&format!("Cut error: {e}"));
            let _ = fs::remove_file(&cut_path);
            Err(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_time_strings() {
        assert_eq!(time_to_seconds(Some("90")), Some(90.0));
        assert_eq!(time_to_seconds(Some("01:30")), Some(90.0));
        assert_eq!(time_to_seconds(Some("01:01:30.5")), Some(3690.5));
        assert_eq!(time_to_seconds(Some(" 00:00:00 ")), Some(0.0));
        assert_eq!(time_to_seconds(Some("")), None);
        assert_eq!(time_to_seconds(None), None);
        assert_eq!(time_to_seconds(Some("abc")), None);
        assert_eq!(time_to_seconds(Some("-5")), None);
        assert_eq!(time_to_seconds(Some("inf")), None);
        assert_eq!(time_to_seconds(Some("1:2:3:4")), None);
    }

    #[test]
    fn clips_keep_audio_containers_and_force_mp4_for_video() {
        let out = |name: &str| cut_output_path(Path::new(name));
        assert_eq!(out("a.mp3"), Path::new("a_cut.mp3"));
        assert_eq!(out("a.M4A"), Path::new("a_cut.M4A"));
        assert_eq!(out("a.mp4"), Path::new("a_cut.mp4"));
        assert_eq!(out("a.webm"), Path::new("a_cut.mp4"));
        assert_eq!(out("a.mkv"), Path::new("a_cut.mp4"));
    }

    const BOTH: Media = Media { duration: Some(750.0), has_video: true, has_audio: true, frame_seconds: 1.0 / 60.0 };

    #[test]
    fn a_single_range_is_an_accurate_seek_and_a_fresh_encode() {
        let args = cut_args(Path::new("in.mp4"), Path::new("out.mp4"), &[(61.3, 76.3)], &BOTH).unwrap();
        assert_eq!(
            args,
            [
                "-y", "-nostdin", "-nostats", "-progress", "pipe:1",
                "-ss", "61.3", "-t", "15", "-i", "in.mp4",
                "-map", "0:v:0", "-vf", EVEN_SIZE, "-map", "0:a:0",
                "-map_metadata", "0", "-map_chapters", "-1",
                "-c:v", "libx264", "-preset", "veryfast", "-crf", "18", "-pix_fmt", "yuv420p",
                "-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart", "out.mp4",
            ]
        );

        let whole = cut_args(Path::new("in.webm"), Path::new("out.mp4"), &[(0.0, f64::INFINITY)], &BOTH).unwrap();
        assert_eq!(&whole[5..7], ["-i", "in.webm"]);
        assert!(!whole.iter().any(|a| a == "-ss" || a == "-t"));
    }

    #[test]
    fn several_ranges_are_joined_by_the_concat_filter() {
        let keep = [(48.0, 69.17), (112.5, 411.0)];
        let args = cut_args(Path::new("in.mp4"), Path::new("out.mp4"), &keep, &BOTH).unwrap();
        assert_eq!(
            &args[5..18],
            [
                "-ss", "48", "-t", "21.17", "-i", "in.mp4",
                "-ss", "112.5", "-t", "298.5", "-i", "in.mp4",
                "-filter_complex",
            ]
        );
        assert_eq!(
            args[18],
            format!("[0:v:0][0:a:0][1:v:0][1:a:0]concat=n=2:v=1:a=1[joined][a];[joined]{EVEN_SIZE}[v]")
        );
        assert_eq!(&args[19..23], ["-map", "[v]", "-map", "[a]"]);

        let silent = Media { has_audio: false, ..BOTH };
        let args = cut_args(Path::new("in.mp4"), Path::new("out.mp4"), &keep, &silent).unwrap();
        assert!(args.contains(&format!("[0:v:0][1:v:0]concat=n=2:v=1:a=0[joined];[joined]{EVEN_SIZE}[v]")));
        assert!(!args.iter().any(|a| a == "[a]" || a == "-c:a"));
    }

    #[test]
    fn audio_files_are_re_encoded_in_their_own_format() {
        let cover = Media { has_video: false, frame_seconds: 0.0, ..BOTH };
        let keep = [(0.0, 69.17), (112.5, 750.0)];
        let args = cut_args(Path::new("in.mp3"), Path::new("out.mp3"), &keep, &cover).unwrap();
        assert!(args.contains(&"[0:a:0][1:a:0]concat=n=2:v=0:a=1[a]".to_string()));
        let tail = [
            "-map", "[a]", "-map", "0:v?", "-c:v", "copy", "-map_metadata", "0", "-map_chapters", "-1",
            "-c:a", "libmp3lame", "-b:a", "192k", "-id3v2_version", "3", "out.mp3",
        ];
        assert_eq!(&args[args.len() - tail.len()..], tail);

        let m4a = cut_args(Path::new("in.m4a"), Path::new("out.m4a"), &[(5.0, 9.0)], &BOTH).unwrap();
        assert!(m4a.windows(2).any(|w| w == ["-c:a", "aac"]));
        assert!(!m4a.iter().any(|a| a == "libx264" || a == "-movflags"));

        let nothing = Media { has_audio: false, ..BOTH };
        assert!(cut_args(Path::new("in.mp3"), Path::new("out.mp3"), &keep, &nothing).is_none());
        assert!(cut_args(Path::new("in.mp4"), Path::new("out.mp4"), &[], &BOTH).is_none());
    }

    #[test]
    fn no_video_cut_is_ever_a_stream_copy() {
        for keep in [vec![(3.0, 9.0)], vec![(0.0, 5.0), (8.0, f64::INFINITY)]] {
            let args = cut_args(Path::new("in.mp4"), Path::new("out.mp4"), &keep, &BOTH).unwrap();
            assert!(!args.iter().any(|a| a == "copy" || a == "-c"));
            assert!(args.iter().any(|a| a == "libx264"));
        }
    }

    #[test]
    fn progress_runs_against_the_kept_length() {
        assert_eq!(kept_seconds(&[(48.0, 69.0), (112.0, 411.0)], Some(750.0)), Some(320.0));
        assert_eq!(kept_seconds(&[(700.0, f64::INFINITY)], Some(750.0)), Some(50.0));
        assert_eq!(kept_seconds(&[(700.0, f64::INFINITY)], None), None);
    }

    #[test]
    fn probe_output_parsing() {
        let media = parse_probe(
            r#"{"streams":[{"codec_type":"audio","avg_frame_rate":"0/0","disposition":{"attached_pic":0}},
                {"codec_type":"video","avg_frame_rate":"90000/1","disposition":{"attached_pic":1}}],
                "format":{"duration":"750.418000"}}"#,
        )
        .unwrap();
        assert_eq!(
            media,
            Media { duration: Some(750.418), has_video: false, has_audio: true, frame_seconds: 0.0 }
        );
        let media = parse_probe(
            r#"{"streams":[{"codec_type":"video","avg_frame_rate":"30000/1001","disposition":{"attached_pic":0}}],"format":{}}"#,
        )
        .unwrap();
        assert!(media.has_video && !media.has_audio && media.duration.is_none());
        assert!((media.frame_seconds - 1001.0 / 30000.0).abs() < 1e-12);
        assert_eq!(frame_seconds("0/0"), None);
        assert!(parse_probe("").is_none());
    }

    #[test]
    fn parses_ffmpeg_progress() {
        assert_eq!(parse_out_time("out_time_us=1500000"), Some(1.5));
        assert_eq!(parse_out_time("out_time_ms=2000000"), Some(2.0));
        assert_eq!(parse_out_time("out_time_us=N/A"), None);
        assert_eq!(parse_out_time("frame=12"), None);
    }
}
