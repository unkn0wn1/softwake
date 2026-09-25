//! Non-secret Tools Settings on disk.
//!
//! `$XDG_CONFIG_HOME/softwake/tools.json` (else `~/.config/softwake/tools.json`).
//! Tools stay off until the operator enables them. This file holds no secrets.

use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// File name under the Softwake config directory.
pub const TOOLS_FILE_NAME: &str = "tools.json";

/// Largest tools Settings file this backend will read, in bytes.
pub const MAX_TOOLS_SETTINGS_BYTES: usize = 64 * 1024;

const DOCUMENT_VERSION: u32 = 1;

/// When the shell tool must show a confirm-echo readback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmPolicy {
    /// Always confirm before running shell (default; remote/shell safe default).
    #[default]
    Always,
    /// Confirm when confirm-echo requires readback
    /// (mutating verb, alias, absolute/`~` path, or ssh/scp/rsync/sudo).
    MutatingOnly,
    /// Reserved. Same gate as [`Self::MutatingOnly`] in v1 (no allowlist table yet).
    AllowlistedQuiet,
}

impl ConfirmPolicy {
    /// Stable spelling for Settings and logs.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::MutatingOnly => "mutating_only",
            Self::AllowlistedQuiet => "allowlisted_quiet",
        }
    }

    /// Whether this policy forces a readback for `requires_readback` from confirm-echo.
    #[must_use]
    pub const fn must_confirm(self, requires_readback: bool) -> bool {
        match self {
            Self::Always => true,
            Self::MutatingOnly | Self::AllowlistedQuiet => requires_readback,
        }
    }
}

impl std::fmt::Display for ConfirmPolicy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Parse a confirm-policy string from Settings.
///
/// # Errors
///
/// Unknown spelling.
pub fn parse_confirm_policy(value: &str) -> Result<ConfirmPolicy, ToolsSettingsError> {
    match value {
        "always" => Ok(ConfirmPolicy::Always),
        "mutating_only" => Ok(ConfirmPolicy::MutatingOnly),
        "allowlisted_quiet" => Ok(ConfirmPolicy::AllowlistedQuiet),
        _ => Err(ToolsSettingsError::InvalidPolicy {
            value: value.to_owned(),
        }),
    }
}

/// Non-secret Tools Settings. Default: every tool off.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolsSettings {
    /// Document version.
    #[serde(default = "one")]
    pub version: u32,
    /// Operator opt-in for the shell tool. Default `false`.
    #[serde(default)]
    pub shell_enabled: bool,
    /// Confirm gate for shell. Default [`ConfirmPolicy::Always`].
    #[serde(default)]
    pub confirm_policy: ConfirmPolicy,
}

fn one() -> u32 {
    DOCUMENT_VERSION
}

impl Default for ToolsSettings {
    fn default() -> Self {
        Self {
            version: DOCUMENT_VERSION,
            shell_enabled: false,
            confirm_policy: ConfirmPolicy::Always,
        }
    }
}

/// Failure loading or saving tools Settings.
#[derive(Debug, thiserror::Error)]
pub enum ToolsSettingsError {
    /// Path was empty.
    #[error("tools Settings path is empty")]
    EmptyPath,
    /// Neither XDG config nor HOME is set.
    #[error("cannot resolve Softwake config directory: XDG_CONFIG_HOME and HOME are unset")]
    NoConfigDir,
    /// Policy string was not recognised.
    #[error("unknown confirm policy: {value}")]
    InvalidPolicy {
        /// Rejected spelling.
        value: String,
    },
    /// File is larger than [`MAX_TOOLS_SETTINGS_BYTES`].
    #[error("tools Settings file is too large ({len} > {max}): {}", path.display())]
    TooLarge {
        /// Path that was rejected.
        path: PathBuf,
        /// Observed length.
        len: usize,
        /// Allowed maximum.
        max: usize,
    },
    /// JSON or schema failure.
    #[error("invalid tools Settings at {}: {source}", path.display())]
    Invalid {
        /// Path that failed.
        path: PathBuf,
        /// Serde or related error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// Filesystem failure.
    #[error("tools Settings I/O at {}: {source}", path.display())]
    Io {
        /// Path that failed.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },
}

/// File-backed tools Settings.
#[derive(Debug, Clone)]
pub struct FileToolsSettings {
    path: PathBuf,
}

impl FileToolsSettings {
    /// Store at `path`.
    ///
    /// # Errors
    ///
    /// Empty path.
    pub fn new(path: impl Into<PathBuf>) -> Result<Self, ToolsSettingsError> {
        let path = path.into();
        if path.as_os_str().is_empty() {
            return Err(ToolsSettingsError::EmptyPath);
        }
        Ok(Self { path })
    }

    /// Path on disk.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load Settings. Missing file is the default document (shell off).
    ///
    /// # Errors
    ///
    /// Parse / I/O when the file exists but is unusable.
    pub fn load(&self) -> Result<ToolsSettings, ToolsSettingsError> {
        match fs::read(&self.path) {
            Ok(bytes) => decode(&self.path, &bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(ToolsSettings::default()),
            Err(error) => Err(io_err(&self.path, error)),
        }
    }

    /// Replace Settings on disk.
    ///
    /// # Errors
    ///
    /// I/O or serialize errors.
    pub fn save(&self, settings: &ToolsSettings) -> Result<(), ToolsSettingsError> {
        let mut document = settings.clone();
        document.version = DOCUMENT_VERSION;
        let body =
            serde_json::to_vec_pretty(&document).map_err(|source| ToolsSettingsError::Invalid {
                path: self.path.clone(),
                source: Box::new(source),
            })?;
        if body.len() > MAX_TOOLS_SETTINGS_BYTES {
            return Err(ToolsSettingsError::TooLarge {
                path: self.path.clone(),
                len: body.len(),
                max: MAX_TOOLS_SETTINGS_BYTES,
            });
        }
        atomic_write(&self.path, &body)
    }
}

/// `$XDG_CONFIG_HOME/softwake/tools.json`, else `~/.config/softwake/tools.json`.
///
/// # Errors
///
/// Both bases unset.
pub fn resolve_tools_file() -> Result<PathBuf, ToolsSettingsError> {
    resolve_tools_file_from(env::var_os("XDG_CONFIG_HOME"), env::var_os("HOME"))
}

/// Testable resolver.
///
/// # Errors
///
/// Both bases unset or blank.
pub fn resolve_tools_file_from(
    xdg_config_home: Option<impl AsRef<std::ffi::OsStr>>,
    home: Option<impl AsRef<std::ffi::OsStr>>,
) -> Result<PathBuf, ToolsSettingsError> {
    Ok(resolve_config_dir_from(xdg_config_home, home)?.join(TOOLS_FILE_NAME))
}

fn resolve_config_dir_from(
    xdg_config_home: Option<impl AsRef<std::ffi::OsStr>>,
    home: Option<impl AsRef<std::ffi::OsStr>>,
) -> Result<PathBuf, ToolsSettingsError> {
    if let Some(xdg) = xdg_config_home {
        let xdg = PathBuf::from(xdg.as_ref());
        if !xdg.as_os_str().is_empty() {
            return Ok(xdg.join("softwake"));
        }
    }
    if let Some(home) = home {
        let home = PathBuf::from(home.as_ref());
        if !home.as_os_str().is_empty() {
            return Ok(home.join(".config").join("softwake"));
        }
    }
    Err(ToolsSettingsError::NoConfigDir)
}

fn decode(path: &Path, bytes: &[u8]) -> Result<ToolsSettings, ToolsSettingsError> {
    if bytes.len() > MAX_TOOLS_SETTINGS_BYTES {
        return Err(ToolsSettingsError::TooLarge {
            path: path.to_owned(),
            len: bytes.len(),
            max: MAX_TOOLS_SETTINGS_BYTES,
        });
    }
    let mut settings: ToolsSettings =
        serde_json::from_slice(bytes).map_err(|source| ToolsSettingsError::Invalid {
            path: path.to_owned(),
            source: Box::new(source),
        })?;
    settings.version = DOCUMENT_VERSION;
    Ok(settings)
}

fn atomic_write(path: &Path, body: &[u8]) -> Result<(), ToolsSettingsError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            #[cfg(unix)]
            {
                fs::DirBuilder::new()
                    .recursive(true)
                    .mode(0o700)
                    .create(parent)
                    .map_err(|error| io_err(path, error))?;
            }
            #[cfg(not(unix))]
            {
                fs::create_dir_all(parent).map_err(|error| io_err(path, error))?;
            }
            let _ = set_mode(parent, 0o700);
        }
    }
    let temp = path.with_extension("json.tmp");
    {
        #[cfg(unix)]
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temp)
            .map_err(|error| io_err(path, error))?;
        #[cfg(not(unix))]
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temp)
            .map_err(|error| io_err(path, error))?;
        file.write_all(body).map_err(|error| io_err(path, error))?;
        file.sync_all().map_err(|error| io_err(path, error))?;
    }
    let _ = set_mode(&temp, 0o600);
    fs::rename(&temp, path).map_err(|error| io_err(path, error))?;
    let _ = set_mode(path, 0o600);
    Ok(())
}

fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(mode);
        fs::set_permissions(path, permissions)?;
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
    }
    Ok(())
}

fn io_err(path: &Path, source: io::Error) -> ToolsSettingsError {
    ToolsSettingsError::Io {
        path: path.to_owned(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ConfirmPolicy, FileToolsSettings, ToolsSettings, parse_confirm_policy,
        resolve_tools_file_from,
    };

    #[test]
    fn default_keeps_shell_off_and_always_confirms() {
        let settings = ToolsSettings::default();
        assert!(!settings.shell_enabled);
        assert_eq!(settings.confirm_policy, ConfirmPolicy::Always);
        assert!(settings.confirm_policy.must_confirm(false));
        assert!(ConfirmPolicy::MutatingOnly.must_confirm(true));
        assert!(!ConfirmPolicy::MutatingOnly.must_confirm(false));
        assert_eq!(
            ConfirmPolicy::AllowlistedQuiet.as_str(),
            "allowlisted_quiet"
        );
    }

    #[test]
    fn parse_policy_accepts_known_spellings() {
        assert_eq!(
            parse_confirm_policy("always").expect("always"),
            ConfirmPolicy::Always
        );
        assert_eq!(
            parse_confirm_policy("mutating_only").expect("mutating"),
            ConfirmPolicy::MutatingOnly
        );
        assert!(parse_confirm_policy("sometimes").is_err());
    }

    #[test]
    fn resolve_prefers_xdg_config() {
        let path = resolve_tools_file_from(Some("/cfg"), Some("/home")).expect("path");
        assert_eq!(path, std::path::PathBuf::from("/cfg/softwake/tools.json"));
    }

    #[test]
    fn round_trip_save_load() {
        let dir =
            std::env::temp_dir().join(format!("softwake-tools-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("tools.json");
        let store = FileToolsSettings::new(&path).expect("store");
        assert_eq!(
            store.load().expect("missing default"),
            ToolsSettings::default()
        );
        let settings = ToolsSettings {
            shell_enabled: true,
            confirm_policy: ConfirmPolicy::MutatingOnly,
            ..ToolsSettings::default()
        };
        store.save(&settings).expect("save");
        assert_eq!(store.load().expect("load"), settings);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
