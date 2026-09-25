//! Settings commands for opt-in Tools (gated shell).
//!
//! Non-secret flags live in `tools.json`. Softwake keeps tools off until
//! enabled here. The daemon still confirm-gates `shell` and does not spawn
//! until confirm (or a quiet mutating-only read).

use serde::Serialize;
use softwake_tools::{FileToolsSettings, ToolsSettings, parse_confirm_policy, resolve_tools_file};

/// Snapshot returned to the window. No secrets.
#[derive(Debug, Clone, Serialize)]
pub struct ToolsSnapshot {
    /// Operator opt-in for the shell tool. Default false.
    pub shell_enabled: bool,
    /// `always`, `mutating_only`, or `allowlisted_quiet`.
    pub confirm_policy: String,
}

fn snapshot(settings: &ToolsSettings) -> ToolsSnapshot {
    ToolsSnapshot {
        shell_enabled: settings.shell_enabled,
        confirm_policy: settings.confirm_policy.as_str().to_owned(),
    }
}

fn open_settings() -> Result<FileToolsSettings, String> {
    FileToolsSettings::new(resolve_tools_file().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

fn load_snapshot() -> Result<ToolsSnapshot, String> {
    let store = open_settings()?;
    let settings = store.load().map_err(|e| e.to_string())?;
    Ok(snapshot(&settings))
}

/// Load Tools Settings.
#[tauri::command]
pub fn tools_snapshot() -> Result<ToolsSnapshot, String> {
    load_snapshot()
}

/// Save Tools Settings (enable flags and confirm policy).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // Tauri command args are owned JSON values.
pub fn tools_save(shell_enabled: bool, confirm_policy: String) -> Result<ToolsSnapshot, String> {
    let store = open_settings()?;
    let policy = parse_confirm_policy(&confirm_policy).map_err(|e| e.to_string())?;
    let settings = ToolsSettings {
        shell_enabled,
        confirm_policy: policy,
        ..ToolsSettings::default()
    };
    store.save(&settings).map_err(|e| e.to_string())?;
    Ok(snapshot(&settings))
}

#[cfg(test)]
mod tests {
    use super::snapshot;
    use softwake_tools::{ConfirmPolicy, ToolsSettings};

    #[test]
    fn snapshot_defaults_off_and_always() {
        let snap = snapshot(&ToolsSettings::default());
        assert!(!snap.shell_enabled);
        assert_eq!(snap.confirm_policy, ConfirmPolicy::Always.as_str());
    }
}
