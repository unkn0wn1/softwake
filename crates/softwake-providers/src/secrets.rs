//! Provider secret bag.
//!
//! The on-disk path is unchanged. Version 2 prefers the OS keyring: the file is a
//! pointer and the secret fields live in one keyring item. Plaintext remains an
//! opt-in fallback. Version 1 is still read. See [ADR 0012](../../docs/ADR-0012-model-providers.md).

use std::collections::BTreeMap;
use std::env;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::account_oauth::{AccountConnection, AccountProvider};
use crate::oauth::OAuthTokenSet;
use crate::secrets_file::{self, FileSecretStore};
use crate::secrets_keyring::{self, KeyringSecretStore};

/// File name under the Softwake state directory.
pub const SECRETS_FILE_NAME: &str = "secrets.json";

/// Largest secrets document or keyring payload this backend will accept, in bytes.
pub const MAX_SECRETS_BYTES: usize = 256 * 1024;

/// Keyring service name for the provider bag.
pub const KEYRING_SERVICE: &str = "softwake";

/// Keyring account name for the provider bag. One item holds the whole bag.
pub const KEYRING_USER: &str = "secret-bag";

/// Account used only to probe Secret Service. The value stored there is `ok`.
pub const KEYRING_PROBE_USER: &str = "softwake-probe";

/// Status line when the bag is in the OS keyring.
pub const KEYRING_STATUS: &str = "Provider secrets are stored in the OS keyring.";

/// Warning when the bag is an opt-in plaintext file.
pub const PLAINTEXT_WARNING: &str = "Softwake is storing provider secrets in a local plaintext file (mode 0600). The OS keyring is not in use.";

/// Warning when no file exists and the keyring probe failed.
pub const PLAINTEXT_OPT_IN_MESSAGE: &str = "The OS keyring is unavailable. Choose a local plaintext file to save keys on this machine, or leave keys in the environment.";

/// Fixed sentence for a backend label this build does not accept.
pub const UNSUPPORTED_BACKEND_MESSAGE: &str = "provider secret backend is not supported";

/// Fixed sentence when a pointer's service or user does not match this build.
pub const POINTER_IDENTITY_MESSAGE: &str = "pointer identity does not match this build";

const SECRET_BACKEND_ENV: &str = "SOFTWAKE_SECRET_BACKEND";

/// Version written by this crate. Version 1 is legacy plaintext and is only read.
pub(crate) const CURRENT_VERSION: u32 = 2;

/// Sentence when no usable Google or Microsoft Email account is stored.
pub const NO_CONNECTED_EMAIL_ACCOUNT: &str =
    "no Google or Microsoft account connected (Settings → Email → Connect)";

const LEGACY_VERSION: u32 = 1;

/// In-memory secret bag. [`Debug`] redacts every secret string.
///
/// Backend metadata stays in the on-disk document. This type is only the
/// credential fields plus the version flags callers already match on.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SecretBag {
    /// Document version. `1` is legacy plaintext. `2` is the current bag.
    #[serde(default = "legacy_version")]
    pub version: u32,
    /// `true` when the secret fields were loaded from a plaintext file.
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
    /// Saved `OpenRouter` API key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openrouter_api_key: Option<String>,
    /// Saved OpenAI-compatible API key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_compatible_api_key: Option<String>,
    /// Saved SMTP password for opt-in live email.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email_smtp_password: Option<String>,
    /// Telegram Bot API token (Messengers). Never logged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telegram_bot_token: Option<String>,
    /// Shared secret for authenticated webhook wake (ADR-0038). Never logged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_secret: Option<String>,
    /// Google account connections (Email OAuth). Zero or more.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub google_connections: Vec<AccountConnection>,
    /// Microsoft account connections (Email OAuth). Zero or more.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub microsoft_connections: Vec<AccountConnection>,
    /// Provider user id of the active Google account. Missing means the first row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_google_connection_id: Option<String>,
    /// Provider user id of the active Microsoft account. Missing means the first row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_microsoft_connection_id: Option<String>,
    /// MCP server auth secrets keyed by server id (ADR-0031). Never logged.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub mcp_secrets: BTreeMap<String, String>,
}

fn legacy_version() -> u32 {
    LEGACY_VERSION
}

fn always_true() -> bool {
    true
}

fn redact_secret(value: Option<&String>) -> Option<&'static str> {
    value.map(|_| "<redacted>")
}

impl std::fmt::Debug for SecretBag {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SecretBag")
            .field("version", &self.version)
            .field("plaintext", &self.plaintext)
            .field("xai_api_key", &redact_secret(self.xai_api_key.as_ref()))
            .field(
                "openai_api_key",
                &redact_secret(self.openai_api_key.as_ref()),
            )
            .field("xai_oauth", &self.xai_oauth)
            .field(
                "openrouter_api_key",
                &redact_secret(self.openrouter_api_key.as_ref()),
            )
            .field(
                "openai_compatible_api_key",
                &redact_secret(self.openai_compatible_api_key.as_ref()),
            )
            .field(
                "email_smtp_password",
                &redact_secret(self.email_smtp_password.as_ref()),
            )
            .field(
                "telegram_bot_token",
                &redact_secret(self.telegram_bot_token.as_ref()),
            )
            .field(
                "webhook_secret",
                &redact_secret(self.webhook_secret.as_ref()),
            )
            .field("google_connections", &self.google_connections)
            .field("microsoft_connections", &self.microsoft_connections)
            .field(
                "active_google_connection_id",
                &self.active_google_connection_id,
            )
            .field(
                "active_microsoft_connection_id",
                &self.active_microsoft_connection_id,
            )
            .field(
                "mcp_secrets",
                &self
                    .mcp_secrets
                    .keys()
                    .map(|k| (k.as_str(), "<redacted>"))
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl SecretBag {
    /// Empty legacy bag. A missing file loads as this value.
    ///
    /// A keyring load uses [`Self::keyring_empty`] instead.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            version: LEGACY_VERSION,
            plaintext: true,
            xai_api_key: None,
            openai_api_key: None,
            xai_oauth: None,
            openrouter_api_key: None,
            openai_compatible_api_key: None,
            email_smtp_password: None,
            telegram_bot_token: None,
            webhook_secret: None,
            google_connections: Vec::new(),
            microsoft_connections: Vec::new(),
            active_google_connection_id: None,
            active_microsoft_connection_id: None,
            mcp_secrets: BTreeMap::new(),
        }
    }

    /// Empty bag after the keyring answered and had no item.
    #[must_use]
    pub fn keyring_empty() -> Self {
        Self {
            version: CURRENT_VERSION,
            plaintext: false,
            ..Self::empty()
        }
    }

    fn provider_mut(
        &mut self,
        provider: AccountProvider,
    ) -> (&mut Vec<AccountConnection>, &mut Option<String>) {
        match provider {
            AccountProvider::Google => (
                &mut self.google_connections,
                &mut self.active_google_connection_id,
            ),
            AccountProvider::Microsoft => (
                &mut self.microsoft_connections,
                &mut self.active_microsoft_connection_id,
            ),
        }
    }

    fn provider_ref(&self, provider: AccountProvider) -> (&[AccountConnection], Option<&str>) {
        match provider {
            AccountProvider::Google => (
                &self.google_connections,
                self.active_google_connection_id.as_deref(),
            ),
            AccountProvider::Microsoft => (
                &self.microsoft_connections,
                self.active_microsoft_connection_id.as_deref(),
            ),
        }
    }

    /// Insert or replace one Email OAuth account. Identity is [`AccountConnection::id`].
    ///
    /// An email match replaces a row only when at least one of the two ids is empty.
    /// The first account in an empty list becomes active. A later account does not.
    pub fn upsert_account(&mut self, provider: AccountProvider, next: AccountConnection) {
        let (list, active) = self.provider_mut(provider);
        let next_id = next.id.clone();
        if let Some(index) = upsert_index(list, &next) {
            list[index] = next;
        } else {
            list.push(next);
        }
        assign_active_after_upsert(list, active, &next_id);
    }

    /// Remove one account by connection id or full email.
    ///
    /// # Errors
    ///
    /// Empty identity, no match, or two rows with the same email.
    pub fn remove_account(
        &mut self,
        provider: AccountProvider,
        identity: &str,
    ) -> Result<AccountConnection, String> {
        let identity = identity.trim();
        if identity.is_empty() {
            return Err("no account matches that id or email".to_owned());
        }
        let (list, active) = self.provider_mut(provider);
        let index = find_identity(list, identity)?;
        let removed = list.remove(index);
        let marker_missing = match active.as_deref() {
            None => true,
            Some(id) => !list.iter().any(|row| row.id == id),
        };
        if marker_missing {
            *active = list.first().and_then(|row| {
                if row.id.is_empty() {
                    None
                } else {
                    Some(row.id.clone())
                }
            });
        }
        Ok(removed)
    }

    /// Mark one stored account active for its provider.
    ///
    /// # Errors
    ///
    /// Unknown identity, or a row whose id is empty.
    pub fn set_active_account(
        &mut self,
        provider: AccountProvider,
        identity: &str,
    ) -> Result<(), String> {
        let identity = identity.trim();
        if identity.is_empty() {
            return Err("no account matches that id or email".to_owned());
        }
        let (list, active) = self.provider_mut(provider);
        let index = find_identity(list, identity)?;
        let id = list[index].id.clone();
        if id.is_empty() {
            return Err("reconnect this account before setting it active".to_owned());
        }
        *active = Some(id);
        Ok(())
    }

    /// Active row, or the first row when the marker is missing or dangling.
    ///
    /// Includes a row that has no tokens so Settings can still list it.
    #[must_use]
    pub fn effective_account(&self, provider: AccountProvider) -> Option<&AccountConnection> {
        let (list, active_id) = self.provider_ref(provider);
        if list.is_empty() {
            return None;
        }
        if let Some(id) = active_id.filter(|value| !value.is_empty()) {
            if let Some(found) = list.iter().find(|row| row.id == id) {
                return Some(found);
            }
        }
        list.first()
    }

    /// Choose the Email OAuth account for a tool call.
    ///
    /// `hint` is a connection id or an email substring. An empty hint uses the
    /// single usable account, otherwise the active Google account when any
    /// Google account is usable, otherwise the active Microsoft account.
    ///
    /// # Errors
    ///
    /// No usable account, no match, or more than one match.
    pub fn resolve_account(
        &self,
        hint: Option<&str>,
    ) -> Result<(AccountProvider, AccountConnection), String> {
        let hint = hint.map(str::trim).filter(|value| !value.is_empty());
        if let Some(hint) = hint {
            self.resolve_hint(hint)
        } else {
            self.resolve_default()
        }
    }

    fn usable_rows(&self) -> Vec<(AccountProvider, &AccountConnection)> {
        let mut rows = Vec::new();
        for connection in &self.google_connections {
            if connection_has_secret(connection) {
                rows.push((AccountProvider::Google, connection));
            }
        }
        for connection in &self.microsoft_connections {
            if connection_has_secret(connection) {
                rows.push((AccountProvider::Microsoft, connection));
            }
        }
        rows
    }

    fn resolve_default(&self) -> Result<(AccountProvider, AccountConnection), String> {
        let rows = self.usable_rows();
        match rows.len() {
            0 => Err(NO_CONNECTED_EMAIL_ACCOUNT.to_owned()),
            1 => Ok((rows[0].0, rows[0].1.clone())),
            _ => {
                if rows
                    .iter()
                    .any(|(provider, _)| *provider == AccountProvider::Google)
                {
                    let picked = pick_usable(
                        &self.google_connections,
                        self.active_google_connection_id.as_deref(),
                    )
                    .ok_or_else(|| NO_CONNECTED_EMAIL_ACCOUNT.to_owned())?;
                    Ok((AccountProvider::Google, picked.clone()))
                } else {
                    let picked = pick_usable(
                        &self.microsoft_connections,
                        self.active_microsoft_connection_id.as_deref(),
                    )
                    .ok_or_else(|| NO_CONNECTED_EMAIL_ACCOUNT.to_owned())?;
                    Ok((AccountProvider::Microsoft, picked.clone()))
                }
            }
        }
    }

    fn resolve_hint(&self, hint: &str) -> Result<(AccountProvider, AccountConnection), String> {
        let rows = self.usable_rows();
        let id_hits: Vec<_> = rows
            .iter()
            .copied()
            .filter(|(_, row)| row.id == hint)
            .collect();
        if let Some((provider, row)) = only_hit(&id_hits, hint)? {
            return Ok((provider, row.clone()));
        }
        let email_hits: Vec<_> = rows
            .iter()
            .copied()
            .filter(|(_, row)| email_matches(row, hint, true))
            .collect();
        if let Some((provider, row)) = only_hit(&email_hits, hint)? {
            return Ok((provider, row.clone()));
        }
        let substring_hits: Vec<_> = rows
            .iter()
            .copied()
            .filter(|(_, row)| email_matches(row, hint, false))
            .collect();
        if let Some((provider, row)) = only_hit(&substring_hits, hint)? {
            return Ok((provider, row.clone()));
        }
        Err(format!("no connected account matches '{hint}'"))
    }
}

/// Where the secret fields are kept for this process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretBackend {
    /// OS keyring item, with a pointer file on disk after the first save.
    Keyring,
    /// Opt-in or legacy plaintext file.
    Plaintext,
    /// No file yet and the keyring probe failed. Load is empty. Save needs opt-in.
    Unavailable,
}

impl SecretBackend {
    /// Wire value for Settings: `keyring`, `plaintext`, or `unavailable`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Keyring => "keyring",
            Self::Plaintext => "plaintext",
            Self::Unavailable => "unavailable",
        }
    }
}

/// Status returned to Settings. The message is never a secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageReport {
    /// Active backend.
    pub backend: SecretBackend,
    /// Status or warning. Never a key or token.
    pub message: String,
}

impl StorageReport {
    pub(crate) fn keyring() -> Self {
        Self {
            backend: SecretBackend::Keyring,
            message: KEYRING_STATUS.to_owned(),
        }
    }

    pub(crate) fn plaintext() -> Self {
        Self {
            backend: SecretBackend::Plaintext,
            message: PLAINTEXT_WARNING.to_owned(),
        }
    }

    pub(crate) fn unavailable() -> Self {
        Self {
            backend: SecretBackend::Unavailable,
            message: PLAINTEXT_OPT_IN_MESSAGE.to_owned(),
        }
    }
}

/// Failure from a [`SecretStore`]. Display text never includes a secret or a raw backend label.
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
        source: Box<std::io::Error>,
    },

    /// JSON that this crate will not treat as a bag.
    ///
    /// The serde error is dropped. Its [`Display`](std::fmt::Display) can quote the file body.
    #[error("provider secrets file {} is not valid json", path.display())]
    Invalid {
        /// Path read.
        path: PathBuf,
    },

    /// Unsupported document version.
    #[error(
        "provider secrets file {} has unsupported version {version}",
        path.display()
    )]
    UnsupportedVersion {
        /// Path read.
        path: PathBuf,
        /// Version field.
        version: u32,
    },

    /// File or payload too large.
    #[error("provider secrets file {} is {len} bytes; max is {max}", path.display())]
    TooLarge {
        /// Path read.
        path: PathBuf,
        /// Byte length.
        len: usize,
        /// Cap.
        max: usize,
    },

    /// Env value or on-disk label this build does not accept. `reason` is a fixed sentence.
    #[error("{reason}")]
    UnsupportedBackend {
        /// Fixed sentence. Never a caller-supplied label.
        reason: &'static str,
    },

    /// The keyring was required, or a pointer could not be read, and the service did not answer.
    #[error("the OS keyring is unavailable")]
    KeyringUnavailable,

    /// The keyring answered with a failure that is not "no entry".
    ///
    /// The platform error is not attached. Its text can echo a password.
    #[error("the OS keyring rejected the provider secret bag")]
    Keyring,

    /// Save was attempted before the operator opted in to a plaintext file.
    #[error(
        "The OS keyring is unavailable. Choose a local plaintext file to save keys on this machine, or leave keys in the environment."
    )]
    PlaintextOptInRequired,

    /// A keyring pointer contained a secret field.
    #[error(
        "provider secrets file {} is a keyring pointer and must not contain secret fields",
        path.display()
    )]
    PointerHasSecrets {
        /// Path read.
        path: PathBuf,
    },

    /// A plaintext save would replace a keyring pointer.
    #[error(
        "provider secrets file {} is a keyring pointer and cannot be written as plaintext",
        path.display()
    )]
    WrongBackend {
        /// Path that was left unchanged.
        path: PathBuf,
    },

    /// Opt-in was requested while the keyring probe succeeded or a pointer is already on disk.
    #[error("the OS keyring is available")]
    KeyringAvailable,

    /// Opt-in was requested while plaintext is already the resolved store.
    #[error("provider secrets already use a local plaintext file")]
    PlaintextAlreadySelected,
}

/// What is already at the secrets path. The caller supplies the probe result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnDiskKind {
    /// No file.
    Missing,
    /// Version 1, always treated as plaintext.
    LegacyV1,
    /// Version 2 opt-in plaintext.
    PlaintextV2,
    /// Version 2 pointer. The secret fields are not in the file.
    KeyringPointer,
}

/// `SOFTWAKE_SECRET_BACKEND`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendPref {
    /// Unset or blank. Probe decides, except a pointer is always the keyring.
    Auto,
    /// Force the plaintext file. Does not copy a pointer's payload out of the keyring.
    Plaintext,
    /// Force the keyring. Fails closed when the probe fails and the file is not a pointer.
    Keyring,
}

/// Result of [`resolve_backend`], aside from a hard error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendChoice {
    /// Use the keyring. `migrate` writes the pointer after copying a plaintext bag.
    Keyring {
        /// When true, copy the plaintext bag into the keyring before returning the store.
        migrate: bool,
    },
    /// Use the plaintext file. The next plaintext save rewrites version 2.
    Plaintext,
    /// Load an empty bag. Save returns [`SecretStoreError::PlaintextOptInRequired`].
    Unavailable,
}

/// Durable bag. File, keyring, unavailable, and the in-memory fake share this.
pub trait SecretStore: Send {
    /// Load the bag.
    ///
    /// # Errors
    ///
    /// Store-specific read failures. A missing keyring item is an empty bag, not an error.
    fn load(&self) -> Result<SecretBag, SecretStoreError>;

    /// Replace the bag.
    ///
    /// # Errors
    ///
    /// Store-specific write failures.
    fn save(&self, bag: &SecretBag) -> Result<(), SecretStoreError>;

    /// Backend status for Settings. Never includes a secret.
    #[must_use]
    fn report(&self) -> StorageReport;
}

/// Keyring I/O behind an owned client so tests never touch the process-global builder.
pub trait KeyringClient: Send {
    /// Payload JSON, or [`None`] when the item is absent.
    ///
    /// # Errors
    ///
    /// Service failures. Absence is [`None`], not an error.
    fn get_payload(&self) -> Result<Option<String>, SecretStoreError>;

    /// Replace the payload. `json` is the secret bag and must not be logged.
    ///
    /// # Errors
    ///
    /// Service failures.
    fn set_payload(&self, json: &str) -> Result<(), SecretStoreError>;

    /// Remove the item. A missing item is success.
    ///
    /// # Errors
    ///
    /// Service failures other than a missing item.
    fn delete_payload(&self) -> Result<(), SecretStoreError>;
}

/// Load, mutate, save.
///
/// # Errors
///
/// Propagates load or save errors.
pub fn update_bag(
    store: &dyn SecretStore,
    mutator: impl FnOnce(&mut SecretBag),
) -> Result<SecretBag, SecretStoreError> {
    let mut bag = store.load()?;
    mutator(&mut bag);
    store.save(&bag)?;
    Ok(bag)
}

/// Choose a backend. This function does not read the disk and does not talk to dbus.
///
/// # Errors
///
/// [`SecretStoreError::KeyringUnavailable`] when the preference forces the keyring,
/// the probe failed, and the file is not already a pointer.
pub fn resolve_backend(
    on_disk: OnDiskKind,
    pref: BackendPref,
    probe_ok: bool,
) -> Result<BackendChoice, SecretStoreError> {
    if on_disk == OnDiskKind::KeyringPointer {
        return Ok(BackendChoice::Keyring { migrate: false });
    }
    match (on_disk, pref, probe_ok) {
        (OnDiskKind::Missing, BackendPref::Auto | BackendPref::Keyring, true) => {
            Ok(BackendChoice::Keyring { migrate: false })
        }
        (OnDiskKind::Missing, BackendPref::Auto, false) => Ok(BackendChoice::Unavailable),
        (
            OnDiskKind::Missing | OnDiskKind::LegacyV1 | OnDiskKind::PlaintextV2,
            BackendPref::Plaintext,
            _,
        )
        | (OnDiskKind::LegacyV1 | OnDiskKind::PlaintextV2, BackendPref::Auto, false) => {
            Ok(BackendChoice::Plaintext)
        }
        (
            OnDiskKind::Missing | OnDiskKind::LegacyV1 | OnDiskKind::PlaintextV2,
            BackendPref::Keyring,
            false,
        ) => Err(SecretStoreError::KeyringUnavailable),
        (
            OnDiskKind::LegacyV1 | OnDiskKind::PlaintextV2,
            BackendPref::Auto | BackendPref::Keyring,
            true,
        ) => Ok(BackendChoice::Keyring { migrate: true }),
        (OnDiskKind::KeyringPointer, _, _) => Ok(BackendChoice::Keyring { migrate: false }),
    }
}

/// Open the store at `path`, reading `SOFTWAKE_SECRET_BACKEND` and probing once per process.
///
/// # Errors
///
/// Path, parse, preference, probe-forced keyring, or migration failures.
pub fn open_store(path: &Path) -> Result<Box<dyn SecretStore + Send>, SecretStoreError> {
    let pref = pref_from_env()?;
    let probe_ok = secrets_keyring::cached_probe();
    open_store_with(
        path,
        pref,
        probe_ok,
        secrets_keyring::LiveKeyringClient::new(),
    )
}

/// Open a store without reading the environment and without probing.
///
/// Tests pass `probe_ok` and a fake [`KeyringClient`]. This does not call `keyring::Entry`.
///
/// # Errors
///
/// Path, parse, preference, or migration failures.
pub fn open_store_with(
    path: &Path,
    pref: BackendPref,
    probe_ok: bool,
    client: impl KeyringClient + 'static,
) -> Result<Box<dyn SecretStore + Send>, SecretStoreError> {
    if path.as_os_str().is_empty() {
        return Err(SecretStoreError::EmptyPath);
    }
    let kind = secrets_file::classify(path)?;
    let choice = resolve_backend(kind, pref, probe_ok)?;
    match choice {
        BackendChoice::Unavailable => Ok(Box::new(UnavailableSecretStore)),
        BackendChoice::Plaintext => Ok(Box::new(FileSecretStore::new(path)?)),
        BackendChoice::Keyring { migrate } => {
            if migrate {
                secrets_keyring::migrate_to_keyring(path, &client)?;
            }
            Ok(Box::new(KeyringSecretStore::new(path, client)?))
        }
    }
}

/// Write the empty version-2 plaintext opt-in file when resolution is [`BackendChoice::Unavailable`].
///
/// Does not talk to dbus. A keyring choice, including an existing pointer, is left untouched.
///
/// # Errors
///
/// [`SecretStoreError::KeyringAvailable`], [`SecretStoreError::PlaintextAlreadySelected`],
/// or a write error.
pub fn opt_in_plaintext(
    path: &Path,
    pref: BackendPref,
    probe_ok: bool,
) -> Result<StorageReport, SecretStoreError> {
    if path.as_os_str().is_empty() {
        return Err(SecretStoreError::EmptyPath);
    }
    let kind = secrets_file::classify(path)?;
    match resolve_backend(kind, pref, probe_ok)? {
        BackendChoice::Unavailable => {
            FileSecretStore::new(path)?.save(&SecretBag::empty())?;
            Ok(StorageReport::plaintext())
        }
        BackendChoice::Keyring { .. } => Err(SecretStoreError::KeyringAvailable),
        BackendChoice::Plaintext => Err(SecretStoreError::PlaintextAlreadySelected),
    }
}

/// [`opt_in_plaintext`] using the process environment and the cached probe.
///
/// # Errors
///
/// Same as [`opt_in_plaintext`], plus an unsupported env value.
pub fn opt_in_plaintext_resolved(path: &Path) -> Result<StorageReport, SecretStoreError> {
    opt_in_plaintext(path, pref_from_env()?, secrets_keyring::cached_probe())
}

/// Load a bag without probing and without migrating.
///
/// A missing file is empty and is not created. A pointer is read from the keyring.
pub(crate) fn load_unmigrated(path: &Path) -> Result<SecretBag, SecretStoreError> {
    if path.as_os_str().is_empty() {
        return Err(SecretStoreError::EmptyPath);
    }
    match secrets_file::read_on_disk(path)? {
        secrets_file::OnDisk::Missing => Ok(SecretBag::empty()),
        secrets_file::OnDisk::Legacy(bag) | secrets_file::OnDisk::Plaintext(bag) => Ok(bag),
        secrets_file::OnDisk::Pointer => {
            KeyringSecretStore::new(path, secrets_keyring::LiveKeyringClient::new())?.load()
        }
    }
}

/// No file yet, and the operator has not opted in.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnavailableSecretStore;

impl SecretStore for UnavailableSecretStore {
    fn load(&self) -> Result<SecretBag, SecretStoreError> {
        Ok(SecretBag::empty())
    }

    fn save(&self, _bag: &SecretBag) -> Result<(), SecretStoreError> {
        Err(SecretStoreError::PlaintextOptInRequired)
    }

    fn report(&self) -> StorageReport {
        StorageReport::unavailable()
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
    xdg_state_home: Option<impl AsRef<OsStr>>,
    home: Option<impl AsRef<OsStr>>,
) -> Result<PathBuf, SecretStoreError> {
    Ok(resolve_state_dir_from(xdg_state_home, home)?.join(SECRETS_FILE_NAME))
}

fn resolve_state_dir_from(
    xdg_state_home: Option<impl AsRef<OsStr>>,
    home: Option<impl AsRef<OsStr>>,
) -> Result<PathBuf, SecretStoreError> {
    if let Some(xdg) = trimmed_os(xdg_state_home) {
        return Ok(PathBuf::from(xdg).join("softwake"));
    }
    if let Some(home) = trimmed_os(home) {
        return Ok(PathBuf::from(home).join(".local/state/softwake"));
    }
    Err(SecretStoreError::NoStateDir)
}

fn trimmed_os(value: Option<impl AsRef<OsStr>>) -> Option<OsString> {
    let value = value?;
    let text = value.as_ref().to_string_lossy();
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(OsString::from(trimmed))
    }
}

pub(crate) fn pref_from_env() -> Result<BackendPref, SecretStoreError> {
    match env::var(SECRET_BACKEND_ENV) {
        Err(env::VarError::NotPresent) => Ok(BackendPref::Auto),
        Err(env::VarError::NotUnicode(_)) => Err(SecretStoreError::UnsupportedBackend {
            reason: UNSUPPORTED_BACKEND_MESSAGE,
        }),
        Ok(value) => pref_from_str(&value),
    }
}

pub(crate) fn pref_from_str(value: &str) -> Result<BackendPref, SecretStoreError> {
    match value.trim() {
        "" => Ok(BackendPref::Auto),
        "plaintext" => Ok(BackendPref::Plaintext),
        "keyring" => Ok(BackendPref::Keyring),
        _ => Err(SecretStoreError::UnsupportedBackend {
            reason: UNSUPPORTED_BACKEND_MESSAGE,
        }),
    }
}

pub(crate) fn bag_has_secret(bag: &SecretBag) -> bool {
    fn filled(value: Option<&String>) -> bool {
        value.is_some_and(|text| !text.is_empty())
    }
    filled(bag.xai_api_key.as_ref())
        || filled(bag.openai_api_key.as_ref())
        || filled(bag.openrouter_api_key.as_ref())
        || filled(bag.openai_compatible_api_key.as_ref())
        || filled(bag.email_smtp_password.as_ref())
        || filled(bag.telegram_bot_token.as_ref())
        || filled(bag.webhook_secret.as_ref())
        || bag.xai_oauth.as_ref().is_some_and(|tokens| {
            !tokens.access_token.is_empty() || !tokens.refresh_token.is_empty()
        })
        || bag.google_connections.iter().any(connection_has_secret)
        || bag.microsoft_connections.iter().any(connection_has_secret)
        || bag
            .mcp_secrets
            .values()
            .any(|value| !value.trim().is_empty())
}

fn connection_has_secret(connection: &AccountConnection) -> bool {
    !connection.access_token.is_empty() || !connection.refresh_token.is_empty()
}

fn upsert_index(list: &[AccountConnection], next: &AccountConnection) -> Option<usize> {
    if !next.id.is_empty() {
        if let Some(index) = list.iter().position(|row| row.id == next.id) {
            return Some(index);
        }
    }
    let email = next
        .account_email
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    list.iter().position(|row| {
        let same_email = row
            .account_email
            .as_deref()
            .map(str::trim)
            .is_some_and(|existing| existing.eq_ignore_ascii_case(email));
        let id_gap = row.id.is_empty() || next.id.is_empty();
        same_email && id_gap
    })
}

fn assign_active_after_upsert(
    list: &[AccountConnection],
    active: &mut Option<String>,
    next_id: &str,
) {
    if next_id.is_empty() {
        return;
    }
    let only_row = list.len() == 1;
    let already = active.as_deref() == Some(next_id);
    let dangling = active
        .as_deref()
        .is_some_and(|id| !list.iter().any(|row| row.id == id));
    if only_row || already || dangling {
        *active = Some(next_id.to_owned());
    }
}

fn find_identity(list: &[AccountConnection], identity: &str) -> Result<usize, String> {
    if let Some(index) = list.iter().position(|row| row.id == identity) {
        return Ok(index);
    }
    let mut email_hit = None;
    for (index, row) in list.iter().enumerate() {
        let same = row
            .account_email
            .as_deref()
            .map(str::trim)
            .is_some_and(|email| email.eq_ignore_ascii_case(identity));
        if !same {
            continue;
        }
        if email_hit.is_some() {
            return Err("use the connection id".to_owned());
        }
        email_hit = Some(index);
    }
    email_hit.ok_or_else(|| "no account matches that id or email".to_owned())
}

fn pick_usable<'a>(
    list: &'a [AccountConnection],
    active_id: Option<&str>,
) -> Option<&'a AccountConnection> {
    if let Some(id) = active_id.filter(|value| !value.is_empty()) {
        if let Some(found) = list
            .iter()
            .find(|row| row.id == id && connection_has_secret(row))
        {
            return Some(found);
        }
    }
    list.iter().find(|row| connection_has_secret(row))
}

fn email_matches(connection: &AccountConnection, hint: &str, exact: bool) -> bool {
    let Some(email) = connection
        .account_email
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    if exact {
        email.eq_ignore_ascii_case(hint)
    } else {
        email
            .to_ascii_lowercase()
            .contains(&hint.to_ascii_lowercase())
    }
}

fn only_hit<'a>(
    hits: &[(AccountProvider, &'a AccountConnection)],
    hint: &str,
) -> Result<Option<(AccountProvider, &'a AccountConnection)>, String> {
    match hits {
        [] => Ok(None),
        [one] => Ok(Some(*one)),
        many => Err(ambiguous_account(hint, many)),
    }
}

fn ambiguous_account(hint: &str, hits: &[(AccountProvider, &AccountConnection)]) -> String {
    let emails: Vec<&str> = hits
        .iter()
        .filter_map(|(_, row)| {
            row.account_email
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .collect();
    if emails.is_empty() {
        format!("account '{hint}' matches more than one connection; pass the connection id")
    } else {
        format!(
            "account '{hint}' matches more than one connection; pass the connection id ({})",
            emails.join(", ")
        )
    }
}

/// Keyring item body. No version and no backend fields.
#[derive(Serialize, Deserialize)]
pub(crate) struct SecretPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) xai_api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) openai_api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) openrouter_api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) openai_compatible_api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) email_smtp_password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) telegram_bot_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) webhook_secret: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) xai_oauth: Option<OAuthTokenSet>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) google_connections: Vec<AccountConnection>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) microsoft_connections: Vec<AccountConnection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) active_google_connection_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) active_microsoft_connection_id: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) mcp_secrets: BTreeMap<String, String>,
}

pub(crate) fn encode_payload(bag: &SecretBag) -> Result<String, SecretStoreError> {
    let payload = SecretPayload {
        xai_api_key: bag.xai_api_key.clone(),
        openai_api_key: bag.openai_api_key.clone(),
        openrouter_api_key: bag.openrouter_api_key.clone(),
        openai_compatible_api_key: bag.openai_compatible_api_key.clone(),
        email_smtp_password: bag.email_smtp_password.clone(),
        telegram_bot_token: bag.telegram_bot_token.clone(),
        webhook_secret: bag.webhook_secret.clone(),
        xai_oauth: bag.xai_oauth.clone(),
        google_connections: bag.google_connections.clone(),
        microsoft_connections: bag.microsoft_connections.clone(),
        active_google_connection_id: bag.active_google_connection_id.clone(),
        active_microsoft_connection_id: bag.active_microsoft_connection_id.clone(),
        mcp_secrets: bag.mcp_secrets.clone(),
    };
    serde_json::to_string(&payload).map_err(|_| SecretStoreError::Keyring)
}

pub(crate) fn decode_payload(path: &Path, json: &str) -> Result<SecretBag, SecretStoreError> {
    if json.len() > MAX_SECRETS_BYTES {
        return Err(SecretStoreError::TooLarge {
            path: path.to_owned(),
            len: json.len(),
            max: MAX_SECRETS_BYTES,
        });
    }
    if json.trim().is_empty() {
        return Ok(SecretBag::keyring_empty());
    }
    let payload: SecretPayload =
        serde_json::from_str(json).map_err(|_| SecretStoreError::Keyring)?;
    Ok(SecretBag {
        version: CURRENT_VERSION,
        plaintext: false,
        xai_api_key: payload.xai_api_key,
        openai_api_key: payload.openai_api_key,
        xai_oauth: payload.xai_oauth,
        openrouter_api_key: payload.openrouter_api_key,
        openai_compatible_api_key: payload.openai_compatible_api_key,
        email_smtp_password: payload.email_smtp_password,
        telegram_bot_token: payload.telegram_bot_token,
        webhook_secret: payload.webhook_secret,
        google_connections: payload.google_connections,
        microsoft_connections: payload.microsoft_connections,
        active_google_connection_id: payload.active_google_connection_id,
        active_microsoft_connection_id: payload.active_microsoft_connection_id,
        mcp_secrets: payload.mcp_secrets,
    })
}

#[cfg(test)]
#[path = "secrets_tests.rs"]
mod tests;
