//! Persist HUD capsule position under the Softwake config root.
//!
//! File: `$XDG_CONFIG_HOME/softwake/hud-position.json` (or `~/.config/softwake/…`).
//! When present, expand/collapse keeps the window's bottom-right corner (the
//! parked bloom) and rewrites this top-left. It does not jump back to primary
//! bottom-right.

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

/// Top-left that keeps `(old_x + old_w, old_y + old_h)` as the bottom-right corner.
#[must_use]
pub const fn pin_bottom_right(
    old_x: f64,
    old_y: f64,
    old_w: f64,
    old_h: f64,
    new_w: f64,
    new_h: f64,
) -> (f64, f64) {
    (old_x + old_w - new_w, old_y + old_h - new_h)
}

/// A monitor work area in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorkArea {
    /// Left edge.
    pub origin_x: f64,
    /// Top edge.
    pub origin_y: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

/// Clamp a top-left into `area`. A window larger than the area sits on the origin.
#[must_use]
pub const fn clamp_to_work_area(
    area: WorkArea,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> (f64, f64) {
    let max_x = (area.origin_x + area.width - width).max(area.origin_x);
    let max_y = (area.origin_y + area.height - height).max(area.origin_y);
    (x.clamp(area.origin_x, max_x), y.clamp(area.origin_y, max_y))
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

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "positions are exact dyadic rationals")]
mod tests {
    use super::{WorkArea, clamp_to_work_area, pin_bottom_right};

    #[test]
    fn resize_keeps_the_bottom_right_corner() {
        let (x, y) = pin_bottom_right(800.0, 600.0, 120.0, 120.0, 400.0, 480.0);
        assert_eq!((x, y), (520.0, 240.0));
        assert_eq!(x + 400.0, 800.0 + 120.0);
        assert_eq!(y + 480.0, 600.0 + 120.0);
    }

    #[test]
    fn clamp_pulls_an_expanded_panel_back_into_the_work_area() {
        let small = WorkArea {
            origin_x: 0.0,
            origin_y: 0.0,
            width: 800.0,
            height: 600.0,
        };
        let (x, y) = clamp_to_work_area(small, -40.0, -80.0, 400.0, 480.0);
        assert_eq!((x, y), (0.0, 0.0));
        let large = WorkArea {
            origin_x: 0.0,
            origin_y: 0.0,
            width: 1280.0,
            height: 800.0,
        };
        let (x, y) = clamp_to_work_area(large, 2000.0, 2000.0, 400.0, 480.0);
        assert_eq!((x, y), (880.0, 320.0));
    }
}
