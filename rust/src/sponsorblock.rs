use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::Duration;

pub const CATEGORIES: [&str; 3] = ["sponsor", "selfpromo", "interaction"];
const API_URL: &str = "https://sponsor.ajay.app/api/skipSegments";
const TIMEOUT: Duration = Duration::from_secs(4);
const USER_AGENT: &str = "VideoIndiren";
const MIN_KEEP_SECONDS: f64 = 0.1;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub category: String,
}

#[derive(Debug, Serialize)]
pub struct Overview {
    pub segments: Vec<Segment>,
    pub summary: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Removed {
    pub count: usize,
    pub seconds: f64,
}

#[derive(Deserialize)]
struct ApiVideo {
    #[serde(rename = "videoID")]
    video_id: String,
    #[serde(default)]
    segments: Vec<ApiSegment>,
}

#[derive(Deserialize)]
struct ApiSegment {
    category: String,
    #[serde(rename = "actionType")]
    action_type: Option<String>,
    segment: (f64, f64),
}

#[derive(Deserialize)]
struct CutChapter {
    start_time: f64,
    end_time: f64,
}

// The API is queried by hash prefix so it never learns which video is being looked up.
fn hash_prefix(video_id: &str) -> String {
    let digest = Sha256::digest(video_id.as_bytes());
    format!("{:02x}{:02x}", digest[0], digest[1])
}

pub fn parse_segments(body: &str, video_id: &str) -> Vec<Segment> {
    let videos: Vec<ApiVideo> = serde_json::from_str(body).unwrap_or_default();
    let mut segments: Vec<Segment> = videos
        .into_iter()
        .find(|video| video.video_id == video_id)
        .map(|video| video.segments)
        .unwrap_or_default()
        .into_iter()
        .filter(|s| s.action_type.as_deref().unwrap_or("skip") == "skip")
        .filter(|s| CATEGORIES.contains(&s.category.as_str()))
        .filter(|s| s.segment.0 >= 0.0 && s.segment.1 > s.segment.0)
        .map(|s| Segment {
            start: s.segment.0,
            end: s.segment.1,
            category: s.category,
        })
        .collect();
    segments.sort_by(|a, b| a.start.total_cmp(&b.start));
    segments
}

fn covered_seconds(mut ranges: Vec<(f64, f64)>) -> f64 {
    ranges.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut total = 0.0;
    let mut reach = f64::MIN;
    for (start, end) in ranges {
        if end > reach {
            total += end - start.max(reach);
            reach = end;
        }
    }
    total
}

fn clock(seconds: f64) -> String {
    let total = seconds.max(0.0).round() as u64;
    let (h, m, s) = (total / 3600, total / 60 % 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

pub fn summary(segments: &[Segment]) -> String {
    if segments.is_empty() {
        return "Bölüm yok".to_string();
    }
    let seconds = covered_seconds(segments.iter().map(|s| (s.start, s.end)).collect());
    format!("{} bölüm · {}", segments.len(), clock(seconds))
}

/// Reads the `sponsorblock_chapters` yt-dlp reports for a video, as sorted ranges
/// that do not overlap.
pub fn parse_cuts(json: &str) -> Vec<(f64, f64)> {
    let chapters: Vec<CutChapter> = serde_json::from_str(json).unwrap_or_default();
    let mut ranges: Vec<(f64, f64)> = chapters
        .iter()
        .filter(|c| c.start_time >= 0.0 && c.end_time > c.start_time)
        .map(|c| (c.start_time, c.end_time))
        .collect();
    ranges.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (start, end) in ranges {
        match merged.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    merged
}

/// What `cuts` took out of the range `start..end` of the original video.
pub fn removed_within(cuts: &[(f64, f64)], start: f64, end: f64) -> Removed {
    let mut removed = Removed::default();
    for (cut_start, cut_end) in cuts {
        let overlap = cut_end.min(end) - cut_start.max(start);
        if overlap > 0.0 {
            removed.count += 1;
            removed.seconds += overlap;
        }
    }
    removed
}

/// Starts each cut one video frame early. A frame is on screen from its timestamp until the
/// next one, so the frame showing when a segment begins already belongs to that segment.
pub fn lead_cuts(cuts: &[(f64, f64)], frame_seconds: f64) -> Vec<(f64, f64)> {
    cuts.iter()
        .map(|(start, end)| ((start - frame_seconds).max(0.0), *end))
        .collect()
}

/// The parts of the original timeline that stay once `cuts` are taken out of the range
/// `start..end`. An open end is `f64::INFINITY` when the duration is unknown.
pub fn keep_intervals(
    start: f64,
    end: Option<f64>,
    duration: Option<f64>,
    cuts: &[(f64, f64)],
) -> Vec<(f64, f64)> {
    let limit = duration
        .filter(|d| d.is_finite() && *d > 0.0)
        .unwrap_or(f64::INFINITY);
    let end = end.unwrap_or(f64::INFINITY).min(limit);
    let mut cuts = cuts.to_vec();
    cuts.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut kept = Vec::new();
    let mut cursor = start.max(0.0);
    for (cut_start, cut_end) in cuts {
        if cut_end <= cursor {
            continue;
        }
        if cut_start >= end {
            break;
        }
        if cut_start > cursor {
            kept.push((cursor, cut_start));
        }
        cursor = cut_end;
    }
    if cursor < end {
        kept.push((cursor, end));
    }
    kept.retain(|(from, to)| to - from >= MIN_KEEP_SECONDS);
    kept
}

pub fn removed_note(removed: Removed) -> Option<String> {
    (removed.count > 0 && removed.seconds >= 0.5).then(|| {
        format!(
            "{} bölüm çıkarıldı · {} daha kısa",
            removed.count,
            clock(removed.seconds)
        )
    })
}

pub async fn fetch_overview(video_id: &str) -> Option<Overview> {
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(USER_AGENT)
        .build()
        .ok()?;
    let categories = serde_json::to_string(&CATEGORIES).ok()?;
    let response = client
        .get(format!("{API_URL}/{}", hash_prefix(video_id)))
        .query(&[("categories", categories)])
        .send()
        .await
        .ok()?;
    let segments = if response.status() == reqwest::StatusCode::NOT_FOUND {
        Vec::new()
    } else if response.status().is_success() {
        parse_segments(&response.text().await.ok()?, video_id)
    } else {
        return None;
    };
    Some(Overview {
        summary: summary(&segments),
        segments,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RESPONSE: &str = r#"[
        {"videoID":"other_video","segments":[{"category":"sponsor","actionType":"skip","segment":[1,2]}]},
        {"videoID":"dQw4w9WgXcQ","segments":[
            {"category":"interaction","actionType":"skip","segment":[300.5,320.5],"UUID":"b","votes":3},
            {"category":"sponsor","actionType":"skip","segment":[0,61.25],"UUID":"a","videoDuration":130.1,"locked":0},
            {"category":"sponsor","actionType":"mute","segment":[70,80]},
            {"category":"music_offtopic","actionType":"skip","segment":[90,95]},
            {"category":"selfpromo","segment":[310,330.5]},
            {"category":"sponsor","actionType":"skip","segment":[50,50]}
        ]}
    ]"#;

    #[test]
    fn lookups_use_a_four_character_hash_prefix() {
        assert_eq!(hash_prefix("abc"), "ba78");
    }

    #[test]
    fn response_parsing_picks_the_matching_video() {
        let segments = parse_segments(RESPONSE, "dQw4w9WgXcQ");
        let seen: Vec<(f64, f64, &str)> = segments
            .iter()
            .map(|s| (s.start, s.end, s.category.as_str()))
            .collect();
        assert_eq!(
            seen,
            [
                (0.0, 61.25, "sponsor"),
                (300.5, 320.5, "interaction"),
                (310.0, 330.5, "selfpromo"),
            ]
        );
        assert!(parse_segments(RESPONSE, "dQw4w9WgXc").is_empty());
        assert!(parse_segments("Not Found", "dQw4w9WgXcQ").is_empty());
        assert!(parse_segments("[]", "dQw4w9WgXcQ").is_empty());
    }

    #[test]
    fn summaries_count_overlapping_time_once() {
        let segments = parse_segments(RESPONSE, "dQw4w9WgXcQ");
        assert_eq!(summary(&segments), "3 bölüm · 1:31");
        assert_eq!(summary(&segments[..1]), "1 bölüm · 1:01");
        assert_eq!(summary(&[]), "Bölüm yok");
        assert_eq!(clock(3725.4), "1:02:05");
    }

    #[test]
    fn removal_notes() {
        let cuts = parse_cuts(
            r#"[{"start_time": 100, "end_time": 187.6, "category": "selfpromo"},
                {"start_time": 0, "end_time": 14.473, "category": "sponsor", "title": "Sponsor", "type": "skip", "_categories": [["sponsor", 0, 14.473, "Sponsor"]]}]"#,
        );
        assert_eq!(cuts, [(0.0, 14.473), (100.0, 187.6)]);
        let removed = removed_within(&cuts, 0.0, f64::INFINITY);
        assert_eq!(removed.count, 2);
        assert_eq!(removed_note(removed).as_deref(), Some("2 bölüm çıkarıldı · 1:42 daha kısa"));
        assert!(parse_cuts("null").is_empty());
        assert!(parse_cuts("NA").is_empty());
        assert_eq!(removed_note(Removed::default()), None);
    }

    #[test]
    fn overlapping_cuts_are_merged() {
        let cuts = parse_cuts(
            r#"[{"start_time": 50, "end_time": 70}, {"start_time": 10, "end_time": 20},
                {"start_time": 60, "end_time": 65}, {"start_time": 70, "end_time": 80},
                {"start_time": 30, "end_time": 30}, {"start_time": 90, "end_time": 85}]"#,
        );
        assert_eq!(cuts, [(10.0, 20.0), (50.0, 80.0)]);
    }

    #[test]
    fn removal_notes_count_only_what_the_clip_lost() {
        let cuts = [(10.0, 20.0), (50.0, 80.0), (200.0, 210.0)];
        assert_eq!(removed_within(&cuts, 0.0, 5.0), Removed::default());
        assert_eq!(removed_within(&cuts, 15.0, 60.0), Removed { count: 2, seconds: 15.0 });
        assert_eq!(removed_within(&cuts, 20.0, 50.0), Removed::default());
        assert_eq!(removed_within(&cuts, 55.0, 60.0), Removed { count: 1, seconds: 5.0 });
        assert_eq!(removed_within(&cuts, 0.0, f64::INFINITY), Removed { count: 3, seconds: 50.0 });
        assert_eq!(
            removed_note(removed_within(&cuts, 15.0, 60.0)).as_deref(),
            Some("2 bölüm çıkarıldı · 0:15 daha kısa")
        );
    }

    #[test]
    fn kept_intervals_are_the_range_minus_the_cuts() {
        let cuts = [(10.0, 20.0), (50.0, 80.0)];
        let duration = Some(100.0);
        let keep = |start, end| keep_intervals(start, end, duration, &cuts);
        assert_eq!(keep(0.0, None), [(0.0, 10.0), (20.0, 50.0), (80.0, 100.0)]);
        assert_eq!(keep(0.0, Some(5.0)), [(0.0, 5.0)]);
        assert_eq!(keep(5.0, Some(30.0)), [(5.0, 10.0), (20.0, 30.0)]);
        assert_eq!(keep(15.0, Some(60.0)), [(20.0, 50.0)]);
        assert_eq!(keep(25.0, Some(90.0)), [(25.0, 50.0), (80.0, 90.0)]);
        assert_eq!(keep(60.0, None), [(80.0, 100.0)]);
        assert_eq!(keep(30.0, Some(500.0)), [(30.0, 50.0), (80.0, 100.0)]);

        assert!(keep(12.0, Some(18.0)).is_empty());
        assert!(keep(10.0, Some(20.0)).is_empty());
        assert!(keep(55.0, Some(80.0)).is_empty());
        assert!(keep(120.0, None).is_empty());
        assert!(keep_intervals(60.0, None, Some(80.0), &cuts).is_empty());
        assert_eq!(keep_intervals(60.0, None, None, &cuts), [(80.0, f64::INFINITY)]);

        assert_eq!(keep_intervals(3.0, Some(9.5), duration, &[]), [(3.0, 9.5)]);
        assert_eq!(keep_intervals(3.0, None, None, &[]), [(3.0, f64::INFINITY)]);
        assert_eq!(keep_intervals(0.0, None, duration, &[]), [(0.0, 100.0)]);
    }

    #[test]
    fn cuts_begin_a_frame_early() {
        let led = lead_cuts(&[(0.01, 5.0), (69.17, 112.5)], 0.025);
        assert_eq!(led[0], (0.0, 5.0));
        assert!((led[1].0 - 69.145).abs() < 1e-9 && led[1].1 == 112.5);
        assert_eq!(lead_cuts(&[(69.17, 112.5)], 0.0), [(69.17, 112.5)]);
    }

    #[test]
    fn slivers_left_between_cuts_are_dropped() {
        let cuts = [(0.0, 14.5), (14.55, 30.0), (99.95, 100.0)];
        assert_eq!(keep_intervals(0.0, None, Some(100.0), &cuts), [(30.0, 99.95)]);
        assert!(keep_intervals(29.95, Some(30.05), Some(100.0), &cuts).is_empty());
        assert_eq!(
            keep_intervals(9.0, Some(40.0), Some(100.0), &[(30.0, 35.0), (5.0, 10.0)]),
            [(10.0, 30.0), (35.0, 40.0)]
        );

        let real = [(69.17, 112.5)];
        assert_eq!(
            keep_intervals(48.0, Some(411.0), Some(750.4), &real),
            [(48.0, 69.17), (112.5, 411.0)]
        );
        assert_eq!(keep_intervals(61.3, Some(76.3), Some(750.4), &[]), [(61.3, 76.3)]);
        assert_eq!(keep_intervals(0.0, None, Some(750.4), &real), [(0.0, 69.17), (112.5, 750.4)]);
    }
}
