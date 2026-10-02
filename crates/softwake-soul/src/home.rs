//! Per-profile agent home directories (ADR-0052).
//!
//! Layout: `$XDG_DATA_HOME/softwake/homes/<profile_id>/`, else
//! `~/.local/share/softwake/homes/<profile_id>/`. Public docs use only these
//! XDG forms — never host-specific shop paths.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::SoulError;

/// Directory name under the Softwake data root that holds agent homes.
pub const HOMES_DIR_NAME: &str = "homes";

/// Softwake data root: `$XDG_DATA_HOME/softwake` or `$HOME/.local/share/softwake`.
///
/// # Errors
///
/// [`SoulError::Unresolved`] when both XDG data home and HOME are unset.
pub fn resolve_data_dir(
    xdg_data_home: Option<&Path>,
    home: Option<&Path>,
) -> Result<PathBuf, SoulError> {
    if let Some(path) = nonempty(xdg_data_home) {
        return Ok(path.join("softwake"));
    }
    if let Some(path) = nonempty(home) {
        return Ok(path.join(".local").join("share").join("softwake"));
    }
    Err(SoulError::Unresolved)
}

/// Softwake data root from the process environment.
///
/// # Errors
///
/// [`SoulError::Unresolved`] when both sources are unset.
pub fn resolve_data_dir_from_env() -> Result<PathBuf, SoulError> {
    let xdg = env::var_os("XDG_DATA_HOME").map(PathBuf::from);
    let home = env::var_os("HOME").map(PathBuf::from);
    resolve_data_dir(xdg.as_deref(), home.as_deref())
}

/// Absolute path of `profile_id`'s agent home (not necessarily created yet).
///
/// # Errors
///
/// Data-dir resolution failure, or empty / dotted `profile_id`.
pub fn profile_home_dir(data_dir: &Path, profile_id: &str) -> Result<PathBuf, SoulError> {
    let id = profile_id.trim();
    if id.is_empty() || id.starts_with('.') || id.contains('/') || id.contains('\\') {
        return Err(SoulError::InvalidConfig {
            path: data_dir.join(HOMES_DIR_NAME).join(profile_id),
            detail: "invalid profile id for agent home".to_owned(),
        });
    }
    Ok(data_dir.join(HOMES_DIR_NAME).join(id))
}

/// Resolve and create the agent home for `profile_id` (idempotent).
///
/// # Errors
///
/// Resolution or `create_dir_all` failure.
pub fn ensure_profile_home(profile_id: &str) -> Result<PathBuf, SoulError> {
    let data = resolve_data_dir_from_env()?;
    let home = profile_home_dir(&data, profile_id)?;
    fs::create_dir_all(&home).map_err(|source| SoulError::InvalidConfig {
        path: home.clone(),
        detail: source.to_string(),
    })?;
    Ok(home)
}

/// Ensure home under an explicit data root (tests).
///
/// # Errors
///
/// Invalid id or I/O.
pub fn ensure_profile_home_in(data_dir: &Path, profile_id: &str) -> Result<PathBuf, SoulError> {
    let home = profile_home_dir(data_dir, profile_id)?;
    fs::create_dir_all(&home).map_err(|source| SoulError::InvalidConfig {
        path: home.clone(),
        detail: source.to_string(),
    })?;
    Ok(home)
}

fn nonempty(path: Option<&Path>) -> Option<&Path> {
    path.and_then(|p| {
        let s = p.as_os_str();
        if s.is_empty() { None } else { Some(p) }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!("softwake-home-{tag}-{nanos}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp");
        dir
    }

    #[test]
    fn xdg_data_home_wins() {
        let root = temp_root("xdg");
        let data = resolve_data_dir(Some(root.as_path()), None).expect("data");
        assert_eq!(data, root.join("softwake"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn home_fallback() {
        let root = temp_root("home");
        let data = resolve_data_dir(None, Some(root.as_path())).expect("data");
        assert_eq!(data, root.join(".local").join("share").join("softwake"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn ensure_creates_homes_id() {
        let root = temp_root("ensure");
        let data = root.join("softwake");
        let home = ensure_profile_home_in(&data, "sally").expect("home");
        assert_eq!(home, data.join("homes").join("sally"));
        assert!(home.is_dir());
        let again = ensure_profile_home_in(&data, "sally").expect("idempotent");
        assert_eq!(again, home);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_bad_ids() {
        let root = temp_root("bad");
        let data = root.join("softwake");
        assert!(profile_home_dir(&data, "").is_err());
        assert!(profile_home_dir(&data, ".hidden").is_err());
        assert!(profile_home_dir(&data, "a/b").is_err());
        let _ = fs::remove_dir_all(&root);
    }
}
