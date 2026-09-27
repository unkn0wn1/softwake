//! Argument parsers for calendar create, update, and delete.
//!
//! Positional Hands argv, plus optional trailing `account=`. Chat JSON is mapped
//! onto the same shape in `chat_schema`.

use crate::ToolError;
use crate::cloud_read::split_trailing_account;

/// Parsed `calendar_create` args.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarCreateArgs {
    /// Event title (Google summary / Graph subject).
    pub title: String,
    /// Start, RFC3339.
    pub start: String,
    /// End, RFC3339.
    pub end: String,
    /// Optional location.
    pub location: Option<String>,
    /// Optional description.
    pub description: Option<String>,
    /// Optional connection id or email substring.
    pub account: Option<String>,
}

/// Parsed `calendar_update` args. `None` means leave that field unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarUpdateArgs {
    /// Event id.
    pub id: String,
    /// Replacement title, when set.
    pub title: Option<String>,
    /// Replacement start, when set.
    pub start: Option<String>,
    /// Replacement end, when set.
    pub end: Option<String>,
    /// Replacement location, when set. Empty clears it.
    pub location: Option<String>,
    /// Replacement description, when set. Empty clears it.
    pub description: Option<String>,
    /// Optional connection id or email substring.
    pub account: Option<String>,
}

/// Parsed `calendar_delete` args.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarDeleteArgs {
    /// Event id.
    pub id: String,
    /// Optional connection id or email substring.
    pub account: Option<String>,
}

fn needs(name: &str) -> ToolError {
    ToolError::invalid_args(name)
}

fn non_empty(value: &str) -> bool {
    !value.trim().is_empty()
}

/// Light RFC3339 check: a date, a `T`, and a time. Offset may be `Z` or numeric.
fn looks_like_rfc3339(value: &str) -> bool {
    let value = value.trim();
    let Some((date, time)) = value.split_once(['T', 't']) else {
        return false;
    };
    date.len() >= 8
        && date.contains('-')
        && time.len() >= 5
        && time.as_bytes().get(2) == Some(&b':')
}

/// Parse `calendar_create`: title, start, end, optional location and description.
///
/// After the three required slots, either bare positionals (`location`, then
/// description words) or `location=` / `description=` keys. A trailing
/// `account=` is peeled when it sits past those three slots.
///
/// # Errors
///
/// Missing title/start/end, a start or end that is not RFC3339-shaped, or an empty account.
pub fn parse_calendar_create_args(args: &[String]) -> Result<CalendarCreateArgs, ToolError> {
    let tool = crate::CALENDAR_CREATE_TOOL;
    let (account, args) = split_trailing_account(args, 3, tool)?;
    if args.len() < 3 || !non_empty(&args[0]) || !non_empty(&args[1]) || !non_empty(&args[2]) {
        return Err(needs(tool));
    }
    let start = args[1].trim();
    let end = args[2].trim();
    if !looks_like_rfc3339(start) || !looks_like_rfc3339(end) {
        return Err(needs(tool));
    }
    let (location, description) = parse_create_optional(&args[3..])?;
    Ok(CalendarCreateArgs {
        title: args[0].trim().to_owned(),
        start: start.to_owned(),
        end: end.to_owned(),
        location,
        description,
        account,
    })
}

fn parse_create_optional(rest: &[String]) -> Result<(Option<String>, Option<String>), ToolError> {
    let tool = crate::CALENDAR_CREATE_TOOL;
    if rest
        .iter()
        .any(|arg| arg.starts_with("location=") || arg.starts_with("description="))
    {
        let mut location = None;
        let mut description = None;
        for arg in rest {
            if let Some(value) = arg.strip_prefix("location=") {
                location = Some(value.trim().to_owned());
            } else if let Some(value) = arg.strip_prefix("description=") {
                description = Some(value.trim().to_owned());
            } else {
                return Err(needs(tool));
            }
        }
        return Ok((location, description));
    }
    match rest {
        [] => Ok((None, None)),
        [location] => Ok((Some(location.trim().to_owned()), None)),
        [location, description @ ..] => Ok((
            Some(location.trim().to_owned()),
            Some(description.join(" ").trim().to_owned()),
        )),
    }
}

/// Parse `calendar_update`: id, then `title=` / `start=` / `end=` / `location=` / `description=`.
///
/// At least one patch key is required. A trailing `account=` is peeled when more than the id is present.
///
/// # Errors
///
/// Missing id, no patch field, an unknown key, a non-RFC3339 start/end, or an empty account.
pub fn parse_calendar_update_args(args: &[String]) -> Result<CalendarUpdateArgs, ToolError> {
    let tool = crate::CALENDAR_UPDATE_TOOL;
    let (account, args) = split_trailing_account(args, 1, tool)?;
    let Some(id) = args.first() else {
        return Err(needs(tool));
    };
    if !non_empty(id) {
        return Err(needs(tool));
    }
    let mut parsed = CalendarUpdateArgs {
        id: id.trim().to_owned(),
        title: None,
        start: None,
        end: None,
        location: None,
        description: None,
        account,
    };
    if args.len() < 2 {
        return Err(needs(tool));
    }
    for arg in &args[1..] {
        if let Some(value) = arg.strip_prefix("title=") {
            parsed.title = Some(value.trim().to_owned());
        } else if let Some(value) = arg.strip_prefix("start=") {
            let value = value.trim();
            if !looks_like_rfc3339(value) {
                return Err(needs(tool));
            }
            parsed.start = Some(value.to_owned());
        } else if let Some(value) = arg.strip_prefix("end=") {
            let value = value.trim();
            if !looks_like_rfc3339(value) {
                return Err(needs(tool));
            }
            parsed.end = Some(value.to_owned());
        } else if let Some(value) = arg.strip_prefix("location=") {
            parsed.location = Some(value.trim().to_owned());
        } else if let Some(value) = arg.strip_prefix("description=") {
            parsed.description = Some(value.trim().to_owned());
        } else {
            return Err(needs(tool));
        }
    }
    Ok(parsed)
}

/// Parse `calendar_delete`: id, optional trailing `account=`.
///
/// # Errors
///
/// Missing id or an empty account.
pub fn parse_calendar_delete_args(args: &[String]) -> Result<CalendarDeleteArgs, ToolError> {
    let tool = crate::CALENDAR_DELETE_TOOL;
    let (account, args) = split_trailing_account(args, 1, tool)?;
    match args {
        [id] if non_empty(id) => Ok(CalendarDeleteArgs {
            id: id.trim().to_owned(),
            account,
        }),
        _ => Err(needs(tool)),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        parse_calendar_create_args, parse_calendar_delete_args, parse_calendar_update_args,
    };

    #[test]
    fn create_parses_required_optional_and_account() {
        let parsed = parse_calendar_create_args(&[
            "Stand-up".into(),
            "2026-09-28T09:00:00Z".into(),
            "2026-09-28T09:15:00+07:00".into(),
            "location=Zoom".into(),
            "description=daily sync".into(),
            "account=ada@example.com".into(),
        ])
        .expect("create");
        assert_eq!(parsed.title, "Stand-up");
        assert_eq!(parsed.start, "2026-09-28T09:00:00Z");
        assert_eq!(parsed.end, "2026-09-28T09:15:00+07:00");
        assert_eq!(parsed.location.as_deref(), Some("Zoom"));
        assert_eq!(parsed.description.as_deref(), Some("daily sync"));
        assert_eq!(parsed.account.as_deref(), Some("ada@example.com"));
    }

    #[test]
    fn create_positional_location_and_description() {
        let parsed = parse_calendar_create_args(&[
            "Meet".into(),
            "2026-09-28T09:00:00Z".into(),
            "2026-09-28T10:00:00Z".into(),
            "Room A".into(),
            "line one".into(),
            "line two".into(),
        ])
        .expect("positional");
        assert_eq!(parsed.location.as_deref(), Some("Room A"));
        assert_eq!(parsed.description.as_deref(), Some("line one line two"));
        assert!(parsed.account.is_none());
    }

    #[test]
    fn create_rejects_short_and_non_rfc3339() {
        assert!(parse_calendar_create_args(&["only".into()]).is_err());
        assert!(
            parse_calendar_create_args(&[
                "Meet".into(),
                "tomorrow".into(),
                "2026-09-28T10:00:00Z".into(),
            ])
            .is_err()
        );
        assert!(parse_calendar_create_args(&["account=".into()]).is_err());
    }

    #[test]
    fn update_and_delete_peel_account() {
        let updated = parse_calendar_update_args(&[
            "evt-1".into(),
            "title=Moved".into(),
            "start=2026-09-28T11:00:00Z".into(),
            "account=id-9".into(),
        ])
        .expect("update");
        assert_eq!(updated.id, "evt-1");
        assert_eq!(updated.title.as_deref(), Some("Moved"));
        assert_eq!(updated.start.as_deref(), Some("2026-09-28T11:00:00Z"));
        assert!(updated.end.is_none());
        assert_eq!(updated.account.as_deref(), Some("id-9"));

        let deleted =
            parse_calendar_delete_args(&["evt-1".into(), "account=ada@example.com".into()])
                .expect("delete");
        assert_eq!(deleted.id, "evt-1");
        assert_eq!(deleted.account.as_deref(), Some("ada@example.com"));
    }

    #[test]
    fn update_requires_a_field_and_delete_requires_id() {
        assert!(parse_calendar_update_args(&["evt-1".into()]).is_err());
        assert!(parse_calendar_update_args(&["evt-1".into(), "nope".into()]).is_err());
        assert!(parse_calendar_update_args(&["evt-1".into(), "start=tomorrow".into()]).is_err());
        assert!(parse_calendar_delete_args(&[]).is_err());
        assert!(parse_calendar_delete_args(&["evt-1".into(), "extra".into()]).is_err());
        let cleared =
            parse_calendar_update_args(&["evt-1".into(), "location=".into()]).expect("clear");
        assert_eq!(cleared.location.as_deref(), Some(""));
    }
}
