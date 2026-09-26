//! Non-secret Tools Settings on disk.
//!
//! `$XDG_CONFIG_HOME/softwake/tools.json` (else `~/.config/softwake/tools.json`).
//! Each registered tool is Always allow, Ask, or Deny. This file holds no secrets.

use std::collections::BTreeMap;
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

const DOCUMENT_VERSION: u32 = 2;

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

/// Operator choice for one registered tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolPermission {
    /// Run while awake with no pending card.
    AlwaysAllow,
    /// Stage one pending confirmation.
    Ask,
    /// Refuse the call. Nothing runs.
    Deny,
}

impl ToolPermission {
    /// Stable spelling for `tools.json` and the Tools page.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AlwaysAllow => "always_allow",
            Self::Ask => "ask",
            Self::Deny => "deny",
        }
    }
}

impl std::fmt::Display for ToolPermission {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Parse an operator permission spelling.
///
/// # Errors
///
/// Unknown spelling. `always`, `safe`, `confirm`, and `allow` are rejected.
pub fn parse_tool_permission(value: &str) -> Result<ToolPermission, ToolsSettingsError> {
    match value {
        "always_allow" => Ok(ToolPermission::AlwaysAllow),
        "ask" => Ok(ToolPermission::Ask),
        "deny" => Ok(ToolPermission::Deny),
        _ => Err(ToolsSettingsError::InvalidPermission {
            value: value.to_owned(),
        }),
    }
}

/// Default operator mode for a registered tool name.
///
/// `None` means `name` is not one of the phase-2 tools. Callers treat that as deny.
#[must_use]
pub fn default_permission(name: &str) -> Option<ToolPermission> {
    match name {
        crate::ECHO_TOOL => Some(ToolPermission::AlwaysAllow),
        crate::NOTIFY_TOOL | crate::EMAIL_SEND_TOOL | crate::SKILL_SAVE_TOOL => {
            Some(ToolPermission::Ask)
        }
        crate::SHELL_TOOL => Some(ToolPermission::Deny),
        _ => None,
    }
}

/// Non-secret Tools Settings.
///
/// Defaults: `echo` always allow, `notify` and `email_send` ask, `shell` deny.
/// [`ToolsSettings::shell_enabled`] mirrors shell: true when shell is ask or always allow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolsSettings {
    /// Document version. Loads of version 1 are normalized to 2 in memory.
    #[serde(default = "one")]
    pub version: u32,
    /// Mirror of shell permission. True when shell is not deny.
    #[serde(default)]
    pub shell_enabled: bool,
    /// Confirm gate for shell when the operator mode is Ask. Default [`ConfirmPolicy::Always`].
    #[serde(default)]
    pub confirm_policy: ConfirmPolicy,
    /// Operator mode for each registered tool. Unknown keys are dropped on normalize.
    #[serde(default)]
    pub permissions: BTreeMap<String, ToolPermission>,
}

fn one() -> u32 {
    DOCUMENT_VERSION
}

impl Default for ToolsSettings {
    fn default() -> Self {
        let mut settings = Self {
            version: DOCUMENT_VERSION,
            shell_enabled: false,
            confirm_policy: ConfirmPolicy::Always,
            permissions: BTreeMap::new(),
        };
        settings.normalize();
        settings
    }
}

impl ToolsSettings {
    /// Stored mode, or [`default_permission`] for a registered name with no key.
    ///
    /// A name that is not registered is deny.
    #[must_use]
    pub fn permission(&self, name: &str) -> ToolPermission {
        if crate::ToolRegistry::phase2().lookup(name).is_none() {
            return ToolPermission::Deny;
        }
        self.permissions
            .get(name)
            .copied()
            .unwrap_or_else(|| default_permission(name).unwrap_or(ToolPermission::Deny))
    }

    /// Fill defaults, drop unknown keys, and sync the shell mirror.
    ///
    /// A missing `permissions.shell` follows `shell_enabled` (ask when true, deny when false).
    /// When `permissions.shell` is present, that value wins and `shell_enabled` is rewritten.
    /// Load stamps version 2 in memory and does not write the file.
    pub fn normalize(&mut self) {
        if !self.permissions.contains_key(crate::SHELL_TOOL) {
            let shell = if self.shell_enabled {
                ToolPermission::Ask
            } else {
                ToolPermission::Deny
            };
            self.permissions.insert(crate::SHELL_TOOL.to_owned(), shell);
        }
        for name in [
            crate::ECHO_TOOL,
            crate::NOTIFY_TOOL,
            crate::EMAIL_SEND_TOOL,
            crate::SKILL_SAVE_TOOL,
        ] {
            if !self.permissions.contains_key(name) {
                if let Some(permission) = default_permission(name) {
                    self.permissions.insert(name.to_owned(), permission);
                }
            }
        }
        let registry = crate::ToolRegistry::phase2();
        self.permissions
            .retain(|name, _| registry.lookup(name).is_some());
        let shell = self
            .permissions
            .get(crate::SHELL_TOOL)
            .copied()
            .unwrap_or(ToolPermission::Deny);
        self.shell_enabled = shell != ToolPermission::Deny;
        self.version = DOCUMENT_VERSION;
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
    /// Permission string was not recognised.
    #[error("unknown tool permission: {value}")]
    InvalidPermission {
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
        document.normalize();
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
    settings.normalize();
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
        ConfirmPolicy, FileToolsSettings, ToolPermission, ToolsSettings, ToolsSettingsError,
        default_permission, parse_confirm_policy, parse_tool_permission, resolve_tools_file_from,
    };
    use crate::{ECHO_TOOL, EMAIL_SEND_TOOL, NOTIFY_TOOL, SHELL_TOOL, ToolRegistry};

    fn temp_store(label: &str) -> (std::path::PathBuf, FileToolsSettings) {
        let dir = std::env::temp_dir().join(format!(
            "softwake-tools-settings-{}-{label}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("tools.json");
        let store = FileToolsSettings::new(&path).expect("store");
        (dir, store)
    }

    #[test]
    fn default_is_version_2_with_documented_modes() {
        let settings = ToolsSettings::default();
        assert_eq!(settings.version, 2);
        assert!(!settings.shell_enabled);
        assert_eq!(settings.confirm_policy, ConfirmPolicy::Always);
        assert!(settings.confirm_policy.must_confirm(false));
        assert!(ConfirmPolicy::MutatingOnly.must_confirm(true));
        assert!(!ConfirmPolicy::MutatingOnly.must_confirm(false));
        assert!(ConfirmPolicy::AllowlistedQuiet.must_confirm(true));
        assert!(!ConfirmPolicy::AllowlistedQuiet.must_confirm(false));
        assert_eq!(
            ConfirmPolicy::AllowlistedQuiet.as_str(),
            "allowlisted_quiet"
        );
        assert_eq!(settings.permission(ECHO_TOOL), ToolPermission::AlwaysAllow);
        assert_eq!(settings.permission(NOTIFY_TOOL), ToolPermission::Ask);
        assert_eq!(settings.permission(EMAIL_SEND_TOOL), ToolPermission::Ask);
        assert_eq!(settings.permission(SHELL_TOOL), ToolPermission::Deny);
        assert_eq!(settings.permission("volume"), ToolPermission::Deny);
        assert!(!settings.permissions.contains_key("volume"));
    }

    #[test]
    fn every_registered_tool_has_an_explicit_default() {
        for tool in ToolRegistry::phase2().entries() {
            let permission = default_permission(tool.name).unwrap_or_else(|| {
                panic!("{} has no explicit default", tool.name);
            });
            assert_eq!(ToolsSettings::default().permission(tool.name), permission);
        }
        assert!(default_permission("volume").is_none());
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
    fn parse_permission_rejects_policy_spellings() {
        assert_eq!(
            parse_tool_permission("always_allow").expect("always allow"),
            ToolPermission::AlwaysAllow
        );
        assert_eq!(
            parse_tool_permission("ask").expect("ask"),
            ToolPermission::Ask
        );
        assert_eq!(
            parse_tool_permission("deny").expect("deny"),
            ToolPermission::Deny
        );
        for spelling in ["safe", "always", "confirm", "allow"] {
            match parse_tool_permission(spelling) {
                Err(ToolsSettingsError::InvalidPermission { value }) => {
                    assert_eq!(value, spelling);
                }
                other => panic!("expected InvalidPermission for {spelling}, got {other:?}"),
            }
        }
    }

    #[test]
    fn resolve_prefers_xdg_config() {
        let path = resolve_tools_file_from(Some("/cfg"), Some("/home")).expect("path");
        assert_eq!(path, std::path::PathBuf::from("/cfg/softwake/tools.json"));
    }

    #[test]
    fn version1_shell_enabled_loads_ask_and_keeps_policy() {
        let (dir, store) = temp_store("v1-on");
        std::fs::write(
            store.path(),
            r#"{"version":1,"shell_enabled":true,"confirm_policy":"mutating_only"}"#,
        )
        .expect("write");
        let settings = store.load().expect("load");
        assert_eq!(settings.version, 2);
        assert_eq!(settings.permission(SHELL_TOOL), ToolPermission::Ask);
        assert!(settings.shell_enabled);
        assert_eq!(settings.confirm_policy, ConfirmPolicy::MutatingOnly);
        assert_eq!(settings.permission(ECHO_TOOL), ToolPermission::AlwaysAllow);
        assert_eq!(settings.permission(NOTIFY_TOOL), ToolPermission::Ask);
        assert_eq!(settings.permission(EMAIL_SEND_TOOL), ToolPermission::Ask);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn version1_shell_disabled_loads_deny() {
        let (dir, store) = temp_store("v1-off");
        std::fs::write(
            store.path(),
            r#"{"version":1,"shell_enabled":false,"confirm_policy":"always"}"#,
        )
        .expect("write");
        let settings = store.load().expect("load");
        assert_eq!(settings.permission(SHELL_TOOL), ToolPermission::Deny);
        assert!(!settings.shell_enabled);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn present_shell_permission_wins_over_the_bool() {
        let (dir, store) = temp_store("v2-disagree");
        std::fs::write(
            store.path(),
            r#"{"version":2,"shell_enabled":false,"confirm_policy":"always","permissions":{"shell":"always_allow"}}"#,
        )
        .expect("write");
        let settings = store.load().expect("load");
        assert_eq!(settings.permission(SHELL_TOOL), ToolPermission::AlwaysAllow);
        assert!(settings.shell_enabled);
        assert_eq!(settings.version, 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn round_trip_drops_unknown_keys_and_rejects_bad_spellings() {
        let (dir, store) = temp_store("round");
        assert_eq!(
            store.load().expect("missing default"),
            ToolsSettings::default()
        );
        let mut settings = ToolsSettings {
            confirm_policy: ConfirmPolicy::MutatingOnly,
            shell_enabled: false,
            ..ToolsSettings::default()
        };
        settings
            .permissions
            .insert(ECHO_TOOL.to_owned(), ToolPermission::Ask);
        settings
            .permissions
            .insert(NOTIFY_TOOL.to_owned(), ToolPermission::AlwaysAllow);
        settings
            .permissions
            .insert(EMAIL_SEND_TOOL.to_owned(), ToolPermission::Deny);
        settings
            .permissions
            .insert(SHELL_TOOL.to_owned(), ToolPermission::AlwaysAllow);
        settings
            .permissions
            .insert("volume".to_owned(), ToolPermission::Ask);
        settings.normalize();
        assert!(settings.shell_enabled);
        assert!(!settings.permissions.contains_key("volume"));
        store.save(&settings).expect("save");
        let loaded = store.load().expect("load");
        assert_eq!(loaded, settings);
        let body = std::fs::read_to_string(store.path()).expect("body");
        assert!(!body.contains("volume"));
        assert!(body.contains("\"version\": 2"));
        std::fs::write(
            store.path(),
            r#"{"version":2,"permissions":{"echo":"safe"}}"#,
        )
        .expect("bad");
        assert!(store.load().is_err());
        std::fs::write(
            store.path(),
            r#"{"version":2,"permissions":{"echo":"always"}}"#,
        )
        .expect("bad always");
        assert!(store.load().is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
