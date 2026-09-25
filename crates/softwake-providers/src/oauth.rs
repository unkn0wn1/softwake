//! xAI device-code OAuth helpers.
//!
//! Body builders and parsers are network-free. Callers pass a [`crate::transport::Transport`]
//! when they need HTTP.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::constants::{
    XAI_OAUTH_CLIENT_ID, XAI_OAUTH_GRANT_DEVICE, XAI_OAUTH_SCOPE, XAI_REFRESH_SKEW_MS,
};

/// Tokens returned by the device or refresh grant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthTokenSet {
    /// Access token used as a bearer.
    pub access_token: String,
    /// Refresh token. Empty when a refresh response omitted it.
    pub refresh_token: String,
    /// Unix milliseconds when the access token expires.
    pub expires_at_ms: u64,
    /// Token type, usually `Bearer`.
    pub token_type: String,
}

/// Result of starting device authorization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCodeStart {
    /// Opaque device code. Keep this out of the window script when possible.
    pub device_code: String,
    /// Short user code shown in Settings.
    pub user_code: String,
    /// URL the operator opens (complete URI when the server sent one).
    pub verification_url: String,
    /// Suggested poll interval in seconds.
    pub interval_sec: u64,
    /// Unix milliseconds when the device code expires.
    pub expires_at_ms: u64,
}

/// One poll of the token endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevicePoll {
    /// Operator has not finished yet.
    Pending {
        /// Interval to wait before the next poll.
        interval_sec: u64,
    },
    /// Server asked for a slower poll.
    SlowDown {
        /// New interval.
        interval_sec: u64,
    },
    /// Sign-in finished.
    Tokens(OAuthTokenSet),
    /// Operator denied the request.
    Denied {
        /// Safe display message. No tokens.
        message: String,
    },
    /// Device code expired.
    Expired {
        /// Safe display message.
        message: String,
    },
}

/// `application/x-www-form-urlencoded` body for the device-code request.
#[must_use]
pub fn device_code_body() -> String {
    form(&[
        ("client_id", XAI_OAUTH_CLIENT_ID),
        ("scope", XAI_OAUTH_SCOPE),
    ])
}

/// Token poll body for `device_code`.
#[must_use]
pub fn token_poll_body(device_code: &str) -> String {
    form(&[
        ("grant_type", XAI_OAUTH_GRANT_DEVICE),
        ("device_code", device_code),
        ("client_id", XAI_OAUTH_CLIENT_ID),
    ])
}

/// Refresh-token grant body.
#[must_use]
pub fn refresh_body(refresh_token: &str) -> String {
    form(&[
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", XAI_OAUTH_CLIENT_ID),
    ])
}

/// Parse a successful device-code response.
#[must_use]
pub fn parse_device_start(payload: &Value, now_ms: u64) -> Option<DeviceCodeStart> {
    let device_code = string_field(payload, "device_code")?;
    let user_code = string_field(payload, "user_code")?;
    let verification_url = string_field(payload, "verification_uri_complete")
        .or_else(|| string_field(payload, "verification_uri"))?;
    let interval_sec = positive_u64(payload.get("interval")).unwrap_or(5);
    let expires_in = positive_u64(payload.get("expires_in")).unwrap_or(600);
    Some(DeviceCodeStart {
        device_code,
        user_code,
        verification_url,
        interval_sec,
        expires_at_ms: now_ms.saturating_add(expires_in.saturating_mul(1000)),
    })
}

/// Parse a token-endpoint poll response when the HTTP status was an error.
#[must_use]
pub fn parse_device_poll(payload: &Value, interval_sec: u64) -> DevicePoll {
    match string_field(payload, "error").as_deref() {
        Some("authorization_pending") => DevicePoll::Pending { interval_sec },
        Some("slow_down") => DevicePoll::SlowDown {
            interval_sec: interval_sec.saturating_add(5),
        },
        Some("expired_token") => DevicePoll::Expired {
            message: "The sign-in code expired. Start again.".to_owned(),
        },
        Some("access_denied") => DevicePoll::Denied {
            message: "xAI sign-in was denied.".to_owned(),
        },
        Some(_) => DevicePoll::Denied {
            message: string_field(payload, "error_description")
                .unwrap_or_else(|| "xAI sign-in failed.".to_owned()),
        },
        None => DevicePoll::Denied {
            message: "xAI sign-in returned an unexpected response.".to_owned(),
        },
    }
}

/// Parse a successful token response.
#[must_use]
pub fn parse_token_response(
    payload: &Value,
    now_ms: u64,
    require_refresh: bool,
) -> Option<OAuthTokenSet> {
    let access_token = string_field(payload, "access_token")?;
    let refresh_token = string_field(payload, "refresh_token").unwrap_or_default();
    if require_refresh && refresh_token.is_empty() {
        return None;
    }
    let expires_in = positive_u64(payload.get("expires_in")).unwrap_or(3600);
    Some(OAuthTokenSet {
        access_token,
        refresh_token,
        expires_at_ms: now_ms.saturating_add(expires_in.saturating_mul(1000)),
        token_type: string_field(payload, "token_type").unwrap_or_else(|| "Bearer".to_owned()),
    })
}

/// Whether the access token should be refreshed before use.
#[must_use]
pub fn access_needs_refresh(tokens: &OAuthTokenSet, now_ms: u64) -> bool {
    if tokens.expires_at_ms == 0 {
        return false;
    }
    now_ms >= tokens.expires_at_ms.saturating_sub(XAI_REFRESH_SKEW_MS)
}

/// Keep the previous refresh token when the refresh response omits one.
#[must_use]
pub fn merge_refresh(previous: &OAuthTokenSet, next: OAuthTokenSet) -> OAuthTokenSet {
    OAuthTokenSet {
        access_token: next.access_token,
        refresh_token: if next.refresh_token.is_empty() {
            previous.refresh_token.clone()
        } else {
            next.refresh_token
        },
        expires_at_ms: if next.expires_at_ms == 0 {
            previous.expires_at_ms
        } else {
            next.expires_at_ms
        },
        token_type: if next.token_type.is_empty() {
            previous.token_type.clone()
        } else {
            next.token_type
        },
    }
}

fn form(pairs: &[(&str, &str)]) -> String {
    let mut out = String::new();
    for (i, (key, value)) in pairs.iter().enumerate() {
        if i > 0 {
            out.push('&');
        }
        out.push_str(&encode(key));
        out.push('=');
        out.push_str(&encode(value));
    }
    out
}

fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(byte));
            }
            b' ' => out.push('+'),
            _ => {
                use std::fmt::Write as _;
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
}

fn string_field(payload: &Value, key: &str) -> Option<String> {
    let value = payload.get(key)?.as_str()?.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_owned())
    }
}

fn positive_u64(value: Option<&Value>) -> Option<u64> {
    let number = value?.as_u64()?;
    if number == 0 { None } else { Some(number) }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        DevicePoll, access_needs_refresh, device_code_body, merge_refresh, parse_device_poll,
        parse_device_start, parse_token_response, refresh_body, token_poll_body,
    };
    use crate::constants::XAI_OAUTH_CLIENT_ID;

    #[test]
    fn bodies_include_public_client_id() {
        assert!(device_code_body().contains(XAI_OAUTH_CLIENT_ID));
        assert!(token_poll_body("dev").contains("device_code=dev"));
        assert!(refresh_body("ref").contains("refresh_token=ref"));
    }

    #[test]
    fn parse_start_and_tokens() {
        let start = parse_device_start(
            &json!({
                "device_code": "dc",
                "user_code": "ABCD-EFGH",
                "verification_uri": "https://auth.x.ai/activate",
                "interval": 5,
                "expires_in": 600
            }),
            1_000,
        )
        .expect("start");
        assert_eq!(start.user_code, "ABCD-EFGH");
        assert_eq!(start.expires_at_ms, 601_000);

        let tokens = parse_token_response(
            &json!({
                "access_token": "a",
                "refresh_token": "r",
                "expires_in": 3600,
                "token_type": "Bearer"
            }),
            1_000,
            true,
        )
        .expect("tokens");
        assert_eq!(tokens.access_token, "a");
        assert!(access_needs_refresh(&tokens, tokens.expires_at_ms));
        assert!(!access_needs_refresh(&tokens, 1_000));
    }

    #[test]
    fn poll_pending_and_merge() {
        assert_eq!(
            parse_device_poll(&json!({"error": "authorization_pending"}), 5),
            DevicePoll::Pending { interval_sec: 5 }
        );
        let previous = parse_token_response(
            &json!({
                "access_token": "old",
                "refresh_token": "keep",
                "expires_in": 10
            }),
            0,
            true,
        )
        .expect("prev");
        let next = parse_token_response(
            &json!({
                "access_token": "new",
                "expires_in": 20
            }),
            0,
            false,
        )
        .expect("next");
        let merged = merge_refresh(&previous, next);
        assert_eq!(merged.access_token, "new");
        assert_eq!(merged.refresh_token, "keep");
    }
}
