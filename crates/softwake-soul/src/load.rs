//! Read `soul.md` and `user.md`, then render one instruction document.

use std::fs::File;
use std::io::{ErrorKind, Read};
use std::path::Path;

use crate::{SoulError, SoulFile, SoulPaths};

/// Largest `soul.md` or `user.md` that will be loaded.
///
/// One mebibyte is enough for a voice prompt and rejects a multi-megabyte
/// paste before it is decoded.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;

// The allowlist name matches the phase-1 tool (`echo`). See ADR 0004.
const POLICY: &str = "\
State: awake.
Tool allowlist: echo.
Confirm rules: placeholder (confirmation is not wired yet).
";

/// Validated text of `soul.md` and `user.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoulPack {
    identity: String,
    profile: String,
}

impl SoulPack {
    /// `soul.md` body, unchanged aside from validation.
    #[must_use]
    pub fn identity(&self) -> &str {
        &self.identity
    }

    /// `user.md` body, unchanged aside from validation.
    #[must_use]
    pub fn user_profile(&self) -> &str {
        &self.profile
    }

    /// System instructions for an awake session.
    ///
    /// Sections, in order:
    ///
    /// - Identity (`soul.md`)
    /// - User profile (`user.md`)
    /// - Runtime policy stub: state is awake, the tool allowlist is `echo`,
    ///   and confirm rules are still a placeholder
    #[must_use]
    pub fn render_instructions(&self) -> String {
        format!(
            "# Identity\n\n{identity}\n\n# User profile\n\n{profile}\n\n# Runtime policy\n\n{POLICY}",
            identity = self.identity.trim_end(),
            profile = self.profile.trim_end(),
        )
    }
}

/// Whether a pack read succeeded, without retaining the file text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoulStatus {
    valid: bool,
    reason: Option<String>,
}

impl SoulStatus {
    /// The last read produced a [`SoulPack`].
    #[must_use]
    pub const fn valid() -> Self {
        Self {
            valid: true,
            reason: None,
        }
    }

    /// The last read failed. `reason` is a short operator-facing sentence.
    #[must_use]
    pub fn invalid(reason: impl Into<String>) -> Self {
        Self {
            valid: false,
            reason: Some(reason.into()),
        }
    }

    /// Status for a load. The [`SoulPack`] text is dropped.
    #[must_use]
    pub fn from_result(result: &Result<SoulPack, SoulError>) -> Self {
        match result {
            Ok(_) => Self::valid(),
            Err(error) => Self::invalid(error.to_string()),
        }
    }

    /// `true` when [`Self::from_result`] saw `Ok`.
    #[must_use]
    pub const fn is_valid(&self) -> bool {
        self.valid
    }

    /// Why the pack is invalid.
    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }
}

/// Read the two files [`SoulPaths`] names.
///
/// # Errors
///
/// Returns [`SoulError`] when either file is missing, empty, not UTF-8,
/// larger than [`MAX_FILE_BYTES`], or cannot be read. `soul.md` is checked
/// first.
pub fn load(paths: &SoulPaths) -> Result<SoulPack, SoulError> {
    let identity = read_markdown(paths.soul(), SoulFile::Soul)?;
    let profile = read_markdown(paths.user(), SoulFile::User)?;
    Ok(SoulPack { identity, profile })
}

/// Read `soul.md` and `user.md` from `dir`.
///
/// A missing directory is a missing `soul.md`, not a panic.
///
/// # Errors
///
/// See [`load`].
pub fn try_load(dir: &Path) -> Result<SoulPack, SoulError> {
    load(&SoulPaths::in_dir(dir))
}

fn read_markdown(path: &Path, file: SoulFile) -> Result<String, SoulError> {
    let path = path.to_path_buf();
    let opened = match File::open(&path) {
        Ok(opened) => opened,
        Err(source) if source.kind() == ErrorKind::NotFound => {
            return Err(SoulError::Missing { file, path });
        }
        Err(source) => return Err(SoulError::Read { file, path, source }),
    };

    let mut limited = opened.take(MAX_FILE_BYTES.saturating_add(1));
    let mut bytes = Vec::new();
    limited
        .read_to_end(&mut bytes)
        .map_err(|source| SoulError::Read {
            file,
            path: path.clone(),
            source,
        })?;

    let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if len > MAX_FILE_BYTES {
        return Err(SoulError::TooLarge {
            file,
            path,
            max: MAX_FILE_BYTES,
        });
    }

    let text = String::from_utf8(bytes).map_err(|_| SoulError::InvalidUtf8 {
        file,
        path: path.clone(),
    })?;
    if text.trim().is_empty() {
        return Err(SoulError::Empty { file, path });
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{MAX_FILE_BYTES, SoulStatus, load, try_load};
    use crate::{SoulError, SoulFile, SoulPaths};

    struct TempPack {
        path: PathBuf,
    }

    impl TempPack {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(1);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("softwake-soul-{}-{n}", std::process::id()));
            fs::create_dir_all(&path).expect("temp dir");
            Self { path }
        }

        fn write(&self, soul: &str, user: &str) {
            fs::write(self.path.join("soul.md"), soul).expect("soul");
            fs::write(self.path.join("user.md"), user).expect("user");
        }

        fn write_bytes(&self, name: &str, bytes: &[u8]) {
            let mut file = fs::File::create(self.path.join(name)).expect("create");
            file.write_all(bytes).expect("write");
        }
    }

    impl Drop for TempPack {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn happy_path_renders_identity_profile_and_policy() {
        let dir = TempPack::new();
        dir.write("I am Softwake.\n", "Name: Ada.\n");
        let pack = try_load(&dir.path).expect("load");
        assert_eq!(pack.identity(), "I am Softwake.\n");
        assert_eq!(pack.user_profile(), "Name: Ada.\n");
        assert_eq!(
            pack.render_instructions(),
            "# Identity\n\nI am Softwake.\n\n# User profile\n\nName: Ada.\n\n# Runtime policy\n\nState: awake.\nTool allowlist: echo.\nConfirm rules: placeholder (confirmation is not wired yet).\n"
        );
        let status = SoulStatus::from_result(&Ok(pack));
        assert!(status.is_valid());
        assert_eq!(status.reason(), None);
    }

    #[test]
    fn missing_directory_is_a_missing_soul_file() {
        let dir = TempPack::new();
        let missing = dir.path.join("nope");
        let error = try_load(&missing).expect_err("missing");
        assert!(matches!(
            error,
            SoulError::Missing {
                file: SoulFile::Soul,
                ..
            }
        ));
        assert!(error.to_string().contains("soul.md"));
        let status = SoulStatus::from_result(&Err(error));
        assert!(!status.is_valid());
        assert!(status.reason().unwrap_or("").contains("missing soul.md"));
    }

    #[test]
    fn missing_user_file_names_user_md() {
        let dir = TempPack::new();
        fs::write(dir.path.join("soul.md"), "present\n").expect("soul");
        let error = try_load(&dir.path).expect_err("missing user");
        assert!(matches!(
            error,
            SoulError::Missing {
                file: SoulFile::User,
                ..
            }
        ));
        assert!(error.to_string().contains("user.md"));
    }

    #[test]
    fn empty_and_whitespace_files_are_rejected() {
        let dir = TempPack::new();
        dir.write("", "present\n");
        let error = try_load(&dir.path).expect_err("empty soul");
        assert!(matches!(
            error,
            SoulError::Empty {
                file: SoulFile::Soul,
                ..
            }
        ));

        dir.write(" \n\t", "present\n");
        let error = try_load(&dir.path).expect_err("whitespace soul");
        assert!(matches!(error, SoulError::Empty { .. }));

        dir.write("present\n", "\n");
        let error = try_load(&dir.path).expect_err("empty user");
        assert!(matches!(
            error,
            SoulError::Empty {
                file: SoulFile::User,
                ..
            }
        ));
    }

    #[test]
    fn invalid_utf8_is_rejected() {
        let dir = TempPack::new();
        dir.write("placeholder\n", "user\n");
        dir.write_bytes("soul.md", b"ok\xff\xfe");
        let error = try_load(&dir.path).expect_err("utf-8 soul");
        assert!(matches!(
            error,
            SoulError::InvalidUtf8 {
                file: SoulFile::Soul,
                ..
            }
        ));
        assert!(error.to_string().contains("UTF-8"));

        dir.write("soul\n", "user\n");
        dir.write_bytes("user.md", &[0x80]);
        let error = try_load(&dir.path).expect_err("utf-8 user");
        assert!(matches!(
            error,
            SoulError::InvalidUtf8 {
                file: SoulFile::User,
                ..
            }
        ));
    }

    #[test]
    fn oversized_file_is_rejected_and_exact_cap_is_accepted() {
        let dir = TempPack::new();
        let max = usize::try_from(MAX_FILE_BYTES).expect("cap fits usize");
        dir.write_bytes("soul.md", &vec![b'a'; max]);
        dir.write_bytes("user.md", b"user\n");
        let pack = try_load(&dir.path).expect("exact cap");
        assert_eq!(pack.identity().len(), max);

        dir.write_bytes("soul.md", &vec![b'a'; max + 1]);
        let error = try_load(&dir.path).expect_err("over cap");
        assert!(error.to_string().contains("exceeds"));
        match error {
            SoulError::TooLarge {
                file: SoulFile::Soul,
                max: reported,
                ..
            } => assert_eq!(reported, MAX_FILE_BYTES),
            other => panic!("expected too large, got {other}"),
        }
    }

    #[test]
    fn reload_sees_changed_files() {
        let dir = TempPack::new();
        dir.write("first soul\n", "first user\n");
        let first = try_load(&dir.path).expect("first");
        assert!(first.render_instructions().contains("first soul"));
        assert!(first.render_instructions().contains("first user"));

        dir.write("second soul\n", "second user\n");
        let second = load(&SoulPaths::in_dir(&dir.path)).expect("second");
        assert_eq!(second.identity(), "second soul\n");
        assert_eq!(second.user_profile(), "second user\n");
        assert!(!second.render_instructions().contains("first soul"));
        assert!(second.render_instructions().contains("# Runtime policy"));
        assert!(second.render_instructions().contains("State: awake."));
    }

    #[test]
    fn reading_a_directory_as_a_file_is_an_io_error() {
        let dir = TempPack::new();
        let soul_as_dir = dir.path.join("not-a-file");
        fs::create_dir(&soul_as_dir).expect("dir");
        let user = dir.path.join("user.md");
        fs::write(&user, "user\n").expect("user");
        let error = load(&SoulPaths::new(soul_as_dir, user)).expect_err("directory");
        assert!(matches!(
            error,
            SoulError::Read {
                file: SoulFile::Soul,
                ..
            }
        ));
    }

    #[test]
    fn shipped_templates_load() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("soul");
        let pack = try_load(&root).expect("templates");
        let text = pack.render_instructions();
        assert!(text.contains("# Identity"));
        assert!(text.contains("# User profile"));
        assert!(text.contains("# Runtime policy"));
        assert!(text.contains("State: awake."));
        assert!(!text.trim().is_empty());
    }

    #[test]
    fn status_invalid_keeps_the_reason_without_a_pack() {
        let status = SoulStatus::invalid("missing soul.md");
        assert!(!status.is_valid());
        assert_eq!(status.reason(), Some("missing soul.md"));
    }
}
