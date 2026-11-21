use std::path::Path;

pub fn clipboard_text() -> Option<String> {
    None
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

const NOT_YET: &str = "Bu özellik Android'de henüz yok.";

pub async fn pick_folder(_initial: String) -> Result<Option<String>, String> {
    Err(NOT_YET.to_string())
}

pub fn show_in_folder(_path: &Path, _file_path: &str) -> Result<(), String> {
    Err(NOT_YET.to_string())
}

pub fn open_with_default_app(_path: &Path) -> Result<(), String> {
    Err(NOT_YET.to_string())
}

pub struct KeepAwake;

impl KeepAwake {
    pub fn new() -> Self {
        Self
    }
}

pub fn hide_path(_path: &Path) {}
