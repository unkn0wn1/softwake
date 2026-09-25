//! Settings commands for model providers.
//!
//! Secrets stay in the providers crate's XDG bag. The window script receives
//! status only: never keys or tokens.

use std::str::FromStr;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use softwake_providers::{
    CredentialKind, DevicePoll, FileProviderSettings, FileSecretStore, OAuthTokenSet,
    PLAINTEXT_WARNING, PROVIDER_REGISTRY, ProviderId, ProviderSettings, apply_test_outcome,
    poll_device_code, resolve_bearer, resolve_providers_file, resolve_secrets_file, run_test,
    start_device_code,
};

#[cfg(not(feature = "live-http"))]
use softwake_providers::MockTransport;
#[cfg(feature = "live-http")]
use softwake_providers::live::LiveTransport;

/// Public device-code fields while sign-in is in progress.
#[derive(Debug, Clone, Serialize)]
pub struct OAuthPendingView {
    /// User code shown in Settings.
    pub user_code: String,
    /// URL the operator opens.
    pub verification_url: String,
    /// Poll interval seconds.
    pub interval_sec: u64,
    /// Expiry as unix milliseconds.
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone)]
struct PendingDevice {
    device_code: String,
    view: OAuthPendingView,
}

static PENDING_OAUTH: Mutex<Option<PendingDevice>> = Mutex::new(None);

/// One provider row for the picker.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderRow {
    /// Wire id.
    pub id: String,
    /// Label.
    pub label: String,
    /// Credential form: `xai-oauth`, `xai-key`, or `openai-key`.
    pub credential: String,
}

/// Snapshot returned to the window. No secrets.
#[derive(Debug, Clone, Serialize)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "one has_* flag per credential kind; matches the existing Settings snapshot shape"
)]
pub struct ProviderSnapshot {
    /// Selected provider id.
    pub selected_provider: String,
    /// Selected model id (may be empty).
    pub selected_model: String,
    /// Registry rows.
    pub providers: Vec<ProviderRow>,
    /// Cached models for the selected provider.
    pub models: Vec<String>,
    /// Last Test ok flag for the selected provider.
    pub last_test_ok: Option<bool>,
    /// Last Test message for the selected provider.
    pub last_test_message: String,
    /// Whether an xAI API key is saved.
    pub has_xai_key: bool,
    /// Whether an `OpenAI` API key is saved.
    pub has_openai_key: bool,
    /// Whether an `OpenRouter` API key is saved.
    pub has_openrouter_key: bool,
    /// Whether an OpenAI-compatible API key is saved.
    pub has_openai_compatible_key: bool,
    /// Configured OpenAI-compatible base URL (may be empty).
    pub openai_compatible_base_url: String,
    /// Whether xAI OAuth tokens are saved.
    pub has_xai_oauth: bool,
    /// In-progress device-code sign-in, if any.
    pub oauth_pending: Option<OAuthPendingView>,
    /// Plaintext secret-bag warning.
    pub plaintext_warning: String,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn open_settings() -> Result<FileProviderSettings, String> {
    FileProviderSettings::new(resolve_providers_file().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

fn open_secrets() -> Result<FileSecretStore, String> {
    FileSecretStore::new(resolve_secrets_file().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

fn credential_name(kind: CredentialKind) -> &'static str {
    match kind {
        CredentialKind::XaiOauth => "xai-oauth",
        CredentialKind::XaiKey => "xai-key",
        CredentialKind::OpenaiKey => "openai-key",
        CredentialKind::OpenrouterKey => "openrouter-key",
        CredentialKind::OpenaiCompatibleKey => "openai-compatible-key",
    }
}

#[allow(
    clippy::fn_params_excessive_bools,
    reason = "mirrors ProviderSnapshot has_* fields for each credential kind"
)]
fn snapshot_from(
    settings: &ProviderSettings,
    has_xai_key: bool,
    has_openai_key: bool,
    has_openrouter_key: bool,
    has_openai_compatible_key: bool,
    has_xai_oauth: bool,
) -> Result<ProviderSnapshot, String> {
    let pending = PENDING_OAUTH
        .lock()
        .map_err(|_| "provider oauth lock is poisoned".to_owned())?
        .as_ref()
        .map(|pending| pending.view.clone());
    let selected = settings.selected_provider;
    let last = settings.last_test.get(selected.as_str());
    Ok(ProviderSnapshot {
        selected_provider: selected.to_string(),
        selected_model: settings.selected_model.clone(),
        providers: PROVIDER_REGISTRY
            .iter()
            .map(|row| ProviderRow {
                id: row.id.to_string(),
                label: row.label.to_owned(),
                credential: credential_name(row.credential).to_owned(),
            })
            .collect(),
        models: settings.models_for(selected).to_vec(),
        last_test_ok: last.map(|report| report.ok),
        last_test_message: last.map_or_else(String::new, |report| report.message.clone()),
        has_xai_key,
        has_openai_key,
        has_openrouter_key,
        has_openai_compatible_key,
        openai_compatible_base_url: settings.openai_compatible_base_url.clone(),
        has_xai_oauth,
        oauth_pending: pending,
        plaintext_warning: PLAINTEXT_WARNING.to_owned(),
    })
}

fn load_snapshot() -> Result<ProviderSnapshot, String> {
    let settings = open_settings()?.load().map_err(|e| e.to_string())?;
    let bag = open_secrets()?.load().map_err(|e| e.to_string())?;
    snapshot_from(
        &settings,
        bag.xai_api_key
            .as_ref()
            .is_some_and(|k| !k.trim().is_empty()),
        bag.openai_api_key
            .as_ref()
            .is_some_and(|k| !k.trim().is_empty()),
        bag.openrouter_api_key
            .as_ref()
            .is_some_and(|k| !k.trim().is_empty()),
        bag.openai_compatible_api_key
            .as_ref()
            .is_some_and(|k| !k.trim().is_empty()),
        bag.xai_oauth
            .as_ref()
            .is_some_and(|t| !t.access_token.trim().is_empty()),
    )
}

/// Current provider Settings snapshot.
#[tauri::command]
pub fn provider_snapshot() -> Result<ProviderSnapshot, String> {
    load_snapshot()
}

/// Select the acting provider.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri deserializes command arguments as owned values"
)]
pub fn provider_select(provider_id: String) -> Result<ProviderSnapshot, String> {
    let id: ProviderId = ProviderId::from_str(&provider_id).map_err(|e| e.to_string())?;
    let store = open_settings()?;
    let mut settings = store.load().map_err(|e| e.to_string())?;
    settings.selected_provider = id;
    if !settings
        .models_for(id)
        .iter()
        .any(|m| m == &settings.selected_model)
    {
        settings.selected_model.clear();
    }
    store.save(&settings).map_err(|e| e.to_string())?;
    load_snapshot()
}

/// Save an API key for `xai-key` or `openai`.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri deserializes command arguments as owned values"
)]
pub fn provider_set_key(provider_id: String, key: String) -> Result<ProviderSnapshot, String> {
    let id: ProviderId = ProviderId::from_str(&provider_id).map_err(|e| e.to_string())?;
    let trimmed = key.trim().to_owned();
    if trimmed.is_empty() {
        return Err("API key is empty".to_owned());
    }
    let store = open_secrets()?;
    store
        .update(|bag| match id {
            ProviderId::XaiKey => bag.xai_api_key = Some(trimmed.clone()),
            ProviderId::Openai => bag.openai_api_key = Some(trimmed.clone()),
            ProviderId::Openrouter => bag.openrouter_api_key = Some(trimmed.clone()),
            ProviderId::OpenaiCompatible => bag.openai_compatible_api_key = Some(trimmed.clone()),
            ProviderId::XaiOauth => {}
        })
        .map_err(|e| e.to_string())?;
    let _ = trimmed;
    load_snapshot()
}

/// Clear a saved API key or OAuth tokens for the provider.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri deserializes command arguments as owned values"
)]
pub fn provider_clear_cred(provider_id: String) -> Result<ProviderSnapshot, String> {
    let id: ProviderId = ProviderId::from_str(&provider_id).map_err(|e| e.to_string())?;
    let store = open_secrets()?;
    store
        .update(|bag| match id {
            ProviderId::XaiKey => bag.xai_api_key = None,
            ProviderId::Openai => bag.openai_api_key = None,
            ProviderId::Openrouter => bag.openrouter_api_key = None,
            ProviderId::OpenaiCompatible => bag.openai_compatible_api_key = None,
            ProviderId::XaiOauth => bag.xai_oauth = None,
        })
        .map_err(|e| e.to_string())?;
    if id == ProviderId::XaiOauth {
        *PENDING_OAUTH
            .lock()
            .map_err(|_| "provider oauth lock is poisoned".to_owned())? = None;
    }
    load_snapshot()
}

/// Start xAI device-code sign-in.
#[tauri::command]
pub fn provider_oauth_start() -> Result<ProviderSnapshot, String> {
    #[cfg(not(feature = "live-http"))]
    {
        let _ = MockTransport::new();
        return Err(
            "live HTTP is disabled in this build; rebuild softwake-ui with the live-http feature"
                .to_owned(),
        );
    }
    #[cfg(feature = "live-http")]
    {
        let transport = LiveTransport::new();
        let start = start_device_code(&transport, now_ms()).map_err(|e| e.to_string())?;
        let view = OAuthPendingView {
            user_code: start.user_code,
            verification_url: start.verification_url,
            interval_sec: start.interval_sec,
            expires_at_ms: start.expires_at_ms,
        };
        *PENDING_OAUTH
            .lock()
            .map_err(|_| "provider oauth lock is poisoned".to_owned())? = Some(PendingDevice {
            device_code: start.device_code,
            view,
        });
        load_snapshot()
    }
}

/// Poll xAI device-code sign-in once.
#[tauri::command]
pub fn provider_oauth_poll() -> Result<ProviderSnapshot, String> {
    #[cfg(not(feature = "live-http"))]
    {
        return Err(
            "live HTTP is disabled in this build; rebuild softwake-ui with the live-http feature"
                .to_owned(),
        );
    }
    #[cfg(feature = "live-http")]
    {
        let (device_code, interval_sec) = {
            let guard = PENDING_OAUTH
                .lock()
                .map_err(|_| "provider oauth lock is poisoned".to_owned())?;
            let pending = guard
                .as_ref()
                .ok_or_else(|| "no xAI sign-in in progress".to_owned())?;
            (pending.device_code.clone(), pending.view.interval_sec)
        };
        let transport = LiveTransport::new();
        let poll = poll_device_code(&transport, &device_code, interval_sec, now_ms())
            .map_err(|e| e.to_string())?;
        match poll {
            DevicePoll::Pending { interval_sec } | DevicePoll::SlowDown { interval_sec } => {
                if let Some(pending) = PENDING_OAUTH
                    .lock()
                    .map_err(|_| "provider oauth lock is poisoned".to_owned())?
                    .as_mut()
                {
                    pending.view.interval_sec = interval_sec;
                }
            }
            DevicePoll::Tokens(tokens) => {
                save_oauth_tokens(tokens)?;
                *PENDING_OAUTH
                    .lock()
                    .map_err(|_| "provider oauth lock is poisoned".to_owned())? = None;
            }
            DevicePoll::Denied { message } | DevicePoll::Expired { message } => {
                *PENDING_OAUTH
                    .lock()
                    .map_err(|_| "provider oauth lock is poisoned".to_owned())? = None;
                return Err(message);
            }
        }
        load_snapshot()
    }
}

/// Drop saved xAI OAuth tokens and any pending sign-in.
#[tauri::command]
pub fn provider_oauth_sign_out() -> Result<ProviderSnapshot, String> {
    provider_clear_cred(ProviderId::XaiOauth.to_string())
}

/// Run Test for the selected provider. Fills the model list on success.
#[tauri::command]
pub fn provider_test() -> Result<ProviderSnapshot, String> {
    #[cfg(not(feature = "live-http"))]
    {
        let _ = MockTransport::new();
        return Err(
            "live HTTP is disabled in this build; rebuild softwake-ui with the live-http feature"
                .to_owned(),
        );
    }
    #[cfg(feature = "live-http")]
    {
        let settings_store = open_settings()?;
        let mut settings = settings_store.load().map_err(|e| e.to_string())?;
        let secrets = open_secrets()?;
        let bag = secrets.load().map_err(|e| e.to_string())?;
        let provider = settings.selected_provider;
        let env_xai = std::env::var("XAI_API_KEY").ok();
        let env_openai = std::env::var("OPENAI_API_KEY").ok();
        let env_openrouter = std::env::var("OPENROUTER_API_KEY").ok();
        let env_openai_compatible = std::env::var("OPENAI_COMPATIBLE_API_KEY").ok();
        let bearer = resolve_bearer(
            provider,
            &bag,
            env_xai.as_deref(),
            env_openai.as_deref(),
            env_openrouter.as_deref(),
            env_openai_compatible.as_deref(),
        )
        .unwrap_or_default();
        let api_base =
            softwake_providers::resolve_api_base(provider, &settings).unwrap_or_default();
        let transport = LiveTransport::new();
        let outcome = run_test(&transport, provider, &bearer, &api_base, now_ms());
        apply_test_outcome(&mut settings, provider, &outcome, now_ms());
        settings_store.save(&settings).map_err(|e| e.to_string())?;
        load_snapshot()
    }
}

/// Save the OpenAI-compatible base URL (non-secret).
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri deserializes command arguments as owned values"
)]
pub fn provider_set_base_url(base_url: String) -> Result<ProviderSnapshot, String> {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return Err("base URL is empty".to_owned());
    }
    softwake_providers::normalize_compatible_base(trimmed).map_err(|e| e.to_string())?;
    let store = open_settings()?;
    let mut settings = store.load().map_err(|e| e.to_string())?;
    settings.set_openai_compatible_base_url(trimmed);
    store.save(&settings).map_err(|e| e.to_string())?;
    load_snapshot()
}

/// Pick a chat model from the cached Test catalog.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri deserializes command arguments as owned values"
)]
pub fn provider_set_model(model_id: String) -> Result<ProviderSnapshot, String> {
    let store = open_settings()?;
    let mut settings = store.load().map_err(|e| e.to_string())?;
    let model = model_id.trim().to_owned();
    if model.is_empty() {
        return Err("model id is empty".to_owned());
    }
    if !settings
        .models_for(settings.selected_provider)
        .iter()
        .any(|id| id == &model)
    {
        return Err("model is not in the Test catalog; run Test first".to_owned());
    }
    settings.selected_model = model;
    store.save(&settings).map_err(|e| e.to_string())?;
    load_snapshot()
}

fn save_oauth_tokens(tokens: OAuthTokenSet) -> Result<(), String> {
    open_secrets()?
        .update(|bag| {
            bag.xai_oauth = Some(tokens);
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}
