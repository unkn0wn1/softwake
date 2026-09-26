//! Persist Settings UI prefs under the Softwake config root.
//!
//! File: `$XDG_CONFIG_HOME/softwake/ui-prefs.json` (or `~/.config/softwake/…`).
//! Text size survives restart; missing file → default `x-small`.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use softwake_soul::resolve_config_dir;

/// Relative UI text scale for the Settings window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum TextSize {
    /// Smallest labels.
    XxSmall,
    /// Default: denser than browser medium.
    #[default]
    XSmall,
    /// Between default and medium.
    Small,
    /// Comfortable reading size.
    Medium,
    /// Largest labels.
    Large,
}

impl TextSize {
    /// Wire / `data-text-size` value.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::XxSmall => "xx-small",
            Self::XSmall => "x-small",
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Large => "large",
        }
    }

    /// Parse a control value; unknown → [`TextSize::XSmall`].
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "xx-small" => Self::XxSmall,
            "small" => Self::Small,
            "medium" => Self::Medium,
            "large" => Self::Large,
            // "x-small" and anything unknown.
            _ => Self::XSmall,
        }
    }
}

/// Softwake Settings window prefs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiPrefs {
    /// Root text scale for every Settings pane.
    #[serde(default)]
    pub text_size: TextSize,
}

impl Default for UiPrefs {
    fn default() -> Self {
        Self {
            text_size: TextSize::XSmall,
        }
    }
}

const FILE_NAME: &str = "ui-prefs.json";

fn config_dir() -> Option<PathBuf> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let home = std::env::var_os("HOME").map(PathBuf::from);
    resolve_config_dir(xdg.as_deref(), home.as_deref()).ok()
}

fn file_path() -> Option<PathBuf> {
    Some(config_dir()?.join(FILE_NAME))
}

/// Load prefs, or defaults when missing / unreadable.
#[must_use]
pub fn load() -> UiPrefs {
    let Some(path) = file_path() else {
        return UiPrefs::default();
    };
    let Ok(bytes) = fs::read(&path) else {
        return UiPrefs::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// Replace prefs on disk.
///
/// # Errors
///
/// Config path unresolved or write failure.
pub fn save(prefs: &UiPrefs) -> Result<(), String> {
    let dir = config_dir().ok_or_else(|| "Softwake config directory is unresolved".to_owned())?;
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let path = dir.join(FILE_NAME);
    let body = serde_json::to_vec_pretty(prefs).map_err(|error| error.to_string())?;
    fs::write(&path, body).map_err(|error| error.to_string())
}

/// Snapshot for the Settings General control.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiPrefsSnapshot {
    /// Current text size wire value.
    pub text_size: String,
}

impl From<&UiPrefs> for UiPrefsSnapshot {
    fn from(prefs: &UiPrefs) -> Self {
        Self {
            text_size: prefs.text_size.as_str().to_owned(),
        }
    }
}

/// Load UI prefs for the Settings window.
#[tauri::command]
pub fn ui_prefs_snapshot() -> UiPrefsSnapshot {
    UiPrefsSnapshot::from(&load())
}

/// Save UI text size (`xx-small` … `large`).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // Tauri command args are owned JSON values.
pub fn ui_prefs_set_text_size(text_size: String) -> Result<UiPrefsSnapshot, String> {
    let prefs = UiPrefs {
        text_size: TextSize::parse(&text_size),
    };
    save(&prefs)?;
    Ok(UiPrefsSnapshot::from(&prefs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_x_small() {
        assert_eq!(UiPrefs::default().text_size, TextSize::XSmall);
        assert_eq!(TextSize::default().as_str(), "x-small");
    }

    #[test]
    fn parse_round_trips() {
        for size in [
            TextSize::XxSmall,
            TextSize::XSmall,
            TextSize::Small,
            TextSize::Medium,
            TextSize::Large,
        ] {
            assert_eq!(TextSize::parse(size.as_str()), size);
        }
        assert_eq!(TextSize::parse("nope"), TextSize::XSmall);
    }

    #[test]
    fn serde_default_text_size() {
        let prefs: UiPrefs = serde_json::from_str("{}").expect("empty object");
        assert_eq!(prefs.text_size, TextSize::XSmall);
        let body = serde_json::to_string(&UiPrefs {
            text_size: TextSize::Large,
        })
        .expect("ser");
        assert!(body.contains("large"));
    }
}
