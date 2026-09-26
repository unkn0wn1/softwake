//! Settings commands for per-tool permissions.
//!
//! Non-secret modes live in `tools.json`. Each registered tool is Always allow,
//! Ask, or Deny. Shell confirm policy applies when shell is Ask. This pane does
//! not enable email sign-in.

use serde::{Deserialize, Serialize};
use softwake_tools::{
    ConfirmPolicy, FileToolsSettings, ToolPermission, ToolRegistry, ToolsSettings,
    parse_confirm_policy, parse_tool_permission, resolve_tools_file,
};

/// One registered tool as the Tools page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolRow {
    /// Registry name.
    pub name: String,
    /// Registry description.
    pub description: String,
    /// Registry floor spelling: `safe` or `confirm`.
    pub registry_floor: String,
    /// Operator mode: `always_allow`, `ask`, or `deny`.
    pub permission: String,
}

/// Snapshot returned to the window. No secrets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolsSnapshot {
    /// Document version after normalize.
    pub version: u32,
    /// `always`, `mutating_only`, or `allowlisted_quiet`.
    pub confirm_policy: String,
    /// Mirror of shell permission. True when shell is not deny.
    pub shell_enabled: bool,
    /// Every registered tool, in registry order.
    pub tools: Vec<ToolRow>,
}

/// One row in a Tools save.
#[derive(Debug, Clone, Deserialize)]
pub struct ToolPermissionUpdate {
    /// Registered tool name.
    pub name: String,
    /// `always_allow`, `ask`, or `deny`.
    pub permission: String,
}

fn snapshot(settings: &ToolsSettings) -> ToolsSnapshot {
    let tools = ToolRegistry::phase2()
        .entries()
        .iter()
        .map(|tool| ToolRow {
            name: tool.name.to_owned(),
            description: tool.description.to_owned(),
            registry_floor: tool.risk.as_str().to_owned(),
            permission: settings.permission(tool.name).as_str().to_owned(),
        })
        .collect();
    ToolsSnapshot {
        version: settings.version,
        confirm_policy: settings.confirm_policy.as_str().to_owned(),
        shell_enabled: settings.shell_enabled,
        tools,
    }
}

fn open_settings() -> Result<FileToolsSettings, String> {
    FileToolsSettings::new(resolve_tools_file().map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())
}

fn load_snapshot() -> Result<ToolsSnapshot, String> {
    let store = open_settings()?;
    let settings = store.load().map_err(|error| error.to_string())?;
    Ok(snapshot(&settings))
}

fn parse_updates(
    permissions: Vec<ToolPermissionUpdate>,
) -> Result<Vec<(String, ToolPermission)>, String> {
    let registry = ToolRegistry::phase2();
    let mut parsed = Vec::with_capacity(permissions.len());
    for update in permissions {
        if registry.lookup(&update.name).is_none() {
            return Err(format!("unknown tool: {}", update.name));
        }
        let permission =
            parse_tool_permission(&update.permission).map_err(|error| error.to_string())?;
        parsed.push((update.name, permission));
    }
    Ok(parsed)
}

fn save_settings(
    store: &FileToolsSettings,
    confirm_policy: ConfirmPolicy,
    updates: Vec<(String, ToolPermission)>,
) -> Result<ToolsSettings, String> {
    let mut settings = store.load().map_err(|error| error.to_string())?;
    settings.confirm_policy = confirm_policy;
    for (name, permission) in updates {
        settings.permissions.insert(name, permission);
    }
    settings.normalize();
    store.save(&settings).map_err(|error| error.to_string())?;
    store.load().map_err(|error| error.to_string())
}

fn set_one(
    store: &FileToolsSettings,
    name: &str,
    permission: ToolPermission,
) -> Result<ToolsSettings, String> {
    if ToolRegistry::phase2().lookup(name).is_none() {
        return Err(format!("unknown tool: {name}"));
    }
    let mut settings = store.load().map_err(|error| error.to_string())?;
    settings.permissions.insert(name.to_owned(), permission);
    settings.normalize();
    store.save(&settings).map_err(|error| error.to_string())?;
    store.load().map_err(|error| error.to_string())
}

/// Load Tools Settings.
#[tauri::command]
pub fn tools_snapshot() -> Result<ToolsSnapshot, String> {
    load_snapshot()
}

/// Save confirm policy and the supplied tool modes.
///
/// An unknown name returns an error and does not write. A registered name
/// missing from `permissions` keeps the value already on disk.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // Tauri command args are owned JSON values.
pub fn tools_save(
    confirm_policy: String,
    permissions: Vec<ToolPermissionUpdate>,
) -> Result<ToolsSnapshot, String> {
    let policy = parse_confirm_policy(&confirm_policy).map_err(|error| error.to_string())?;
    let updates = parse_updates(permissions)?;
    let store = open_settings()?;
    let settings = save_settings(&store, policy, updates)?;
    Ok(snapshot(&settings))
}

/// Load, change one tool, and save. Other tools and confirm policy stay.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // Tauri command args are owned JSON values.
pub fn tools_set_permission(name: String, permission: String) -> Result<ToolsSnapshot, String> {
    let permission = parse_tool_permission(&permission).map_err(|error| error.to_string())?;
    let store = open_settings()?;
    let settings = set_one(&store, &name, permission)?;
    Ok(snapshot(&settings))
}

#[cfg(test)]
mod tests {
    use super::{set_one, snapshot};
    use softwake_tools::{
        ConfirmPolicy, FileToolsSettings, ToolPermission, ToolRegistry, ToolsSettings,
    };

    fn temp_store() -> (std::path::PathBuf, FileToolsSettings) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let dir =
            std::env::temp_dir().join(format!("softwake-ui-tools-{}-{nanos}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let store = FileToolsSettings::new(dir.join("tools.json")).expect("store");
        (dir, store)
    }

    #[test]
    fn snapshot_defaults_list_every_registered_tool() {
        let snap = snapshot(&ToolsSettings::default());
        assert_eq!(snap.version, 2);
        assert!(!snap.shell_enabled);
        assert_eq!(snap.confirm_policy, ConfirmPolicy::Always.as_str());
        let registry = ToolRegistry::phase2();
        assert_eq!(snap.tools.len(), registry.entries().len());
        for (row, tool) in snap.tools.iter().zip(registry.entries()) {
            assert_eq!(row.name, tool.name);
            assert_eq!(row.description, tool.description);
            assert_eq!(row.registry_floor, tool.risk.as_str());
            assert_eq!(
                row.permission,
                ToolsSettings::default().permission(tool.name).as_str()
            );
        }
    }

    #[test]
    fn set_one_permission_preserves_policy_and_the_other_tools() {
        let (dir, store) = temp_store();
        let settings = ToolsSettings {
            confirm_policy: ConfirmPolicy::MutatingOnly,
            ..ToolsSettings::default()
        };
        store.save(&settings).expect("seed");
        let updated = set_one(&store, "notify", ToolPermission::AlwaysAllow).expect("set");
        assert_eq!(updated.confirm_policy, ConfirmPolicy::MutatingOnly);
        assert_eq!(updated.permission("notify"), ToolPermission::AlwaysAllow);
        assert_eq!(updated.permission("echo"), ToolPermission::AlwaysAllow);
        assert_eq!(updated.permission("email_send"), ToolPermission::Ask);
        assert_eq!(updated.permission("shell"), ToolPermission::Deny);
        assert!(!updated.shell_enabled);
        let snap = snapshot(&updated);
        assert_eq!(snap.confirm_policy, "mutating_only");
        assert!(
            snap.tools
                .iter()
                .any(|row| row.name == "notify" && row.permission == "always_allow")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn unknown_tool_name_is_rejected_before_write() {
        let (dir, store) = temp_store();
        store.save(&ToolsSettings::default()).expect("seed");
        let before = std::fs::read(store.path()).expect("before");
        let rejected = super::parse_updates(vec![super::ToolPermissionUpdate {
            name: "volume".to_owned(),
            permission: "ask".to_owned(),
        }]);
        assert!(
            rejected
                .expect_err("volume")
                .contains("unknown tool: volume")
        );
        assert_eq!(std::fs::read(store.path()).expect("unchanged"), before);
        let _ = std::fs::remove_dir_all(dir);
    }
}
