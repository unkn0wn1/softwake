//! Secret bag on disk.
//!
//! The bag is plaintext at rest in v1. The document carries `"plaintext": true`
//! so a later encrypting format can refuse or migrate it. File mode is `0600`.

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};

use crate::oauth::OAuthTokenSet;

/// File name under the Softwake state directory.
pub const SECRETS_FILE_NAME: &str = "secrets.json";

/// Largest secrets file this backend will read, in bytes.
pub const MAX_SECRETS_BYTES: usize = 256 * 1024;

const DOCUMENT_VERSION: u32 = 1;

/// Warning printed conceptually on load/save. Callers may surface it in UI.
pub const PLAINTEXT_WARNING: &str = "Softwake stores provider secrets in a local plaintext file (mode 0600). Prefer the environment for keys you do not want on disk.";

/// In-memory secret bag. Never log its fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SecretBag {
    /// Document version.
    #[serde(default = "one")]
    pub version: u32,
    /// Always true in v1. Marks plaintext-at-rest.
    #[serde(default = "always_true")]
    pub plaintext: bool,
    /// Saved xAI API key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xai_api_key: Option<String>,
    /// Saved `OpenAI` API key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_api_key: Option<String>,
    /// xAI OAuth tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xai_oauth: Option<OAuthTokenSet>,
}

fn one() -> u32 {
    1
}

fn always_true() -> bool {
    true
}

impl SecretBag {
    /// Empty bag.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            version: DOCUMENT_VERSION,
            plaintext: true,
            xai_api_key: None,
            openai_api_key: None,
            xai_oauth: None,
        }
    }
}

/// Durable secret bag. Creates parent dirs mode `0700` and the file mode `0600`.
#[derive(Debug, Clone)]
pub struct FileSecretStore {
    path: PathBuf,
}

/// Failure from [`FileSecretStore`].
#[derive(Debug, thiserror::Error)]
pub enum SecretStoreError {
    /// `XDG_STATE_HOME` and `HOME` were both unset or blank.
    #[error("provider secrets state directory is unset")]
    NoStateDir,

    /// Path was empty.
    #[error("provider secrets path is empty")]
    EmptyPath,

    /// Filesystem failure.
    #[error("provider secrets file {} could not be accessed", path.display())]
    Io {
        /// Path involved.
        path: PathBuf,
        /// Source error.
        #[source]
        source: Box<io::Error>,
    },

    /// JSON parse failure.
    #[error("provider secrets file {} is not valid json", path.display())]
    Invalid {
        /// Path read.
        path: PathBuf,
        /// Parse error.
        #[source]
        source: Box<serde_json::Error>,
    },

    /// Unsupported document version.
    #[error("provider secrets file {} has unsupported version {version}", path.display())]
    UnsupportedVersion {
        /// Path read.
        path: PathBuf,
        /// Version field.
        version: u32,
    },

    /// File too large.
    #[error("provider secrets file {} is {len} bytes; max is {max}", path.display())]
    TooLarge {
        /// Path read.
        path: PathBuf,
        /// Byte length.
        len: usize,
        /// Cap.
        max: usize,
    },
}

impl FileSecretStore {
    /// Store at `path`. Does not read or create the file yet.
    ///
    /// # Errors
    ///
    /// Returns [`SecretStoreError::EmptyPath`] when `path` is empty.
    pub fn new(path: impl Into<PathBuf>) -> Result<Self, SecretStoreError> {
        let path = path.into();
        if path.as_os_str().is_empty() {
            return Err(SecretStoreError::EmptyPath);
        }
        Ok(Self { path })
    }

    /// Path this store reads and writes.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load the bag. A missing file is an empty bag.
    ///
    /// # Errors
    ///
    /// Returns parse, version, size, or I/O errors when the file exists but is unusable.
    pub fn load(&self) -> Result<SecretBag, SecretStoreError> {
        match fs::read(&self.path) {
            Ok(bytes) => decode_bytes(&self.path, &bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(SecretBag::empty()),
            Err(error) => Err(io_err(&self.path, error)),
        }
    }

    /// Replace the bag on disk.
    ///
    /// # Errors
    ///
    /// Returns I/O errors when the directory or file cannot be written.
    pub fn save(&self, bag: &SecretBag) -> Result<(), SecretStoreError> {
        let mut document = bag.clone();
        document.version = DOCUMENT_VERSION;
        document.plaintext = true;
        let body =
            serde_json::to_vec_pretty(&document).map_err(|source| SecretStoreError::Invalid {
                path: self.path.clone(),
                source: Box::new(source),
            })?;
        if body.len() > MAX_SECRETS_BYTES {
            return Err(SecretStoreError::TooLarge {
                path: self.path.clone(),
                len: body.len(),
                max: MAX_SECRETS_BYTES,
            });
        }
        atomic_write(&self.path, &body)
    }

    /// Load, mutate, save.
    ///
    /// # Errors
    ///
    /// Propagates load or save errors.
    pub fn update<F>(&self, mutator: F) -> Result<SecretBag, SecretStoreError>
    where
        F: FnOnce(&mut SecretBag),
    {
        let mut bag = self.load()?;
        mutator(&mut bag);
        self.save(&bag)?;
        Ok(bag)
    }
}

/// `$XDG_STATE_HOME/softwake/secrets.json`, else `~/.local/state/softwake/secrets.json`.
///
/// # Errors
///
/// Returns [`SecretStoreError::NoStateDir`] when both bases are unset.
pub fn resolve_secrets_file() -> Result<PathBuf, SecretStoreError> {
    resolve_secrets_file_from(env::var_os("XDG_STATE_HOME"), env::var_os("HOME"))
}

/// Testable path resolver.
///
/// # Errors
///
/// Returns [`SecretStoreError::NoStateDir`] when both bases are unset or blank.
pub fn resolve_secrets_file_from(
    xdg_state_home: Option<impl AsRef<std::ffi::OsStr>>,
    home: Option<impl AsRef<std::ffi::OsStr>>,
) -> Result<PathBuf, SecretStoreError> {
    Ok(resolve_state_dir_from(xdg_state_home, home)?.join(SECRETS_FILE_NAME))
}

fn resolve_state_dir_from(
    xdg_state_home: Option<impl AsRef<std::ffi::OsStr>>,
    home: Option<impl AsRef<std::ffi::OsStr>>,
) -> Result<PathBuf, SecretStoreError> {
    if let Some(xdg) = trimmed_os(xdg_state_home) {
        return Ok(PathBuf::from(xdg).join("softwake"));
    }
    if let Some(home) = trimmed_os(home) {
        return Ok(PathBuf::from(home).join(".local/state/softwake"));
    }
    Err(SecretStoreError::NoStateDir)
}

fn trimmed_os(value: Option<impl AsRef<std::ffi::OsStr>>) -> Option<std::ffi::OsString> {
    let value = value?;
    let text = value.as_ref().to_string_lossy();
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(std::ffi::OsString::from(trimmed))
    }
}

fn decode_bytes(path: &Path, bytes: &[u8]) -> Result<SecretBag, SecretStoreError> {
    if bytes.len() > MAX_SECRETS_BYTES {
        return Err(SecretStoreError::TooLarge {
            path: path.to_owned(),
            len: bytes.len(),
            max: MAX_SECRETS_BYTES,
        });
    }
    let bag: SecretBag =
        serde_json::from_slice(bytes).map_err(|source| SecretStoreError::Invalid {
            path: path.to_owned(),
            source: Box::new(source),
        })?;
    if bag.version != DOCUMENT_VERSION {
        return Err(SecretStoreError::UnsupportedVersion {
            path: path.to_owned(),
            version: bag.version,
        });
    }
    Ok(bag)
}

fn atomic_write(path: &Path, body: &[u8]) -> Result<(), SecretStoreError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(parent)
                .map_err(|error| io_err(path, error))?;
        }
    }
    let temp = path.with_extension("json.tmp");
    {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temp)
            .map_err(|error| io_err(path, error))?;
        file.write_all(body).map_err(|error| io_err(path, error))?;
        file.sync_all().map_err(|error| io_err(path, error))?;
    }
    fs::rename(&temp, path).map_err(|error| io_err(path, error))?;
    // Best-effort mode on the final path (rename may preserve temp mode).
    let _ = File::open(path).and_then(|file| {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = file.metadata()?.permissions();
        perms.set_mode(0o600);
        fs::set_permissions(path, perms)
    });
    Ok(())
}

fn io_err(path: &Path, source: io::Error) -> SecretStoreError {
    SecretStoreError::Io {
        path: path.to_owned(),
        source: Box::new(source),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{FileSecretStore, PLAINTEXT_WARNING, resolve_secrets_file_from};
    use crate::oauth::OAuthTokenSet;

    struct TempDir {
        path: std::path::PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(1);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "softwake-providers-secrets-{}-{n}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).expect("temp");
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn round_trip_keys_and_tokens() {
        let dir = TempDir::new();
        let store = FileSecretStore::new(dir.path.join("secrets.json")).expect("store");
        assert!(store.load().expect("missing").xai_api_key.is_none());
        store
            .update(|bag| {
                bag.xai_api_key = Some("xai-secret".to_owned());
                bag.openai_api_key = Some("sk-test".to_owned());
                bag.xai_oauth = Some(OAuthTokenSet {
                    access_token: "a".to_owned(),
                    refresh_token: "r".to_owned(),
                    expires_at_ms: 9,
                    token_type: "Bearer".to_owned(),
                });
            })
            .expect("save");
        let loaded = store.load().expect("load");
        assert_eq!(loaded.xai_api_key.as_deref(), Some("xai-secret"));
        assert_eq!(loaded.openai_api_key.as_deref(), Some("sk-test"));
        assert_eq!(
            loaded.xai_oauth.as_ref().map(|t| t.refresh_token.as_str()),
            Some("r")
        );
        assert!(loaded.plaintext);
        assert!(!PLAINTEXT_WARNING.is_empty());
    }

    #[test]
    fn resolve_prefers_xdg_state() {
        let path = resolve_secrets_file_from(Some("/state"), Some("/home")).expect("path");
        assert_eq!(
            path,
            std::path::PathBuf::from("/state/softwake/secrets.json")
        );
    }
}
