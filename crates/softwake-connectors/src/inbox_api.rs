//! Gmail and Microsoft Graph inbox URL builders + JSON parsers (network-free).

use serde_json::Value;

/// Default page size for list/search.
pub const DEFAULT_INBOX_MAX: u32 = 10;
/// Hard cap so a tool call cannot request a huge dump.
pub const MAX_INBOX_MAX: u32 = 50;

/// One inbox row for tool detail text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxMessage {
    /// Provider message id (Gmail or Graph).
    pub id: String,
    /// Thread id when the API provided one.
    pub thread_id: String,
    /// Subject when known.
    pub subject: String,
    /// From header / sender display.
    pub from: String,
    /// Date / received time as returned (not re-parsed).
    pub date: String,
    /// Snippet or body preview.
    pub snippet: String,
    /// Full body text when fetched (get only).
    pub body: String,
}

/// Clamp `requested` into `1..=MAX_INBOX_MAX`, defaulting empty/zero to [`DEFAULT_INBOX_MAX`].
#[must_use]
pub fn clamp_inbox_max(requested: Option<u32>) -> u32 {
    match requested {
        None | Some(0) => DEFAULT_INBOX_MAX,
        Some(n) => n.min(MAX_INBOX_MAX),
    }
}

/// Gmail messages.list URL.
#[must_use]
pub fn gmail_list_url(max: u32, query: Option<&str>) -> String {
    let mut url =
        format!("https://gmail.googleapis.com/gmail/v1/users/me/messages?maxResults={max}");
    if let Some(q) = query.map(str::trim).filter(|s| !s.is_empty()) {
        url.push_str("&q=");
        url.push_str(&encode_query(q));
    }
    url
}

/// Gmail messages.get URL (`format=full`).
#[must_use]
pub fn gmail_get_url(id: &str) -> String {
    format!(
        "https://gmail.googleapis.com/gmail/v1/users/me/messages/{}?format=full",
        encode_path(id)
    )
}

/// Microsoft Graph messages list / search URL.
#[must_use]
pub fn graph_list_url(max: u32, search: Option<&str>) -> String {
    let mut url = format!(
        "https://graph.microsoft.com/v1.0/me/messages?$top={max}&$select=id,conversationId,subject,from,receivedDateTime,bodyPreview,body&$orderby=receivedDateTime%20desc"
    );
    if let Some(q) = search.map(str::trim).filter(|s| !s.is_empty()) {
        // Graph $search requires ConsistencyLevel: eventual (caller sets header).
        url.push_str("&$search=");
        url.push_str(&encode_query(&format!("\"{q}\"")));
    }
    url
}

/// Microsoft Graph message get URL.
#[must_use]
pub fn graph_get_url(id: &str) -> String {
    format!(
        "https://graph.microsoft.com/v1.0/me/messages/{}?$select=id,conversationId,subject,from,receivedDateTime,bodyPreview,body",
        encode_path(id)
    )
}

/// Parse Gmail messages.list JSON into id stubs (subject filled by follow-up or left empty).
///
/// # Errors
///
/// Invalid JSON.
pub fn parse_gmail_list(body: &str) -> Result<Vec<InboxMessage>, String> {
    let value: Value =
        serde_json::from_str(body).map_err(|_| "Gmail list was not JSON".to_owned())?;
    let Some(items) = value.get("messages").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for item in items {
        let id = text(item, "id").unwrap_or_default();
        if id.is_empty() {
            continue;
        }
        out.push(InboxMessage {
            id,
            thread_id: text(item, "threadId").unwrap_or_default(),
            subject: String::new(),
            from: String::new(),
            date: String::new(),
            snippet: String::new(),
            body: String::new(),
        });
    }
    Ok(out)
}

/// Parse one Gmail messages.get JSON body.
///
/// # Errors
///
/// Invalid JSON or missing id.
pub fn parse_gmail_message(body: &str) -> Result<InboxMessage, String> {
    let value: Value =
        serde_json::from_str(body).map_err(|_| "Gmail message was not JSON".to_owned())?;
    let id = text(&value, "id").ok_or_else(|| "Gmail message missing id".to_owned())?;
    let headers = value
        .pointer("/payload/headers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let subject = header_value(&headers, "Subject").unwrap_or_default();
    let from = header_value(&headers, "From").unwrap_or_default();
    let date = header_value(&headers, "Date").unwrap_or_default();
    let snippet = text(&value, "snippet").unwrap_or_default();
    let body_text = extract_gmail_body(value.get("payload"));
    Ok(InboxMessage {
        id,
        thread_id: text(&value, "threadId").unwrap_or_default(),
        subject,
        from,
        date,
        snippet,
        body: body_text,
    })
}

/// Parse Graph messages list JSON.
///
/// # Errors
///
/// Invalid JSON.
pub fn parse_graph_list(body: &str) -> Result<Vec<InboxMessage>, String> {
    let value: Value =
        serde_json::from_str(body).map_err(|_| "Graph mail list was not JSON".to_owned())?;
    let Some(items) = value.get("value").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for item in items {
        if let Ok(msg) = parse_graph_message_value(item) {
            out.push(msg);
        }
    }
    Ok(out)
}

/// Parse one Graph message JSON body.
///
/// # Errors
///
/// Invalid JSON or missing id.
pub fn parse_graph_message(body: &str) -> Result<InboxMessage, String> {
    let value: Value =
        serde_json::from_str(body).map_err(|_| "Graph mail message was not JSON".to_owned())?;
    parse_graph_message_value(&value)
}

fn parse_graph_message_value(value: &Value) -> Result<InboxMessage, String> {
    let id = text(value, "id").ok_or_else(|| "Graph message missing id".to_owned())?;
    let from = value
        .pointer("/from/emailAddress/address")
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .pointer("/from/emailAddress/name")
                .and_then(Value::as_str)
        })
        .unwrap_or("")
        .to_owned();
    let body = value
        .pointer("/body/content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    Ok(InboxMessage {
        id,
        thread_id: text(value, "conversationId").unwrap_or_default(),
        subject: text(value, "subject").unwrap_or_default(),
        from,
        date: text(value, "receivedDateTime").unwrap_or_default(),
        snippet: text(value, "bodyPreview").unwrap_or_default(),
        body,
    })
}

/// Operator-facing multi-line detail for a list/search result.
#[must_use]
pub fn format_inbox_list(provider: &str, messages: &[InboxMessage]) -> String {
    if messages.is_empty() {
        return format!("{provider}: (no messages)");
    }
    let mut lines = vec![format!("{provider}: {} message(s)", messages.len())];
    for (i, msg) in messages.iter().enumerate() {
        let subject = if msg.subject.is_empty() {
            "(no subject)"
        } else {
            msg.subject.as_str()
        };
        let from = if msg.from.is_empty() {
            "?"
        } else {
            msg.from.as_str()
        };
        let date = if msg.date.is_empty() {
            ""
        } else {
            msg.date.as_str()
        };
        let snippet = truncate(&msg.snippet, 120);
        lines.push(format!(
            "{}. id={} from={} date={} subject={} | {snippet}",
            i + 1,
            msg.id,
            from,
            date,
            subject
        ));
    }
    lines.join("\n")
}

/// Operator-facing detail for one message.
#[must_use]
pub fn format_inbox_message(provider: &str, msg: &InboxMessage) -> String {
    let body = if msg.body.trim().is_empty() {
        msg.snippet.clone()
    } else {
        truncate(&msg.body, 4000)
    };
    format!(
        "{provider}: id={}\nthread={}\nfrom={}\ndate={}\nsubject={}\n\n{body}",
        msg.id, msg.thread_id, msg.from, msg.date, msg.subject
    )
}

fn extract_gmail_body(payload: Option<&Value>) -> String {
    let Some(payload) = payload else {
        return String::new();
    };
    if let Some(data) = payload
        .pointer("/body/data")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        if let Some(decoded) = decode_b64url(data) {
            return decoded;
        }
    }
    if let Some(parts) = payload.get("parts").and_then(Value::as_array) {
        // Prefer text/plain.
        for part in parts {
            let mime = text(part, "mimeType").unwrap_or_default();
            if mime == "text/plain" {
                if let Some(data) = part
                    .pointer("/body/data")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                {
                    if let Some(decoded) = decode_b64url(data) {
                        return decoded;
                    }
                }
            }
        }
        for part in parts {
            let nested = extract_gmail_body(Some(part));
            if !nested.is_empty() {
                return nested;
            }
        }
    }
    String::new()
}

fn decode_b64url(data: &str) -> Option<String> {
    use std::collections::HashMap;
    // Minimal URL-safe base64 decode without extra deps.
    let remapped = data.replace('-', "+").replace('_', "/");
    let pad = match remapped.len() % 4 {
        0 => "",
        2 => "==",
        3 => "=",
        _ => return None,
    };
    let full = format!("{remapped}{pad}");
    let table: HashMap<u8, u8> = {
        let mut m = HashMap::new();
        for (i, c) in b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
            .iter()
            .enumerate()
        {
            m.insert(*c, u8::try_from(i).expect("b64 alphabet fits u8"));
        }
        m
    };
    let mut bytes = Vec::new();
    let chars: Vec<u8> = full.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    for chunk in chars.chunks(4) {
        if chunk.len() < 2 {
            return None;
        }
        let a = *table.get(&chunk[0])?;
        let b = *table.get(&chunk[1])?;
        let c = chunk.get(2).and_then(|x| table.get(x).copied());
        let d = chunk.get(3).and_then(|x| table.get(x).copied());
        bytes.push((a << 2) | (b >> 4));
        if let Some(c) = c {
            if chunk.get(2).copied() != Some(b'=') {
                bytes.push((b << 4) | (c >> 2));
            }
            if let Some(d) = d {
                if chunk.get(3).copied() != Some(b'=') {
                    bytes.push((c << 6) | d);
                }
            }
        }
    }
    String::from_utf8(bytes).ok()
}

fn header_value(headers: &[Value], name: &str) -> Option<String> {
    for header in headers {
        let n = text(header, "name")?;
        if n.eq_ignore_ascii_case(name) {
            return text(header, "value");
        }
    }
    None
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn truncate(text: &str, max: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max {
        return trimmed.to_owned();
    }
    let cut: String = trimmed.chars().take(max).collect();
    format!("{cut}…")
}

fn encode_query(value: &str) -> String {
    let mut out = String::new();
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => {
                out.push('%');
                out.push(hex(b >> 4));
                out.push(hex(b & 0xf));
            }
        }
    }
    out
}

fn encode_path(value: &str) -> String {
    let mut out = String::new();
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push('%');
                out.push(hex(b >> 4));
                out.push(hex(b & 0xf));
            }
        }
    }
    out
}

fn hex(nibble: u8) -> char {
    char::from(b"0123456789ABCDEF"[nibble as usize])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_and_urls() {
        assert_eq!(clamp_inbox_max(None), 10);
        assert_eq!(clamp_inbox_max(Some(0)), 10);
        assert_eq!(clamp_inbox_max(Some(100)), 50);
        assert!(gmail_list_url(5, Some("from:ada")).contains("maxResults=5"));
        assert!(gmail_list_url(5, Some("from:ada")).contains("q=from%3Aada"));
        assert!(graph_list_url(3, Some("invoice")).contains("$search="));
        assert!(gmail_get_url("abc+1").contains("messages/abc%2B1"));
    }

    #[test]
    fn parse_gmail_list_and_message() {
        let list = r#"{"messages":[{"id":"m1","threadId":"t1"}]}"#;
        let rows = parse_gmail_list(list).expect("list");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "m1");

        // "Hi" in url-safe base64: SGk=
        let msg = r#"{
            "id":"m1","threadId":"t1","snippet":"snip",
            "payload":{
              "headers":[
                {"name":"Subject","value":"Hello"},
                {"name":"From","value":"ada@example.com"},
                {"name":"Date","value":"Sun, 27 Sep 2026"}
              ],
              "body":{"data":"SGk"}
            }
        }"#;
        let parsed = parse_gmail_message(msg).expect("msg");
        assert_eq!(parsed.subject, "Hello");
        assert_eq!(parsed.from, "ada@example.com");
        assert_eq!(parsed.body, "Hi");
        let detail = format_inbox_message("gmail", &parsed);
        assert!(detail.contains("subject=Hello"));
    }

    #[test]
    fn graph_list_parses() {
        let body = r#"{
          "value":[{
            "id":"g1",
            "conversationId":"c1",
            "subject":"Hi",
            "from":{"emailAddress":{"address":"bob@example.com"}},
            "receivedDateTime":"2026-09-27T01:00:00Z",
            "bodyPreview":"yo",
            "body":{"content":"yo body"}
          }]
        }"#;
        let rows = parse_graph_list(body).expect("list");
        assert_eq!(rows[0].from, "bob@example.com");
        assert!(format_inbox_list("graph", &rows).contains("bob@example.com"));
    }
}
