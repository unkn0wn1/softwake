//! In-process editors for the four soul-pack files.
//!
//! The window resolves the soul directory the same way the daemon does when
//! no `--soul-dir` flag is passed. Save does not call the daemon. A separate
//! `reload_soul` command applies a valid pack on the next awake.

#![allow(
    clippy::module_name_repetitions,
    reason = "PackSnapshot, read_pack, and write_pack are the public names for this module"
)]

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::Serialize;
use softwake_soul::{MAX_FILE_BYTES, SoulPaths, try_load};

/// Editor buffers plus whether [`try_load`] accepts the directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PackSnapshot {
    /// Resolved directory, for display.
    pub dir: String,
    /// `soul.md` text, or empty when that file cannot be read.
    pub soul: String,
    /// `user.md` text, or empty when that file cannot be read.
    pub user: String,
    /// `rules.md` text, or empty when that file cannot be read.
    pub rules: String,
    /// `glossary.md` text, or empty when that file cannot be read.
    pub glossary: String,
    /// `true` when [`try_load`] succeeds.
    pub ok: bool,
    /// [`softwake_soul::SoulError`] sentence when `ok` is false.
    pub reason: Option<String>,
}

/// Read editor buffers from `dir` and validate with [`try_load`].
///
/// A missing file becomes an empty string. This does not create `dir` or any file.
#[must_use]
pub fn read_pack(dir: &Path) -> PackSnapshot {
    let paths = SoulPaths::in_dir(dir);
    snapshot(
        dir,
        read_editor(paths.soul()),
        read_editor(paths.user()),
        read_editor(paths.rules()),
        read_editor(paths.glossary()),
    )
}

/// Write the four files, then return a snapshot of `dir`.
///
/// Each body is refused when it is larger than [`MAX_FILE_BYTES`], before any
/// file is replaced. A later validation failure stays in the snapshot; the
/// files remain as written.
///
/// # Errors
///
/// Returns an error when a body exceeds the cap, `dir` cannot be created, or
/// a file cannot be written.
pub fn write_pack(
    dir: &Path,
    soul: &str,
    user: &str,
    rules: &str,
    glossary: &str,
) -> Result<PackSnapshot, String> {
    reject_oversized("soul.md", soul)?;
    reject_oversized("user.md", user)?;
    reject_oversized("rules.md", rules)?;
    reject_oversized("glossary.md", glossary)?;
    ensure_dir(dir)?;
    let paths = SoulPaths::in_dir(dir);
    atomic_write(paths.soul(), soul.as_bytes())?;
    atomic_write(paths.user(), user.as_bytes())?;
    atomic_write(paths.rules(), rules.as_bytes())?;
    atomic_write(paths.glossary(), glossary.as_bytes())?;
    Ok(read_pack(dir))
}

fn snapshot(
    dir: &Path,
    soul: String,
    user: String,
    rules: String,
    glossary: String,
) -> PackSnapshot {
    let (ok, reason) = match try_load(dir) {
        Ok(_) => (true, None),
        Err(error) => (false, Some(error.to_string())),
    };
    PackSnapshot {
        dir: dir.to_string_lossy().into_owned(),
        soul,
        user,
        rules,
        glossary,
        ok,
        reason,
    }
}

fn reject_oversized(name: &str, body: &str) -> Result<(), String> {
    let len = u64::try_from(body.len()).unwrap_or(u64::MAX);
    if len > MAX_FILE_BYTES {
        return Err(format!("{name} exceeds {MAX_FILE_BYTES} bytes"));
    }
    Ok(())
}

fn read_editor(path: &Path) -> String {
    let Ok(file) = File::open(path) else {
        return String::new();
    };
    let mut limited = file.take(MAX_FILE_BYTES);
    let mut bytes = Vec::new();
    if limited.read_to_end(&mut bytes).is_err() {
        return String::new();
    }
    match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) => String::from_utf8_lossy(&error.into_bytes()).into_owned(),
    }
}

/// Create `dir` as `0o700` when it is missing. An existing directory is left alone.
fn ensure_dir(dir: &Path) -> Result<(), String> {
    if dir.is_dir() {
        return Ok(());
    }
    if dir.exists() {
        return Err(format!("{} is not a directory", dir.display()));
    }
    #[cfg(unix)]
    {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
        let mut perms = fs::metadata(dir)
            .map_err(|error| format!("cannot set mode on {}: {error}", dir.display()))?
            .permissions();
        perms.set_mode(0o700);
        fs::set_permissions(dir, perms)
            .map_err(|error| format!("cannot set mode on {}: {error}", dir.display()))?;
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(dir)
            .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
    }
    Ok(())
}

fn atomic_write(path: &Path, body: &[u8]) -> Result<(), String> {
    let temp = temp_sibling(path)?;
    let wrote = write_temp(&temp, path, body);
    if let Err(error) = wrote {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }
    if let Err(error) = fs::rename(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(format!("cannot replace {}: {error}", path.display()));
    }
    // Rename can keep the previous inode's mode. Force the mode providers use.
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(path)
            .map_err(|error| format!("cannot set mode on {}: {error}", path.display()))?
            .permissions();
        perms.set_mode(0o600);
        fs::set_permissions(path, perms)
            .map_err(|error| format!("cannot set mode on {}: {error}", path.display()))?;
    }
    Ok(())
}

fn write_temp(temp: &Path, dest: &Path, body: &[u8]) -> Result<(), String> {
    #[cfg(unix)]
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(temp)
        .map_err(|error| format!("cannot write {}: {error}", dest.display()))?;
    #[cfg(not(unix))]
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(temp)
        .map_err(|error| format!("cannot write {}: {error}", dest.display()))?;
    file.write_all(body)
        .map_err(|error| format!("cannot write {}: {error}", dest.display()))?;
    file.sync_all()
        .map_err(|error| format!("cannot write {}: {error}", dest.display()))?;
    Ok(())
}

fn temp_sibling(path: &Path) -> Result<PathBuf, String> {
    let Some(name) = path.file_name() else {
        return Err(format!("cannot write {}", path.display()));
    };
    let mut tmp_name = std::ffi::OsString::from(name);
    tmp_name.push(".tmp");
    Ok(match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(tmp_name),
        _ => PathBuf::from(tmp_name),
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{read_pack, write_pack};
    use softwake_soul::MAX_FILE_BYTES;

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(1);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("softwake-ui-pack-{}-{n}", std::process::id()));
            fs::create_dir_all(&path).expect("temp dir");
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[cfg(unix)]
    fn mode_of(path: &std::path::Path) -> u32 {
        fs::metadata(path).expect("metadata").permissions().mode() & 0o777
    }

    const GLOSSARY: &str = "docs → /path/to/docs\n";

    #[cfg(unix)]
    #[test]
    fn write_and_read_round_trip_uses_mode_0600() {
        let dir = TempDir::new();
        let snap = write_pack(
            &dir.path,
            "soul body\n",
            "user body\n",
            "rules body\n",
            GLOSSARY,
        )
        .expect("write");
        assert!(snap.ok, "{:?}", snap.reason);
        assert_eq!(snap.reason, None);
        assert_eq!(snap.soul, "soul body\n");
        assert_eq!(snap.user, "user body\n");
        assert_eq!(snap.rules, "rules body\n");
        assert_eq!(snap.glossary, GLOSSARY);
        assert_eq!(read_pack(&dir.path), snap);
        for name in ["soul.md", "user.md", "rules.md", "glossary.md"] {
            assert_eq!(mode_of(&dir.path.join(name)), 0o600, "{name}");
        }

        let soul = dir.path.join("soul.md");
        fs::set_permissions(&soul, fs::Permissions::from_mode(0o644)).expect("loosen");
        assert_eq!(mode_of(&soul), 0o644);
        write_pack(
            &dir.path,
            "soul body\n",
            "user body\n",
            "rules body\n",
            GLOSSARY,
        )
        .expect("rewrite");
        assert_eq!(mode_of(&soul), 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn new_directory_is_mode_0700_and_read_does_not_create_files() {
        let dir = TempDir::new();
        let child = dir.path.join("soul");
        let snap = write_pack(&child, "soul\n", "user\n", "rules\n", GLOSSARY).expect("create");
        assert!(snap.ok, "{:?}", snap.reason);
        assert_eq!(mode_of(&child), 0o700);

        let absent = dir.path.join("absent");
        let missing = read_pack(&absent);
        assert!(!missing.ok);
        let reason = missing.reason.expect("reason");
        assert!(reason.contains("soul.md"), "{reason}");
        assert!(!absent.exists());
    }

    #[test]
    fn missing_rules_keeps_the_other_buffers() {
        let dir = TempDir::new();
        write_pack(
            &dir.path,
            "soul body\n",
            "user body\n",
            "rules body\n",
            GLOSSARY,
        )
        .expect("write");
        fs::remove_file(dir.path.join("rules.md")).expect("remove rules");
        let snap = read_pack(&dir.path);
        assert!(!snap.ok);
        let reason = snap.reason.expect("reason");
        assert!(reason.contains("rules.md"), "{reason}");
        assert_eq!(snap.soul, "soul body\n");
        assert_eq!(snap.user, "user body\n");
        assert_eq!(snap.rules, "");
        assert_eq!(snap.glossary, GLOSSARY);
    }

    #[test]
    fn empty_soul_is_invalid_after_save() {
        let dir = TempDir::new();
        let snap = write_pack(&dir.path, " \n\t", "user\n", "rules\n", GLOSSARY).expect("write");
        assert!(!snap.ok);
        let reason = snap.reason.expect("reason");
        assert!(reason.contains("soul.md"), "{reason}");
        assert!(reason.contains("empty"), "{reason}");
        assert_eq!(
            fs::read_to_string(dir.path.join("soul.md")).expect("soul"),
            " \n\t"
        );
    }

    #[test]
    fn oversized_body_is_rejected_before_write() {
        let dir = TempDir::new();
        let prior = write_pack(&dir.path, "soul\n", "user\n", "rules\n", GLOSSARY).expect("prior");
        let big = "a".repeat(usize::try_from(MAX_FILE_BYTES).expect("cap") + 1);
        let error = write_pack(&dir.path, &big, "user\n", "rules\n", GLOSSARY).expect_err("over");
        assert!(error.contains("soul.md"), "{error}");
        assert!(error.contains("exceeds"), "{error}");
        let after = read_pack(&dir.path);
        assert_eq!(after.soul, prior.soul);
        assert_eq!(after.user, prior.user);
        assert_eq!(after.rules, prior.rules);
        assert_eq!(after.glossary, prior.glossary);
        assert!(after.ok);

        let empty = TempDir::new();
        let error = write_pack(&empty.path, "soul\n", &big, "rules\n", GLOSSARY).expect_err("user");
        assert!(error.contains("user.md"), "{error}");
        for name in ["soul.md", "user.md", "rules.md", "glossary.md"] {
            assert!(!empty.path.join(name).exists(), "{name}");
        }
    }

    #[test]
    fn exact_cap_still_saves() {
        let dir = TempDir::new();
        let body = "a".repeat(usize::try_from(MAX_FILE_BYTES).expect("cap"));
        let snap = write_pack(&dir.path, &body, "user\n", "rules\n", GLOSSARY).expect("cap");
        assert!(snap.ok, "{:?}", snap.reason);
        assert_eq!(
            snap.soul.len(),
            usize::try_from(MAX_FILE_BYTES).expect("cap")
        );
    }

    #[test]
    fn bad_glossary_is_saved_and_invalid() {
        let dir = TempDir::new();
        let glossary = "docs → /path/to/docs\ndocs → /path/to/other\n";
        let snap = write_pack(&dir.path, "soul\n", "user\n", "rules\n", glossary).expect("write");
        assert!(!snap.ok);
        let reason = snap.reason.expect("reason");
        assert!(reason.contains("invalid glossary"), "{reason}");
        assert!(reason.contains("duplicate alias"), "{reason}");
        assert_eq!(
            fs::read_to_string(dir.path.join("glossary.md")).expect("glossary"),
            glossary
        );
        assert_eq!(snap.glossary, glossary);
        assert!(dir.path.join("soul.md").is_file());
    }
}
