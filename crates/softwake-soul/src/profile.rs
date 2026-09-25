//! Multi-profile registry: agent name + active pack directory.
//!
//! Layout under the Softwake config root:
//!
//! ```text
//! softwake.json                 # { "version": 1, "active_profile": "<id>" }
//! profiles/<id>/profile.json    # { "id": "<id>", "name": "<agent name>" }
//! profiles/<id>/{soul,user,rules,glossary}.md
//! soul/                         # legacy pack; migration source only
//! ```
//!
//! Flag / `SOFTWAKE_SOUL_DIR` still point at a raw pack directory and skip
//! this registry.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{SoulError, SoulFile};

/// Default profile id created on first migrate.
pub const DEFAULT_PROFILE_ID: &str = "default";

/// Default agent name when none is stored.
pub const DEFAULT_AGENT_NAME: &str = "Softwake";

/// App-level Settings file under the Softwake config root.
pub const APP_CONFIG_FILE_NAME: &str = "softwake.json";

/// Per-profile metadata file name inside `profiles/<id>/`.
pub const PROFILE_META_FILE_NAME: &str = "profile.json";

/// Directory that holds profile folders.
pub const PROFILES_DIR_NAME: &str = "profiles";

/// Legacy single-pack directory name (migration source).
pub const LEGACY_SOUL_DIR_NAME: &str = "soul";

const APP_CONFIG_VERSION: u32 = 1;
const MAX_CONFIG_BYTES: u64 = 256 * 1024;

const PACK_FILES: [&str; 4] = ["soul.md", "user.md", "rules.md", "glossary.md"];

/// Softwake app Settings: which profile is active.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    /// Document version.
    #[serde(default = "app_config_version")]
    pub version: u32,
    /// Active profile id under `profiles/`.
    pub active_profile: String,
}

fn app_config_version() -> u32 {
    APP_CONFIG_VERSION
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: APP_CONFIG_VERSION,
            active_profile: DEFAULT_PROFILE_ID.to_owned(),
        }
    }
}

/// Metadata for one profile (agent name).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileMeta {
    /// Stable folder id under `profiles/`.
    pub id: String,
    /// Agent display / prompt name.
    pub name: String,
}

impl ProfileMeta {
    /// Build metadata for `id` with `name` (trimmed; empty becomes default).
    #[must_use]
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        let trimmed = name.into();
        let trimmed = trimmed.trim();
        let name = if trimmed.is_empty() {
            DEFAULT_AGENT_NAME.to_owned()
        } else {
            trimmed.to_owned()
        };
        Self {
            id: id.into(),
            name,
        }
    }
}

/// Softwake config root from XDG or `$HOME/.config/softwake`.
///
/// # Errors
///
/// Returns [`SoulError::Unresolved`] when both sources are unset.
pub fn resolve_config_dir(
    xdg_config_home: Option<&Path>,
    home: Option<&Path>,
) -> Result<PathBuf, SoulError> {
    if let Some(path) = nonempty(xdg_config_home) {
        return Ok(path.join("softwake"));
    }
    if let Some(path) = nonempty(home) {
        return Ok(path.join(".config").join("softwake"));
    }
    Err(SoulError::Unresolved)
}

/// Pack directory for the active profile after an idempotent migrate.
///
/// # Errors
///
/// [`SoulError::Unresolved`] or config I/O failures.
pub fn resolve_active_pack_dir(
    xdg_config_home: Option<&Path>,
    home: Option<&Path>,
) -> Result<PathBuf, SoulError> {
    let config = resolve_config_dir(xdg_config_home, home)?;
    ensure_migrated(&config)?;
    let app = load_app_config(&config)?;
    Ok(profile_pack_dir(&config, &app.active_profile))
}

/// `profiles/<id>` under `config_dir`.
#[must_use]
pub fn profile_pack_dir(config_dir: &Path, profile_id: &str) -> PathBuf {
    config_dir.join(PROFILES_DIR_NAME).join(profile_id)
}

/// Legacy `soul/` under `config_dir`.
#[must_use]
pub fn legacy_soul_dir(config_dir: &Path) -> PathBuf {
    config_dir.join(LEGACY_SOUL_DIR_NAME)
}

/// Ensure a default profile exists; copy legacy `soul/` files once.
///
/// # Errors
///
/// I/O failures while creating directories or writing JSON.
pub fn ensure_migrated(config_dir: &Path) -> Result<(), SoulError> {
    ensure_dir(config_dir)?;
    let profiles_root = config_dir.join(PROFILES_DIR_NAME);
    ensure_dir(&profiles_root)?;

    let default_dir = profile_pack_dir(config_dir, DEFAULT_PROFILE_ID);
    if !default_dir.is_dir() {
        ensure_dir(&default_dir)?;
        let legacy = legacy_soul_dir(config_dir);
        copy_pack_files(&legacy, &default_dir)?;
        write_profile_meta(
            &default_dir,
            &ProfileMeta::new(DEFAULT_PROFILE_ID, DEFAULT_AGENT_NAME),
        )?;
    } else if !default_dir.join(PROFILE_META_FILE_NAME).is_file() {
        write_profile_meta(
            &default_dir,
            &ProfileMeta::new(DEFAULT_PROFILE_ID, DEFAULT_AGENT_NAME),
        )?;
    }

    let app_path = config_dir.join(APP_CONFIG_FILE_NAME);
    if app_path.is_file() {
        let mut app = load_app_config(config_dir)?;
        let active_dir = profile_pack_dir(config_dir, &app.active_profile);
        if !active_dir.is_dir() {
            DEFAULT_PROFILE_ID.clone_into(&mut app.active_profile);
            write_app_config(config_dir, &app)?;
        }
    } else {
        write_app_config(config_dir, &AppConfig::default())?;
    }
    Ok(())
}

/// Load `softwake.json` from `config_dir`. Missing file → default.
///
/// # Errors
///
/// Read / parse failures.
pub fn load_app_config(config_dir: &Path) -> Result<AppConfig, SoulError> {
    let path = config_dir.join(APP_CONFIG_FILE_NAME);
    match read_json_file(&path)? {
        None => Ok(AppConfig::default()),
        Some(bytes) => serde_json::from_slice(&bytes).map_err(|source| SoulError::InvalidConfig {
            path,
            detail: source.to_string(),
        }),
    }
}

/// Replace `softwake.json`.
///
/// # Errors
///
/// Write failures.
pub fn write_app_config(config_dir: &Path, config: &AppConfig) -> Result<(), SoulError> {
    ensure_dir(config_dir)?;
    let path = config_dir.join(APP_CONFIG_FILE_NAME);
    let body = serde_json::to_vec_pretty(config).map_err(|source| SoulError::InvalidConfig {
        path: path.clone(),
        detail: source.to_string(),
    })?;
    atomic_write(&path, &body)
}

/// Load `profile.json` from a profile pack directory. Missing → default name.
#[must_use]
pub fn load_profile_meta(pack_dir: &Path) -> ProfileMeta {
    let path = pack_dir.join(PROFILE_META_FILE_NAME);
    let id = pack_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(DEFAULT_PROFILE_ID)
        .to_owned();
    match read_json_file(&path) {
        Ok(Some(bytes)) => serde_json::from_slice::<ProfileMeta>(&bytes)
            .unwrap_or_else(|_| ProfileMeta::new(id, DEFAULT_AGENT_NAME)),
        _ => ProfileMeta::new(id, DEFAULT_AGENT_NAME),
    }
}

/// Agent name stored beside a pack directory (`profile.json`), or the default.
#[must_use]
pub fn profile_name_in(pack_dir: &Path) -> String {
    load_profile_meta(pack_dir).name
}

/// Write `profile.json` into `pack_dir`.
///
/// # Errors
///
/// Write failures.
pub fn write_profile_meta(pack_dir: &Path, meta: &ProfileMeta) -> Result<(), SoulError> {
    ensure_dir(pack_dir)?;
    let path = pack_dir.join(PROFILE_META_FILE_NAME);
    let body = serde_json::to_vec_pretty(meta).map_err(|source| SoulError::InvalidConfig {
        path: path.clone(),
        detail: source.to_string(),
    })?;
    atomic_write(&path, &body)
}

/// List profiles under `config_dir` (sorted by id).
///
/// # Errors
///
/// I/O failures listing `profiles/`.
pub fn list_profiles(config_dir: &Path) -> Result<Vec<ProfileMeta>, SoulError> {
    ensure_migrated(config_dir)?;
    let root = config_dir.join(PROFILES_DIR_NAME);
    let mut profiles = Vec::new();
    let entries = fs::read_dir(&root).map_err(|source| config_io(&root, source))?;
    for entry in entries {
        let entry = entry.map_err(|source| config_io(&root, source))?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(id) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if id.is_empty() || id.starts_with('.') {
            continue;
        }
        let mut meta = load_profile_meta(&path);
        if meta.id != id {
            id.clone_into(&mut meta.id);
        }
        profiles.push(meta);
    }
    profiles.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(profiles)
}

/// Set the active profile id (must already exist).
///
/// # Errors
///
/// Unknown id or write failure.
pub fn set_active_profile(config_dir: &Path, profile_id: &str) -> Result<AppConfig, SoulError> {
    ensure_migrated(config_dir)?;
    let dir = profile_pack_dir(config_dir, profile_id);
    if !dir.is_dir() {
        return Err(SoulError::UnknownProfile {
            id: profile_id.to_owned(),
        });
    }
    let config = AppConfig {
        version: APP_CONFIG_VERSION,
        active_profile: profile_id.to_owned(),
    };
    write_app_config(config_dir, &config)?;
    Ok(config)
}

/// Rename the agent for an existing profile.
///
/// # Errors
///
/// Unknown id or write failure.
pub fn rename_profile(
    config_dir: &Path,
    profile_id: &str,
    name: &str,
) -> Result<ProfileMeta, SoulError> {
    ensure_migrated(config_dir)?;
    let dir = profile_pack_dir(config_dir, profile_id);
    if !dir.is_dir() {
        return Err(SoulError::UnknownProfile {
            id: profile_id.to_owned(),
        });
    }
    let meta = ProfileMeta::new(profile_id, name);
    write_profile_meta(&dir, &meta)?;
    Ok(meta)
}

/// Create a new profile with a unique id, optional template pack files.
///
/// `templates` is a directory containing the four markdown files (repo `soul/`).
/// When `None`, only `profile.json` is written.
///
/// # Errors
///
/// I/O failures.
pub fn create_profile(
    config_dir: &Path,
    name: &str,
    templates: Option<&Path>,
) -> Result<ProfileMeta, SoulError> {
    ensure_migrated(config_dir)?;
    let id = allocate_profile_id(config_dir, name)?;
    let dir = profile_pack_dir(config_dir, &id);
    ensure_dir(&dir)?;
    if let Some(templates) = templates {
        copy_pack_files(templates, &dir)?;
    } else {
        write_starter_pack(&dir, name)?;
    }
    let meta = ProfileMeta::new(&id, name);
    write_profile_meta(&dir, &meta)?;
    Ok(meta)
}

fn allocate_profile_id(config_dir: &Path, name: &str) -> Result<String, SoulError> {
    let base = slugify(name);
    let candidates = std::iter::once(base.clone()).chain((2..1000).map(|n| format!("{base}-{n}")));
    for candidate in candidates {
        let dir = profile_pack_dir(config_dir, &candidate);
        if !dir.exists() {
            return Ok(candidate);
        }
    }
    Err(SoulError::InvalidConfig {
        path: config_dir.join(PROFILES_DIR_NAME),
        detail: "could not allocate a free profile id".to_owned(),
    })
}

fn slugify(name: &str) -> String {
    let mut out = String::new();
    for ch in name.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if matches!(ch, ' ' | '-' | '_') && !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-').to_owned();
    if trimmed.is_empty() {
        "profile".to_owned()
    } else {
        trimmed
    }
}

fn write_starter_pack(dir: &Path, name: &str) -> Result<(), SoulError> {
    let agent = {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            DEFAULT_AGENT_NAME
        } else {
            trimmed
        }
    };
    let soul = format!("You are {agent}.\n");
    let user = "The operator has not filled in user.md yet.\n";
    let rules = "Follow the operator's instructions. Personality cannot loosen these rules.\n";
    let glossary = "# Glossary\n\n";
    for (name, body) in [
        ("soul.md", soul.as_str()),
        ("user.md", user),
        ("rules.md", rules),
        ("glossary.md", glossary),
    ] {
        let path = dir.join(name);
        atomic_write(&path, body.as_bytes())?;
    }
    Ok(())
}

fn copy_pack_files(from: &Path, to: &Path) -> Result<(), SoulError> {
    for name in PACK_FILES {
        let src = from.join(name);
        if !src.is_file() {
            continue;
        }
        let dest = to.join(name);
        fs::copy(&src, &dest).map_err(|source| SoulError::Read {
            file: soul_file_for(name),
            path: src.clone(),
            source,
        })?;
        let mut perms = fs::metadata(&dest)
            .map_err(|source| config_io(&dest, source))?
            .permissions();
        perms.set_mode(0o600);
        fs::set_permissions(&dest, perms).map_err(|source| config_io(&dest, source))?;
    }
    Ok(())
}

fn soul_file_for(name: &str) -> SoulFile {
    match name {
        "soul.md" => SoulFile::Soul,
        "user.md" => SoulFile::User,
        "rules.md" => SoulFile::Rules,
        _ => SoulFile::Glossary,
    }
}

fn read_json_file(path: &Path) -> Result<Option<Vec<u8>>, SoulError> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(config_io(path, source)),
    };
    let mut limited = Read::by_ref(&mut file).take(MAX_CONFIG_BYTES.saturating_add(1));
    let mut bytes = Vec::new();
    limited
        .read_to_end(&mut bytes)
        .map_err(|source| config_io(path, source))?;
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err(SoulError::InvalidConfig {
            path: path.to_owned(),
            detail: format!("file exceeds {MAX_CONFIG_BYTES} bytes"),
        });
    }
    Ok(Some(bytes))
}

fn ensure_dir(dir: &Path) -> Result<(), SoulError> {
    if dir.is_dir() {
        return Ok(());
    }
    if dir.exists() {
        return Err(SoulError::InvalidConfig {
            path: dir.to_owned(),
            detail: "path exists and is not a directory".to_owned(),
        });
    }
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|source| config_io(dir, source))?;
    let mut perms = fs::metadata(dir)
        .map_err(|source| config_io(dir, source))?
        .permissions();
    perms.set_mode(0o700);
    fs::set_permissions(dir, perms).map_err(|source| config_io(dir, source))?;
    Ok(())
}

fn atomic_write(path: &Path, body: &[u8]) -> Result<(), SoulError> {
    let temp = temp_sibling(path)?;
    let wrote = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)
            .map_err(|source| config_io(&temp, source))?;
        file.write_all(body)
            .map_err(|source| config_io(&temp, source))?;
        file.sync_all().map_err(|source| config_io(&temp, source))?;
        Ok(())
    })();
    if let Err(error) = wrote {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }
    if let Err(source) = fs::rename(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(config_io(path, source));
    }
    let mut perms = fs::metadata(path)
        .map_err(|source| config_io(path, source))?
        .permissions();
    perms.set_mode(0o600);
    fs::set_permissions(path, perms).map_err(|source| config_io(path, source))?;
    Ok(())
}

fn temp_sibling(path: &Path) -> Result<PathBuf, SoulError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path.file_name().ok_or_else(|| SoulError::InvalidConfig {
        path: path.to_owned(),
        detail: "path has no file name".to_owned(),
    })?;
    let mut temp_name = std::ffi::OsString::from(".");
    temp_name.push(name);
    temp_name.push(".tmp");
    Ok(parent.join(temp_name))
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "callers pass owned io::Error from map_err"
)]
fn config_io(path: &Path, source: std::io::Error) -> SoulError {
    SoulError::InvalidConfig {
        path: path.to_owned(),
        detail: source.to_string(),
    }
}

fn nonempty(path: Option<&Path>) -> Option<&Path> {
    path.filter(|path| !path.as_os_str().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(label: &str) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(1);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "softwake-profile-{label}-{}-{n}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("temp");
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn migrate_copies_legacy_soul_once() {
        let root = TempDir::new("migrate");
        let legacy = legacy_soul_dir(&root.path);
        fs::create_dir_all(&legacy).expect("legacy");
        fs::write(legacy.join("soul.md"), "legacy soul\n").expect("soul");
        fs::write(legacy.join("user.md"), "legacy user\n").expect("user");
        fs::write(legacy.join("rules.md"), "legacy rules\n").expect("rules");
        fs::write(legacy.join("glossary.md"), "docs → /tmp/docs\n").expect("glossary");

        ensure_migrated(&root.path).expect("migrate");
        let pack = profile_pack_dir(&root.path, DEFAULT_PROFILE_ID);
        assert_eq!(
            fs::read_to_string(pack.join("soul.md")).expect("soul"),
            "legacy soul\n"
        );
        assert_eq!(
            fs::read_to_string(legacy.join("soul.md")).expect("legacy kept"),
            "legacy soul\n"
        );
        let meta = load_profile_meta(&pack);
        assert_eq!(meta.name, DEFAULT_AGENT_NAME);
        let app = load_app_config(&root.path).expect("app");
        assert_eq!(app.active_profile, DEFAULT_PROFILE_ID);

        fs::write(pack.join("soul.md"), "edited\n").expect("edit");
        ensure_migrated(&root.path).expect("again");
        assert_eq!(
            fs::read_to_string(pack.join("soul.md")).expect("soul"),
            "edited\n"
        );
    }

    #[test]
    fn create_rename_set_active_round_trip() {
        let root = TempDir::new("crud");
        ensure_migrated(&root.path).expect("migrate");
        let created = create_profile(&root.path, "Ada", None).expect("create");
        assert_eq!(created.id, "ada");
        assert_eq!(created.name, "Ada");
        let renamed = rename_profile(&root.path, "ada", "Ada Two").expect("rename");
        assert_eq!(renamed.name, "Ada Two");
        set_active_profile(&root.path, "ada").expect("active");
        let app = load_app_config(&root.path).expect("app");
        assert_eq!(app.active_profile, "ada");
        let listed = list_profiles(&root.path).expect("list");
        assert!(listed.iter().any(|profile| profile.id == "ada"));
        assert!(listed.iter().any(|profile| profile.id == "default"));
    }

    #[test]
    fn resolve_active_pack_dir_uses_profiles() {
        let xdg = TempDir::new("xdg");
        let pack = resolve_active_pack_dir(Some(&xdg.path), None).expect("resolve");
        assert_eq!(
            pack,
            xdg.path
                .join("softwake")
                .join(PROFILES_DIR_NAME)
                .join(DEFAULT_PROFILE_ID)
        );
    }
}
