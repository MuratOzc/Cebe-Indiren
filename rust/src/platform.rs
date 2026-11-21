use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::sync::mpsc::{channel, Sender};

const ES_SYSTEM_REQUIRED: u32 = 0x0000_0001;
const ES_CONTINUOUS: u32 = 0x8000_0000;
const FILE_ATTRIBUTE_HIDDEN: u32 = 0x0000_0002;
const INVALID_FILE_ATTRIBUTES: u32 = u32::MAX;
const CF_UNICODETEXT: u32 = 13;
const MAX_CLIPBOARD_UNITS: usize = 16 * 1024;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[link(name = "kernel32")]
extern "system" {
    fn SetThreadExecutionState(flags: u32) -> u32;
    fn GetFileAttributesW(path: *const u16) -> u32;
    fn SetFileAttributesW(path: *const u16, attributes: u32) -> i32;
    fn GlobalLock(handle: *mut c_void) -> *mut c_void;
    fn GlobalUnlock(handle: *mut c_void) -> i32;
    fn GlobalSize(handle: *mut c_void) -> usize;
}

#[link(name = "user32")]
extern "system" {
    fn OpenClipboard(owner: *mut c_void) -> i32;
    fn CloseClipboard() -> i32;
    fn IsClipboardFormatAvailable(format: u32) -> i32;
    fn GetClipboardData(format: u32) -> *mut c_void;
}

/// Text currently on the clipboard, or `None` when it holds none, holds more than a few
/// links' worth, or another program has the clipboard open.
pub fn clipboard_text() -> Option<String> {
    unsafe {
        if IsClipboardFormatAvailable(CF_UNICODETEXT) == 0 || OpenClipboard(std::ptr::null_mut()) == 0 {
            return None;
        }
        let text = read_open_clipboard();
        CloseClipboard();
        text
    }
}

unsafe fn read_open_clipboard() -> Option<String> {
    let handle = GetClipboardData(CF_UNICODETEXT);
    if handle.is_null() {
        return None;
    }
    let units = GlobalSize(handle) / std::mem::size_of::<u16>();
    if units == 0 || units > MAX_CLIPBOARD_UNITS {
        return None;
    }
    let data = GlobalLock(handle) as *const u16;
    if data.is_null() {
        return None;
    }
    let wide = std::slice::from_raw_parts(data, units);
    let len = wide.iter().position(|unit| *unit == 0).unwrap_or(units);
    let text = String::from_utf16_lossy(&wide[..len]);
    GlobalUnlock(handle);
    Some(text)
}

pub trait NoConsole {
    fn no_console(&mut self) -> &mut Self;
}

impl NoConsole for std::process::Command {
    fn no_console(&mut self) -> &mut Self {
        self.creation_flags(CREATE_NO_WINDOW)
    }
}

impl NoConsole for tokio::process::Command {
    fn no_console(&mut self) -> &mut Self {
        self.creation_flags(CREATE_NO_WINDOW)
    }
}

pub fn kill_process_tree(pid: u32) {
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .no_console()
        .output();
}

pub async fn pick_folder(initial: String) -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(move || {
        let mut dialog = rfd::FileDialog::new();
        if !initial.is_empty() && Path::new(&initial).exists() {
            dialog = dialog.set_directory(&initial);
        }
        dialog
            .pick_folder()
            .map(|p| p.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| e.to_string())
}

pub fn show_in_folder(path: &Path, file_path: &str) -> Result<(), String> {
    let mut explorer = std::process::Command::new("explorer");
    if path.is_file() {
        let clean = file_path.replace('/', "\\");
        explorer.raw_arg(format!("/select,\"{clean}\""));
    } else {
        explorer.arg(path);
    }
    explorer.spawn().map(|_| ()).map_err(|e| e.to_string())
}

pub fn open_with_default_app(path: &Path) -> Result<(), String> {
    std::process::Command::new("explorer")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub struct KeepAwake {
    _release: Sender<()>,
}

impl KeepAwake {
    pub fn new() -> Self {
        let (release, wait) = channel::<()>();
        // The execution state is per-thread; only the thread that set it can clear it.
        std::thread::spawn(move || unsafe {
            SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED);
            let _ = wait.recv();
            SetThreadExecutionState(ES_CONTINUOUS);
        });
        Self { _release: release }
    }
}

pub fn hide_path(path: &Path) {
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let attrs = GetFileAttributesW(wide.as_ptr());
        if attrs != INVALID_FILE_ATTRIBUTES {
            SetFileAttributesW(wide.as_ptr(), attrs | FILE_ATTRIBUTE_HIDDEN);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::windows::fs::MetadataExt;

    #[test]
    fn hidden_directories_stay_usable() {
        let dir = std::env::temp_dir().join(format!("videoindiren_hide_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        hide_path(&dir);
        let attrs = fs::metadata(&dir).unwrap().file_attributes();
        assert_ne!(attrs & FILE_ATTRIBUTE_HIDDEN, 0);

        fs::create_dir_all(dir.join("full")).unwrap();
        fs::write(dir.join("full").join("a.part"), b"x").unwrap();
        fs::remove_dir_all(&dir).unwrap();
        assert!(!dir.exists());

        hide_path(&dir);
    }

    #[test]
    fn clipboard_can_be_read_whatever_it_holds() {
        if let Some(text) = clipboard_text() {
            assert!(text.encode_utf16().count() <= MAX_CLIPBOARD_UNITS);
        }
    }

    #[test]
    fn keep_awake_releases_on_drop() {
        drop(KeepAwake::new());
    }
}
