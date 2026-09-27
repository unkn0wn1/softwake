//! Google Calendar and Microsoft Graph calendar URL builders + parsers.

use serde_json::Value;

/// Default upcoming window in days.
pub const DEFAULT_CALENDAR_DAYS: u32 = 7;
/// Default max events.
pub const DEFAULT_CALENDAR_MAX: u32 = 20;
/// Hard cap.
pub const MAX_CALENDAR_MAX: u32 = 50;

/// One calendar event for tool detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveCalendarEvent {
    /// Provider event id.
    pub id: String,
    /// Title / subject.
    pub title: String,
    /// Start instant or date (API spelling).
    pub start: String,
    /// End instant or date.
    pub end: String,
    /// Location when present.
    pub location: String,
    /// Description / body preview (truncated by caller if needed).
    pub description: String,
}

/// Clamp days to `1..=90`.
#[must_use]
pub fn clamp_calendar_days(requested: Option<u32>) -> u32 {
    match requested {
        None | Some(0) => DEFAULT_CALENDAR_DAYS,
        Some(n) => n.min(90),
    }
}

/// Clamp max events.
#[must_use]
pub fn clamp_calendar_max(requested: Option<u32>) -> u32 {
    match requested {
        None | Some(0) => DEFAULT_CALENDAR_MAX,
        Some(n) => n.min(MAX_CALENDAR_MAX),
    }
}

/// Google Calendar events.list for `primary` between `time_min` and `time_max` (RFC3339).
#[must_use]
pub fn google_events_url(time_min: &str, time_max: &str, max: u32) -> String {
    format!(
        "https://www.googleapis.com/calendar/v3/calendars/primary/events?singleEvents=true&orderBy=startTime&showDeleted=false&maxResults={max}&timeMin={}&timeMax={}",
        encode_q(time_min),
        encode_q(time_max)
    )
}

/// Google Calendar events.get.
#[must_use]
pub fn google_event_get_url(id: &str) -> String {
    format!(
        "https://www.googleapis.com/calendar/v3/calendars/primary/events/{}",
        encode_path(id)
    )
}

/// Graph calendarView URL.
#[must_use]
pub fn graph_calendar_view_url(start: &str, end: &str, max: u32) -> String {
    format!(
        "https://graph.microsoft.com/v1.0/me/calendarView?startDateTime={}&endDateTime={}&$top={max}&$orderby=start/dateTime&$select=id,subject,start,end,location,bodyPreview",
        encode_q(start),
        encode_q(end)
    )
}

/// Graph events.get.
#[must_use]
pub fn graph_event_get_url(id: &str) -> String {
    format!(
        "https://graph.microsoft.com/v1.0/me/events/{}?$select=id,subject,start,end,location,bodyPreview,body",
        encode_path(id)
    )
}

/// Parse Google events.list / get list-shaped JSON.
///
/// # Errors
///
/// Invalid JSON.
pub fn parse_google_events(body: &str) -> Result<Vec<LiveCalendarEvent>, String> {
    let value: Value =
        serde_json::from_str(body).map_err(|_| "Google Calendar list was not JSON".to_owned())?;
    if value.get("items").is_some() {
        let items = value
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut out = Vec::new();
        for item in &items {
            if let Ok(ev) = parse_google_event_value(item) {
                out.push(ev);
            }
        }
        return Ok(out);
    }
    Ok(vec![parse_google_event_value(&value)?])
}

/// Parse one Google event object (get response).
///
/// # Errors
///
/// Invalid JSON or missing id.
pub fn parse_google_event(body: &str) -> Result<LiveCalendarEvent, String> {
    let value: Value =
        serde_json::from_str(body).map_err(|_| "Google Calendar event was not JSON".to_owned())?;
    parse_google_event_value(&value)
}

fn parse_google_event_value(value: &Value) -> Result<LiveCalendarEvent, String> {
    let id = text(value, "id").ok_or_else(|| "Google event missing id".to_owned())?;
    let start = value
        .pointer("/start/dateTime")
        .and_then(Value::as_str)
        .or_else(|| value.pointer("/start/date").and_then(Value::as_str))
        .unwrap_or("")
        .to_owned();
    let end = value
        .pointer("/end/dateTime")
        .and_then(Value::as_str)
        .or_else(|| value.pointer("/end/date").and_then(Value::as_str))
        .unwrap_or("")
        .to_owned();
    Ok(LiveCalendarEvent {
        id,
        title: text(value, "summary").unwrap_or_default(),
        start,
        end,
        location: text(value, "location").unwrap_or_default(),
        description: text(value, "description").unwrap_or_default(),
    })
}

/// Parse Graph calendarView list.
///
/// # Errors
///
/// Invalid JSON.
pub fn parse_graph_events(body: &str) -> Result<Vec<LiveCalendarEvent>, String> {
    let value: Value =
        serde_json::from_str(body).map_err(|_| "Graph calendar list was not JSON".to_owned())?;
    let Some(items) = value.get("value").and_then(Value::as_array) else {
        // Single get payload.
        return Ok(vec![parse_graph_event_value(&value)?]);
    };
    let mut out = Vec::new();
    for item in items {
        if let Ok(ev) = parse_graph_event_value(item) {
            out.push(ev);
        }
    }
    Ok(out)
}

/// Parse one Graph event JSON.
///
/// # Errors
///
/// Invalid JSON or missing id.
pub fn parse_graph_event(body: &str) -> Result<LiveCalendarEvent, String> {
    let value: Value =
        serde_json::from_str(body).map_err(|_| "Graph calendar event was not JSON".to_owned())?;
    parse_graph_event_value(&value)
}

fn parse_graph_event_value(value: &Value) -> Result<LiveCalendarEvent, String> {
    let id = text(value, "id").ok_or_else(|| "Graph event missing id".to_owned())?;
    let start = value
        .pointer("/start/dateTime")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let end = value
        .pointer("/end/dateTime")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let location = value
        .pointer("/location/displayName")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let description = text(value, "bodyPreview")
        .or_else(|| {
            value
                .pointer("/body/content")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_default();
    Ok(LiveCalendarEvent {
        id,
        title: text(value, "subject").unwrap_or_default(),
        start,
        end,
        location,
        description,
    })
}

/// Multi-line list detail.
#[must_use]
pub fn format_calendar_list(provider: &str, events: &[LiveCalendarEvent]) -> String {
    if events.is_empty() {
        return format!("{provider}: (no upcoming events)");
    }
    let mut lines = vec![format!("{provider}: {} event(s)", events.len())];
    for (i, ev) in events.iter().enumerate() {
        let title = if ev.title.is_empty() {
            "(no title)"
        } else {
            ev.title.as_str()
        };
        lines.push(format!(
            "{}. id={} start={} end={} title={} loc={}",
            i + 1,
            ev.id,
            ev.start,
            ev.end,
            title,
            ev.location
        ));
    }
    lines.join("\n")
}

/// One-event detail.
#[must_use]
pub fn format_calendar_event(provider: &str, ev: &LiveCalendarEvent) -> String {
    let desc = truncate(&ev.description, 2000);
    format!(
        "{provider}: id={}\ntitle={}\nstart={}\nend={}\nlocation={}\n\n{desc}",
        ev.id, ev.title, ev.start, ev.end, ev.location
    )
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

pub(crate) fn encode_path(value: &str) -> String {
    encode_q(value)
}

fn hex(nibble: u8) -> char {
    char::from(b"0123456789ABCDEF"[nibble as usize])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_and_parse_google() {
        let url = google_events_url("2026-09-27T00:00:00Z", "2026-10-04T00:00:00Z", 10);
        assert!(url.contains("timeMin=2026-09-27T00%3A00%3A00Z"));
        let body = r#"{
          "items":[{
            "id":"e1",
            "summary":"Stand-up",
            "start":{"dateTime":"2026-09-28T09:00:00Z"},
            "end":{"dateTime":"2026-09-28T09:15:00Z"},
            "location":"Zoom",
            "description":"daily"
          }]
        }"#;
        let events = parse_google_events(body).expect("parse");
        assert_eq!(events[0].title, "Stand-up");
        assert!(format_calendar_list("google", &events).contains("Stand-up"));
    }

    #[test]
    fn graph_events_parse() {
        let body = r#"{
          "value":[{
            "id":"m1",
            "subject":"1:1",
            "start":{"dateTime":"2026-09-28T10:00:00.0000000"},
            "end":{"dateTime":"2026-09-28T10:30:00.0000000"},
            "location":{"displayName":"Room A"},
            "bodyPreview":"notes"
          }]
        }"#;
        let events = parse_graph_events(body).expect("parse");
        assert_eq!(events[0].location, "Room A");
    }
}
