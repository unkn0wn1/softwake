//! Settings commands for opt-in live email.
//!
//! Non-secret fields live in `email.json`. The SMTP password stays in the
//! provider secret bag. This pane does not send mail; Test only validates
//! configuration. Confirm-gated `email_send` remains on the daemon.

use serde::Serialize;
use softwake_connectors::{
    EmailSettings, FileEmailSettings, LiveEmail, parse_live_email_mode, resolve_email_file,
};
use softwake_providers::{SecretBag, SecretStore, update_bag};

/// Snapshot returned to the window. No secrets.
#[derive(Debug, Clone, Serialize)]
pub struct EmailSnapshot {
    /// Operator opt-in. Default false.
    pub live_enabled: bool,
    /// SMTP host (may be empty).
    pub smtp_host: String,
    /// SMTP port.
    pub smtp_port: u16,
    /// SMTP username (may be empty).
    pub username: String,
    /// From address (may be empty).
    pub from_address: String,
    /// `draft_only` or `send`.
    pub mode: String,
    /// Whether an SMTP password is saved in the secret bag.
    pub has_password: bool,
    /// Last Test ok flag.
    pub last_test_ok: Option<bool>,
    /// Last Test message. Never a password.
    pub last_test_message: String,
    /// `keyring`, `plaintext`, or `unavailable`.
    pub storage_backend: String,
    /// Storage status. Never a secret.
    pub storage_message: String,
}

fn open_settings() -> Result<FileEmailSettings, String> {
    FileEmailSettings::new(resolve_email_file().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

fn open_secrets() -> Result<Box<dyn SecretStore + Send>, String> {
    let path = softwake_providers::resolve_secrets_file().map_err(|error| error.to_string())?;
    softwake_providers::open_store(&path).map_err(|error| error.to_string())
}

fn password_present(bag: &SecretBag) -> bool {
    bag.email_smtp_password
        .as_ref()
        .is_some_and(|password| !password.is_empty())
}

fn snapshot_from(
    settings: &EmailSettings,
    bag: &SecretBag,
    backend: &str,
    message: &str,
) -> EmailSnapshot {
    EmailSnapshot {
        live_enabled: settings.live_enabled,
        smtp_host: settings.smtp_host.clone(),
        smtp_port: settings.smtp_port,
        username: settings.username.clone(),
        from_address: settings.from_address.clone(),
        mode: settings.mode.as_str().to_owned(),
        has_password: password_present(bag),
        last_test_ok: settings.last_test_ok,
        last_test_message: settings.last_test_message.clone(),
        storage_backend: backend.to_owned(),
        storage_message: message.to_owned(),
    }
}

fn load_snapshot() -> Result<EmailSnapshot, String> {
    let settings_store = open_settings()?;
    let settings = settings_store.load().map_err(|e| e.to_string())?;
    let secrets = open_secrets()?;
    let bag = secrets.load().map_err(|e| e.to_string())?;
    let report = secrets.report();
    Ok(snapshot_from(
        &settings,
        &bag,
        report.backend.as_str(),
        &report.message,
    ))
}

/// Read email Settings. The snapshot has no password.
#[tauri::command]
pub fn email_snapshot() -> Result<EmailSnapshot, String> {
    load_snapshot()
}

/// Save non-secret fields and optional password. Does not send mail.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri deserializes command arguments as owned values"
)]
pub fn email_save(
    live_enabled: bool,
    smtp_host: String,
    smtp_port: u16,
    username: String,
    from_address: String,
    mode: String,
    password: Option<String>,
) -> Result<EmailSnapshot, String> {
    let mode = parse_live_email_mode(&mode).map_err(|e| e.to_string())?;
    let settings_store = open_settings()?;
    let mut settings = settings_store.load().map_err(|e| e.to_string())?;
    settings.live_enabled = live_enabled;
    settings.smtp_host = smtp_host;
    settings.smtp_port = if smtp_port == 0 {
        softwake_connectors::DEFAULT_SMTP_PORT
    } else {
        smtp_port
    };
    settings.username = username;
    settings.from_address = from_address;
    settings.mode = mode;
    settings_store.save(&settings).map_err(|e| e.to_string())?;

    if let Some(password) = password {
        let trimmed = password.trim().to_owned();
        if !trimmed.is_empty() {
            let secrets = open_secrets()?;
            update_bag(&*secrets, |bag| {
                bag.email_smtp_password = Some(trimmed);
            })
            .map_err(|e| e.to_string())?;
        }
    }
    load_snapshot()
}

/// Clear the saved SMTP password. Does not change non-secret Settings.
#[tauri::command]
pub fn email_clear_password() -> Result<EmailSnapshot, String> {
    let secrets = open_secrets()?;
    update_bag(&*secrets, |bag| {
        bag.email_smtp_password = None;
    })
    .map_err(|e| e.to_string())?;
    load_snapshot()
}

/// Validate configuration without opening a socket and without sending mail.
#[tauri::command]
pub fn email_test() -> Result<EmailSnapshot, String> {
    let settings_store = open_settings()?;
    let mut settings = settings_store.load().map_err(|e| e.to_string())?;
    let secrets = open_secrets()?;
    let bag = secrets.load().map_err(|e| e.to_string())?;
    let live = LiveEmail::new(settings.clone(), password_present(&bag));
    match live.test_connection() {
        Ok(message) => {
            settings.last_test_ok = Some(true);
            message.clone_into(&mut settings.last_test_message);
        }
        Err(error) => {
            settings.last_test_ok = Some(false);
            settings.last_test_message = error.to_string();
        }
    }
    settings_store.save(&settings).map_err(|e| e.to_string())?;
    let report = secrets.report();
    Ok(snapshot_from(
        &settings,
        &bag,
        report.backend.as_str(),
        &report.message,
    ))
}

#[cfg(test)]
mod tests {
    use softwake_connectors::LiveEmailMode;

    #[test]
    fn mode_spellings_match_settings() {
        assert_eq!(LiveEmailMode::DraftOnly.as_str(), "draft_only");
        assert_eq!(LiveEmailMode::Send.as_str(), "send");
    }
}
