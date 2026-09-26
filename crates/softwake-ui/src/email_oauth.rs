//! Google / Microsoft Email OAuth (PKCE + loopback).
//!
//! Connect starts a loopback listener, opens the authorize URL, and completes
//! token exchange on a background thread. The Email snapshot exposes pending
//! status and connected account emails. Tokens stay in the secret bag.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(not(feature = "live-http"))]
use softwake_providers::MockTransport;
#[cfg(feature = "live-http")]
use softwake_providers::live::LiveTransport;
use softwake_providers::{
    AccountConnection, AccountProvider, OAUTH_CLIENT_MISSING, PkceStart, SecretStore,
    exchange_and_profile, google_authorize_url, microsoft_authorize_url,
    publisher_google_client_id, publisher_google_client_secret, publisher_microsoft_client_id,
    revoke_google_refresh, update_bag,
};

use crate::oauth_open::{BROWSER_NOTE, openable_authorize_url};
use tauri_plugin_opener::OpenerExt;

const OAUTH_TIMEOUT: Duration = Duration::from_secs(300);
const ACCEPT_SLICE: Duration = Duration::from_millis(500);

#[derive(Debug, Clone)]
struct PendingOauth {
    provider: AccountProvider,
    message: String,
    authorize_url: String,
    generation: u64,
}

#[derive(Debug, Default)]
struct OauthState {
    pending: Option<PendingOauth>,
    cancel: Arc<AtomicBool>,
    last_error: String,
    generation: u64,
}

static OAUTH: LazyLock<Mutex<OauthState>> = LazyLock::new(|| {
    Mutex::new(OauthState {
        pending: None,
        cancel: Arc::new(AtomicBool::new(false)),
        last_error: String::new(),
        generation: 0,
    })
});

fn now_ms() -> u64 {
    let Ok(duration) = SystemTime::now().duration_since(UNIX_EPOCH) else {
        return 0;
    };
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn open_secrets() -> Result<Box<dyn SecretStore + Send>, String> {
    let path = softwake_providers::resolve_secrets_file().map_err(|error| error.to_string())?;
    softwake_providers::open_store(&path).map_err(|error| error.to_string())
}

/// Public OAuth fields for the Email snapshot (no tokens).
#[derive(Debug, Clone)]
pub struct EmailOauthView {
    pub google_connected: bool,
    pub google_email: String,
    pub microsoft_connected: bool,
    pub microsoft_email: String,
    pub oauth_pending: String,
    pub oauth_message: String,
    pub oauth_authorize_url: String,
    pub oauth_error: String,
}

impl EmailOauthView {
    pub fn from_bag(bag: &softwake_providers::SecretBag) -> Self {
        let (pending, message, url, error) = read_pending();
        let google = bag.google_connections.first();
        let microsoft = bag.microsoft_connections.first();
        Self {
            google_connected: google.is_some(),
            google_email: google
                .and_then(|c| c.account_email.clone())
                .unwrap_or_default(),
            microsoft_connected: microsoft.is_some(),
            microsoft_email: microsoft
                .and_then(|c| c.account_email.clone())
                .unwrap_or_default(),
            oauth_pending: pending,
            oauth_message: message,
            oauth_authorize_url: url,
            oauth_error: error,
        }
    }
}

fn read_pending() -> (String, String, String, String) {
    let Ok(guard) = OAUTH.lock() else {
        return (
            "none".to_owned(),
            String::new(),
            String::new(),
            "oauth lock is poisoned".to_owned(),
        );
    };
    match &guard.pending {
        Some(pending) => (
            pending.provider.as_str().to_owned(),
            pending.message.clone(),
            pending.authorize_url.clone(),
            guard.last_error.clone(),
        ),
        None => (
            "none".to_owned(),
            String::new(),
            String::new(),
            guard.last_error.clone(),
        ),
    }
}

/// Start Google or Microsoft Connect.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri injects AppHandle by value"
)]
pub fn email_oauth_connect(
    app: tauri::AppHandle,
    provider: String,
) -> Result<crate::email::EmailSnapshot, String> {
    let provider = AccountProvider::parse(&provider)?;
    #[cfg(not(feature = "live-http"))]
    {
        let _ = app;
        let _ = MockTransport::new();
        return Err(
            "live HTTP is disabled in this build; rebuild softwake-ui with the live-http feature"
                .to_owned(),
        );
    }
    #[cfg(feature = "live-http")]
    {
        let (client_id, client_secret) = match provider {
            AccountProvider::Google => {
                let id =
                    publisher_google_client_id().ok_or_else(|| OAUTH_CLIENT_MISSING.to_owned())?;
                (id, publisher_google_client_secret())
            }
            AccountProvider::Microsoft => {
                let id = publisher_microsoft_client_id()
                    .ok_or_else(|| OAUTH_CLIENT_MISSING.to_owned())?;
                (id, None)
            }
        };

        // Cancel any in-flight attempt.
        email_oauth_cancel_inner();

        let pkce = PkceStart::generate()?;
        let public_host = match provider {
            AccountProvider::Google => "127.0.0.1",
            AccountProvider::Microsoft => "localhost",
        };
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let redirect_uri = format!("http://{public_host}:{port}/callback");
        let authorize_url = match provider {
            AccountProvider::Google => {
                google_authorize_url(&client_id, &redirect_uri, &pkce.state, &pkce.challenge)
            }
            AccountProvider::Microsoft => {
                microsoft_authorize_url(&client_id, &redirect_uri, &pkce.state, &pkce.challenge)
            }
        };

        let cancel = Arc::new(AtomicBool::new(false));
        let generation = {
            let mut guard = OAUTH
                .lock()
                .map_err(|_| "oauth lock is poisoned".to_owned())?;
            guard.generation = guard.generation.saturating_add(1);
            guard.cancel = Arc::clone(&cancel);
            guard.last_error.clear();
            guard.pending = Some(PendingOauth {
                provider,
                message: "Waiting for sign-in in the browser…".to_owned(),
                authorize_url: authorize_url.clone(),
                generation: guard.generation,
            });
            guard.generation
        };

        if openable_authorize_url(&authorize_url) {
            if let Err(err) = app.opener().open_url(authorize_url.clone(), None::<&str>) {
                eprintln!("softwake-ui: could not open authorize URL: {err}");
                let mut guard = OAUTH
                    .lock()
                    .map_err(|_| "oauth lock is poisoned".to_owned())?;
                if let Some(pending) = guard.pending.as_mut() {
                    if pending.generation == generation {
                        BROWSER_NOTE.clone_into(&mut pending.message);
                    }
                }
            }
        } else {
            let mut guard = OAUTH
                .lock()
                .map_err(|_| "oauth lock is poisoned".to_owned())?;
            if let Some(pending) = guard.pending.as_mut() {
                if pending.generation == generation {
                    BROWSER_NOTE.clone_into(&mut pending.message);
                }
            }
        }

        let verifier = pkce.verifier;
        let state = pkce.state;
        thread::spawn(move || {
            let outcome = run_exchange(
                provider,
                &listener,
                &state,
                &redirect_uri,
                &client_id,
                client_secret.as_deref(),
                &verifier,
                &cancel,
            );
            finish_pending(generation, provider, outcome);
        });

        crate::email::email_snapshot()
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "exchange needs listener + PKCE + client fields"
)]
fn run_exchange(
    provider: AccountProvider,
    listener: &TcpListener,
    expected_state: &str,
    redirect_uri: &str,
    client_id: &str,
    client_secret: Option<&str>,
    verifier: &str,
    cancel: &AtomicBool,
) -> Result<AccountConnection, String> {
    let code = wait_for_code(listener, expected_state, cancel)?;
    #[cfg(feature = "live-http")]
    {
        let transport = LiveTransport::new();
        exchange_and_profile(
            provider,
            &transport,
            &code,
            redirect_uri,
            client_id,
            verifier,
            client_secret,
            now_ms(),
        )
    }
    #[cfg(not(feature = "live-http"))]
    {
        let _ = (
            provider,
            code,
            redirect_uri,
            client_id,
            client_secret,
            verifier,
        );
        Err("live HTTP is disabled".to_owned())
    }
}

fn finish_pending(
    generation: u64,
    provider: AccountProvider,
    outcome: Result<AccountConnection, String>,
) {
    let Ok(mut guard) = OAUTH.lock() else {
        return;
    };
    let still = guard
        .pending
        .as_ref()
        .is_some_and(|p| p.generation == generation);
    if !still {
        return;
    }
    match outcome {
        Ok(connection) => {
            if let Ok(secrets) = open_secrets() {
                let save = update_bag(&*secrets, |bag| match provider {
                    AccountProvider::Google => {
                        bag.google_connections = vec![connection];
                    }
                    AccountProvider::Microsoft => {
                        bag.microsoft_connections = vec![connection];
                    }
                });
                if let Err(error) = save {
                    guard.last_error = error.to_string();
                } else {
                    guard.last_error.clear();
                }
            } else {
                "could not open secret store".clone_into(&mut guard.last_error);
            }
        }
        Err(error) => {
            guard.last_error = error;
        }
    }
    guard.pending = None;
}

fn wait_for_code(
    listener: &TcpListener,
    expected_state: &str,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let deadline = Instant::now() + OAUTH_TIMEOUT;
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err("sign-in cancelled".to_owned());
        }
        if Instant::now() >= deadline {
            return Err("sign-in timed out".to_owned());
        }
        match listener.accept() {
            Ok((stream, _)) => {
                if let Some(code) = handle_callback(stream, expected_state) {
                    return Ok(code);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(ACCEPT_SLICE);
            }
            Err(error) => return Err(error.to_string()),
        }
    }
}

fn handle_callback(mut stream: TcpStream, expected_state: &str) -> Option<String> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buf = [0u8; 4096];
    let n = stream.read(&mut buf).unwrap_or(0);
    let request = String::from_utf8_lossy(&buf[..n]);
    let line = request.lines().next().unwrap_or("");
    // GET /callback?code=...&state=... HTTP/1.1
    let path = line.split_whitespace().nth(1).unwrap_or("");
    if !path.starts_with("/callback") {
        let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
        return None;
    }
    let query = path.split_once('?').map_or("", |(_, q)| q);
    let mut code = None;
    let mut state = None;
    for pair in query.split('&') {
        let mut parts = pair.splitn(2, '=');
        let key = parts.next().unwrap_or("");
        let value = parts.next().unwrap_or("");
        match key {
            "code" => code = Some(url_decode(value)),
            "state" => state = Some(url_decode(value)),
            _ => {}
        }
    }
    if state.as_deref() != Some(expected_state) {
        let body = b"state mismatch";
        let _ = write!(
            stream,
            "HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(body);
        return None;
    }
    let Some(code) = code.filter(|c| !c.is_empty()) else {
        let body = b"missing code";
        let _ = write!(
            stream,
            "HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(body);
        return None;
    };
    let body =
        b"<html><body><p>Softwake sign-in complete. You can close this window.</p></body></html>";
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(body);
    Some(code)
}

fn url_decode(value: &str) -> String {
    let mut out = Vec::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let h = hex_val(bytes[i + 1]);
                let l = hex_val(bytes[i + 2]);
                if let (Some(h), Some(l)) = (h, l) {
                    out.push((h << 4) | l);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn email_oauth_cancel_inner() {
    if let Ok(mut guard) = OAUTH.lock() {
        guard.cancel.store(true, Ordering::SeqCst);
        guard.pending = None;
        guard.cancel = Arc::new(AtomicBool::new(false));
    }
}

/// Cancel an in-flight Connect.
#[tauri::command]
pub fn email_oauth_cancel() -> Result<crate::email::EmailSnapshot, String> {
    email_oauth_cancel_inner();
    if let Ok(mut guard) = OAUTH.lock() {
        guard.last_error.clear();
    }
    crate::email::email_snapshot()
}

/// Disconnect Google or Microsoft and drop tokens from the bag.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri deserializes command arguments as owned values"
)]
pub fn email_oauth_disconnect(provider: String) -> Result<crate::email::EmailSnapshot, String> {
    let provider = AccountProvider::parse(&provider)?;
    let secrets = open_secrets()?;
    let bag = secrets.load().map_err(|e| e.to_string())?;
    if provider == AccountProvider::Google {
        if let Some(connection) = bag.google_connections.first() {
            if !connection.refresh_token.is_empty() {
                #[cfg(feature = "live-http")]
                {
                    let transport = LiveTransport::new();
                    revoke_google_refresh(&transport, &connection.refresh_token);
                }
            }
        }
    }
    update_bag(&*secrets, |bag| match provider {
        AccountProvider::Google => bag.google_connections.clear(),
        AccountProvider::Microsoft => bag.microsoft_connections.clear(),
    })
    .map_err(|e| e.to_string())?;
    if let Ok(mut guard) = OAUTH.lock() {
        guard.last_error.clear();
    }
    crate::email::email_snapshot()
}

#[cfg(test)]
mod tests {
    use super::{handle_callback, url_decode};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn url_decode_percent() {
        assert_eq!(url_decode("a%2Fb"), "a/b");
        assert_eq!(url_decode("a+b"), "a b");
    }

    #[test]
    fn callback_returns_code_when_state_matches() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            handle_callback(stream, "state-xyz")
        });
        let mut client = std::net::TcpStream::connect(addr).expect("connect");
        write!(
            client,
            "GET /callback?code=the-code&state=state-xyz HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n"
        )
        .expect("write");
        let mut response = String::new();
        let _ = client.read_to_string(&mut response);
        assert!(response.contains("200 OK"));
        assert_eq!(server.join().expect("join"), Some("the-code".to_owned()));
    }
}
