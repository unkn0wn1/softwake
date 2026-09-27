//! Google Calendar and Microsoft Graph event write builders.
//!
//! Network-free: URL strings and JSON bodies only. The daemon posts them
//! under `live-http`. v1 writes the primary calendar (Google
//! `calendars/primary`, Graph `/me/events`). Attendees and a calendar picker
//! are out of this slice.
//!
//! Google `start` / `end` are `{ "dateTime": <caller RFC3339> }` with no
//! separate `timeZone` (the offset lives in the string). Graph always sends
//! `{ "dateTime": <caller string>, "timeZone": "UTC" }`.

use serde_json::{Map, Value};

use crate::calendar_api::encode_path;

const GOOGLE_EVENTS: &str = "https://www.googleapis.com/calendar/v3/calendars/primary/events";
const GRAPH_EVENTS: &str = "https://graph.microsoft.com/v1.0/me/events";

/// Fields for a create or patch. `None` is omitted from the JSON body.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CalendarEventWrite {
    /// Google `summary` / Graph `subject`.
    pub title: Option<String>,
    /// RFC3339 start. Google stores it as `dateTime` only.
    pub start: Option<String>,
    /// RFC3339 end. Google stores it as `dateTime` only.
    pub end: Option<String>,
    /// Free-text location. Graph nests it under `location.displayName`.
    pub location: Option<String>,
    /// Description. Graph sends `body.contentType = Text`.
    pub description: Option<String>,
}

/// `POST …/calendars/primary/events`.
#[must_use]
pub fn google_event_create_url() -> String {
    GOOGLE_EVENTS.to_owned()
}

/// `PATCH …/calendars/primary/events/{id}`.
#[must_use]
pub fn google_event_patch_url(id: &str) -> String {
    format!("{GOOGLE_EVENTS}/{}", encode_path(id))
}

/// `DELETE …/calendars/primary/events/{id}`.
#[must_use]
pub fn google_event_delete_url(id: &str) -> String {
    google_event_patch_url(id)
}

/// `POST /me/events`.
#[must_use]
pub fn graph_event_create_url() -> String {
    GRAPH_EVENTS.to_owned()
}

/// `PATCH /me/events/{id}`.
#[must_use]
pub fn graph_event_patch_url(id: &str) -> String {
    format!("{GRAPH_EVENTS}/{}", encode_path(id))
}

/// `DELETE /me/events/{id}`.
#[must_use]
pub fn graph_event_delete_url(id: &str) -> String {
    graph_event_patch_url(id)
}

/// Google Calendar event JSON. Only fields that are `Some` are included.
#[must_use]
pub fn google_event_write_body(fields: &CalendarEventWrite) -> String {
    let mut map = Map::new();
    if let Some(title) = &fields.title {
        map.insert("summary".to_owned(), Value::String(title.clone()));
    }
    if let Some(start) = &fields.start {
        map.insert("start".to_owned(), google_date_time(start));
    }
    if let Some(end) = &fields.end {
        map.insert("end".to_owned(), google_date_time(end));
    }
    if let Some(location) = &fields.location {
        map.insert("location".to_owned(), Value::String(location.clone()));
    }
    if let Some(description) = &fields.description {
        map.insert("description".to_owned(), Value::String(description.clone()));
    }
    Value::Object(map).to_string()
}

/// Graph event JSON. Only fields that are `Some` are included.
///
/// `timeZone` is always `UTC`. `dateTime` is the caller string unchanged.
#[must_use]
pub fn graph_event_write_body(fields: &CalendarEventWrite) -> String {
    let mut map = Map::new();
    if let Some(title) = &fields.title {
        map.insert("subject".to_owned(), Value::String(title.clone()));
    }
    if let Some(start) = &fields.start {
        map.insert("start".to_owned(), graph_date_time(start));
    }
    if let Some(end) = &fields.end {
        map.insert("end".to_owned(), graph_date_time(end));
    }
    if let Some(location) = &fields.location {
        map.insert(
            "location".to_owned(),
            serde_json::json!({ "displayName": location }),
        );
    }
    if let Some(description) = &fields.description {
        map.insert(
            "body".to_owned(),
            serde_json::json!({ "contentType": "Text", "content": description }),
        );
    }
    Value::Object(map).to_string()
}

/// Read `id` from a Google or Graph create/update response.
///
/// # Errors
///
/// The body is not a JSON object, or `id` is missing or empty.
pub fn parse_written_event_id(body: &str) -> Result<String, String> {
    let value: Value = serde_json::from_str(body)
        .map_err(|_| "calendar write response was not JSON".to_owned())?;
    value
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "calendar write response missing id".to_owned())
}

/// Stable create detail: `created google <id>` or `created graph <id>`.
#[must_use]
pub fn format_calendar_created(provider: &str, id: &str) -> String {
    format!("created {provider} {id}")
}

/// Stable update detail: `updated google <id>` or `updated graph <id>`.
#[must_use]
pub fn format_calendar_updated(provider: &str, id: &str) -> String {
    format!("updated {provider} {id}")
}

/// Stable delete detail: `deleted google <id>` or `deleted graph <id>`.
#[must_use]
pub fn format_calendar_deleted(provider: &str, id: &str) -> String {
    format!("deleted {provider} {id}")
}

fn google_date_time(value: &str) -> Value {
    serde_json::json!({ "dateTime": value })
}

fn graph_date_time(value: &str) -> Value {
    serde_json::json!({ "dateTime": value, "timeZone": "UTC" })
}

#[cfg(test)]
mod tests {
    use super::{
        CalendarEventWrite, format_calendar_created, format_calendar_deleted,
        format_calendar_updated, google_event_create_url, google_event_delete_url,
        google_event_patch_url, google_event_write_body, graph_event_create_url,
        graph_event_delete_url, graph_event_patch_url, graph_event_write_body,
        parse_written_event_id,
    };
    use serde_json::Value;

    fn sample() -> CalendarEventWrite {
        CalendarEventWrite {
            title: Some("Stand-up".to_owned()),
            start: Some("2026-09-28T09:00:00Z".to_owned()),
            end: Some("2026-09-28T09:15:00+07:00".to_owned()),
            location: Some("Zoom".to_owned()),
            description: Some("daily \"sync\"".to_owned()),
        }
    }

    #[test]
    fn google_urls_are_primary_collection() {
        assert_eq!(
            google_event_create_url(),
            "https://www.googleapis.com/calendar/v3/calendars/primary/events"
        );
        assert!(!google_event_create_url().contains('?'));
        assert_eq!(
            google_event_patch_url("evt/1 a"),
            "https://www.googleapis.com/calendar/v3/calendars/primary/events/evt%2F1%20a"
        );
        assert_eq!(
            google_event_delete_url("evt/1 a"),
            google_event_patch_url("evt/1 a")
        );
    }

    #[test]
    fn graph_urls_are_me_events() {
        assert_eq!(
            graph_event_create_url(),
            "https://graph.microsoft.com/v1.0/me/events"
        );
        assert_eq!(
            graph_event_patch_url("id+2"),
            "https://graph.microsoft.com/v1.0/me/events/id%2B2"
        );
        assert_eq!(
            graph_event_delete_url("id+2"),
            graph_event_patch_url("id+2")
        );
    }

    #[test]
    fn google_body_uses_datetime_only_and_omits_absent_fields() {
        let body = google_event_write_body(&sample());
        let value: Value = serde_json::from_str(&body).expect("json");
        assert_eq!(value["summary"].as_str(), Some("Stand-up"));
        assert_eq!(
            value["start"]["dateTime"].as_str(),
            Some("2026-09-28T09:00:00Z")
        );
        assert!(value["start"].get("timeZone").is_none());
        assert_eq!(
            value["end"]["dateTime"].as_str(),
            Some("2026-09-28T09:15:00+07:00")
        );
        assert_eq!(value["location"].as_str(), Some("Zoom"));
        assert_eq!(value["description"].as_str(), Some("daily \"sync\""));
        assert!(value.get("subject").is_none());
        assert!(value.get("attendees").is_none());

        let patch = google_event_write_body(&CalendarEventWrite {
            title: Some("Moved".to_owned()),
            ..CalendarEventWrite::default()
        });
        let patch: Value = serde_json::from_str(&patch).expect("json");
        assert_eq!(patch["summary"].as_str(), Some("Moved"));
        assert!(patch.get("start").is_none());
        assert!(patch.get("end").is_none());
        assert!(patch.get("location").is_none());
        assert!(patch.get("description").is_none());
    }

    #[test]
    fn graph_body_sets_utc_timezone_and_text_body() {
        let body = graph_event_write_body(&sample());
        let value: Value = serde_json::from_str(&body).expect("json");
        assert_eq!(value["subject"].as_str(), Some("Stand-up"));
        assert_eq!(
            value["start"]["dateTime"].as_str(),
            Some("2026-09-28T09:00:00Z")
        );
        assert_eq!(value["start"]["timeZone"].as_str(), Some("UTC"));
        assert_eq!(
            value["end"]["dateTime"].as_str(),
            Some("2026-09-28T09:15:00+07:00")
        );
        assert_eq!(value["end"]["timeZone"].as_str(), Some("UTC"));
        assert_eq!(value["location"]["displayName"].as_str(), Some("Zoom"));
        assert_eq!(value["body"]["contentType"].as_str(), Some("Text"));
        assert_eq!(value["body"]["content"].as_str(), Some("daily \"sync\""));
        assert!(value.get("summary").is_none());
        assert!(value.get("attendees").is_none());
    }

    #[test]
    fn parse_written_event_id_reads_id() {
        assert_eq!(parse_written_event_id(r#"{"id":"e1"}"#).expect("id"), "e1");
        assert_eq!(
            parse_written_event_id(r#"{"id":"  g-9  "}"#).expect("trim"),
            "g-9"
        );
        assert!(parse_written_event_id("{}").is_err());
        assert!(parse_written_event_id(r#"{"id":""}"#).is_err());
        assert!(parse_written_event_id(r#"{"id":1}"#).is_err());
        assert!(parse_written_event_id("nope").is_err());
    }

    #[test]
    fn detail_strings_are_stable() {
        assert_eq!(format_calendar_created("google", "e1"), "created google e1");
        assert_eq!(format_calendar_created("graph", "m1"), "created graph m1");
        assert_eq!(format_calendar_updated("google", "e1"), "updated google e1");
        assert_eq!(format_calendar_updated("graph", "m1"), "updated graph m1");
        assert_eq!(format_calendar_deleted("google", "e1"), "deleted google e1");
        assert_eq!(format_calendar_deleted("graph", "m1"), "deleted graph m1");
    }
}
