//! In-process multi-profile Settings: list, create, rename, switch, edit packs.

#![allow(
    clippy::needless_pass_by_value,
    reason = "Tauri deserializes command arguments as owned values"
)]

use std::path::{Path, PathBuf};

use serde::Serialize;
use softwake_ipc::{Client, resolve_socket_path};
use softwake_soul::{
    create_profile, ensure_migrated, list_profiles, load_app_config, load_profile_meta,
    profile_name_in, profile_owner_from_pack_dir, profile_pack_dir, rename_profile,
    resolve_config_dir, resolve_main_profile_id, resolve_soul_dir, set_active_profile,
    set_allow_all, set_global_doc_flags, set_profile_role, set_profile_tts_voice,
    try_load_effective,
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
#[allow(
    clippy::struct_excessive_bools,
    reason = "wire snapshot: is_main plus three independent use_global flags"
)]
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
    /// Pack editors for the selected profile (own files, not the global preview).
    pub pack: PackSnapshot,
    /// True when the selected profile owns the global docs.
    pub is_main: bool,
    /// Main profile id (`default`, or the first id if that folder is gone).
    pub main_id: String,
    /// `use_global_user` for the selected profile. Main reports true.
    pub use_global_user: bool,
    /// `use_global_glossary` for the selected profile. Main reports true.
    pub use_global_glossary: bool,
    /// `use_global_rules` for the selected profile. Main reports true.
    pub use_global_rules: bool,
    /// Profile `allow_all` (ADR-0052): Ask tools auto-run except `software_install`.
    pub allow_all: bool,
    /// Profile role (`general` or `coding`).
    pub role: String,
    /// Saved TTS voice for the selected profile. Empty means Default (Eve).
    pub tts_voice: String,
    /// Main `user.md` body for the read-only preview.
    pub global_user: String,
    /// Main `rules.md` body for the read-only preview.
    pub global_rules: String,
    /// Main `glossary.md` body for the read-only preview.
    pub global_glossary: String,
}

/// Global pane: the main profile's user, rules, and glossary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GlobalDocsSnapshot {
    /// Main profile id.
    pub main_id: String,
    /// Pack directory.
    pub dir: String,
    /// `user.md`.
    pub user: String,
    /// `rules.md`.
    pub rules: String,
    /// `glossary.md`.
    pub glossary: String,
    /// Whether [`try_load`] accepts the main pack.
    pub ok: bool,
    /// Why the pack is invalid, when `ok` is false.
    pub reason: Option<String>,
}

/// Result of a HUD left-rail profile switch (ADR-0041).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HudSwitchProfileResult {
    /// Profiles snapshot with the new active id selected.
    pub snapshot: ProfilesSnapshot,
    /// Daemon `/refresh` reply (or a local note when the daemon was unreachable).
    pub refresh_message: String,
    /// True when the daemon Ask(`/refresh`) call succeeded.
    pub refresh_ok: bool,
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
        .map(|meta| ProfileRow {
            active: meta.id == app.active_profile,
            pack_ok: try_load_effective(config, &meta.id).is_ok(),
            id: meta.id,
            name: meta.name,
        })
        .collect::<Vec<_>>();
    let selected_name = profile_name_in(&selected_dir);
    let pack = pack_with_effective_ok(config, &selected, pack::read_pack(&selected_dir));
    let main_id = resolve_main_profile_id(config).unwrap_or_else(|_| "default".to_owned());
    let is_main = selected == main_id;
    let meta = load_profile_meta(&selected_dir);
    let global = pack::read_pack(&profile_pack_dir(config, &main_id));
    Ok(ProfilesSnapshot {
        config_dir: config.display().to_string(),
        active_id: app.active_profile,
        selected_id: selected,
        selected_name,
        profiles: rows,
        pack,
        is_main,
        main_id,
        use_global_user: is_main || meta.use_global_user,
        use_global_glossary: is_main || meta.use_global_glossary,
        use_global_rules: is_main || meta.use_global_rules,
        allow_all: meta.allow_all,
        role: meta.role.clone(),
        tts_voice: meta.tts_voice,
        global_user: global.user,
        global_rules: global.rules,
        global_glossary: global.glossary,
    })
}

fn pack_with_effective_ok(config: &Path, profile_id: &str, mut pack: PackSnapshot) -> PackSnapshot {
    match try_load_effective(config, profile_id) {
        Ok(_) => {
            pack.ok = true;
            pack.reason = None;
        }
        Err(error) => {
            pack.ok = false;
            pack.reason = Some(error.to_string());
        }
    }
    pack
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
    let previous = load_app_config(&config)
        .map(|app| app.active_profile)
        .unwrap_or_default();
    if previous != id {
        // Voice file first, so a HUD poll that sees the new active id also
        // sees this profile's saved voice.
        let _ = apply_saved_tts_voice(&config, &id);
    }
    set_active_profile(&config, &id).map_err(|error| error.to_string())?;
    snapshot_at(&config, &id)
}

/// Save Use-global checkboxes. Main is forced to its own files.
#[tauri::command]
pub fn profile_set_global_flags(
    id: String,
    use_global_user: bool,
    use_global_glossary: bool,
    use_global_rules: bool,
) -> Result<ProfilesSnapshot, String> {
    let config = config_dir()?;
    set_global_doc_flags(
        &config,
        id.trim(),
        use_global_user,
        use_global_glossary,
        use_global_rules,
    )
    .map_err(|error| error.to_string())?;
    snapshot_at(&config, id.trim())
}

#[tauri::command]
pub fn profile_set_allow_all(id: String, allow_all: bool) -> Result<ProfilesSnapshot, String> {
    let config = config_dir()?;
    set_allow_all(&config, &id, allow_all).map_err(|error| error.to_string())?;
    snapshot_at(&config, id.trim())
}

#[tauri::command]
pub fn profile_set_role(id: String, role: String) -> Result<ProfilesSnapshot, String> {
    let config = config_dir()?;
    set_profile_role(&config, &id, &role).map_err(|error| error.to_string())?;
    snapshot_at(&config, id.trim())
}

/// Save the selected profile's TTS voice. Empty is Default (Eve).
///
/// When `id` is the active profile, also write the live `selected_tts_voice`.
#[tauri::command]
pub fn profile_set_tts_voice(id: String, voice: String) -> Result<ProfilesSnapshot, String> {
    let config = config_dir()?;
    let id = id.trim();
    let canonical = softwake_providers::canonical_stored_tts_voice(&voice)
        .ok_or_else(|| "that voice is not a built-in xAI TTS voice".to_owned())?;
    set_profile_tts_voice(&config, id, &canonical).map_err(|error| error.to_string())?;
    let active = load_app_config(&config)
        .map(|app| app.active_profile)
        .unwrap_or_default();
    if active == id {
        crate::providers::write_live_tts_voice(&canonical)?;
    }
    snapshot_at(&config, id)
}

/// Copy a profile's saved TTS voice into `selected_tts_voice`.
///
/// Unknown stored text becomes empty. Does not rewrite `profile.json`.
fn apply_saved_tts_voice(config: &Path, profile_id: &str) -> Result<(), String> {
    let meta = load_profile_meta(&profile_pack_dir(config, profile_id));
    let voice = softwake_providers::canonical_stored_tts_voice(&meta.tts_voice).unwrap_or_default();
    crate::providers::write_live_tts_voice(&voice)
}

/// Read the main profile's user, rules, and glossary for the Global pane.
#[tauri::command]
pub fn global_docs_snapshot() -> Result<GlobalDocsSnapshot, String> {
    let config = config_dir()?;
    ensure_migrated(&config).map_err(|error| error.to_string())?;
    read_global_docs(&config)
}

/// Write the main profile's user, rules, and glossary. Soul is left as-is.
#[tauri::command]
pub fn global_docs_save(
    user: String,
    rules: String,
    glossary: String,
) -> Result<GlobalDocsSnapshot, String> {
    let config = config_dir()?;
    ensure_migrated(&config).map_err(|error| error.to_string())?;
    let main = resolve_main_profile_id(&config).map_err(|error| error.to_string())?;
    let dir = profile_pack_dir(&config, &main);
    let existing = pack::read_pack(&dir);
    pack::write_pack(&dir, &existing.soul, &user, &rules, &glossary)?;
    read_global_docs(&config)
}

fn read_global_docs(config: &Path) -> Result<GlobalDocsSnapshot, String> {
    let main = resolve_main_profile_id(config).map_err(|error| error.to_string())?;
    let dir = profile_pack_dir(config, &main);
    let pack = pack::read_pack(&dir);
    Ok(GlobalDocsSnapshot {
        main_id: main,
        dir: pack.dir,
        user: pack.user,
        rules: pack.rules,
        glossary: pack.glossary,
        ok: pack.ok,
        reason: pack.reason,
    })
}

/// HUD left-rail switch: set active profile on disk, then run `/refresh` effects.
///
/// Does **not** wake Softwake from sleep. `/refresh` retargets the soul from the
/// new active pack (and rediscovers MCP when awake). See ADR-0041.
#[tauri::command]
pub fn hud_switch_profile(id: String) -> Result<HudSwitchProfileResult, String> {
    let id = id.trim().to_owned();
    if id.is_empty() {
        return Err("profile id is blank".to_owned());
    }
    let config = config_dir()?;
    ensure_migrated(&config).map_err(|error| error.to_string())?;
    let dir = profile_pack_dir(&config, &id);
    if !dir.is_dir() {
        return Err(format!("unknown profile `{id}`"));
    }
    let app = load_app_config(&config).map_err(|error| error.to_string())?;
    let already = app.active_profile == id;
    if !already {
        let _ = apply_saved_tts_voice(&config, &id);
        set_active_profile(&config, &id).map_err(|error| error.to_string())?;
    }
    let snapshot = snapshot_at(&config, &id)?;
    let (refresh_ok, refresh_message) = match ask_refresh() {
        Ok(msg) => (true, msg),
        Err(err) => {
            let note = if already {
                format!("Already active `{id}`; daemon refresh skipped: {err}")
            } else {
                format!(
                    "Active profile set to `{id}`; daemon refresh failed (applies on next successful /refresh): {err}"
                )
            };
            (false, note)
        }
    };
    Ok(HudSwitchProfileResult {
        snapshot,
        refresh_message,
        refresh_ok,
    })
}

/// Ask the running daemon for `/refresh` without waking from sleep/hibernate.
fn ask_refresh() -> Result<String, String> {
    let path = resolve_socket_path(None).map_err(|error| error.to_string())?;
    let mut client = Client::connect(&path).map_err(|error| error.to_string())?;
    // Short timeout: refresh is local (no model). Longer only if MCP rediscover stalls.
    client
        .set_read_timeout(Some(std::time::Duration::from_secs(30)))
        .map_err(|error| error.to_string())?;
    let status = client
        .call_ask("/refresh")
        .map_err(|error| error.to_string())?;
    Ok(status.message.unwrap_or_else(|| "Refreshed".to_owned()))
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
    let existing = pack::read_pack(&dir);
    // Inherited docs keep the profile's own file. The editor is showing the
    // global preview, and writing that preview would copy main onto the profile.
    let user = if should_write_own(&dir, DocKind::User) {
        user
    } else {
        existing.user.clone()
    };
    let rules = if should_write_own(&dir, DocKind::Rules) {
        rules
    } else {
        existing.rules.clone()
    };
    let glossary = if should_write_own(&dir, DocKind::Glossary) {
        glossary
    } else {
        existing.glossary
    };
    let written = pack::write_pack(&dir, &soul, &user, &rules, &glossary)?;
    if let Some((config, id)) = profile_owner_from_pack_dir(&dir) {
        return Ok(pack_with_effective_ok(&config, &id, written));
    }
    Ok(written)
}

enum DocKind {
    User,
    Rules,
    Glossary,
}

fn should_write_own(dir: &Path, kind: DocKind) -> bool {
    let Some((config, id)) = profile_owner_from_pack_dir(dir) else {
        return true;
    };
    let Ok(main) = resolve_main_profile_id(&config) else {
        return true;
    };
    if id == main {
        return true;
    }
    let meta = load_profile_meta(dir);
    match kind {
        DocKind::User => !meta.use_global_user,
        DocKind::Rules => !meta.use_global_rules,
        DocKind::Glossary => !meta.use_global_glossary,
    }
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

    #[test]
    fn set_active_profile_disk_round_trip() {
        let home = TempHome::new();
        let config = home.path.join(".config").join("softwake");
        softwake_soul::ensure_migrated(&config).expect("migrate");
        let created = softwake_soul::create_profile(&config, "Sally", None).expect("create");
        set_active_profile(&config, &created.id).expect("active");
        let app = load_app_config(&config).expect("app");
        assert_eq!(app.active_profile, created.id);
        let snap = snapshot_at(&config, &created.id).expect("snap");
        assert_eq!(snap.active_id, created.id);
        assert!(
            snap.profiles
                .iter()
                .any(|row| row.active && row.id == created.id)
        );
    }

    #[test]
    fn snapshot_at_rejects_unknown_profile() {
        let home = TempHome::new();
        let config = home.path.join(".config").join("softwake");
        softwake_soul::ensure_migrated(&config).expect("migrate");
        let err = snapshot_at(&config, "no-such-profile").expect_err("unknown");
        assert!(err.contains("unknown profile"), "{err}");
    }
}
