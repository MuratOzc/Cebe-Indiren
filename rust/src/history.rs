use crate::config::{app_data_dir, write_json};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_ENTRIES: usize = 60;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub path: String,
    pub name: String,
    #[serde(default)]
    pub is_dir: bool,
    #[serde(default)]
    pub saved_at: u64,
    #[serde(flatten)]
    pub details: Details,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Details {
    pub title: String,
    pub thumbnail: String,
    pub duration: f64,
    pub size: u64,
    pub url: String,
    pub audio: bool,
}

#[derive(Debug, Serialize)]
pub struct HistoryItem {
    #[serde(flatten)]
    pub entry: HistoryEntry,
    pub exists: bool,
}

fn history_file() -> PathBuf {
    app_data_dir().join("history.json")
}

pub fn same_file(a: &str, b: &str) -> bool {
    a.replace('/', "\\").to_lowercase() == b.replace('/', "\\").to_lowercase()
}

pub fn load_history() -> Vec<HistoryEntry> {
    load_from(&history_file())
}

fn load_from(path: &Path) -> Vec<HistoryEntry> {
    let mut entries: Vec<HistoryEntry> = fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default();
    entries.retain(|e| !e.path.is_empty());
    entries.truncate(MAX_ENTRIES);
    entries
}

pub fn save_history(entries: &[HistoryEntry]) {
    write_json(&history_file(), entries);
}

#[cfg(test)]
pub fn record(entries: &mut Vec<HistoryEntry>, path: &str, name: &str, is_dir: bool) {
    record_with(entries, path, name, is_dir, Details::default());
}

pub fn record_with(
    entries: &mut Vec<HistoryEntry>,
    path: &str,
    name: &str,
    is_dir: bool,
    details: Details,
) {
    entries.retain(|e| !same_file(&e.path, path));
    entries.insert(
        0,
        HistoryEntry {
            path: path.to_string(),
            name: name.to_string(),
            is_dir,
            saved_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            details,
        },
    );
    entries.truncate(MAX_ENTRIES);
}

pub fn remove(entries: &mut Vec<HistoryEntry>, path: &str) -> bool {
    let before = entries.len();
    entries.retain(|e| !same_file(&e.path, path));
    entries.len() != before
}

pub fn items(entries: &[HistoryEntry]) -> Vec<HistoryItem> {
    entries
        .iter()
        .map(|entry| HistoryItem {
            exists: Path::new(&entry.path).exists(),
            entry: entry.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_first_without_duplicates_and_capped() {
        let mut entries = Vec::new();
        record(&mut entries, r"D:\dl\a.mp4", "a.mp4", false);
        record(&mut entries, r"D:\dl\b.mp4", "b.mp4", false);
        record(&mut entries, r"D:\dl\a.mp4", "a.mp4", false);
        let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, [r"D:\dl\a.mp4", r"D:\dl\b.mp4"]);

        for n in 0..MAX_ENTRIES + 5 {
            record(&mut entries, &format!(r"D:\dl\{n}.mp4"), "x", false);
        }
        assert_eq!(entries.len(), MAX_ENTRIES);
        assert_eq!(entries[0].path, format!(r"D:\dl\{}.mp4", MAX_ENTRIES + 4));

        let newest = entries[0].path.clone();
        assert!(remove(&mut entries, &newest));
        assert!(!remove(&mut entries, r"D:\dl\never.mp4"));
        assert_eq!(entries.len(), MAX_ENTRIES - 1);
    }

    #[test]
    fn paths_compare_like_the_file_system() {
        assert!(same_file(r"D:\DL\A.mp4", "d:/dl/a.mp4"));
        assert!(!same_file(r"D:\dl\a.mp4", r"D:\dl\b.mp4"));
    }

    #[test]
    fn entries_from_older_versions_still_load() {
        let old: Vec<HistoryEntry> =
            serde_json::from_str(r#"[{"path":"D:\\dl\\a.mp4","name":"a.mp4","is_dir":false,"saved_at":5}]"#).unwrap();
        assert_eq!(old[0].details, Details::default());
        assert_eq!(old[0].saved_at, 5);

        let mut entries = Vec::new();
        let details = Details {
            title: "A".to_string(),
            thumbnail: "https://i.x/a.jpg".to_string(),
            duration: 12.5,
            size: 2048,
            url: "https://a.b/c".to_string(),
            audio: true,
        };
        record_with(&mut entries, r"D:\dl\a.mp3", "a.mp3", false, details.clone());
        let json = serde_json::to_string(&items(&entries)).unwrap();
        let flat: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(flat[0]["title"], "A");
        assert_eq!(flat[0]["size"], 2048);
        assert_eq!(flat[0]["exists"], false);
        let back: Vec<HistoryEntry> = serde_json::from_str(&serde_json::to_string(&entries).unwrap()).unwrap();
        assert_eq!(back[0].details, details);
    }

    #[test]
    fn survives_a_round_trip_and_a_corrupt_file() {
        let file = std::env::temp_dir().join(format!("videoindiren_hist_{}.json", std::process::id()));
        let mut entries = Vec::new();
        record(&mut entries, r"D:\dl\Liste", "Liste", true);
        write_json(&file, entries.as_slice());
        assert_eq!(load_from(&file), entries);

        fs::write(&file, "{not json").unwrap();
        assert!(load_from(&file).is_empty());
        let _ = fs::remove_file(&file);
        assert!(load_from(&file).is_empty());
    }
}
