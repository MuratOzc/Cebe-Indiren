use crate::android;
use crate::state::lock;
use std::path::Path;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock};

#[derive(serde::Deserialize)]
struct Text {
    text: Option<String>,
}

fn read_text(command: &str) -> Option<String> {
    android::call::<Text>(command, serde_json::json!({})).ok()?.text
}

pub fn clipboard_text() -> Option<String> {
    read_text("readClipboard")
}

pub fn shared_text() -> Option<String> {
    read_text("takeSharedText")
}

pub trait NoConsole {
    fn no_console(&mut self) -> &mut Self;
}

impl NoConsole for std::process::Command {
    fn no_console(&mut self) -> &mut Self {
        self
    }
}

impl NoConsole for tokio::process::Command {
    fn no_console(&mut self) -> &mut Self {
        self
    }
}

pub fn kill_process_tree(pid: u32) {
    let _ = std::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .output();
}

#[derive(serde::Serialize)]
struct PathArg<'a> {
    path: &'a str,
}

#[derive(serde::Deserialize)]
struct PickedFolder {
    path: Option<String>,
}

pub async fn pick_folder(initial: String) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        android::call::<PickedFolder>("pickFolder", PathArg { path: &initial }).map(|picked| picked.path)
    })
    .await
    .map_err(|e| e.to_string())?
}

pub fn show_in_folder(_path: &Path, _file_path: &str) -> Result<(), String> {
    android::call::<serde_json::Value>("showDownloads", serde_json::json!({})).map(drop)
}

pub fn open_with_default_app(path: &Path) -> Result<(), String> {
    let path = path.to_string_lossy();
    android::call::<serde_json::Value>("openFile", PathArg { path: &path }).map(drop)
}

enum Service {
    Start,
    Progress(i32),
    Stop,
    Finished(bool, String),
}

fn service() -> &'static Mutex<Sender<Service>> {
    static SERVICE: OnceLock<Mutex<Sender<Service>>> = OnceLock::new();
    SERVICE.get_or_init(|| {
        let (send, receive) = channel::<Service>();
        std::thread::spawn(move || {
            while let Ok(mut message) = receive.recv() {
                while let Service::Progress(_) = message {
                    match receive.try_recv() {
                        Ok(next) => message = next,
                        Err(_) => break,
                    }
                }
                let (command, payload) = match message {
                    Service::Start => ("startService", serde_json::json!({})),
                    Service::Progress(percent) => ("showProgress", serde_json::json!({ "percent": percent })),
                    Service::Stop => ("stopService", serde_json::json!({})),
                    Service::Finished(ok, text) => ("notifyFinished", serde_json::json!({ "ok": ok, "text": text })),
                };
                let _ = android::call::<serde_json::Value>(command, payload);
            }
        });
        Mutex::new(send)
    })
}

fn send(message: Service) {
    let _ = lock(service()).send(message);
}

pub fn download_finished(ok: bool, text: &str) {
    send(Service::Finished(ok, text.to_string()));
}

pub fn show_progress(percent: i32) {
    send(Service::Progress(percent));
}

pub struct KeepAwake;

impl KeepAwake {
    pub fn new() -> Self {
        send(Service::Start);
        Self
    }
}

impl Drop for KeepAwake {
    fn drop(&mut self) {
        send(Service::Stop);
    }
}

pub fn hide_path(_path: &Path) {}

pub fn media_changed(paths: &[&Path]) {
    let paths: Vec<String> = paths.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    std::thread::spawn(move || {
        let _ = android::call::<serde_json::Value>("scanPaths", serde_json::json!({ "paths": paths }));
    });
}
