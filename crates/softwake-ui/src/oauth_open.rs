//! Predicates for URLs Softwake may open in the system browser.
//!
//! Rust `open_url` does not consult the capability scope. The plugin click
//! script does. These predicates and `opener:allow-open-url` name the same set.

/// Shown when the browser was not opened. OS error text stays on stderr.
#[cfg(feature = "live-http")]
pub(crate) const BROWSER_NOTE: &str = "Could not open the browser. Use the address below.";

#[cfg(not(feature = "live-http"))]
#[allow(dead_code, reason = "shared with email_oauth when live-http is off")]
pub(crate) const BROWSER_NOTE: &str = "Could not open the browser. Use the address below.";

/// Whether `raw` may be passed to `open_url` for xAI device-code verification.
///
/// Accepts `https://auth.x.ai` and `https://auth.x.ai/…` only: https, that
/// host in lowercase, no userinfo, no port, and no control characters.
pub(crate) fn openable_verification_url(raw: &str) -> bool {
    openable_https_host(raw, "auth.x.ai", true)
}

/// Whether `raw` may be passed to `open_url` for Google / Microsoft authorize.
///
/// Accepts `https://accounts.google.com/…` and
/// `https://login.microsoftonline.com/…` only.
pub(crate) fn openable_authorize_url(raw: &str) -> bool {
    openable_https_host(raw, "accounts.google.com", false)
        || openable_https_host(raw, "login.microsoftonline.com", false)
}

fn openable_https_host(raw: &str, host: &str, allow_bare_host: bool) -> bool {
    if raw.chars().any(char::is_control) {
        return false;
    }
    let Ok(parsed) = url::Url::parse(raw) else {
        return false;
    };
    if parsed.scheme() != "https" || parsed.host_str() != Some(host) {
        return false;
    }
    if parsed.port().is_some() || !parsed.username().is_empty() || parsed.password().is_some() {
        return false;
    }
    let prefix = format!("https://{host}");
    if allow_bare_host && raw == prefix {
        return true;
    }
    raw.starts_with(&format!("{prefix}/"))
}

#[cfg(test)]
mod tests {
    use super::{openable_authorize_url, openable_verification_url};

    #[test]
    fn accepts_activate_urls() {
        assert!(openable_verification_url("https://auth.x.ai"), "host only");
        assert!(
            openable_verification_url("https://auth.x.ai/activate"),
            "activate"
        );
        assert!(
            openable_verification_url("https://auth.x.ai/activate?user_code=ABCD-EFGH"),
            "user code query"
        );
    }

    #[test]
    fn rejects_urls_outside_the_allowlist() {
        let rejected = [
            "http://auth.x.ai/activate",
            "https://auth.x.ai.evil.com/activate",
            "https://evil.example/activate",
            "https://user:pass@auth.x.ai/activate",
            "https://auth.x.ai:443/activate",
            "https://AUTH.X.AI/activate",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "",
            "https://auth.x.ai/activate\n",
        ];
        for raw in rejected {
            assert!(!openable_verification_url(raw), "{raw}");
        }
    }

    #[test]
    fn accepts_google_and_microsoft_authorize() {
        assert!(openable_authorize_url(
            "https://accounts.google.com/o/oauth2/v2/auth?client_id=x"
        ));
        assert!(openable_authorize_url(
            "https://login.microsoftonline.com/common/oauth2/v2.0/authorize?x=1"
        ));
        assert!(!openable_authorize_url("https://accounts.google.com"));
        assert!(!openable_authorize_url(
            "http://accounts.google.com/o/oauth2/v2/auth"
        ));
        assert!(!openable_authorize_url(
            "https://evil.example/accounts.google.com/o/oauth2/v2/auth"
        ));
    }
}
