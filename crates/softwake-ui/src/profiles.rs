//! In-process multi-profile Settings: list, create, rename, switch, edit packs.

#![allow(
    clippy::needless_pass_by_value,
    reason = "Tauri deserializes command arguments as owned values"
)]

use std::path::{Path, PathBuf};

use serde::Serialize;
use softwake_soul::{
    create_profile, ensure_migrated, list_profiles, load_app_config, profile_name_in,
    profile_pack_dir, rename_profile, resolve_config_dir, resolve_soul_dir, set_active_profile,
    try_load,
};

use crate::pack::{self, PackSnapshot};

/// One row in the Profiles list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProfileRow {
    /// Stable folder id.
    pub id: String,
    /// Agent display name.
    pub name: String,
    /// Whether this id is `softwake.json`'s active profile.
    pub active: bool,
    /// Whether the four pack files currently validate.
    pub pack_ok: bool,
}

/// Profiles pane snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProfilesSnapshot {
    /// Softwake config root (display).
    pub config_dir: String,
    /// Active profile id.
    pub active_id: String,
    /// Profile selected in the editors (may differ from active).
    pub selected_id: String,
    /// Agent name for the selected profile.
    pub selected_name: String,
    /// All profiles, sorted by id.
    pub profiles: Vec<ProfileRow>,
    /// Pack editors for the selected profile.
    pub pack: PackSnapshot,
}

fn config_dir() -> Result<PathBuf, String> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME");
    let home = std::env::var_os("HOME");
    resolve_config_dir(
        xdg.as_ref().map(PathBuf::from).as_deref(),
        home.as_ref().map(PathBuf::from).as_deref(),
    )
    .map_err(|error| error.to_string())
}

fn snapshot_at(config: &Path, selected_id: &str) -> Result<ProfilesSnapshot, String> {
    ensure_migrated(config).map_err(|error| error.to_string())?;
    let app = load_app_config(config).map_err(|error| error.to_string())?;
    let selected = if selected_id.is_empty() {
        app.active_profile.clone()
    } else {
        selected_id.to_owned()
    };
    let selected_dir = profile_pack_dir(config, &selected);
    if !selected_dir.is_dir() {
        return Err(format!("unknown profile `{selected}`"));
    }
    let profiles = list_profiles(config).map_err(|error| error.to_string())?;
    let rows = profiles
        .into_iter()
        .map(|meta| {
            let dir = profile_pack_dir(config, &meta.id);
            ProfileRow {
                active: meta.id == app.active_profile,
                pack_ok: try_load(&dir).is_ok(),
                id: meta.id,
                name: meta.name,
            }
        })
        .collect::<Vec<_>>();
    let selected_name = profile_name_in(&selected_dir);
    let pack = pack::read_pack(&selected_dir);
    Ok(ProfilesSnapshot {
        config_dir: config.display().to_string(),
        active_id: app.active_profile,
        selected_id: selected,
        selected_name,
        profiles: rows,
        pack,
    })
}

/// List profiles and load the selected (or active) pack into the editors.
#[tauri::command]
pub fn profiles_snapshot(selected_id: Option<String>) -> Result<ProfilesSnapshot, String> {
    let config = config_dir()?;
    let selected = selected_id.unwrap_or_default();
    snapshot_at(&config, &selected)
}

/// Create a profile from starter templates and select it in the editors.
#[tauri::command]
pub fn profile_create(name: String) -> Result<ProfilesSnapshot, String> {
    let config = config_dir()?;
    let meta = create_profile(&config, &name, None).map_err(|error| error.to_string())?;
    snapshot_at(&config, &meta.id)
}

/// Persist the agent name for a profile.
#[tauri::command]
pub fn profile_rename(id: String, name: String) -> Result<ProfilesSnapshot, String> {
    let config = config_dir()?;
    rename_profile(&config, &id, &name).map_err(|error| error.to_string())?;
    snapshot_at(&config, &id)
}

/// Make `id` the active profile used by the daemon when no soul override is set.
#[tauri::command]
pub fn profile_set_active(id: String) -> Result<ProfilesSnapshot, String> {
    let config = config_dir()?;
    set_active_profile(&config, &id).map_err(|error| error.to_string())?;
    snapshot_at(&config, &id)
}

/// Read pack editors for a profile id (does not change active).
#[tauri::command]
pub fn pack_snapshot(profile_id: Option<String>) -> Result<PackSnapshot, String> {
    let dir = pack_dir_for(profile_id.as_deref())?;
    Ok(pack::read_pack(&dir))
}

/// Write pack editors for a profile id.
#[tauri::command]
pub fn pack_save(
    profile_id: Option<String>,
    soul: String,
    user: String,
    rules: String,
    glossary: String,
) -> Result<PackSnapshot, String> {
    let dir = pack_dir_for(profile_id.as_deref())?;
    pack::write_pack(&dir, &soul, &user, &rules, &glossary)
}

fn pack_dir_for(profile_id: Option<&str>) -> Result<PathBuf, String> {
    match profile_id {
        Some(id) if !id.is_empty() => {
            let config = config_dir()?;
            ensure_migrated(&config).map_err(|error| error.to_string())?;
            let dir = profile_pack_dir(&config, id);
            if !dir.is_dir() {
                return Err(format!("unknown profile `{id}`"));
            }
            Ok(dir)
        }
        _ => {
            // Legacy: active profile (or flag/env override via resolve_soul_dir).
            let resolved = resolve_soul_dir(None).map_err(|error| error.to_string())?;
            Ok(resolved.path().to_path_buf())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TempHome {
        path: PathBuf,
    }

    impl TempHome {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(1);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("softwake-ui-profiles-{}-{n}", std::process::id()));
            std::fs::create_dir_all(&path).expect("temp");
            Self { path }
        }
    }

    impl Drop for TempHome {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn snapshot_at_migrates_and_lists_default() {
        let home = TempHome::new();
        let config = home.path.join(".config").join("softwake");
        // Write a legacy soul so migrate has something to copy.
        let legacy = config.join("soul");
        std::fs::create_dir_all(&legacy).expect("legacy");
        for (name, body) in [
            ("soul.md", "legacy soul\n"),
            ("user.md", "legacy user\n"),
            ("rules.md", "legacy rules\n"),
            ("glossary.md", "# Glossary\n\n"),
        ] {
            std::fs::write(legacy.join(name), body).expect("write");
        }
        let snap = snapshot_at(&config, "").expect("snap");
        assert_eq!(snap.active_id, "default");
        assert_eq!(snap.selected_id, "default");
        assert!(snap.profiles.iter().any(|row| row.id == "default"));
        assert!(snap.pack.ok);
        assert!(snap.pack.soul.contains("legacy soul"));
    }
}
