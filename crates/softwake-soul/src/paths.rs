//! Soul directory and the four file paths inside it.
//!
//! Constructing these types does not read the files. [`SoulDir::load`](crate::SoulDir::load)
//! and [`crate::load`] do.

use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::{SoulError, SoulPack, try_load};

/// Directory that holds `soul.md`, `user.md`, `rules.md`, and `glossary.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoulDir {
    path: PathBuf,
}

impl SoulDir {
    /// Record a directory. It is not created and its files are not opened.
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Directory path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The four pack files inside this directory.
    #[must_use]
    pub fn paths(&self) -> SoulPaths {
        SoulPaths::in_dir(&self.path)
    }

    /// Read and validate the pack in this directory.
    ///
    /// # Errors
    ///
    /// See [`crate::load`].
    pub fn load(&self) -> Result<SoulPack, SoulError> {
        try_load(&self.path)
    }
}

/// Locations of the four context-pack files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoulPaths {
    soul: PathBuf,
    user: PathBuf,
    rules: PathBuf,
    glossary: PathBuf,
}

impl SoulPaths {
    /// Record the paths. None of the files are opened.
    #[must_use]
    pub fn new(soul: PathBuf, user: PathBuf, rules: PathBuf, glossary: PathBuf) -> Self {
        Self {
            soul,
            user,
            rules,
            glossary,
        }
    }

    /// `soul.md`, `user.md`, `rules.md`, and `glossary.md` inside `dir`.
    #[must_use]
    pub fn in_dir(dir: &Path) -> Self {
        Self::new(
            dir.join("soul.md"),
            dir.join("user.md"),
            dir.join("rules.md"),
            dir.join("glossary.md"),
        )
    }

    /// Path that will be read as `soul.md`.
    #[must_use]
    pub fn soul(&self) -> &Path {
        &self.soul
    }

    /// Path that will be read as `user.md`.
    #[must_use]
    pub fn user(&self) -> &Path {
        &self.user
    }

    /// Path that will be read as `rules.md`.
    #[must_use]
    pub fn rules(&self) -> &Path {
        &self.rules
    }

    /// Path that will be read as `glossary.md`.
    #[must_use]
    pub fn glossary(&self) -> &Path {
        &self.glossary
    }
}

/// Soul directory from a flag, then the environment, then the active profile.
///
/// Order:
///
/// 1. `flag`, when it is set and not empty (raw pack dir; skips profiles)
/// 2. `SOFTWAKE_SOUL_DIR`, when it is set and not empty (same)
/// 3. Active profile pack under `$XDG_CONFIG_HOME/softwake/profiles/<id>/`,
///    after an idempotent migrate from legacy `soul/` when needed
/// 4. Same under `$HOME/.config/softwake/profiles/<id>/`
///
/// The directory does not have to exist yet when a flag or env override is
/// used. Profile resolution creates the default profile on first use.
///
/// # Errors
///
/// Returns [`SoulError::Unresolved`] when every source is unset, or a profile
/// config I/O error from migrate.
pub fn resolve_soul_dir(flag: Option<&Path>) -> Result<SoulDir, SoulError> {
    let env_dir = path_from_env("SOFTWAKE_SOUL_DIR");
    let xdg = path_from_env("XDG_CONFIG_HOME");
    let home = path_from_env("HOME");
    resolve_soul_dir_from(flag, env_dir.as_deref(), xdg.as_deref(), home.as_deref())
}

/// [`resolve_soul_dir`] with the environment passed in.
///
/// Empty paths count as unset. Tests use this so they do not change process
/// environment variables.
///
/// # Errors
///
/// Returns [`SoulError::Unresolved`] when every source is unset.
pub fn resolve_soul_dir_from(
    flag: Option<&Path>,
    env_dir: Option<&Path>,
    xdg_config_home: Option<&Path>,
    home: Option<&Path>,
) -> Result<SoulDir, SoulError> {
    if let Some(path) = nonempty(flag) {
        return Ok(SoulDir::new(path.to_path_buf()));
    }
    if let Some(path) = nonempty(env_dir) {
        return Ok(SoulDir::new(path.to_path_buf()));
    }
    let pack = crate::profile::resolve_active_pack_dir(xdg_config_home, home)?;
    Ok(SoulDir::new(pack))
}

fn path_from_env(key: &str) -> Option<PathBuf> {
    path_from_env_value(env::var_os(key).as_deref())
}

fn path_from_env_value(value: Option<&OsStr>) -> Option<PathBuf> {
    let value = value?;
    if value.is_empty() {
        None
    } else {
        Some(PathBuf::from(value))
    }
}

fn nonempty(path: Option<&Path>) -> Option<&Path> {
    path.filter(|path| !path.as_os_str().is_empty())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{SoulDir, SoulPaths, path_from_env_value, resolve_soul_dir_from};
    use crate::SoulError;

    #[test]
    fn stores_paths_without_reading_them() {
        let paths = SoulPaths::new(
            PathBuf::from("soul.md"),
            PathBuf::from("user.md"),
            PathBuf::from("rules.md"),
            PathBuf::from("glossary.md"),
        );
        assert_eq!(paths.soul(), PathBuf::from("soul.md").as_path());
        assert_eq!(paths.user(), PathBuf::from("user.md").as_path());
        assert_eq!(paths.rules(), PathBuf::from("rules.md").as_path());
        assert_eq!(paths.glossary(), PathBuf::from("glossary.md").as_path());
    }

    #[test]
    fn in_dir_joins_the_required_names() {
        let paths = SoulPaths::in_dir(Path::new("pack"));
        assert_eq!(paths.soul(), Path::new("pack/soul.md"));
        assert_eq!(paths.user(), Path::new("pack/user.md"));
        assert_eq!(paths.rules(), Path::new("pack/rules.md"));
        assert_eq!(paths.glossary(), Path::new("pack/glossary.md"));
        let dir = SoulDir::new(PathBuf::from("pack"));
        assert_eq!(dir.paths(), paths);
        assert_eq!(dir.path(), Path::new("pack"));
    }

    #[test]
    fn flag_wins_over_env_and_xdg() {
        let dir = resolve_soul_dir_from(
            Some(Path::new("from-flag")),
            Some(Path::new("from-env")),
            Some(Path::new("from-xdg")),
            Some(Path::new("from-home")),
        )
        .expect("flag");
        assert_eq!(dir.path(), Path::new("from-flag"));
    }

    #[test]
    fn env_wins_over_xdg_and_home() {
        let dir = resolve_soul_dir_from(
            None,
            Some(Path::new("from-env")),
            Some(Path::new("from-xdg")),
            Some(Path::new("from-home")),
        )
        .expect("env");
        assert_eq!(dir.path(), Path::new("from-env"));
    }

    fn temp_root(label: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("softwake-paths-{label}-{}-{n}", std::process::id()));
        fs::create_dir_all(&path).expect("temp root");
        path
    }

    #[test]
    fn xdg_config_home_uses_active_profile_pack() {
        let xdg = temp_root("xdg");
        let home = temp_root("home-unused");
        let dir = resolve_soul_dir_from(None, None, Some(xdg.as_path()), Some(home.as_path()))
            .expect("xdg");
        assert_eq!(
            dir.path(),
            xdg.join("softwake").join("profiles").join("default")
        );
        let _ = fs::remove_dir_all(&xdg);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn home_uses_active_profile_when_xdg_is_unset() {
        let home = temp_root("home");
        let dir = resolve_soul_dir_from(None, None, None, Some(home.as_path())).expect("home");
        assert_eq!(
            dir.path(),
            home.join(".config")
                .join("softwake")
                .join("profiles")
                .join("default")
        );
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn empty_paths_are_skipped() {
        let home = temp_root("empty");
        let dir = resolve_soul_dir_from(
            Some(Path::new("")),
            Some(Path::new("")),
            Some(Path::new("")),
            Some(home.as_path()),
        )
        .expect("home");
        assert_eq!(
            dir.path(),
            home.join(".config")
                .join("softwake")
                .join("profiles")
                .join("default")
        );
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn missing_home_and_xdg_is_unresolved() {
        let error = resolve_soul_dir_from(None, None, None, None).expect_err("unset");
        assert!(matches!(error, SoulError::Unresolved));
        assert!(error.to_string().contains("SOFTWAKE_SOUL_DIR"));
    }

    #[test]
    fn blank_env_values_are_unset() {
        assert_eq!(path_from_env_value(None), None);
        assert_eq!(path_from_env_value(Some(OsStr::new(""))), None);
        assert_eq!(
            path_from_env_value(Some(OsStr::new("softwake/soul"))),
            Some(PathBuf::from("softwake/soul"))
        );
    }
}
