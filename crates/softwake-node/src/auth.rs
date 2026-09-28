//! Bearer pairing-secret auth (constant-time compare).

use crate::http::HttpRequest;

/// Env var for the companion pairing secret.
pub const PAIRING_SECRET_ENV: &str = "SOFTWAKE_NODE_PAIRING_SECRET";

/// Constant-time equality for UTF-8 secrets (length mismatch → false).
#[must_use]
pub fn ct_eq(a: &str, b: &str) -> bool {
    let aa = a.as_bytes();
    let bb = b.as_bytes();
    if aa.len() != bb.len() {
        return false;
    }
    let mut diff = 0_u8;
    for (x, y) in aa.iter().zip(bb.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Extract bearer from Authorization or X-Softwake-Remote-Token.
#[must_use]
pub fn extract_token(req: &HttpRequest) -> Option<String> {
    let mut bearer = None;
    let mut alias = None;
    for (name, value) in &req.headers {
        let lower = name.to_ascii_lowercase();
        if lower == "authorization" {
            let trimmed = value.trim();
            if let Some(rest) = trimmed.strip_prefix("Bearer ") {
                bearer = Some(rest.trim().to_owned());
            } else if let Some(rest) = trimmed.strip_prefix("bearer ") {
                bearer = Some(rest.trim().to_owned());
            }
        } else if lower == "x-softwake-remote-token" {
            alias = Some(value.trim().to_owned());
        }
    }
    bearer.or(alias).filter(|s| !s.is_empty())
}

/// True when secret configured and token matches.
#[must_use]
pub fn authorized(req: &HttpRequest, expected: &str) -> bool {
    if expected.is_empty() {
        return false;
    }
    extract_token(req).is_some_and(|got| ct_eq(&got, expected))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::HttpRequest;

    #[test]
    fn ct_eq_rejects_length_mismatch() {
        assert!(!ct_eq("abc", "ab"));
        assert!(ct_eq("secret", "secret"));
        assert!(!ct_eq("secret", "Secret"));
    }

    #[test]
    fn extract_reads_bearer_or_alias() {
        let mut req = HttpRequest {
            method: "GET".into(),
            path: "/".into(),
            headers: vec![("Authorization".into(), "Bearer tok".into())],
            body: String::new(),
        };
        assert_eq!(extract_token(&req).as_deref(), Some("tok"));
        req.headers = vec![("X-Softwake-Remote-Token".into(), "alias".into())];
        assert_eq!(extract_token(&req).as_deref(), Some("alias"));
    }
}
