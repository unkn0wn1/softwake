//! Live inbox / calendar / Drive tools (Gmail, Google Calendar, Drive, MS Graph).
//!
//! Requires daemon `live-http` and a connected Email OAuth account. Prefer Google
//! when both are connected. See ADR-0030.

#![cfg_attr(not(feature = "live-http"), allow(dead_code))]

use softwake_connectors::{
    CALENDAR, CALENDAR_GET, CALENDAR_LIST, DRIVE, DRIVE_GET, DRIVE_LIST, DRIVE_SEARCH, EMAIL,
    EMAIL_GET, EMAIL_LIST, EMAIL_SEARCH, clamp_calendar_days, clamp_calendar_max, clamp_drive_max,
    clamp_inbox_max, format_calendar_event, format_calendar_list, format_drive_file,
    format_drive_list, format_inbox_list, format_inbox_message, gmail_get_url, gmail_list_url,
    google_drive_export_text_url, google_drive_get_url, google_drive_list_url,
    google_drive_media_url, google_event_get_url, google_events_url, graph_approot_children_url,
    graph_approot_search_url, graph_calendar_view_url, graph_drive_content_url,
    graph_drive_item_url, graph_event_get_url, graph_get_url, graph_list_url, is_cheap_text_mime,
    parse_gmail_list, parse_gmail_message, parse_google_drive_file, parse_google_drive_list,
    parse_google_event, parse_google_events, parse_graph_drive_file, parse_graph_drive_list,
    parse_graph_event, parse_graph_events, parse_graph_list, parse_graph_message,
    truncate_drive_text,
};
#[cfg(feature = "live-http")]
use softwake_providers::ensure_fresh_account;
use softwake_providers::{
    AccountConnection, AccountProvider, open_store, resolve_secrets_file, update_bag,
};
use softwake_tools::now_ms;
use softwake_tools::{
    CalendarGetArgs, CalendarListArgs, DriveGetArgs, DriveListArgs, DriveSearchArgs, EmailGetArgs,
    EmailListArgs, EmailSearchArgs,
};

const NO_ACCOUNT: &str = "no Google or Microsoft account connected (Settings → Email → Connect)";
#[allow(dead_code)]
const LIVE_REQUIRED: &str = "live-http is required for inbox/calendar/Drive tools in this build";

/// Run `email_list` against the connected account.
pub(crate) fn run_email_list(args: &EmailListArgs) -> Result<String, String> {
    let max = clamp_inbox_max(args.max_results);
    with_account(|provider, bearer| match provider {
        AccountProvider::Google => {
            let url = gmail_list_url(max, None);
            let body = get_json(&url, bearer, None)?;
            let stubs = parse_gmail_list(&body)?;
            // Enrich up to max with metadata gets (cheap for small pages).
            let mut messages = Vec::new();
            for stub in stubs.into_iter().take(max as usize) {
                let get_url = gmail_get_url(&stub.id);
                if let Ok(raw) = get_json(&get_url, bearer, None) {
                    if let Ok(msg) = parse_gmail_message(&raw) {
                        messages.push(msg);
                        continue;
                    }
                }
                messages.push(stub);
            }
            Ok(format_inbox_list("gmail", &messages))
        }
        AccountProvider::Microsoft => {
            let url = graph_list_url(max, None);
            let body = get_json(&url, bearer, None)?;
            let messages = parse_graph_list(&body)?;
            Ok(format_inbox_list("graph", &messages))
        }
    })
}

/// Run `email_search`.
pub(crate) fn run_email_search(args: &EmailSearchArgs) -> Result<String, String> {
    let max = clamp_inbox_max(args.max_results);
    let query = args.query.trim();
    if query.is_empty() {
        return Err("email_search needs query".to_owned());
    }
    with_account(|provider, bearer| match provider {
        AccountProvider::Google => {
            let url = gmail_list_url(max, Some(query));
            let body = get_json(&url, bearer, None)?;
            let stubs = parse_gmail_list(&body)?;
            let mut messages = Vec::new();
            for stub in stubs.into_iter().take(max as usize) {
                let get_url = gmail_get_url(&stub.id);
                if let Ok(raw) = get_json(&get_url, bearer, None) {
                    if let Ok(msg) = parse_gmail_message(&raw) {
                        messages.push(msg);
                        continue;
                    }
                }
                messages.push(stub);
            }
            Ok(format_inbox_list("gmail", &messages))
        }
        AccountProvider::Microsoft => {
            let url = graph_list_url(max, Some(query));
            let body = get_json(&url, bearer, Some(&[("ConsistencyLevel", "eventual")]))?;
            let messages = parse_graph_list(&body)?;
            Ok(format_inbox_list("graph", &messages))
        }
    })
}

/// Run `email_get`.
pub(crate) fn run_email_get(args: &EmailGetArgs) -> Result<String, String> {
    let id = args.id.trim();
    if id.is_empty() {
        return Err("email_get needs id".to_owned());
    }
    with_account(|provider, bearer| match provider {
        AccountProvider::Google => {
            let url = gmail_get_url(id);
            let body = get_json(&url, bearer, None)?;
            let msg = parse_gmail_message(&body)?;
            Ok(format_inbox_message("gmail", &msg))
        }
        AccountProvider::Microsoft => {
            let url = graph_get_url(id);
            let body = get_json(&url, bearer, None)?;
            let msg = parse_graph_message(&body)?;
            Ok(format_inbox_message("graph", &msg))
        }
    })
}

/// Run `calendar_list`.
pub(crate) fn run_calendar_list(args: &CalendarListArgs) -> Result<String, String> {
    let days = clamp_calendar_days(args.days);
    let max = clamp_calendar_max(args.max_results);
    let now = chrono_now();
    let end = now + i64::from(days) * 86_400;
    let time_min = format_rfc3339(now);
    let time_max = format_rfc3339(end);
    with_account(|provider, bearer| match provider {
        AccountProvider::Google => {
            let url = google_events_url(&time_min, &time_max, max);
            let body = get_json(&url, bearer, None)?;
            let events = parse_google_events(&body)?;
            Ok(format_calendar_list("google", &events))
        }
        AccountProvider::Microsoft => {
            let url = graph_calendar_view_url(&time_min, &time_max, max);
            let body = get_json(
                &url,
                bearer,
                Some(&[("Prefer", "outlook.timezone=\"UTC\"")]),
            )?;
            let events = parse_graph_events(&body)?;
            Ok(format_calendar_list("graph", &events))
        }
    })
}

/// Run `calendar_get`.
pub(crate) fn run_calendar_get(args: &CalendarGetArgs) -> Result<String, String> {
    let id = args.id.trim();
    if id.is_empty() {
        return Err("calendar_get needs id".to_owned());
    }
    with_account(|provider, bearer| match provider {
        AccountProvider::Google => {
            let url = google_event_get_url(id);
            let body = get_json(&url, bearer, None)?;
            let event = parse_google_event(&body)?;
            Ok(format_calendar_event("google", &event))
        }
        AccountProvider::Microsoft => {
            let url = graph_event_get_url(id);
            let body = get_json(&url, bearer, None)?;
            let event = parse_graph_event(&body)?;
            Ok(format_calendar_event("graph", &event))
        }
    })
}

/// Run `drive_list`.
pub(crate) fn run_drive_list(args: &DriveListArgs) -> Result<String, String> {
    let max = clamp_drive_max(args.max_results);
    with_account(|provider, bearer| match provider {
        AccountProvider::Google => {
            let url = google_drive_list_url(max, None);
            let body = get_json(&url, bearer, None)?;
            let files = parse_google_drive_list(&body)?;
            Ok(format_drive_list("google", &files))
        }
        AccountProvider::Microsoft => {
            let url = graph_approot_children_url(max);
            let body = get_json(&url, bearer, None)?;
            let files = parse_graph_drive_list(&body)?;
            Ok(format_drive_list("graph", &files))
        }
    })
}

/// Run `drive_search`.
pub(crate) fn run_drive_search(args: &DriveSearchArgs) -> Result<String, String> {
    let max = clamp_drive_max(args.max_results);
    let query = args.query.trim();
    if query.is_empty() {
        return Err("drive_search needs query".to_owned());
    }
    with_account(|provider, bearer| match provider {
        AccountProvider::Google => {
            let url = google_drive_list_url(max, Some(query));
            let body = get_json(&url, bearer, None)?;
            let files = parse_google_drive_list(&body)?;
            Ok(format_drive_list("google", &files))
        }
        AccountProvider::Microsoft => {
            let url = graph_approot_search_url(query, max);
            let body = get_json(&url, bearer, None)?;
            let files = parse_graph_drive_list(&body)?;
            Ok(format_drive_list("graph", &files))
        }
    })
}

/// Run `drive_get`.
pub(crate) fn run_drive_get(args: &DriveGetArgs) -> Result<String, String> {
    let id = args.id.trim();
    if id.is_empty() {
        return Err("drive_get needs id".to_owned());
    }
    with_account(|provider, bearer| match provider {
        AccountProvider::Google => {
            let url = google_drive_get_url(id);
            let body = get_json(&url, bearer, None)?;
            let mut file = parse_google_drive_file(&body)?;
            if args.read_text && is_cheap_text_mime(&file.mime_type) {
                let text_url = if file.mime_type == "application/vnd.google-apps.document" {
                    google_drive_export_text_url(id)
                } else {
                    google_drive_media_url(id)
                };
                if let Ok(raw) = get_text(&text_url, bearer) {
                    file.text = truncate_drive_text(&raw);
                }
            }
            Ok(format_drive_file("google", &file))
        }
        AccountProvider::Microsoft => {
            let url = graph_drive_item_url(id);
            let body = get_json(&url, bearer, None)?;
            let mut file = parse_graph_drive_file(&body)?;
            if args.read_text && is_cheap_text_mime(&file.mime_type) {
                let content_url = graph_drive_content_url(id);
                if let Ok(raw) = get_text(&content_url, bearer) {
                    file.text = truncate_drive_text(&raw);
                }
            }
            Ok(format_drive_file("graph", &file))
        }
    })
}

/// Connector action for a cloud read tool name.
#[must_use]
pub(crate) fn connector_action_for(tool: &str) -> Option<(&'static str, &'static str)> {
    match tool {
        softwake_tools::EMAIL_LIST_TOOL => Some((EMAIL, EMAIL_LIST)),
        softwake_tools::EMAIL_SEARCH_TOOL => Some((EMAIL, EMAIL_SEARCH)),
        softwake_tools::EMAIL_GET_TOOL => Some((EMAIL, EMAIL_GET)),
        softwake_tools::CALENDAR_LIST_TOOL => Some((CALENDAR, CALENDAR_LIST)),
        softwake_tools::CALENDAR_GET_TOOL => Some((CALENDAR, CALENDAR_GET)),
        softwake_tools::DRIVE_LIST_TOOL => Some((DRIVE, DRIVE_LIST)),
        softwake_tools::DRIVE_SEARCH_TOOL => Some((DRIVE, DRIVE_SEARCH)),
        softwake_tools::DRIVE_GET_TOOL => Some((DRIVE, DRIVE_GET)),
        _ => None,
    }
}

fn with_account<F>(f: F) -> Result<String, String>
where
    F: FnOnce(AccountProvider, &str) -> Result<String, String>,
{
    #[cfg(not(feature = "live-http"))]
    {
        let _ = f;
        Err(LIVE_REQUIRED.to_owned())
    }
    #[cfg(feature = "live-http")]
    {
        let (provider, connection) = load_preferred_account()?;
        let transport =
            softwake_providers::live::LiveTransport::bounded(std::time::Duration::from_secs(30));
        let fresh = ensure_fresh_account(provider, &transport, &connection, now_ms())?;
        if fresh.access_token != connection.access_token
            || fresh.expires_at_ms != connection.expires_at_ms
        {
            persist_account(provider, fresh.clone())?;
        }
        f(provider, &fresh.access_token)
    }
}

fn load_preferred_account() -> Result<(AccountProvider, AccountConnection), String> {
    let path = resolve_secrets_file().map_err(|e| e.to_string())?;
    let store = open_store(&path).map_err(|e| e.to_string())?;
    let bag = store.load().map_err(|e| e.to_string())?;
    if let Some(connection) = bag.google_connections.first().cloned() {
        if !connection.access_token.is_empty() || !connection.refresh_token.is_empty() {
            return Ok((AccountProvider::Google, connection));
        }
    }
    if let Some(connection) = bag.microsoft_connections.first().cloned() {
        if !connection.access_token.is_empty() || !connection.refresh_token.is_empty() {
            return Ok((AccountProvider::Microsoft, connection));
        }
    }
    Err(NO_ACCOUNT.to_owned())
}

fn persist_account(provider: AccountProvider, connection: AccountConnection) -> Result<(), String> {
    let path = resolve_secrets_file().map_err(|e| e.to_string())?;
    let store = open_store(&path).map_err(|e| e.to_string())?;
    update_bag(store.as_ref(), |bag| match provider {
        AccountProvider::Google => bag.google_connections = vec![connection],
        AccountProvider::Microsoft => bag.microsoft_connections = vec![connection],
    })
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(feature = "live-http")]
fn get_json(
    url: &str,
    bearer: &str,
    extra_headers: Option<&[(&str, &str)]>,
) -> Result<String, String> {
    get_body(url, bearer, extra_headers)
}

#[cfg(feature = "live-http")]
fn get_text(url: &str, bearer: &str) -> Result<String, String> {
    get_body(url, bearer, None)
}

#[cfg(feature = "live-http")]
fn get_body(
    url: &str,
    bearer: &str,
    extra_headers: Option<&[(&str, &str)]>,
) -> Result<String, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(10))
        .timeout_read(std::time::Duration::from_secs(30))
        .timeout(std::time::Duration::from_secs(30))
        .build();
    let mut req = agent
        .get(url)
        .set("Authorization", &format!("Bearer {bearer}"));
    if let Some(headers) = extra_headers {
        for (k, v) in headers {
            req = req.set(k, v);
        }
    }
    let response = match req.call() {
        Ok(response) => response,
        Err(ureq::Error::Status(_, response)) => response,
        Err(_) => return Err("cloud API network transport failed".to_owned()),
    };
    let status = response.status();
    let body = response
        .into_string()
        .map_err(|_| "cloud API body read failed".to_owned())?;
    if !(200..300).contains(&status) {
        return Err(cloud_http_error(status, &body));
    }
    Ok(body)
}

#[cfg(not(feature = "live-http"))]
fn get_json(_url: &str, _bearer: &str, _extra: Option<&[(&str, &str)]>) -> Result<String, String> {
    Err(LIVE_REQUIRED.to_owned())
}

#[cfg(not(feature = "live-http"))]
fn get_text(_url: &str, _bearer: &str) -> Result<String, String> {
    Err(LIVE_REQUIRED.to_owned())
}

/// Build a token-free cloud HTTP error. Prefer Google/Graph `error.message` when present.
fn cloud_http_error(status: u16, body: &str) -> String {
    let mut message = format!("cloud API HTTP {status}");
    if let Some(detail) = cloud_error_detail(body) {
        message.push_str(": ");
        message.push_str(&detail);
    }
    if status == 403 {
        message.push_str(
            " — If inbox fails while calendar/Drive work: enable Gmail API on the publisher GCP project (docs/oauth-clients.md). If the token lacks gmail.readonly, Disconnect and Connect Google in Settings → Email. No /refresh needed; Softwake reads the live secret bag.",
        );
    }
    message
}

fn cloud_error_detail(body: &str) -> Option<String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(msg) = value
            .pointer("/error/message")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Some(truncate_error_detail(msg));
        }
        if let Some(msg) = value
            .get("message")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Some(truncate_error_detail(msg));
        }
    }
    let flat: String = trimmed
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.is_empty() {
        None
    } else {
        Some(truncate_error_detail(&flat))
    }
}

fn truncate_error_detail(detail: &str) -> String {
    const MAX: usize = 240;
    let trimmed = detail.trim();
    if trimmed.chars().count() <= MAX {
        return trimmed.to_owned();
    }
    let truncated: String = trimmed.chars().take(MAX).collect();
    format!("{truncated}…")
}

#[allow(
    clippy::cast_possible_wrap,
    reason = "unix ms to secs fits i64 for Softwake lifetimes"
)]
fn chrono_now() -> i64 {
    i64::try_from(now_ms() / 1000).unwrap_or(0)
}

fn format_rfc3339(unix_secs: i64) -> String {
    // Manual UTC Y-M-D from unix days. Casts are intentional and bounded for civil dates.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        clippy::cast_sign_loss,
        reason = "civil date parts from unix day count are in-range for formatting"
    )]
    {
        let secs = unix_secs.max(0) as u64;
        let days = secs / 86_400;
        let rem = secs % 86_400;
        let hour = rem / 3600;
        let min = (rem % 3600) / 60;
        let sec = rem % 60;
        let (y, m, d) = civil_from_days(days as i64);
        format!("{y:04}-{m:02}-{d:02}T{hour:02}:{min:02}:{sec:02}Z")
    }
}

/// Howard Hinnant `civil_from_days` (proleptic Gregorian).
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    reason = "algorithm yields month/day in 1..=31 and year in i32 range for Softwake windows"
)]
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::{
        civil_from_days, cloud_error_detail, cloud_http_error, connector_action_for, format_rfc3339,
    };
    use softwake_connectors::{CALENDAR, CALENDAR_LIST, EMAIL, EMAIL_LIST};

    #[test]
    fn connector_map_and_rfc3339() {
        assert_eq!(
            connector_action_for(softwake_tools::EMAIL_LIST_TOOL),
            Some((EMAIL, EMAIL_LIST))
        );
        assert_eq!(
            connector_action_for(softwake_tools::CALENDAR_LIST_TOOL),
            Some((CALENDAR, CALENDAR_LIST))
        );
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert!(format_rfc3339(0).starts_with("1970-01-01T00:00:00Z"));
    }

    #[test]
    fn cloud_http_403_surfaces_gmail_api_hint() {
        let body = r#"{"error":{"code":403,"message":"Gmail API has not been used in project 1 before or it is disabled. Enable it by visiting https://console.developers.google.com/apis/api/gmail.googleapis.com/overview?project=1 then retry."}}"#;
        let err = cloud_http_error(403, body);
        assert!(err.contains("cloud API HTTP 403"));
        assert!(err.contains("Gmail API has not been used"));
        assert!(err.contains("enable Gmail API"));
        assert!(err.contains("Disconnect and Connect Google"));
        assert!(err.contains("No /refresh needed"));
        assert!(!err.contains("ya29."));
    }

    #[test]
    fn cloud_error_detail_truncates() {
        let long = "x".repeat(400);
        let detail = cloud_error_detail(&long).expect("detail");
        assert!(detail.ends_with('…'));
        assert!(detail.chars().count() <= 241);
    }
}
