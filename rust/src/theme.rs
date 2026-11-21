use serde::{Deserialize, Serialize};
use tauri::window::Color;
use tauri::{Theme, WebviewWindow};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    Light,
    Dark,
    #[default]
    #[serde(other)]
    System,
}

impl ThemeMode {
    pub fn native(self) -> Option<Theme> {
        match self {
            Self::Light => Some(Theme::Light),
            Self::Dark => Some(Theme::Dark),
            Self::System => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
            Self::System => "system",
        }
    }
}

// Must match --bg of each theme in frontend/styles.css, or the window flashes on startup and resize.
pub fn background(theme: Theme) -> Color {
    match theme {
        Theme::Light => Color(0xf6, 0xf6, 0xf7, 0xff),
        _ => Color(0x17, 0x17, 0x1a, 0xff),
    }
}

pub fn init_script(mode: ThemeMode) -> String {
    format!("window.__APP_THEME__ = '{}';", mode.as_str())
}

pub fn apply_to_window(window: &WebviewWindow, mode: ThemeMode) {
    let _ = window.set_theme(mode.native());
    let resolved = mode
        .native()
        .or_else(|| window.theme().ok())
        .unwrap_or(Theme::Dark);
    let _ = window.set_background_color(Some(background(resolved)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SavedConfig;

    #[test]
    fn theme_is_optional_and_tolerates_unknown_values() {
        let old: SavedConfig = serde_json::from_str(r#"{"download_dir":null}"#).unwrap();
        assert_eq!(old.theme, ThemeMode::System);

        let light: SavedConfig =
            serde_json::from_str(r#"{"download_dir":null,"theme":"light"}"#).unwrap();
        assert_eq!(light.theme, ThemeMode::Light);

        let odd: SavedConfig =
            serde_json::from_str(r#"{"download_dir":null,"theme":"sepia"}"#).unwrap();
        assert_eq!(odd.theme, ThemeMode::System);
    }

    #[test]
    fn init_script_uses_the_serialized_name() {
        for mode in [ThemeMode::System, ThemeMode::Light, ThemeMode::Dark] {
            let name = serde_json::to_string(&mode).unwrap();
            assert_eq!(
                init_script(mode),
                format!("window.__APP_THEME__ = '{}';", name.trim_matches('"'))
            );
        }
    }
}
