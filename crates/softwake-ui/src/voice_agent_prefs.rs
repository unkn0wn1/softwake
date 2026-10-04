//! Settings → Providers (xAI only) Voice Agent continuous S2S toggle.
//!
//! Persist `voice_agent_s2s` in `softwake.json`, then IPC `reload_voice_agent`
//! so the running daemon starts or stops the realtime session without restart.

use std::fmt::Write;

use serde::{Deserialize, Serialize};
use softwake_ipc::{Client, resolve_socket_path};
use softwake_soul::{load_app_config, resolve_config_dir, set_voice_agent_s2s};

/// Snapshot for the General pane.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VoiceAgentS2sSnapshot {
    /// Whether continuous S2S is enabled in softwake.json.
    pub enabled: bool,
    /// Whether the running daemon accepted a live reload.
    pub live_applied: bool,
    /// Short status line for the pane.
    pub message: String,
}

fn config_dir() -> Result<std::path::PathBuf, String> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from);
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    resolve_config_dir(xdg.as_deref(), home.as_deref()).map_err(|error| error.to_string())
}

fn snapshot_from(enabled: bool, live_applied: bool, message: String) -> VoiceAgentS2sSnapshot {
    VoiceAgentS2sSnapshot {
        enabled,
        live_applied,
        message,
    }
}

/// Read `voice_agent_s2s` from `dir/softwake.json`.
///
/// A missing file is off. A parse error or an unreadable path is an error so
/// the UI does not paint the checkbox off and then save that.
pub(crate) fn read_voice_agent_s2s_enabled(dir: &std::path::Path) -> Result<bool, String> {
    match load_app_config(dir) {
        Ok(app) => Ok(app.voice_agent_s2s),
        Err(error) => Err(error.to_string()),
    }
}

/// Load the current toggle from softwake.json (default off).
#[tauri::command]
pub fn voice_agent_s2s_snapshot() -> Result<VoiceAgentS2sSnapshot, String> {
    let dir = config_dir()?;
    let enabled = read_voice_agent_s2s_enabled(&dir)?;
    Ok(snapshot_from(
        enabled,
        false,
        if enabled {
            "Voice Agent S2S on (file)".to_owned()
        } else {
            "Voice Agent S2S off (default STT→chat→TTS)".to_owned()
        },
    ))
}

/// Persist the toggle and live-reload the daemon bridge.
#[tauri::command]
pub fn voice_agent_s2s_set(enabled: bool) -> Result<VoiceAgentS2sSnapshot, String> {
    let dir = config_dir()?;
    let app = set_voice_agent_s2s(&dir, enabled).map_err(|error| error.to_string())?;

    let mut live_applied = false;
    let mut message = if app.voice_agent_s2s {
        "Saved Voice Agent S2S on".to_owned()
    } else {
        "Saved Voice Agent S2S off".to_owned()
    };
    match (|| -> Result<(), String> {
        let path = resolve_socket_path(None).map_err(|error| error.to_string())?;
        let mut client = Client::connect(&path).map_err(|error| error.to_string())?;
        client
            .reload_voice_agent()
            .map_err(|error| error.to_string())?;
        Ok(())
    })() {
        Ok(()) => {
            live_applied = true;
            message.push_str(" — applied live");
        }
        Err(error) => {
            let _ = write!(message, " — saved; live apply skipped ({error})");
        }
    }

    Ok(snapshot_from(app.voice_agent_s2s, live_applied, message))
}

#[cfg(test)]
mod tests {
    use super::read_voice_agent_s2s_enabled;
    use softwake_soul::APP_CONFIG_FILE_NAME;

    #[test]
    fn snapshot_read_errors_on_corrupt_or_unreadable_config() {
        let root = std::env::temp_dir().join(format!(
            "sw-s2s-snap-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        std::fs::create_dir_all(&root).expect("dir");
        let missing = read_voice_agent_s2s_enabled(&root).expect("missing file is off");
        assert!(!missing);

        let path = root.join(APP_CONFIG_FILE_NAME);
        std::fs::write(&path, b"{not json").expect("corrupt");
        let corrupt = read_voice_agent_s2s_enabled(&root).expect_err("corrupt json");
        assert!(!corrupt.is_empty(), "{corrupt}");

        std::fs::remove_file(&path).expect("remove file");
        std::fs::create_dir(&path).expect("directory named softwake.json");
        let unreadable = read_voice_agent_s2s_enabled(&root).expect_err("directory");
        assert!(!unreadable.is_empty(), "{unreadable}");

        let _ = std::fs::remove_dir_all(&root);
    }
}
