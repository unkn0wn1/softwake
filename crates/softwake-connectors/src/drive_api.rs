//! Google Drive and Microsoft Graph `OneDrive` (user drive) URL builders + parsers.

use serde_json::Value;

/// Default page size.
pub const DEFAULT_DRIVE_MAX: u32 = 20;
/// Hard cap.
pub const MAX_DRIVE_MAX: u32 = 50;
/// Max bytes when reading a cheap text body.
pub const MAX_DRIVE_TEXT_BYTES: usize = 32 * 1024;

/// One Drive / `OneDrive` file row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveDriveFile {
    /// Provider file id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// MIME type when known.
    pub mime_type: String,
    /// Modified time (API spelling).
    pub modified: String,
    /// Size as decimal string when known.
    pub size: String,
    /// Web view link when known.
    pub web_link: String,
    /// Optional text body (get + `read_text`).
    pub text: String,
}

/// Clamp max files.
#[must_use]
pub fn clamp_drive_max(requested: Option<u32>) -> u32 {
    match requested {
        None | Some(0) => DEFAULT_DRIVE_MAX,
        Some(n) => n.min(MAX_DRIVE_MAX),
    }
}

/// Google Drive files.list (requires `drive.readonly` for full user Drive visibility).
#[must_use]
pub fn google_drive_list_url(max: u32, query: Option<&str>) -> String {
    let fields = "files(id,name,mimeType,modifiedTime,size,webViewLink)";
    let mut url = format!(
        "https://www.googleapis.com/drive/v3/files?pageSize={max}&fields={}&spaces=drive",
        encode_q(fields)
    );
    if let Some(q) = query.map(str::trim).filter(|s| !s.is_empty()) {
        // Name contains when the query has no Drive operators.
        let q_final = if q.contains(':') {
            q.to_owned()
        } else {
            format!("name contains '{}'", q.replace('\'', "\\'"))
        };
        url.push_str("&q=");
        url.push_str(&encode_q(&q_final));
    }
    url
}

/// Google Drive files.get metadata.
#[must_use]
pub fn google_drive_get_url(id: &str) -> String {
    format!(
        "https://www.googleapis.com/drive/v3/files/{}?fields=id,name,mimeType,modifiedTime,size,webViewLink",
        encode_path(id)
    )
}

/// Google Docs export as plain text.
#[must_use]
pub fn google_drive_export_text_url(id: &str) -> String {
    format!(
        "https://www.googleapis.com/drive/v3/files/{}/export?mimeType=text%2Fplain",
        encode_path(id)
    )
}

/// Google Drive media download (`alt=media`) for text/* files.
#[must_use]
pub fn google_drive_media_url(id: &str) -> String {
    format!(
        "https://www.googleapis.com/drive/v3/files/{}?alt=media",
        encode_path(id)
    )
}

/// Microsoft Graph user drive root children list.
#[must_use]
pub fn graph_drive_root_children_url(max: u32) -> String {
    format!(
        "https://graph.microsoft.com/v1.0/me/drive/root/children?$top={max}&$select=id,name,file,size,lastModifiedDateTime,webUrl"
    )
}

/// Microsoft Graph drive search under user drive root.
#[must_use]
pub fn graph_drive_root_search_url(query: &str, max: u32) -> String {
    format!(
        "https://graph.microsoft.com/v1.0/me/drive/root/search(q='{}')?$top={max}&$select=id,name,file,size,lastModifiedDateTime,webUrl",
        encode_q_single_quoted(query)
    )
}

/// Microsoft Graph drive item get.
#[must_use]
pub fn graph_drive_item_url(id: &str) -> String {
    format!(
        "https://graph.microsoft.com/v1.0/me/drive/items/{}?$select=id,name,file,size,lastModifiedDateTime,webUrl",
        encode_path(id)
    )
}

/// Microsoft Graph drive item content (media).
#[must_use]
pub fn graph_drive_content_url(id: &str) -> String {
    format!(
        "https://graph.microsoft.com/v1.0/me/drive/items/{}/content",
        encode_path(id)
    )
}

/// Whether a MIME type is cheap to read as UTF-8 text.
#[must_use]
pub fn is_cheap_text_mime(mime: &str) -> bool {
    let m = mime.trim().to_ascii_lowercase();
    m.starts_with("text/")
        || m == "application/json"
        || m == "application/xml"
        || m == "application/javascript"
        || m == "application/vnd.google-apps.document"
}

/// Parse Google Drive files.list JSON.
///
/// # Errors
///
/// Invalid JSON.
pub fn parse_google_drive_list(body: &str) -> Result<Vec<LiveDriveFile>, String> {
    let value: Value =
        serde_json::from_str(body).map_err(|_| "Google Drive list was not JSON".to_owned())?;
    let Some(items) = value.get("files").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for item in items {
        if let Ok(file) = parse_google_drive_value(item) {
            out.push(file);
        }
    }
    Ok(out)
}

/// Parse one Google Drive file metadata JSON.
///
/// # Errors
///
/// Invalid JSON or missing id.
pub fn parse_google_drive_file(body: &str) -> Result<LiveDriveFile, String> {
    let value: Value =
        serde_json::from_str(body).map_err(|_| "Google Drive file was not JSON".to_owned())?;
    parse_google_drive_value(&value)
}

fn parse_google_drive_value(value: &Value) -> Result<LiveDriveFile, String> {
    let id = text(value, "id").ok_or_else(|| "Google Drive file missing id".to_owned())?;
    Ok(LiveDriveFile {
        id,
        name: text(value, "name").unwrap_or_default(),
        mime_type: text(value, "mimeType").unwrap_or_default(),
        modified: text(value, "modifiedTime").unwrap_or_default(),
        size: value
            .get("size")
            .and_then(|v| match v {
                Value::String(s) => Some(s.clone()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
            .unwrap_or_default(),
        web_link: text(value, "webViewLink").unwrap_or_default(),
        text: String::new(),
    })
}

/// Parse Graph drive children / search / get JSON.
///
/// # Errors
///
/// Invalid JSON.
pub fn parse_graph_drive_list(body: &str) -> Result<Vec<LiveDriveFile>, String> {
    let value: Value =
        serde_json::from_str(body).map_err(|_| "Graph Drive list was not JSON".to_owned())?;
    if let Some(items) = value.get("value").and_then(Value::as_array) {
        let mut out = Vec::new();
        for item in items {
            if let Ok(file) = parse_graph_drive_value(item) {
                out.push(file);
            }
        }
        return Ok(out);
    }
    Ok(vec![parse_graph_drive_value(&value)?])
}

/// Parse one Graph drive item.
///
/// # Errors
///
/// Invalid JSON or missing id.
pub fn parse_graph_drive_file(body: &str) -> Result<LiveDriveFile, String> {
    let value: Value =
        serde_json::from_str(body).map_err(|_| "Graph Drive file was not JSON".to_owned())?;
    parse_graph_drive_value(&value)
}

fn parse_graph_drive_value(value: &Value) -> Result<LiveDriveFile, String> {
    let id = text(value, "id").ok_or_else(|| "Graph Drive file missing id".to_owned())?;
    let mime = value
        .pointer("/file/mimeType")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    Ok(LiveDriveFile {
        id,
        name: text(value, "name").unwrap_or_default(),
        mime_type: mime,
        modified: text(value, "lastModifiedDateTime").unwrap_or_default(),
        size: value
            .get("size")
            .and_then(|v| match v {
                Value::Number(n) => Some(n.to_string()),
                Value::String(s) => Some(s.clone()),
                _ => None,
            })
            .unwrap_or_default(),
        web_link: text(value, "webUrl").unwrap_or_default(),
        text: String::new(),
    })
}

/// List detail.
#[must_use]
pub fn format_drive_list(provider: &str, files: &[LiveDriveFile]) -> String {
    if files.is_empty() {
        return format!("{provider}: (no files; Google drive.readonly / Microsoft Files.Read)");
    }
    let mut lines = vec![format!("{provider}: {} file(s)", files.len())];
    for (i, file) in files.iter().enumerate() {
        lines.push(format!(
            "{}. id={} name={} mime={} modified={} size={}",
            i + 1,
            file.id,
            file.name,
            file.mime_type,
            file.modified,
            file.size
        ));
    }
    lines.join("\n")
}

/// One-file detail.
#[must_use]
pub fn format_drive_file(provider: &str, file: &LiveDriveFile) -> String {
    let mut out = format!(
        "{provider}: id={}\nname={}\nmime={}\nmodified={}\nsize={}\nlink={}",
        file.id, file.name, file.mime_type, file.modified, file.size, file.web_link
    );
    if !file.text.is_empty() {
        out.push_str("\n\n");
        out.push_str(&truncate(&file.text, MAX_DRIVE_TEXT_BYTES));
    }
    out
}

/// Truncate downloaded text to [`MAX_DRIVE_TEXT_BYTES`] chars (approx).
#[must_use]
pub fn truncate_drive_text(text: &str) -> String {
    truncate(text, MAX_DRIVE_TEXT_BYTES)
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

fn encode_q(value: &str) -> String {
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

fn encode_q_single_quoted(value: &str) -> String {
    encode_q(&value.replace('\'', "''"))
}

fn encode_path(value: &str) -> String {
    encode_q(value)
}

fn hex(nibble: u8) -> char {
    char::from(b"0123456789ABCDEF"[nibble as usize])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_url_and_parse() {
        let url = google_drive_list_url(5, Some("notes"));
        assert!(url.contains("pageSize=5"));
        assert!(url.contains("name%20contains"));
        let body = r#"{
          "files":[{
            "id":"f1",
            "name":"notes.txt",
            "mimeType":"text/plain",
            "modifiedTime":"2026-09-27T00:00:00.000Z",
            "size":"12",
            "webViewLink":"https://drive.google.com/file/d/f1"
          }]
        }"#;
        let files = parse_google_drive_list(body).expect("parse");
        assert_eq!(files[0].name, "notes.txt");
        assert!(is_cheap_text_mime("text/plain"));
        assert!(is_cheap_text_mime("application/vnd.google-apps.document"));
        assert!(!is_cheap_text_mime("application/pdf"));
    }

    #[test]
    fn graph_drive_root_urls() {
        let children = graph_drive_root_children_url(10);
        assert!(children.contains("/me/drive/root/children"));
        assert!(!children.contains("approot"));
        let search = graph_drive_root_search_url("notes", 5);
        assert!(search.contains("/me/drive/root/search(q='notes')"));
        assert!(!search.contains("approot"));
    }

    #[test]
    fn graph_drive_root_parse() {
        let body = r#"{
          "value":[{
            "id":"i1",
            "name":"clip.txt",
            "file":{"mimeType":"text/plain"},
            "size":4,
            "lastModifiedDateTime":"2026-09-27T01:00:00Z",
            "webUrl":"https://example"
          }]
        }"#;
        let files = parse_graph_drive_list(body).expect("parse");
        assert_eq!(files[0].mime_type, "text/plain");
        assert!(format_drive_list("graph", &files).contains("clip.txt"));
    }
}
