//! Soul directory and the two file paths inside it.
//!
//! Constructing these types does not read the files. [`SoulDir::load`](crate::SoulDir::load)
//! and [`crate::load`] do.

use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::{SoulError, SoulPack, try_load};

/// Directory that holds `soul.md` and `user.md`.
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

    /// `soul.md` and `user.md` inside this directory.
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

/// Locations of the two phase-1 soul files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoulPaths {
    soul: PathBuf,
    user: PathBuf,
}

impl SoulPaths {
    /// Record the paths. Neither file is opened.
    #[must_use]
    pub fn new(soul: PathBuf, user: PathBuf) -> Self {
        Self { soul, user }
    }

    /// `soul.md` and `user.md` inside `dir`.
    #[must_use]
    pub fn in_dir(dir: &Path) -> Self {
        Self::new(dir.join("soul.md"), dir.join("user.md"))
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
}

/// Soul directory from a flag, then the environment, then XDG.
///
/// Order:
///
/// 1. `flag`, when it is set and not empty
/// 2. `SOFTWAKE_SOUL_DIR`, when it is set and not empty
/// 3. `$XDG_CONFIG_HOME/softwake/soul`, when `XDG_CONFIG_HOME` is set and not empty
/// 4. `$HOME/.config/softwake/soul`, when `HOME` is set and not empty
///
/// The directory does not have to exist yet.
///
/// # Errors
///
/// Returns [`SoulError::Unresolved`] when every source is unset.
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
    if let Some(path) = nonempty(xdg_config_home) {
        return Ok(SoulDir::new(path.join("softwake").join("soul")));
    }
    if let Some(path) = nonempty(home) {
        return Ok(SoulDir::new(
            path.join(".config").join("softwake").join("soul"),
        ));
    }
    Err(SoulError::Unresolved)
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
    use std::path::{Path, PathBuf};

    use super::{SoulDir, SoulPaths, path_from_env_value, resolve_soul_dir_from};
    use crate::SoulError;

    #[test]
    fn stores_paths_without_reading_them() {
        let paths = SoulPaths::new(PathBuf::from("soul.md"), PathBuf::from("user.md"));
        assert_eq!(paths.soul(), PathBuf::from("soul.md").as_path());
        assert_eq!(paths.user(), PathBuf::from("user.md").as_path());
    }

    #[test]
    fn in_dir_joins_the_required_names() {
        let paths = SoulPaths::in_dir(Path::new("pack"));
        assert_eq!(paths.soul(), Path::new("pack/soul.md"));
        assert_eq!(paths.user(), Path::new("pack/user.md"));
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

    #[test]
    fn xdg_config_home_appends_softwake_soul() {
        let dir = resolve_soul_dir_from(
            None,
            None,
            Some(Path::new("from-xdg")),
            Some(Path::new("from-home")),
        )
        .expect("xdg");
        assert_eq!(dir.path(), Path::new("from-xdg/softwake/soul"));
    }

    #[test]
    fn home_uses_dot_config_when_xdg_is_unset() {
        let dir =
            resolve_soul_dir_from(None, None, None, Some(Path::new("from-home"))).expect("home");
        assert_eq!(dir.path(), Path::new("from-home/.config/softwake/soul"));
    }

    #[test]
    fn empty_paths_are_skipped() {
        let dir = resolve_soul_dir_from(
            Some(Path::new("")),
            Some(Path::new("")),
            Some(Path::new("")),
            Some(Path::new("from-home")),
        )
        .expect("home");
        assert_eq!(dir.path(), Path::new("from-home/.config/softwake/soul"));
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
