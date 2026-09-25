//! Persist HUD capsule position under the Softwake config root.
//!
//! File: `$XDG_CONFIG_HOME/softwake/hud-position.json` (or `~/.config/softwake/…`).
//! When present, expand/collapse resizes without re-anchoring to bottom-right.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use softwake_soul::resolve_config_dir;

/// Logical top-left of a user-placed HUD.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HudPosition {
    /// Logical X.
    pub x: f64,
    /// Logical Y.
    pub y: f64,
}

const FILE_NAME: &str = "hud-position.json";

fn config_dir() -> Option<PathBuf> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let home = std::env::var_os("HOME").map(PathBuf::from);
    resolve_config_dir(xdg.as_deref(), home.as_deref()).ok()
}

fn file_path() -> Option<PathBuf> {
    Some(config_dir()?.join(FILE_NAME))
}

/// Load a saved HUD position, if any.
#[must_use]
pub fn load() -> Option<HudPosition> {
    let path = file_path()?;
    let bytes = fs::read(&path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Remember where the operator dragged the capsule.
///
/// # Errors
///
/// Config path unresolved or write failure.
pub fn save(position: HudPosition) -> Result<(), String> {
    let dir = config_dir().ok_or_else(|| "Softwake config directory is unresolved".to_owned())?;
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let path = dir.join(FILE_NAME);
    let body = serde_json::to_vec_pretty(&position).map_err(|error| error.to_string())?;
    fs::write(&path, body).map_err(|error| error.to_string())
}

/// Forget a user placement so the next layout uses primary bottom-right.
///
/// # Errors
///
/// Delete failure other than not-found.
#[allow(dead_code)]
pub fn clear() -> Result<(), String> {
    let Some(path) = file_path() else {
        return Ok(());
    };
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}
