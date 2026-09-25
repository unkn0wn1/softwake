//! Non-secret provider Settings on disk.

use std::collections::BTreeMap;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};

use crate::ids::ProviderId;

/// File name under the Softwake config directory.
pub const PROVIDERS_FILE_NAME: &str = "providers.json";

/// Largest providers Settings file this backend will read, in bytes.
pub const MAX_SETTINGS_BYTES: usize = 256 * 1024;

const DOCUMENT_VERSION: u32 = 1;

/// Cached model list from a successful Test.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ModelCache {
    /// Chat model ids in catalog order.
    #[serde(default)]
    pub chat_models: Vec<String>,
    /// Unix milliseconds when Test stored this list. Zero means unknown.
    #[serde(default)]
    pub fetched_at_ms: u64,
}

/// Last Test outcome shown in Settings. No secrets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TestReport {
    /// Whether the last Test passed.
    pub ok: bool,
    /// Safe display message.
    pub message: String,
}

/// Non-secret provider Settings document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderSettings {
    /// Document version.
    #[serde(default = "one")]
    pub version: u32,
    /// Selected provider for the acting session.
    pub selected_provider: ProviderId,
    /// Selected chat model id for that provider. Empty until the operator picks one.
    #[serde(default)]
    pub selected_model: String,
    /// Per-provider model catalogs from Test.
    #[serde(default)]
    pub model_cache: BTreeMap<String, ModelCache>,
    /// Last Test report per provider id string.
    #[serde(default)]
    pub last_test: BTreeMap<String, TestReport>,
}

fn one() -> u32 {
    1
}

impl Default for ProviderSettings {
    fn default() -> Self {
        Self {
            version: DOCUMENT_VERSION,
            selected_provider: ProviderId::XaiKey,
            selected_model: String::new(),
            model_cache: BTreeMap::new(),
            last_test: BTreeMap::new(),
        }
    }
}

impl ProviderSettings {
    /// Model ids shown in the picker for `provider`. Empty until Test.
    #[must_use]
    pub fn models_for(&self, provider: ProviderId) -> &[String] {
        self.model_cache
            .get(provider.as_str())
            .map_or(&[], |cache| cache.chat_models.as_slice())
    }

    /// Store a catalog after a passing Test.
    pub fn store_models(&mut self, provider: ProviderId, models: Vec<String>, fetched_at_ms: u64) {
        self.model_cache.insert(
            provider.as_str().to_owned(),
            ModelCache {
                chat_models: models,
                fetched_at_ms,
            },
        );
    }

    /// Record a Test outcome without clearing a previous catalog on failure.
    pub fn store_test(&mut self, provider: ProviderId, report: TestReport) {
        self.last_test.insert(provider.as_str().to_owned(), report);
    }
}

/// Durable non-secret Settings.
#[derive(Debug, Clone)]
pub struct FileProviderSettings {
    path: PathBuf,
}

/// Failure from [`FileProviderSettings`].
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    /// Config base unset.
    #[error("provider settings config directory is unset")]
    NoConfigDir,

    /// Empty path.
    #[error("provider settings path is empty")]
    EmptyPath,

    /// I/O failure.
    #[error("provider settings file {} could not be accessed", path.display())]
    Io {
        /// Path.
        path: PathBuf,
        /// Source.
        #[source]
        source: Box<io::Error>,
    },

    /// Bad JSON.
    #[error("provider settings file {} is not valid json", path.display())]
    Invalid {
        /// Path.
        path: PathBuf,
        /// Source.
        #[source]
        source: Box<serde_json::Error>,
    },

    /// Bad version.
    #[error("provider settings file {} has unsupported version {version}", path.display())]
    UnsupportedVersion {
        /// Path.
        path: PathBuf,
        /// Version.
        version: u32,
    },

    /// Too large.
    #[error("provider settings file {} is {len} bytes; max is {max}", path.display())]
    TooLarge {
        /// Path.
        path: PathBuf,
        /// Length.
        len: usize,
        /// Cap.
        max: usize,
    },
}

impl FileProviderSettings {
    /// Store at `path`.
    ///
    /// # Errors
    ///
    /// Empty path.
    pub fn new(path: impl Into<PathBuf>) -> Result<Self, SettingsError> {
        let path = path.into();
        if path.as_os_str().is_empty() {
            return Err(SettingsError::EmptyPath);
        }
        Ok(Self { path })
    }

    /// Path on disk.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load Settings. Missing file is the default document.
    ///
    /// # Errors
    ///
    /// Parse / I/O when the file exists but is unusable.
    pub fn load(&self) -> Result<ProviderSettings, SettingsError> {
        match fs::read(&self.path) {
            Ok(bytes) => decode(&self.path, &bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Ok(ProviderSettings::default())
            }
            Err(error) => Err(io_err(&self.path, error)),
        }
    }

    /// Replace Settings on disk.
    ///
    /// # Errors
    ///
    /// I/O or serialize errors.
    pub fn save(&self, settings: &ProviderSettings) -> Result<(), SettingsError> {
        let mut document = settings.clone();
        document.version = DOCUMENT_VERSION;
        let body =
            serde_json::to_vec_pretty(&document).map_err(|source| SettingsError::Invalid {
                path: self.path.clone(),
                source: Box::new(source),
            })?;
        if body.len() > MAX_SETTINGS_BYTES {
            return Err(SettingsError::TooLarge {
                path: self.path.clone(),
                len: body.len(),
                max: MAX_SETTINGS_BYTES,
            });
        }
        atomic_write(&self.path, &body)
    }
}

/// `$XDG_CONFIG_HOME/softwake/providers.json`, else `~/.config/softwake/providers.json`.
///
/// # Errors
///
/// Both bases unset.
pub fn resolve_providers_file() -> Result<PathBuf, SettingsError> {
    resolve_providers_file_from(env::var_os("XDG_CONFIG_HOME"), env::var_os("HOME"))
}

/// Testable resolver.
///
/// # Errors
///
/// Both bases unset or blank.
pub fn resolve_providers_file_from(
    xdg_config_home: Option<impl AsRef<std::ffi::OsStr>>,
    home: Option<impl AsRef<std::ffi::OsStr>>,
) -> Result<PathBuf, SettingsError> {
    Ok(resolve_config_dir_from(xdg_config_home, home)?.join(PROVIDERS_FILE_NAME))
}

fn resolve_config_dir_from(
    xdg_config_home: Option<impl AsRef<std::ffi::OsStr>>,
    home: Option<impl AsRef<std::ffi::OsStr>>,
) -> Result<PathBuf, SettingsError> {
    if let Some(xdg) = trimmed_os(xdg_config_home) {
        return Ok(PathBuf::from(xdg).join("softwake"));
    }
    if let Some(home) = trimmed_os(home) {
        return Ok(PathBuf::from(home).join(".config/softwake"));
    }
    Err(SettingsError::NoConfigDir)
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

fn decode(path: &Path, bytes: &[u8]) -> Result<ProviderSettings, SettingsError> {
    if bytes.len() > MAX_SETTINGS_BYTES {
        return Err(SettingsError::TooLarge {
            path: path.to_owned(),
            len: bytes.len(),
            max: MAX_SETTINGS_BYTES,
        });
    }
    let settings: ProviderSettings =
        serde_json::from_slice(bytes).map_err(|source| SettingsError::Invalid {
            path: path.to_owned(),
            source: Box::new(source),
        })?;
    if settings.version != DOCUMENT_VERSION {
        return Err(SettingsError::UnsupportedVersion {
            path: path.to_owned(),
            version: settings.version,
        });
    }
    Ok(settings)
}

fn atomic_write(path: &Path, body: &[u8]) -> Result<(), SettingsError> {
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
    let _ = File::open(path).and_then(|file| {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = file.metadata()?.permissions();
        perms.set_mode(0o600);
        fs::set_permissions(path, perms)
    });
    Ok(())
}

fn io_err(path: &Path, source: io::Error) -> SettingsError {
    SettingsError::Io {
        path: path.to_owned(),
        source: Box::new(source),
    }
}

#[cfg(test)]
mod tests {
    use super::{FileProviderSettings, ProviderSettings, resolve_providers_file_from};
    use crate::ids::ProviderId;

    #[test]
    fn default_picker_empty_until_test() {
        let settings = ProviderSettings::default();
        assert!(settings.models_for(ProviderId::XaiKey).is_empty());
    }

    #[test]
    fn resolve_prefers_xdg_config() {
        let path = resolve_providers_file_from(Some("/cfg"), Some("/home")).expect("path");
        assert_eq!(
            path,
            std::path::PathBuf::from("/cfg/softwake/providers.json")
        );
    }

    #[test]
    fn round_trip_settings_file() {
        let dir = std::env::temp_dir().join(format!(
            "softwake-providers-settings-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let store = FileProviderSettings::new(dir.join("providers.json")).expect("store");
        let mut settings = ProviderSettings {
            selected_provider: ProviderId::Openai,
            ..ProviderSettings::default()
        };
        settings.store_models(ProviderId::Openai, vec!["gpt-4.1-mini".to_owned()], 42);
        store.save(&settings).expect("save");
        let loaded = store.load().expect("load");
        assert_eq!(loaded.selected_provider, ProviderId::Openai);
        assert_eq!(
            loaded.models_for(ProviderId::Openai),
            &["gpt-4.1-mini".to_owned()]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
