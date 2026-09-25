//! Plaintext secret file and the version-2 pointer document.
//!
//! Writes are an atomic rename. The file mode is `0600`. A parent directory this
//! module creates is mode `0700`.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use crate::oauth::OAuthTokenSet;
use crate::secrets::{
    CURRENT_VERSION, KEYRING_SERVICE, KEYRING_USER, MAX_SECRETS_BYTES, POINTER_IDENTITY_MESSAGE,
    SecretBag, SecretStore, SecretStoreError, StorageReport, UNSUPPORTED_BACKEND_MESSAGE,
};

const SECRET_KEYS: &[&str] = &[
    "xai_api_key",
    "openai_api_key",
    "openrouter_api_key",
    "openai_compatible_api_key",
    "xai_oauth",
];

/// Parsed secrets file.
pub(crate) enum OnDisk {
    /// No file at the path.
    Missing,
    /// Version 1 plaintext bag. `plaintext` is forced on.
    Legacy(SecretBag),
    /// Version 2 plaintext bag.
    Plaintext(SecretBag),
    /// Version 2 keyring pointer. Secret fields were absent.
    Pointer,
}

/// Durable plaintext bag. Refuses to replace a keyring pointer.
#[derive(Debug, Clone)]
pub struct FileSecretStore {
    path: PathBuf,
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

    /// Load, mutate, save.
    ///
    /// # Errors
    ///
    /// Propagates load or save errors.
    pub fn update<F>(&self, mutator: F) -> Result<SecretBag, SecretStoreError>
    where
        F: FnOnce(&mut SecretBag),
    {
        crate::secrets::update_bag(self, mutator)
    }
}

impl SecretStore for FileSecretStore {
    fn load(&self) -> Result<SecretBag, SecretStoreError> {
        match read_on_disk(&self.path)? {
            OnDisk::Missing => Ok(SecretBag::empty()),
            OnDisk::Legacy(bag) | OnDisk::Plaintext(bag) => Ok(bag),
            OnDisk::Pointer => Err(SecretStoreError::WrongBackend {
                path: self.path.clone(),
            }),
        }
    }

    fn save(&self, bag: &SecretBag) -> Result<(), SecretStoreError> {
        match read_on_disk(&self.path) {
            Ok(OnDisk::Pointer) => {
                return Err(SecretStoreError::WrongBackend {
                    path: self.path.clone(),
                });
            }
            Ok(_) | Err(SecretStoreError::Invalid { .. }) => {}
            Err(error) => return Err(error),
        }
        write_plaintext(&self.path, bag)
    }

    fn report(&self) -> StorageReport {
        StorageReport::plaintext()
    }
}

/// Classify the file. Parse errors propagate. This does not probe the keyring.
///
/// # Errors
///
/// Size, JSON, version, pointer, and I/O errors.
pub(crate) fn classify(path: &Path) -> Result<crate::secrets::OnDiskKind, SecretStoreError> {
    Ok(match read_on_disk(path)? {
        OnDisk::Missing => crate::secrets::OnDiskKind::Missing,
        OnDisk::Legacy(_) => crate::secrets::OnDiskKind::LegacyV1,
        OnDisk::Plaintext(_) => crate::secrets::OnDiskKind::PlaintextV2,
        OnDisk::Pointer => crate::secrets::OnDiskKind::KeyringPointer,
    })
}

/// Read and classify. Does not create the file.
///
/// # Errors
///
/// Size, JSON, version, pointer, and I/O errors.
pub(crate) fn read_on_disk(path: &Path) -> Result<OnDisk, SecretStoreError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(OnDisk::Missing),
        Err(error) => return Err(io_err(path, error)),
    };
    if bytes.len() > MAX_SECRETS_BYTES {
        return Err(SecretStoreError::TooLarge {
            path: path.to_owned(),
            len: bytes.len(),
            max: MAX_SECRETS_BYTES,
        });
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| SecretStoreError::Invalid {
        path: path.to_owned(),
    })?;
    if !value.is_object() {
        return Err(SecretStoreError::Invalid {
            path: path.to_owned(),
        });
    }
    let Ok(version) = version_field(&value) else {
        return Err(SecretStoreError::Invalid {
            path: path.to_owned(),
        });
    };
    if version == 1 {
        return Ok(OnDisk::Legacy(legacy_bag(path, value)?));
    }
    if version != CURRENT_VERSION {
        return Err(SecretStoreError::UnsupportedVersion {
            path: path.to_owned(),
            version,
        });
    }
    match value.get("backend").and_then(Value::as_str) {
        Some("keyring") => parse_pointer(path, value),
        Some("plaintext") => Ok(OnDisk::Plaintext(plaintext_bag(path, value)?)),
        _ => Err(SecretStoreError::UnsupportedBackend {
            reason: UNSUPPORTED_BACKEND_MESSAGE,
        }),
    }
}

fn version_field(value: &Value) -> Result<u32, ()> {
    let Some(version) = value.get("version") else {
        return Ok(1);
    };
    if version.is_null() {
        return Ok(1);
    }
    let number = version.as_u64().ok_or(())?;
    u32::try_from(number).map_err(|_| ())
}

fn legacy_bag(path: &Path, value: Value) -> Result<SecretBag, SecretStoreError> {
    let mut bag: SecretBag =
        serde_json::from_value(value).map_err(|_| SecretStoreError::Invalid {
            path: path.to_owned(),
        })?;
    bag.version = 1;
    bag.plaintext = true;
    Ok(bag)
}

fn plaintext_bag(path: &Path, value: Value) -> Result<SecretBag, SecretStoreError> {
    let mut bag: SecretBag =
        serde_json::from_value(value).map_err(|_| SecretStoreError::Invalid {
            path: path.to_owned(),
        })?;
    bag.version = CURRENT_VERSION;
    bag.plaintext = true;
    Ok(bag)
}

fn parse_pointer(path: &Path, value: Value) -> Result<OnDisk, SecretStoreError> {
    if has_secret_key(&value) {
        return Err(SecretStoreError::PointerHasSecrets {
            path: path.to_owned(),
        });
    }
    let pointer: PointerFile =
        serde_json::from_value(value).map_err(|_| SecretStoreError::Invalid {
            path: path.to_owned(),
        })?;
    if pointer.version != CURRENT_VERSION
        || pointer.plaintext
        || pointer.backend != "keyring"
        || pointer.keyring_service != KEYRING_SERVICE
        || pointer.keyring_user != KEYRING_USER
    {
        return Err(SecretStoreError::UnsupportedBackend {
            reason: POINTER_IDENTITY_MESSAGE,
        });
    }
    Ok(OnDisk::Pointer)
}

fn has_secret_key(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    SECRET_KEYS.iter().any(|key| object.contains_key(*key))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PointerFile {
    version: u32,
    plaintext: bool,
    backend: String,
    keyring_service: String,
    keyring_user: String,
}

pub(crate) fn write_plaintext(path: &Path, bag: &SecretBag) -> Result<(), SecretStoreError> {
    let document = PlaintextFile {
        version: CURRENT_VERSION,
        plaintext: true,
        backend: "plaintext",
        plaintext_opt_in: true,
        xai_api_key: bag.xai_api_key.clone(),
        openai_api_key: bag.openai_api_key.clone(),
        openrouter_api_key: bag.openrouter_api_key.clone(),
        openai_compatible_api_key: bag.openai_compatible_api_key.clone(),
        xai_oauth: bag.xai_oauth.clone(),
    };
    let body = serde_json::to_vec_pretty(&document).map_err(|_| SecretStoreError::Invalid {
        path: path.to_owned(),
    })?;
    if body.len() > MAX_SECRETS_BYTES {
        return Err(SecretStoreError::TooLarge {
            path: path.to_owned(),
            len: body.len(),
            max: MAX_SECRETS_BYTES,
        });
    }
    atomic_write(path, &body)
}

pub(crate) fn write_pointer(path: &Path) -> Result<(), SecretStoreError> {
    let document = PointerWrite {
        version: CURRENT_VERSION,
        plaintext: false,
        backend: "keyring",
        keyring_service: KEYRING_SERVICE,
        keyring_user: KEYRING_USER,
    };
    let body = serde_json::to_vec_pretty(&document).map_err(|_| SecretStoreError::Invalid {
        path: path.to_owned(),
    })?;
    atomic_write(path, &body)
}

#[derive(Serialize)]
struct PlaintextFile {
    version: u32,
    plaintext: bool,
    backend: &'static str,
    plaintext_opt_in: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    xai_api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    openai_api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    openrouter_api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    openai_compatible_api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    xai_oauth: Option<OAuthTokenSet>,
}

#[derive(Serialize)]
struct PointerWrite {
    version: u32,
    plaintext: bool,
    backend: &'static str,
    keyring_service: &'static str,
    keyring_user: &'static str,
}

pub(crate) fn atomic_write(path: &Path, body: &[u8]) -> Result<(), SecretStoreError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(parent)
                .map_err(|error| io_err(path, error))?;
            let _ = set_mode(parent, 0o700);
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
    let _ = set_mode(path, 0o600);
    Ok(())
}

fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_mode(mode);
    fs::set_permissions(path, perms)
}

pub(crate) fn io_err(path: &Path, source: io::Error) -> SecretStoreError {
    SecretStoreError::Io {
        path: path.to_owned(),
        source: Box::new(source),
    }
}

#[cfg(test)]
#[path = "secrets_file_tests.rs"]
mod tests;
