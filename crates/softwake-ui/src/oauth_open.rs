//! Predicate for the xAI device-code verification URL.
//!
//! Rust `open_url` does not consult the capability scope. The plugin click
//! script does. This predicate and `opener:allow-open-url` name the same set.

/// Shown when the browser was not opened. OS error text stays on stderr.
#[cfg(feature = "live-http")]
pub(crate) const BROWSER_NOTE: &str = "Could not open the browser. Use the address below.";

/// Whether `raw` may be passed to `open_url` or used as an anchor `href`.
///
/// Accepts `https://auth.x.ai` and `https://auth.x.ai/…` only: https, that
/// host in lowercase, no userinfo, no port, and no control characters.
#[cfg_attr(
    not(feature = "live-http"),
    allow(
        dead_code,
        reason = "called from provider_oauth_start when live-http is enabled; unit tests always run it"
    )
)]
pub(crate) fn openable_verification_url(raw: &str) -> bool {
    if raw.chars().any(char::is_control) {
        return false;
    }
    let Ok(parsed) = url::Url::parse(raw) else {
        return false;
    };
    if parsed.scheme() != "https" || parsed.host_str() != Some("auth.x.ai") {
        return false;
    }
    if parsed.port().is_some() || !parsed.username().is_empty() || parsed.password().is_some() {
        return false;
    }
    raw == "https://auth.x.ai" || raw.starts_with("https://auth.x.ai/")
}

#[cfg(test)]
mod tests {
    use super::openable_verification_url;

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
}
