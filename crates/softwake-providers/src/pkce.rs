//! PKCE (RFC 7636) helpers for authorization-code OAuth.
//!
//! Network-free. Callers build authorize URLs and token bodies separately.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

const PKCE_TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";

/// Length of the code verifier in characters (43–128 per RFC 7636).
pub const VERIFIER_LEN: usize = 64;

/// Generate a cryptographically random code verifier.
///
/// # Errors
///
/// Returns an error string when the OS RNG fails.
pub fn code_verifier() -> Result<String, String> {
    let mut bytes = [0u8; VERIFIER_LEN];
    getrandom::getrandom(&mut bytes).map_err(|_| "could not read random bytes".to_owned())?;
    // Map to unreserved characters: ALPHA / DIGIT / "-" / "." / "_" / "~"
    let mut out = String::with_capacity(VERIFIER_LEN);
    for byte in bytes {
        out.push(PKCE_TABLE[(byte as usize) % PKCE_TABLE.len()] as char);
    }
    Ok(out)
}

/// Generate a random OAuth `state` parameter (same alphabet as the verifier).
///
/// # Errors
///
/// Returns an error string when the OS RNG fails.
pub fn oauth_state() -> Result<String, String> {
    code_verifier().map(|v| v[..32].to_owned())
}

/// S256 code challenge for `code_verifier`.
#[must_use]
pub fn code_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

#[cfg(test)]
mod tests {
    use super::{VERIFIER_LEN, code_challenge, code_verifier, oauth_state};

    #[test]
    fn verifier_length_and_alphabet() {
        let verifier = code_verifier().expect("rng");
        assert_eq!(verifier.len(), VERIFIER_LEN);
        assert!(
            verifier
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '~')),
            "{verifier}"
        );
    }

    #[test]
    fn challenge_is_stable_for_known_vector() {
        // RFC 7636 appendix B
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            code_challenge(verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn state_is_shorter_than_verifier() {
        let state = oauth_state().expect("rng");
        assert_eq!(state.len(), 32);
    }
}
