//! Soul-pack paths.
//!
//! Loading, size caps, and instruction rendering come later. [`SoulPaths`]
//! only records where `soul.md` and `user.md` will be read from, so constructing
//! it does not touch the filesystem.

use std::path::{Path, PathBuf};

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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::SoulPaths;

    #[test]
    fn stores_paths_without_reading_them() {
        let paths = SoulPaths::new(PathBuf::from("soul.md"), PathBuf::from("user.md"));
        assert_eq!(paths.soul(), PathBuf::from("soul.md").as_path());
        assert_eq!(paths.user(), PathBuf::from("user.md").as_path());
    }
}
