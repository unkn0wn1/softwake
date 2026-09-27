//! Google and Microsoft authorization-code OAuth (PKCE).
//!
//! Body builders and parsers are network-free. Callers pass a [`crate::transport::Transport`]
//! when they need HTTP. Publisher client ids come from process env — never from Settings.

use std::env;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::pkce;
use crate::transport::{Transport, TransportError};

/// Shown when a required publisher client id is unset.
pub const OAUTH_CLIENT_MISSING: &str = "This build has no OAuth client configured";

/// Google authorize endpoint.
pub const GOOGLE_AUTHORIZE_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
/// Google token endpoint.
pub const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
/// Google userinfo endpoint.
pub const GOOGLE_USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v2/userinfo";
/// Google token revoke endpoint.
pub const GOOGLE_REVOKE_URL: &str = "https://oauth2.googleapis.com/revoke";

/// Microsoft authorize endpoint (common tenant).
pub const MICROSOFT_AUTHORIZE_URL: &str =
    "https://login.microsoftonline.com/common/oauth2/v2.0/authorize";
/// Microsoft token endpoint (common tenant).
pub const MICROSOFT_TOKEN_URL: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/token";
/// Microsoft Graph profile.
pub const MICROSOFT_PROFILE_URL: &str =
    "https://graph.microsoft.com/v1.0/me?$select=id,mail,userPrincipalName";

/// Combined Google scopes: openid/email + calendar + drive + gmail send/read.
pub const GOOGLE_EMAIL_SCOPES: &str = "openid email https://www.googleapis.com/auth/calendar.readonly https://www.googleapis.com/auth/drive.file https://www.googleapis.com/auth/gmail.send https://www.googleapis.com/auth/gmail.readonly";

/// Combined Microsoft Graph scopes: identity + calendar + `AppFolder` + mail.
pub const MICROSOFT_EMAIL_SCOPES: &str = "openid profile email offline_access User.Read Calendars.Read Files.ReadWrite.AppFolder Mail.Send Mail.Read";

const CLIENT_ID_MAX: usize = 200;
const SECRET_MAX: usize = 500;

/// Which account provider the Email pane connects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccountProvider {
    /// Google (Gmail / Calendar / Drive).
    Google,
    /// Microsoft (Outlook / Calendar / `OneDrive` `AppFolder`).
    Microsoft,
}

impl AccountProvider {
    /// Parse `google` or `microsoft`.
    ///
    /// # Errors
    ///
    /// Unknown spelling.
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "google" => Ok(Self::Google),
            "microsoft" => Ok(Self::Microsoft),
            _ => Err(format!("unknown account provider: {value}")),
        }
    }

    /// Wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Google => "google",
            Self::Microsoft => "microsoft",
        }
    }
}

/// One signed-in Google or Microsoft account. Tokens are redacted in [`Debug`].
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountConnection {
    /// Provider user id.
    pub id: String,
    /// Access token.
    pub access_token: String,
    /// Refresh token.
    pub refresh_token: String,
    /// Unix milliseconds when the access token expires.
    pub expires_at_ms: u64,
    /// Token type, usually `Bearer`.
    pub token_type: String,
    /// Granted scope string from the token response (or requested scopes).
    pub scope: String,
    /// Account email when the profile endpoint returned one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_email: Option<String>,
}

impl std::fmt::Debug for AccountConnection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AccountConnection")
            .field("id", &self.id)
            .field("access_token", &"<redacted>")
            .field("refresh_token", &"<redacted>")
            .field("expires_at_ms", &self.expires_at_ms)
            .field("token_type", &self.token_type)
            .field("scope", &self.scope)
            .field("account_email", &self.account_email)
            .finish()
    }
}

/// Read `SOFTWAKE_GOOGLE_CLIENT_ID` (or `MeetRec` alias) from process env or
/// `oauth-clients.env`.
#[must_use]
pub fn publisher_google_client_id() -> Option<String> {
    clean_id(publisher_value(&[
        "SOFTWAKE_GOOGLE_CLIENT_ID",
        "MEETREC_GOOGLE_CLIENT_ID",
    ]))
}

/// Read optional `SOFTWAKE_GOOGLE_CLIENT_SECRET` (or `MeetRec` alias).
#[must_use]
pub fn publisher_google_client_secret() -> Option<String> {
    clean_secret(publisher_value(&[
        "SOFTWAKE_GOOGLE_CLIENT_SECRET",
        "MEETREC_GOOGLE_CLIENT_SECRET",
    ]))
}

/// Read `SOFTWAKE_MICROSOFT_CLIENT_ID` (or `MeetRec` alias) from process env or
/// `oauth-clients.env`.
#[must_use]
pub fn publisher_microsoft_client_id() -> Option<String> {
    clean_id(publisher_value(&[
        "SOFTWAKE_MICROSOFT_CLIENT_ID",
        "MEETREC_MICROSOFT_CLIENT_ID",
    ]))
}

fn env_first(keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = env::var_os(key).and_then(|v| v.into_string().ok()) {
            return Some(value);
        }
    }
    None
}

fn file_value(keys: &[&str]) -> Option<String> {
    let map = load_oauth_clients_file();
    for key in keys {
        if let Some(value) = map.get(*key) {
            return Some(value.clone());
        }
    }
    None
}

fn publisher_value(keys: &[&str]) -> Option<String> {
    env_first(keys).or_else(|| file_value(keys))
}

fn load_oauth_clients_file() -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let Some(path) = oauth_clients_path() else {
        return map;
    };
    let Ok(bytes) = fs::read(&path) else {
        return map;
    };
    let Ok(body) = String::from_utf8(bytes) else {
        return map;
    };
    for (key, value) in parse_oauth_clients_env(&body) {
        map.insert(key, value);
    }
    map
}

const OAUTH_CLIENTS_FILE: &str = "oauth-clients.env";

/// Softwake publisher OAuth keys that may be loaded from the config file.
const PUBLISHER_OAUTH_KEYS: &[&str] = &[
    "SOFTWAKE_GOOGLE_CLIENT_ID",
    "SOFTWAKE_GOOGLE_CLIENT_SECRET",
    "SOFTWAKE_MICROSOFT_CLIENT_ID",
    // MeetRec aliases — same desktop clients on a shared maintainer machine.
    "MEETREC_GOOGLE_CLIENT_ID",
    "MEETREC_GOOGLE_CLIENT_SECRET",
    "MEETREC_MICROSOFT_CLIENT_ID",
];

fn oauth_clients_path() -> Option<PathBuf> {
    let config = if let Some(xdg) = env::var_os("XDG_CONFIG_HOME") {
        let path = PathBuf::from(xdg);
        if path.as_os_str().is_empty() {
            return None;
        }
        path
    } else {
        let home = PathBuf::from(env::var_os("HOME")?);
        if home.as_os_str().is_empty() {
            return None;
        }
        home.join(".config")
    };
    Some(config.join("softwake").join(OAUTH_CLIENTS_FILE))
}

/// Parse `KEY=VALUE` lines; `#` comments and blank lines are ignored.
///
/// Values may be optionally single- or double-quoted. Does not expand shell
/// variables. Never logs values.
#[must_use]
pub fn parse_oauth_clients_env(body: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for raw in body.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() || !PUBLISHER_OAUTH_KEYS.contains(&key) {
            continue;
        }
        let trimmed = value.trim();
        let value = strip_wrapping_quotes(trimmed);
        out.push((key.to_owned(), value));
    }
    out
}

fn strip_wrapping_quotes(raw: &str) -> String {
    let bytes = raw.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return raw[1..raw.len() - 1].to_owned();
        }
    }
    raw.to_owned()
}

/// Warm the publisher OAuth file lookup (no-op beyond a read).
///
/// [`publisher_google_client_id`] and friends already read
/// `$XDG_CONFIG_HOME/softwake/oauth-clients.env` when process env is unset.
/// Process env always wins. Missing file is a no-op. Kept so `softwake-ui`
/// can call this once at startup for a clear load point.
pub fn apply_publisher_oauth_from_config() {
    let _ = load_oauth_clients_file();
}

fn clean_id(value: Option<String>) -> Option<String> {
    let trimmed = value?.trim().to_owned();
    if trimmed.is_empty() || trimmed.len() > CLIENT_ID_MAX || trimmed.contains(['\r', '\n']) {
        return None;
    }
    Some(trimmed)
}

fn clean_secret(value: Option<String>) -> Option<String> {
    let trimmed = value?.trim().to_owned();
    if trimmed.is_empty() || trimmed.len() > SECRET_MAX || trimmed.contains(['\r', '\n']) {
        return None;
    }
    Some(trimmed)
}

/// Build the Google authorize URL (PKCE).
#[must_use]
pub fn google_authorize_url(
    client_id: &str,
    redirect_uri: &str,
    state: &str,
    code_challenge: &str,
) -> String {
    url_with_query(
        GOOGLE_AUTHORIZE_URL,
        &[
            ("client_id", client_id),
            ("redirect_uri", redirect_uri),
            ("response_type", "code"),
            ("scope", GOOGLE_EMAIL_SCOPES),
            ("state", state),
            ("code_challenge", code_challenge),
            ("code_challenge_method", "S256"),
            ("access_type", "offline"),
            ("prompt", "consent"),
            ("include_granted_scopes", "true"),
        ],
    )
}

/// Build the Microsoft authorize URL (PKCE).
#[must_use]
pub fn microsoft_authorize_url(
    client_id: &str,
    redirect_uri: &str,
    state: &str,
    code_challenge: &str,
) -> String {
    url_with_query(
        MICROSOFT_AUTHORIZE_URL,
        &[
            ("client_id", client_id),
            ("redirect_uri", redirect_uri),
            ("response_type", "code"),
            ("scope", MICROSOFT_EMAIL_SCOPES),
            ("state", state),
            ("code_challenge", code_challenge),
            ("code_challenge_method", "S256"),
            ("prompt", "select_account"),
            ("response_mode", "query"),
        ],
    )
}

fn url_with_query(base: &str, pairs: &[(&str, &str)]) -> String {
    let mut out = String::from(base);
    out.push('?');
    for (i, (k, v)) in pairs.iter().enumerate() {
        if i > 0 {
            out.push('&');
        }
        out.push_str(&form_encode(k));
        out.push('=');
        out.push_str(&form_encode(v));
    }
    out
}

fn form_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            b' ' => out.push_str("%20"),
            _ => {
                out.push('%');
                out.push(HEX[(byte >> 4) as usize] as char);
                out.push(HEX[(byte & 0xf) as usize] as char);
            }
        }
    }
    out
}

const HEX: &[u8] = b"0123456789ABCDEF";

/// Token exchange body for an authorization code.
#[must_use]
pub fn token_exchange_body(
    code: &str,
    redirect_uri: &str,
    client_id: &str,
    code_verifier: &str,
    client_secret: Option<&str>,
) -> String {
    let mut pairs = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("client_id", client_id),
        ("code_verifier", code_verifier),
    ];
    if let Some(secret) = client_secret {
        pairs.push(("client_secret", secret));
    }
    form_body(&pairs)
}

/// Refresh-token grant body.
#[must_use]
pub fn refresh_token_body(
    refresh_token: &str,
    client_id: &str,
    client_secret: Option<&str>,
) -> String {
    let mut pairs = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", client_id),
    ];
    if let Some(secret) = client_secret {
        pairs.push(("client_secret", secret));
    }
    form_body(&pairs)
}

fn form_body(pairs: &[(&str, &str)]) -> String {
    let mut out = String::new();
    for (i, (k, v)) in pairs.iter().enumerate() {
        if i > 0 {
            out.push('&');
        }
        out.push_str(&form_encode(k));
        out.push('=');
        out.push_str(&form_encode(v));
    }
    out
}

/// Whether an account access token should be refreshed before use.
#[must_use]
pub fn account_needs_refresh(connection: &AccountConnection, now_ms: u64) -> bool {
    connection.expires_at_ms == 0 || now_ms + 60_000 >= connection.expires_at_ms
}

/// Keep prior refresh token / id / email when a refresh response omits them.
#[must_use]
pub fn merge_account_refresh(
    previous: &AccountConnection,
    next: AccountConnection,
) -> AccountConnection {
    AccountConnection {
        id: if next.id.is_empty() {
            previous.id.clone()
        } else {
            next.id
        },
        access_token: next.access_token,
        refresh_token: if next.refresh_token.is_empty() {
            previous.refresh_token.clone()
        } else {
            next.refresh_token
        },
        expires_at_ms: next.expires_at_ms,
        token_type: next.token_type,
        scope: if next.scope.is_empty() {
            previous.scope.clone()
        } else {
            next.scope
        },
        account_email: next
            .account_email
            .or_else(|| previous.account_email.clone()),
    }
}

/// Refresh Google/Microsoft account access when near expiry.
///
/// # Errors
///
/// Missing publisher client id, transport failure, or token response errors.
pub fn ensure_fresh_account(
    provider: AccountProvider,
    transport: &dyn Transport,
    connection: &AccountConnection,
    now_ms: u64,
) -> Result<AccountConnection, String> {
    if !account_needs_refresh(connection, now_ms) {
        return Ok(connection.clone());
    }
    if connection.refresh_token.trim().is_empty() {
        return Err("account refresh token is missing; reconnect in Settings → Email".to_owned());
    }
    let (token_url, client_id, client_secret, scopes) = match provider {
        AccountProvider::Google => {
            let id = publisher_google_client_id().ok_or_else(|| OAUTH_CLIENT_MISSING.to_owned())?;
            let secret = publisher_google_client_secret();
            (GOOGLE_TOKEN_URL, id, secret, GOOGLE_EMAIL_SCOPES)
        }
        AccountProvider::Microsoft => {
            let id =
                publisher_microsoft_client_id().ok_or_else(|| OAUTH_CLIENT_MISSING.to_owned())?;
            (MICROSOFT_TOKEN_URL, id, None, MICROSOFT_EMAIL_SCOPES)
        }
    };
    let body = refresh_token_body(
        &connection.refresh_token,
        &client_id,
        client_secret.as_deref(),
    );
    let response = transport
        .post_form(token_url, &body)
        .map_err(transport_message)?;
    if !(200..300).contains(&response.status) {
        return Err("account token refresh failed".to_owned());
    }
    let next = parse_token_json(&response.body, now_ms, scopes)?;
    Ok(merge_account_refresh(connection, next))
}

/// Parse a token endpoint JSON body into an [`AccountConnection`] shell (id/email filled later).
///
/// # Errors
///
/// Missing access token or invalid JSON.
pub fn parse_token_json(
    body: &str,
    now_ms: u64,
    fallback_scope: &str,
) -> Result<AccountConnection, String> {
    let value: Value =
        serde_json::from_str(body).map_err(|_| "token response was not JSON".to_owned())?;
    let access = value
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "sign-in did not return an access token".to_owned())?;
    let refresh = value
        .get("refresh_token")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let expires_in = value
        .get("expires_in")
        .and_then(Value::as_u64)
        .unwrap_or(3600);
    let token_type = value
        .get("token_type")
        .and_then(Value::as_str)
        .unwrap_or("Bearer")
        .to_owned();
    let scope = value
        .get("scope")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(fallback_scope)
        .to_owned();
    Ok(AccountConnection {
        id: String::new(),
        access_token: access.to_owned(),
        refresh_token: refresh,
        expires_at_ms: now_ms.saturating_add(expires_in.saturating_mul(1000)),
        token_type,
        scope,
        account_email: None,
    })
}

/// Parse Google userinfo JSON for id + email.
#[must_use]
pub fn parse_google_profile(body: &str) -> (Option<String>, Option<String>) {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return (None, None);
    };
    (text_field(&value, "id"), text_field(&value, "email"))
}

/// Parse Microsoft Graph `me` JSON for id + email.
#[must_use]
pub fn parse_microsoft_profile(body: &str) -> (Option<String>, Option<String>) {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return (None, None);
    };
    let id = text_field(&value, "id");
    let email = text_field(&value, "mail").or_else(|| text_field(&value, "userPrincipalName"));
    (id, email)
}

fn text_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Fresh PKCE pair + state for a connect attempt.
pub struct PkceStart {
    /// Code verifier (keep secret until exchange).
    pub verifier: String,
    /// S256 challenge.
    pub challenge: String,
    /// Opaque state.
    pub state: String,
}

impl PkceStart {
    /// Generate verifier, challenge, and state.
    ///
    /// # Errors
    ///
    /// RNG failure.
    pub fn generate() -> Result<Self, String> {
        let verifier = pkce::code_verifier()?;
        let challenge = pkce::code_challenge(&verifier);
        let state = pkce::oauth_state()?;
        Ok(Self {
            verifier,
            challenge,
            state,
        })
    }
}

/// Exchange an authorization code and load the account profile.
///
/// # Errors
///
/// Transport failure, missing tokens, or missing account id.
#[allow(
    clippy::too_many_arguments,
    reason = "provider + transport + code exchange fields are clearer as separate args"
)]
pub fn exchange_and_profile(
    provider: AccountProvider,
    transport: &dyn Transport,
    code: &str,
    redirect_uri: &str,
    client_id: &str,
    code_verifier: &str,
    client_secret: Option<&str>,
    now_ms: u64,
) -> Result<AccountConnection, String> {
    let (token_url, scopes, profile_url) = match provider {
        AccountProvider::Google => (GOOGLE_TOKEN_URL, GOOGLE_EMAIL_SCOPES, GOOGLE_USERINFO_URL),
        AccountProvider::Microsoft => (
            MICROSOFT_TOKEN_URL,
            MICROSOFT_EMAIL_SCOPES,
            MICROSOFT_PROFILE_URL,
        ),
    };
    let body = token_exchange_body(code, redirect_uri, client_id, code_verifier, client_secret);
    let response = transport
        .post_form(token_url, &body)
        .map_err(transport_message)?;
    if !(200..300).contains(&response.status) {
        return Err("sign-in token exchange failed".to_owned());
    }
    let mut connection = parse_token_json(&response.body, now_ms, scopes)?;
    if provider == AccountProvider::Google && connection.refresh_token.is_empty() {
        return Err("Google sign-in did not return a refresh token".to_owned());
    }
    let profile = transport
        .get_bearer(profile_url, &connection.access_token)
        .map_err(transport_message)?;
    let (id, email) = match provider {
        AccountProvider::Google => parse_google_profile(&profile.body),
        AccountProvider::Microsoft => parse_microsoft_profile(&profile.body),
    };
    let id = id.ok_or_else(|| format!("{} did not return an account id", provider.as_str()))?;
    connection.id = id;
    connection.account_email = email;
    Ok(connection)
}

fn transport_message(error: TransportError) -> String {
    match error {
        TransportError::NoRoute { method, url } => format!("no mock route for {method} {url}"),
        TransportError::Failed { message } => message,
    }
}

/// Best-effort Google refresh-token revoke. Errors are ignored by callers.
pub fn revoke_google_refresh(transport: &dyn Transport, refresh_token: &str) {
    let body = form_body(&[("token", refresh_token)]);
    let _ = transport.post_form(GOOGLE_REVOKE_URL, &body);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn google_authorize_contains_pkce_and_scopes() {
        let url = google_authorize_url(
            "client",
            "http://127.0.0.1:9/callback",
            "state-1",
            "challenge-1",
        );
        assert!(url.starts_with(GOOGLE_AUTHORIZE_URL));
        assert!(url.contains("code_challenge=challenge-1"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("access_type=offline"));
        assert!(
            url.contains("gmail.send")
                || url.contains("gmail%2Esend")
                || url.contains(form_encode("https://www.googleapis.com/auth/gmail.send").as_str())
                || url.contains("gmail")
        );
    }

    #[test]
    fn microsoft_authorize_uses_localhost_friendly_params() {
        let url = microsoft_authorize_url(
            "client",
            "http://localhost:9/callback",
            "state-2",
            "challenge-2",
        );
        assert!(url.starts_with(MICROSOFT_AUTHORIZE_URL));
        assert!(url.contains("response_mode=query"));
        assert!(url.contains("Mail.Send") || url.contains("Mail%2ESend") || url.contains("Mail"));
    }

    #[test]
    fn parse_token_requires_access() {
        let err = parse_token_json("{}", 0, "s").unwrap_err();
        assert!(err.contains("access token"));
        let ok = parse_token_json(
            r#"{"access_token":"a","refresh_token":"r","expires_in":60,"token_type":"Bearer","scope":"openid"}"#,
            1_000,
            "fallback",
        )
        .expect("parse");
        assert_eq!(ok.access_token, "a");
        assert_eq!(ok.refresh_token, "r");
        assert_eq!(ok.expires_at_ms, 61_000);
        assert_eq!(ok.scope, "openid");
    }

    #[test]
    fn profiles_extract_id_and_email() {
        let (id, email) = parse_google_profile(r#"{"id":"g1","email":"ada@example.com"}"#);
        assert_eq!(id.as_deref(), Some("g1"));
        assert_eq!(email.as_deref(), Some("ada@example.com"));
        let (id, email) = parse_microsoft_profile(
            r#"{"id":"m1","mail":null,"userPrincipalName":"ada@contoso.com"}"#,
        );
        assert_eq!(id.as_deref(), Some("m1"));
        assert_eq!(email.as_deref(), Some("ada@contoso.com"));
    }

    #[test]
    fn provider_parse() {
        assert_eq!(
            AccountProvider::parse("google").unwrap(),
            AccountProvider::Google
        );
        assert!(AccountProvider::parse("xai").is_err());
    }
}

#[cfg(test)]
mod publisher_env_tests {
    use super::parse_oauth_clients_env;

    #[test]
    fn parse_oauth_clients_env_skips_unknown_keys() {
        let rows = parse_oauth_clients_env(
            "# comment\nSOFTWAKE_GOOGLE_CLIENT_ID=abc\nIGNORED=x\nMEETREC_MICROSOFT_CLIENT_ID=\"ms-id\"\n",
        );
        assert_eq!(
            rows,
            vec![
                ("SOFTWAKE_GOOGLE_CLIENT_ID".to_owned(), "abc".to_owned()),
                ("MEETREC_MICROSOFT_CLIENT_ID".to_owned(), "ms-id".to_owned()),
            ]
        );
    }
}
