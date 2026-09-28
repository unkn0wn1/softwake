//! Persist Settings UI prefs under the Softwake config root.
//!
//! File: `$XDG_CONFIG_HOME/softwake/ui-prefs.json` (or `~/.config/softwake/…`).
//! Text size survives restart; missing file → default `x-small`.
//! HUD idle collapse survives restart; missing field → 3000 ms.
//! HUD pin and expanded size survive restart when present.

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

/// Default HUD idle before collapse, in milliseconds (3 seconds).
pub const HUD_IDLE_COLLAPSE_DEFAULT_MS: u32 = 3_000;
/// Shortest idle the Settings control can store (1 second).
pub const HUD_IDLE_COLLAPSE_MIN_MS: u32 = 1_000;
/// Longest idle the Settings control can store (30 seconds).
pub const HUD_IDLE_COLLAPSE_MAX_MS: u32 = 30_000;

const fn default_hud_idle_collapse_ms() -> u32 {
    HUD_IDLE_COLLAPSE_DEFAULT_MS
}

/// Clamp `hud_idle_collapse_ms` into [`HUD_IDLE_COLLAPSE_MIN_MS`]..=[`HUD_IDLE_COLLAPSE_MAX_MS`].
#[must_use]
pub fn clamp_hud_idle_collapse_ms(ms: u32) -> u32 {
    ms.clamp(HUD_IDLE_COLLAPSE_MIN_MS, HUD_IDLE_COLLAPSE_MAX_MS)
}

/// Default expanded HUD width (logical pixels).
pub const HUD_EXPANDED_W_DEFAULT: u32 = 520;
/// Default expanded HUD height (logical pixels).
pub const HUD_EXPANDED_H_DEFAULT: u32 = 620;
/// Smallest resizable expanded width.
pub const HUD_EXPANDED_W_MIN: u32 = 360;
/// Largest resizable expanded width.
pub const HUD_EXPANDED_W_MAX: u32 = 1200;
/// Smallest resizable expanded height.
pub const HUD_EXPANDED_H_MIN: u32 = 420;
/// Largest resizable expanded height.
pub const HUD_EXPANDED_H_MAX: u32 = 1200;

/// Default expanded HUD opacity percent (matches prior CSS ~0.55).
pub const HUD_OPACITY_DEFAULT: u8 = 55;
/// Minimum expanded HUD opacity percent.
pub const HUD_OPACITY_MIN: u8 = 35;
/// Maximum expanded HUD opacity percent.
pub const HUD_OPACITY_MAX: u8 = 100;

const fn default_hud_bg_opacity() -> u8 {
    HUD_OPACITY_DEFAULT
}

const fn default_hud_mic_muted_pref() -> bool {
    false
}

/// Clamp opacity percent into [`HUD_OPACITY_MIN`]..=[`HUD_OPACITY_MAX`].
#[must_use]
pub fn clamp_hud_opacity(percent: u8) -> u8 {
    percent.clamp(HUD_OPACITY_MIN, HUD_OPACITY_MAX)
}

const fn default_hud_expanded_w() -> u32 {
    HUD_EXPANDED_W_DEFAULT
}

const fn default_hud_expanded_h() -> u32 {
    HUD_EXPANDED_H_DEFAULT
}

const fn default_hud_pinned() -> bool {
    false
}

/// Clamp expanded width into [`HUD_EXPANDED_W_MIN`]..=[`HUD_EXPANDED_W_MAX`].
#[must_use]
pub fn clamp_hud_expanded_w(width: u32) -> u32 {
    width.clamp(HUD_EXPANDED_W_MIN, HUD_EXPANDED_W_MAX)
}

/// Clamp expanded height into [`HUD_EXPANDED_H_MIN`]..=[`HUD_EXPANDED_H_MAX`].
#[must_use]
pub fn clamp_hud_expanded_h(height: u32) -> u32 {
    height.clamp(HUD_EXPANDED_H_MIN, HUD_EXPANDED_H_MAX)
}

/// Softwake Settings window prefs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiPrefs {
    /// Root text scale for every Settings pane.
    #[serde(default)]
    pub text_size: TextSize,
    /// How long the pointer can stay outside the expanded HUD before it
    /// collapses, in milliseconds.
    #[serde(default = "default_hud_idle_collapse_ms")]
    pub hud_idle_collapse_ms: u32,
    /// When true, the expanded HUD stays open (idle collapse is skipped).
    #[serde(default = "default_hud_pinned")]
    pub hud_pinned: bool,
    /// Last expanded HUD width in logical pixels.
    #[serde(default = "default_hud_expanded_w")]
    pub hud_expanded_w: u32,
    /// Last expanded HUD height in logical pixels.
    #[serde(default = "default_hud_expanded_h")]
    pub hud_expanded_h: u32,
    /// Expanded HUD background opacity percent (35..=100).
    #[serde(default = "default_hud_bg_opacity")]
    pub hud_opacity: u8,
    /// HUD mic mute preference (daemon latch restored on HUD load).
    #[serde(default = "default_hud_mic_muted_pref")]
    pub hud_mic_muted: bool,
}

impl Default for UiPrefs {
    fn default() -> Self {
        Self {
            text_size: TextSize::XSmall,
            hud_idle_collapse_ms: HUD_IDLE_COLLAPSE_DEFAULT_MS,
            hud_pinned: false,
            hud_expanded_w: HUD_EXPANDED_W_DEFAULT,
            hud_expanded_h: HUD_EXPANDED_H_DEFAULT,
            hud_opacity: HUD_OPACITY_DEFAULT,
            hud_mic_muted: false,
        }
    }
}

/// Apply clamps after a deserialize so a hand-edited file cannot go out of range.
#[must_use]
pub fn normalize(mut prefs: UiPrefs) -> UiPrefs {
    prefs.hud_idle_collapse_ms = clamp_hud_idle_collapse_ms(prefs.hud_idle_collapse_ms);
    prefs.hud_expanded_w = clamp_hud_expanded_w(prefs.hud_expanded_w);
    prefs.hud_expanded_h = clamp_hud_expanded_h(prefs.hud_expanded_h);
    prefs.hud_opacity = clamp_hud_opacity(prefs.hud_opacity);
    prefs
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
    normalize(serde_json::from_slice(&bytes).unwrap_or_default())
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
    /// HUD idle collapse delay in milliseconds (already clamped).
    pub hud_idle_collapse_ms: u32,
    /// Whether the expanded HUD is pinned open.
    pub hud_pinned: bool,
    /// Expanded HUD width (logical pixels, clamped).
    pub hud_expanded_w: u32,
    /// Expanded HUD height (logical pixels, clamped).
    pub hud_expanded_h: u32,
    /// Expanded HUD opacity percent (clamped).
    pub hud_opacity: u8,
    /// Persisted mic mute preference.
    pub hud_mic_muted: bool,
}

impl From<&UiPrefs> for UiPrefsSnapshot {
    fn from(prefs: &UiPrefs) -> Self {
        Self {
            text_size: prefs.text_size.as_str().to_owned(),
            hud_idle_collapse_ms: clamp_hud_idle_collapse_ms(prefs.hud_idle_collapse_ms),
            hud_pinned: prefs.hud_pinned,
            hud_expanded_w: clamp_hud_expanded_w(prefs.hud_expanded_w),
            hud_expanded_h: clamp_hud_expanded_h(prefs.hud_expanded_h),
            hud_opacity: clamp_hud_opacity(prefs.hud_opacity),
            hud_mic_muted: prefs.hud_mic_muted,
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
    let mut prefs = load();
    prefs.text_size = TextSize::parse(&text_size);
    save(&prefs)?;
    Ok(UiPrefsSnapshot::from(&prefs))
}

/// Save how long the expanded HUD waits with the pointer outside before collapsing.
///
/// `ms` is clamped to 1000..=30000. The HUD re-reads `ui-prefs.json`; the daemon
/// is not involved.
///
/// # Errors
///
/// Config path unresolved or write failure.
#[tauri::command]
pub fn ui_prefs_set_hud_idle_collapse_ms(ms: u32) -> Result<UiPrefsSnapshot, String> {
    let mut prefs = load();
    prefs.hud_idle_collapse_ms = clamp_hud_idle_collapse_ms(ms);
    save(&prefs)?;
    Ok(UiPrefsSnapshot::from(&prefs))
}

/// Save whether the expanded HUD stays pinned open (skips idle collapse).
///
/// # Errors
///
/// Config path unresolved or write failure.
#[tauri::command]
pub fn ui_prefs_set_hud_pinned(pinned: bool) -> Result<UiPrefsSnapshot, String> {
    let mut prefs = load();
    prefs.hud_pinned = pinned;
    save(&prefs)?;
    Ok(UiPrefsSnapshot::from(&prefs))
}

/// Save the expanded HUD logical size after a resize.
///
/// Width and height are clamped. Collapsed bloom size is unchanged.
///
/// # Errors
///
/// Config path unresolved or write failure.
#[tauri::command]
pub fn ui_prefs_set_hud_expanded_size(width: u32, height: u32) -> Result<UiPrefsSnapshot, String> {
    let mut prefs = load();
    prefs.hud_expanded_w = clamp_hud_expanded_w(width);
    prefs.hud_expanded_h = clamp_hud_expanded_h(height);
    save(&prefs)?;
    Ok(UiPrefsSnapshot::from(&prefs))
}

/// Save expanded HUD opacity percent (clamped 35..=100).
#[tauri::command]
pub fn ui_prefs_set_hud_opacity(percent: u8) -> Result<UiPrefsSnapshot, String> {
    let mut prefs = load();
    prefs.hud_opacity = clamp_hud_opacity(percent);
    save(&prefs)?;
    Ok(UiPrefsSnapshot::from(&prefs))
}

/// Persist HUD mic mute preference (daemon latch is separate via `SetMicMute`).
#[tauri::command]
pub fn ui_prefs_set_hud_mic_muted(muted: bool) -> Result<UiPrefsSnapshot, String> {
    let mut prefs = load();
    prefs.hud_mic_muted = muted;
    save(&prefs)?;
    Ok(UiPrefsSnapshot::from(&prefs))
}

#[cfg(test)]
mod tests {
    #[test]
    fn opacity_clamps_and_defaults() {
        assert_eq!(super::clamp_hud_opacity(10), super::HUD_OPACITY_MIN);
        assert_eq!(super::clamp_hud_opacity(200), super::HUD_OPACITY_MAX);
        assert_eq!(
            super::UiPrefs::default().hud_opacity,
            super::HUD_OPACITY_DEFAULT
        );
        assert!(!super::UiPrefs::default().hud_mic_muted);
    }

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
            hud_idle_collapse_ms: HUD_IDLE_COLLAPSE_DEFAULT_MS,
            ..UiPrefs::default()
        })
        .expect("ser");
        assert!(body.contains("large"));
    }

    #[test]
    fn idle_defaults_when_the_field_is_absent() {
        let prefs: UiPrefs = serde_json::from_str(r#"{"text_size":"large"}"#).expect("parse");
        assert_eq!(prefs.text_size, TextSize::Large);
        assert_eq!(prefs.hud_idle_collapse_ms, HUD_IDLE_COLLAPSE_DEFAULT_MS);
        assert_eq!(UiPrefs::default().hud_idle_collapse_ms, 3_000);
    }

    #[test]
    fn idle_clamps_to_one_through_thirty_seconds() {
        assert_eq!(clamp_hud_idle_collapse_ms(0), 1_000);
        assert_eq!(clamp_hud_idle_collapse_ms(1_000), 1_000);
        assert_eq!(clamp_hud_idle_collapse_ms(3_000), 3_000);
        assert_eq!(clamp_hud_idle_collapse_ms(30_000), 30_000);
        assert_eq!(clamp_hud_idle_collapse_ms(99_000), 30_000);
        let low = normalize(UiPrefs {
            hud_idle_collapse_ms: 50,
            ..UiPrefs::default()
        });
        assert_eq!(low.hud_idle_collapse_ms, 1_000);
        let wire = UiPrefsSnapshot::from(&low);
        assert_eq!(wire.hud_idle_collapse_ms, 1_000);
    }

    #[test]
    fn pin_and_size_default_when_absent() {
        let prefs: UiPrefs = serde_json::from_str(r#"{"text_size":"large"}"#).expect("parse");
        assert!(!prefs.hud_pinned);
        assert_eq!(prefs.hud_expanded_w, HUD_EXPANDED_W_DEFAULT);
        assert_eq!(prefs.hud_expanded_h, HUD_EXPANDED_H_DEFAULT);
        let wire = UiPrefsSnapshot::from(&prefs);
        assert!(!wire.hud_pinned);
        assert_eq!(wire.hud_expanded_w, HUD_EXPANDED_W_DEFAULT);
        assert_eq!(wire.hud_expanded_h, HUD_EXPANDED_H_DEFAULT);
    }

    #[test]
    fn expanded_size_clamps() {
        assert_eq!(clamp_hud_expanded_w(10), HUD_EXPANDED_W_MIN);
        assert_eq!(clamp_hud_expanded_w(9999), HUD_EXPANDED_W_MAX);
        assert_eq!(clamp_hud_expanded_h(10), HUD_EXPANDED_H_MIN);
        assert_eq!(clamp_hud_expanded_h(9999), HUD_EXPANDED_H_MAX);
        let normalized = normalize(UiPrefs {
            hud_expanded_w: 1,
            hud_expanded_h: 99999,
            hud_pinned: true,
            ..UiPrefs::default()
        });
        assert!(normalized.hud_pinned);
        assert_eq!(normalized.hud_expanded_w, HUD_EXPANDED_W_MIN);
        assert_eq!(normalized.hud_expanded_h, HUD_EXPANDED_H_MAX);
    }
}
