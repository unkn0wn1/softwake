//! Gmail `users.messages.send` and Microsoft Graph `sendMail` builders.
//!
//! Network-free: URL strings, RFC 2822, and JSON only. The daemon posts them
//! under `live-http`. [`crate::LiveEmail`] SMTP send stays unwired.

use serde_json::Value;

const GMAIL_SEND_URL: &str = "https://gmail.googleapis.com/gmail/v1/users/me/messages/send";
const GRAPH_SEND_MAIL_URL: &str = "https://graph.microsoft.com/v1.0/me/sendMail";

/// Gmail `users.messages.send` URL. No query string.
#[must_use]
pub fn gmail_send_url() -> String {
    GMAIL_SEND_URL.to_owned()
}

/// RFC 2822 message for Gmail's `raw` field.
///
/// Header values are raw UTF-8 bytes, not RFC 2047; Gmail accepts that on this API.
///
/// `From` is included only when `from` trims to non-empty. CR and LF are removed
/// from `To`, `Subject`, and `From` so a value cannot inject another header.
/// The body is appended unchanged after the blank line.
#[must_use]
pub fn gmail_raw_rfc2822(to: &str, subject: &str, body: &str, from: Option<&str>) -> String {
    let mut message = String::new();
    if let Some(from) = from.map(str::trim).filter(|value| !value.is_empty()) {
        let from = strip_header_breaks(from);
        if !from.is_empty() {
            push_header(&mut message, "From", &from);
        }
    }
    push_header(&mut message, "To", &strip_header_breaks(to));
    push_header(&mut message, "Subject", &strip_header_breaks(subject));
    message.push_str("MIME-Version: 1.0\r\n");
    message.push_str("Content-Type: text/plain; charset=UTF-8\r\n");
    message.push_str("\r\n");
    message.push_str(body);
    message
}

/// JSON body `{"raw":"<base64url>"}` for Gmail send. The `raw` value has no `=` padding.
#[must_use]
pub fn gmail_send_body(to: &str, subject: &str, body: &str, from: Option<&str>) -> String {
    let raw = gmail_raw_rfc2822(to, subject, body, from);
    let encoded = encode_b64url(raw.as_bytes());
    serde_json::json!({ "raw": encoded }).to_string()
}

/// Read the Gmail send response `id`.
///
/// # Errors
///
/// Returns an error when the body is not a JSON object or `id` is missing or empty.
pub fn parse_gmail_send_id(response_body: &str) -> Result<String, String> {
    let value: Value = serde_json::from_str(response_body)
        .map_err(|_| "Gmail send response was not JSON".to_owned())?;
    value
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "Gmail send response missing id".to_owned())
}

/// Microsoft Graph `POST /me/sendMail` URL.
#[must_use]
pub fn graph_send_mail_url() -> String {
    GRAPH_SEND_MAIL_URL.to_owned()
}

/// JSON body for Graph `sendMail`. `contentType` is `Text`. One recipient. Saves to Sent.
///
/// Graph sends as the signed-in user, so this body has no `from`.
#[must_use]
pub fn graph_send_mail_body(to: &str, subject: &str, body: &str) -> String {
    serde_json::json!({
        "message": {
            "subject": subject,
            "body": { "contentType": "Text", "content": body },
            "toRecipients": [ { "emailAddress": { "address": to } } ]
        },
        "saveToSentItems": true
    })
    .to_string()
}

fn push_header(out: &mut String, name: &str, value: &str) {
    out.push_str(name);
    out.push_str(": ");
    out.push_str(value);
    out.push_str("\r\n");
}

fn strip_header_breaks(value: &str) -> String {
    value
        .chars()
        .filter(|ch| *ch != '\r' && *ch != '\n')
        .collect()
}

/// URL-safe base64 without padding (`+`/`/` → `-`/`_`, strip `=`).
fn encode_b64url(bytes: &[u8]) -> String {
    let mut out = String::new();
    let mut chunks = bytes.chunks_exact(3);
    for chunk in chunks.by_ref() {
        let bits = (u32::from(chunk[0]) << 16) | (u32::from(chunk[1]) << 8) | u32::from(chunk[2]);
        push_b64_digits(&mut out, bits, 4);
    }
    match chunks.remainder() {
        [first] => {
            let bits = u32::from(*first) << 16;
            push_b64_digits(&mut out, bits, 2);
        }
        [first, second] => {
            let bits = (u32::from(*first) << 16) | (u32::from(*second) << 8);
            push_b64_digits(&mut out, bits, 3);
        }
        _ => {}
    }
    out
}

fn push_b64_digits(out: &mut String, bits: u32, count: usize) {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    const SHIFTS: [u32; 4] = [18, 12, 6, 0];
    for shift in SHIFTS.into_iter().take(count) {
        let digit = (bits >> shift) & 63;
        let index = usize::try_from(digit).expect("base64 digit fits usize");
        out.push(char::from(TABLE[index]));
    }
}

#[cfg(test)]
mod tests {
    use super::{
        encode_b64url, gmail_raw_rfc2822, gmail_send_body, gmail_send_url, graph_send_mail_body,
        graph_send_mail_url, parse_gmail_send_id,
    };
    use crate::inbox_api::decode_b64url;
    use serde_json::Value;

    #[test]
    fn gmail_send_url_is_exact() {
        assert_eq!(
            gmail_send_url(),
            "https://gmail.googleapis.com/gmail/v1/users/me/messages/send"
        );
        assert!(!gmail_send_url().contains('?'));
    }

    #[test]
    fn rfc2822_omits_from_and_uses_crlf() {
        let raw = gmail_raw_rfc2822("ada@example.com", "hello", "a short note", None);
        assert_eq!(
            raw,
            "To: ada@example.com\r\nSubject: hello\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=UTF-8\r\n\r\na short note"
        );
        assert!(!raw.contains("From:"));
        assert!(raw.contains("\r\n"));
    }

    #[test]
    fn rfc2822_includes_from_when_present() {
        let raw = gmail_raw_rfc2822(
            "ada@example.com",
            "hello",
            "a short note",
            Some("me@example.com"),
        );
        assert!(raw.starts_with("From: me@example.com\r\n"));
        assert!(raw.contains("To: ada@example.com\r\n"));
    }

    #[test]
    fn rfc2822_strips_header_breaks_and_keeps_body() {
        let raw = gmail_raw_rfc2822(
            "ada@example.com\r\nBcc: evil@example.com",
            "hello\nX-Injected: yes",
            "line1\nline2 \"quoted\"",
            Some("me@example.com\r\nBcc: evil@example.com"),
        );
        assert!(raw.contains("To: ada@example.comBcc: evil@example.com\r\n"));
        assert!(raw.contains("Subject: helloX-Injected: yes\r\n"));
        assert!(raw.contains("From: me@example.comBcc: evil@example.com\r\n"));
        assert!(!raw.contains("\r\nBcc:"));
        assert!(!raw.contains("\r\nX-Injected:"));
        assert!(raw.ends_with("\r\n\r\nline1\nline2 \"quoted\""));
        assert!(!raw.contains("=?"));
    }

    #[test]
    fn rfc2822_omits_blank_from() {
        let raw = gmail_raw_rfc2822("ada@example.com", "hello", "body", Some(" \r\n"));
        assert!(!raw.contains("From:"));
    }

    #[test]
    fn rfc2822_subject_is_raw_utf8() {
        let raw = gmail_raw_rfc2822("a@b.c", "héllo", "body", None);
        assert!(raw.contains("Subject: héllo\r\n"));
        assert!(!raw.contains("=?"));
    }

    #[test]
    fn gmail_send_body_round_trips_raw_without_padding() {
        assert_eq!(encode_b64url(b"Hi"), "SGk");
        assert_eq!(encode_b64url(&[0xff, 0xef]), "_-8");
        let json = gmail_send_body("ada@example.com", "say \"hi\"", "Hi", None);
        let value: Value = serde_json::from_str(&json).expect("json");
        let raw = value["raw"].as_str().expect("raw");
        assert!(!raw.contains('='));
        assert!(!raw.contains('+'));
        assert!(!raw.contains('/'));
        let decoded = decode_b64url(raw).expect("decode");
        assert_eq!(
            decoded,
            gmail_raw_rfc2822("ada@example.com", "say \"hi\"", "Hi", None)
        );
        assert!(decoded.contains("Subject: say \"hi\"\r\n"));
        assert!(decoded.ends_with("\r\n\r\nHi"));
    }

    #[test]
    fn parse_gmail_send_id_reads_id() {
        assert_eq!(parse_gmail_send_id(r#"{"id":"18c0"}"#).expect("id"), "18c0");
        assert!(parse_gmail_send_id("{}").is_err());
        assert!(parse_gmail_send_id(r#"{"id":""}"#).is_err());
        assert!(parse_gmail_send_id(r#"{"id":1}"#).is_err());
        assert!(parse_gmail_send_id("nope").is_err());
    }

    #[test]
    fn graph_send_mail_url_is_exact() {
        assert_eq!(
            graph_send_mail_url(),
            "https://graph.microsoft.com/v1.0/me/sendMail"
        );
    }

    #[test]
    fn graph_send_mail_body_escapes_and_saves_sent() {
        let body = graph_send_mail_body("ada@example.com", "say \"hi\"", "a short note");
        let value: Value = serde_json::from_str(&body).expect("json");
        assert_eq!(value["saveToSentItems"].as_bool(), Some(true));
        assert_eq!(
            value["message"]["body"]["contentType"].as_str(),
            Some("Text")
        );
        assert_eq!(
            value["message"]["body"]["content"].as_str(),
            Some("a short note")
        );
        assert_eq!(
            value["message"]["toRecipients"][0]["emailAddress"]["address"].as_str(),
            Some("ada@example.com")
        );
        assert_eq!(value["message"]["subject"].as_str(), Some("say \"hi\""));
        assert!(value.get("from").is_none());
        assert!(value["message"].get("from").is_none());
    }
}
