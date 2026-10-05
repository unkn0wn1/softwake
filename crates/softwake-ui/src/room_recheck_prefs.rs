//! Settings → Rooms toggle for queued-reply re-check.
//!
//! Persist `room_requeue_recheck` in `softwake.json`. The next room fan-out
//! reads the file. There is no live reload.

use serde::{Deserialize, Serialize};
use softwake_soul::{load_app_config, resolve_config_dir, set_room_requeue_recheck};

/// Snapshot for the Rooms pane checkbox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomRecheckSnapshot {
    /// Whether queued replies may be revised before they play.
    pub enabled: bool,
    /// Short status line for the pane.
    pub message: String,
}

fn config_dir() -> Result<std::path::PathBuf, String> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from);
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    resolve_config_dir(xdg.as_deref(), home.as_deref()).map_err(|error| error.to_string())
}

/// Read `room_requeue_recheck` from `dir/softwake.json`.
///
/// A missing file is on. A parse error is returned so the checkbox is not
/// saved from a guess.
pub(crate) fn read_room_requeue_recheck(dir: &std::path::Path) -> Result<bool, String> {
    match load_app_config(dir) {
        Ok(app) => Ok(app.room_requeue_recheck),
        Err(error) => Err(error.to_string()),
    }
}

fn message_for(enabled: bool) -> String {
    if enabled {
        "Queued replies can be revised once before they play".to_owned()
    } else {
        "Queued replies stay as first drafted".to_owned()
    }
}

/// Load the current toggle from softwake.json (default on).
#[tauri::command]
pub fn room_requeue_recheck_snapshot() -> Result<RoomRecheckSnapshot, String> {
    let dir = config_dir()?;
    let enabled = read_room_requeue_recheck(&dir)?;
    Ok(RoomRecheckSnapshot {
        enabled,
        message: message_for(enabled),
    })
}

/// Persist the toggle. The running daemon picks it up on the next fan-out.
#[tauri::command]
pub fn room_requeue_recheck_set(enabled: bool) -> Result<RoomRecheckSnapshot, String> {
    let dir = config_dir()?;
    let app = set_room_requeue_recheck(&dir, enabled).map_err(|error| error.to_string())?;
    Ok(RoomRecheckSnapshot {
        enabled: app.room_requeue_recheck,
        message: message_for(app.room_requeue_recheck),
    })
}

#[cfg(test)]
mod tests {
    use super::read_room_requeue_recheck;
    use softwake_soul::{APP_CONFIG_FILE_NAME, set_room_requeue_recheck};

    #[test]
    fn missing_file_is_on_and_false_round_trips() {
        let root = std::env::temp_dir().join(format!(
            "sw-room-recheck-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos())
        ));
        std::fs::create_dir_all(&root).expect("dir");
        let missing = read_room_requeue_recheck(&root).expect("missing file is on");
        assert!(missing);

        let saved = set_room_requeue_recheck(&root, false).expect("save");
        assert!(!saved.room_requeue_recheck);
        assert!(!read_room_requeue_recheck(&root).expect("read"));

        let path = root.join(APP_CONFIG_FILE_NAME);
        std::fs::write(&path, b"{not json").expect("corrupt");
        let corrupt = read_room_requeue_recheck(&root).expect_err("corrupt json");
        assert!(!corrupt.is_empty(), "{corrupt}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
