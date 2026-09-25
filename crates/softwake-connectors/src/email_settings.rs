//! Non-secret live email Settings on disk.
//!
//! The SMTP password stays in the provider secret bag. This file is
//! `$XDG_CONFIG_HOME/softwake/email.json` (else `~/.config/softwake/email.json`).
//! Live email defaults to off so CI never needs credentials.

use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// File name under the Softwake config directory.
pub const EMAIL_FILE_NAME: &str = "email.json";

/// Largest email Settings file this backend will read, in bytes.
pub const MAX_EMAIL_SETTINGS_BYTES: usize = 64 * 1024;

const DOCUMENT_VERSION: u32 = 1;

/// Default SMTP submission port.
pub const DEFAULT_SMTP_PORT: u16 = 587;

/// How a live backend treats an authorized send.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LiveEmailMode {
    /// Append to a local draft list after confirm. Default. No socket.
    #[default]
    DraftOnly,
    /// Intended real send. This scaffold still returns not-wired; confirm stays required.
    Send,
}

impl LiveEmailMode {
    /// Stable spelling for Settings and logs.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DraftOnly => "draft_only",
            Self::Send => "send",
        }
    }
}

impl std::fmt::Display for LiveEmailMode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Parse a mode string from Settings.
///
/// # Errors
///
/// Unknown spelling.
pub fn parse_live_email_mode(value: &str) -> Result<LiveEmailMode, EmailSettingsError> {
    match value {
        "draft_only" => Ok(LiveEmailMode::DraftOnly),
        "send" => Ok(LiveEmailMode::Send),
        _ => Err(EmailSettingsError::InvalidMode {
            value: value.to_owned(),
        }),
    }
}

/// Non-secret live email Settings. Password is not stored here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmailSettings {
    /// Document version.
    #[serde(default = "one")]
    pub version: u32,
    /// Operator opt-in. Default `false`.
    #[serde(default)]
    pub live_enabled: bool,
    /// SMTP host. Empty until set.
    #[serde(default)]
    pub smtp_host: String,
    /// SMTP port. Default [`DEFAULT_SMTP_PORT`].
    #[serde(default = "default_port")]
    pub smtp_port: u16,
    /// SMTP username. Empty until set.
    #[serde(default)]
    pub username: String,
    /// From address text. Empty until set. Not validated.
    #[serde(default)]
    pub from_address: String,
    /// Draft-only (default) or send scaffold.
    #[serde(default)]
    pub mode: LiveEmailMode,
    /// Last Test ok flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_test_ok: Option<bool>,
    /// Last Test message. Never a password.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub last_test_message: String,
}

fn one() -> u32 {
    DOCUMENT_VERSION
}

fn default_port() -> u16 {
    DEFAULT_SMTP_PORT
}

impl Default for EmailSettings {
    fn default() -> Self {
        Self {
            version: DOCUMENT_VERSION,
            live_enabled: false,
            smtp_host: String::new(),
            smtp_port: DEFAULT_SMTP_PORT,
            username: String::new(),
            from_address: String::new(),
            mode: LiveEmailMode::DraftOnly,
            last_test_ok: None,
            last_test_message: String::new(),
        }
    }
}

impl EmailSettings {
    /// Host, username, and from are non-empty (password checked separately).
    #[must_use]
    pub fn is_smtp_complete(&self) -> bool {
        !self.smtp_host.trim().is_empty()
            && self.smtp_port != 0
            && !self.username.trim().is_empty()
            && !self.from_address.trim().is_empty()
    }
}

/// Failure loading or saving email Settings.
#[derive(Debug, thiserror::Error)]
pub enum EmailSettingsError {
    /// Path was empty.
    #[error("email Settings path is empty")]
    EmptyPath,
    /// Neither XDG config nor HOME is set.
    #[error("cannot resolve Softwake config directory: XDG_CONFIG_HOME and HOME are unset")]
    NoConfigDir,
    /// Mode string was not `draft_only` or `send`.
    #[error("unknown live email mode: {value}")]
    InvalidMode {
        /// Rejected spelling.
        value: String,
    },
    /// File is larger than [`MAX_EMAIL_SETTINGS_BYTES`].
    #[error("email Settings file is too large ({len} > {max}): {}", path.display())]
    TooLarge {
        /// Path that was rejected.
        path: PathBuf,
        /// Observed length.
        len: usize,
        /// Allowed maximum.
        max: usize,
    },
    /// JSON or schema failure.
    #[error("invalid email Settings at {}: {source}", path.display())]
    Invalid {
        /// Path that failed.
        path: PathBuf,
        /// Serde or related error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// Filesystem failure.
    #[error("email Settings I/O at {}: {source}", path.display())]
    Io {
        /// Path that failed.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },
}

/// File-backed email Settings.
#[derive(Debug, Clone)]
pub struct FileEmailSettings {
    path: PathBuf,
}

impl FileEmailSettings {
    /// Store at `path`.
    ///
    /// # Errors
    ///
    /// Empty path.
    pub fn new(path: impl Into<PathBuf>) -> Result<Self, EmailSettingsError> {
        let path = path.into();
        if path.as_os_str().is_empty() {
            return Err(EmailSettingsError::EmptyPath);
        }
        Ok(Self { path })
    }

    /// Path on disk.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load Settings. Missing file is the default document (live off).
    ///
    /// # Errors
    ///
    /// Parse / I/O when the file exists but is unusable.
    pub fn load(&self) -> Result<EmailSettings, EmailSettingsError> {
        match fs::read(&self.path) {
            Ok(bytes) => decode(&self.path, &bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(EmailSettings::default()),
            Err(error) => Err(io_err(&self.path, error)),
        }
    }

    /// Replace Settings on disk.
    ///
    /// # Errors
    ///
    /// I/O or serialize errors.
    pub fn save(&self, settings: &EmailSettings) -> Result<(), EmailSettingsError> {
        let mut document = settings.clone();
        document.version = DOCUMENT_VERSION;
        let body =
            serde_json::to_vec_pretty(&document).map_err(|source| EmailSettingsError::Invalid {
                path: self.path.clone(),
                source: Box::new(source),
            })?;
        if body.len() > MAX_EMAIL_SETTINGS_BYTES {
            return Err(EmailSettingsError::TooLarge {
                path: self.path.clone(),
                len: body.len(),
                max: MAX_EMAIL_SETTINGS_BYTES,
            });
        }
        atomic_write(&self.path, &body)
    }
}

/// `$XDG_CONFIG_HOME/softwake/email.json`, else `~/.config/softwake/email.json`.
///
/// # Errors
///
/// Both bases unset.
pub fn resolve_email_file() -> Result<PathBuf, EmailSettingsError> {
    resolve_email_file_from(env::var_os("XDG_CONFIG_HOME"), env::var_os("HOME"))
}

/// Testable resolver.
///
/// # Errors
///
/// Both bases unset or blank.
pub fn resolve_email_file_from(
    xdg_config_home: Option<impl AsRef<std::ffi::OsStr>>,
    home: Option<impl AsRef<std::ffi::OsStr>>,
) -> Result<PathBuf, EmailSettingsError> {
    Ok(resolve_config_dir_from(xdg_config_home, home)?.join(EMAIL_FILE_NAME))
}

fn resolve_config_dir_from(
    xdg_config_home: Option<impl AsRef<std::ffi::OsStr>>,
    home: Option<impl AsRef<std::ffi::OsStr>>,
) -> Result<PathBuf, EmailSettingsError> {
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
    Err(EmailSettingsError::NoConfigDir)
}

fn decode(path: &Path, bytes: &[u8]) -> Result<EmailSettings, EmailSettingsError> {
    if bytes.len() > MAX_EMAIL_SETTINGS_BYTES {
        return Err(EmailSettingsError::TooLarge {
            path: path.to_owned(),
            len: bytes.len(),
            max: MAX_EMAIL_SETTINGS_BYTES,
        });
    }
    let mut settings: EmailSettings =
        serde_json::from_slice(bytes).map_err(|source| EmailSettingsError::Invalid {
            path: path.to_owned(),
            source: Box::new(source),
        })?;
    settings.version = DOCUMENT_VERSION;
    Ok(settings)
}

fn atomic_write(path: &Path, body: &[u8]) -> Result<(), EmailSettingsError> {
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

fn io_err(path: &Path, source: io::Error) -> EmailSettingsError {
    EmailSettingsError::Io {
        path: path.to_owned(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EmailSettings, FileEmailSettings, LiveEmailMode, parse_live_email_mode,
        resolve_email_file_from,
    };

    #[test]
    fn default_is_live_off_draft_only() {
        let settings = EmailSettings::default();
        assert!(!settings.live_enabled);
        assert_eq!(settings.mode, LiveEmailMode::DraftOnly);
        assert_eq!(settings.smtp_port, 587);
        assert!(!settings.is_smtp_complete());
    }

    #[test]
    fn resolve_prefers_xdg() {
        let path = resolve_email_file_from(Some("/cfg"), Some("/home")).expect("path");
        assert_eq!(path, std::path::PathBuf::from("/cfg/softwake/email.json"));
    }

    #[test]
    fn round_trip_file() {
        let dir =
            std::env::temp_dir().join(format!("softwake-email-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("email.json");
        let store = FileEmailSettings::new(&path).expect("store");
        assert_eq!(store.load().expect("missing"), EmailSettings::default());
        let mut settings = EmailSettings {
            live_enabled: true,
            smtp_host: "smtp.example.com".to_owned(),
            username: "ada".to_owned(),
            from_address: "ada@example.com".to_owned(),
            mode: LiveEmailMode::Send,
            last_test_ok: Some(true),
            last_test_message: "ok".to_owned(),
            ..EmailSettings::default()
        };
        store.save(&settings).expect("save");
        settings.version = 1;
        assert_eq!(store.load().expect("load"), settings);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_mode() {
        assert_eq!(
            parse_live_email_mode("draft_only").expect("draft"),
            LiveEmailMode::DraftOnly
        );
        assert_eq!(
            parse_live_email_mode("send").expect("send"),
            LiveEmailMode::Send
        );
        assert!(parse_live_email_mode("gmail").is_err());
    }
}
