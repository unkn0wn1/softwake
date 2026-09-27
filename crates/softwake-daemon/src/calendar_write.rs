//! Confirmed calendar create, update, and delete over Google Calendar and Graph.
//!
//! Builders live in `softwake-connectors` and do not open a socket. This module
//! posts them when `live-http` is on, through [`crate::cloud_tools::with_account`].
//! v1 writes the primary calendar only.

#[cfg(feature = "live-http")]
use softwake_connectors::{
    CalendarEventWrite, format_calendar_created, format_calendar_deleted, format_calendar_updated,
    google_event_create_url, google_event_delete_url, google_event_patch_url,
    google_event_write_body, graph_event_create_url, graph_event_delete_url, graph_event_patch_url,
    graph_event_write_body, parse_written_event_id,
};
#[cfg(feature = "live-http")]
use softwake_providers::AccountConnection;
#[cfg(any(test, feature = "live-http"))]
use softwake_providers::AccountProvider;
use softwake_tools::{CalendarCreateArgs, CalendarDeleteArgs, CalendarUpdateArgs};

#[cfg(feature = "live-http")]
use crate::cloud_tools::{delete_bearer, patch_json, post_json, with_account};

/// Create an event on the primary calendar.
///
/// Without `live-http` this returns [`crate::cloud_tools::LIVE_REQUIRED`].
///
/// # Errors
///
/// Refresh failure, HTTP failure, or a response without an id.
pub(crate) fn run_calendar_create(args: &CalendarCreateArgs) -> Result<String, String> {
    #[cfg(not(feature = "live-http"))]
    {
        let _ = args;
        Err(crate::cloud_tools::LIVE_REQUIRED.to_owned())
    }
    #[cfg(feature = "live-http")]
    {
        let fields = CalendarEventWrite {
            title: Some(args.title.clone()),
            start: Some(args.start.clone()),
            end: Some(args.end.clone()),
            location: args.location.clone(),
            description: args.description.clone(),
        };
        let kind = WriteKind::Create;
        write_live(args.account.as_deref(), &fields, &kind)
    }
}

/// Patch an event on the primary calendar.
///
/// # Errors
///
/// Refresh failure, HTTP failure, or a response without an id.
pub(crate) fn run_calendar_update(args: &CalendarUpdateArgs) -> Result<String, String> {
    #[cfg(not(feature = "live-http"))]
    {
        let _ = args;
        Err(crate::cloud_tools::LIVE_REQUIRED.to_owned())
    }
    #[cfg(feature = "live-http")]
    {
        let fields = CalendarEventWrite {
            title: args.title.clone(),
            start: args.start.clone(),
            end: args.end.clone(),
            location: args.location.clone(),
            description: args.description.clone(),
        };
        let kind = WriteKind::Update(args.id.clone());
        write_live(args.account.as_deref(), &fields, &kind)
    }
}

/// Delete an event on the primary calendar.
///
/// The detail id is the request id. DELETE often returns an empty body.
///
/// # Errors
///
/// Refresh failure or HTTP failure.
pub(crate) fn run_calendar_delete(args: &CalendarDeleteArgs) -> Result<String, String> {
    #[cfg(not(feature = "live-http"))]
    {
        let _ = args;
        Err(crate::cloud_tools::LIVE_REQUIRED.to_owned())
    }
    #[cfg(feature = "live-http")]
    {
        let fields = CalendarEventWrite::default();
        let kind = WriteKind::Delete(args.id.clone());
        write_live(args.account.as_deref(), &fields, &kind)
    }
}

/// Empty `scope` is unknown and still allows the call.
#[cfg(any(test, feature = "live-http"))]
fn scope_allows_calendar_write(provider: AccountProvider, scope: &str) -> bool {
    if scope.trim().is_empty() {
        return true;
    }
    match provider {
        AccountProvider::Google => {
            scope
                .split_whitespace()
                .any(|token| token == "https://www.googleapis.com/auth/calendar.events")
                || scope.contains("calendar.events")
        }
        AccountProvider::Microsoft => scope
            .split_whitespace()
            .any(|token| token == "Calendars.ReadWrite"),
    }
}

/// Append the reconnect clause only when a stored scope is present and lacks write.
#[cfg(any(test, feature = "live-http"))]
fn scope_failure_hint(provider: AccountProvider, scope: &str, error: String) -> String {
    if error.contains("cloud API HTTP 403")
        && !scope.trim().is_empty()
        && !scope_allows_calendar_write(provider, scope)
    {
        format!(
            "{error} stored scope lacks calendar.events / Calendars.ReadWrite; Settings → Email → Accounts → Remove that account → Connect again."
        )
    } else {
        error
    }
}

#[cfg(feature = "live-http")]
enum WriteKind {
    Create,
    Update(String),
    Delete(String),
}

#[cfg(feature = "live-http")]
fn write_live(
    account: Option<&str>,
    fields: &CalendarEventWrite,
    kind: &WriteKind,
) -> Result<String, String> {
    with_account(account, |provider, connection| match provider {
        AccountProvider::Google => google_write(connection, fields, kind),
        AccountProvider::Microsoft => graph_write(connection, fields, kind),
    })
}

#[cfg(feature = "live-http")]
fn google_write(
    connection: &AccountConnection,
    fields: &CalendarEventWrite,
    kind: &WriteKind,
) -> Result<String, String> {
    match kind {
        WriteKind::Create => {
            let response = exchange(AccountProvider::Google, connection, |token| {
                post_json(
                    &google_event_create_url(),
                    token,
                    &google_event_write_body(fields),
                )
            })?;
            let id = parse_written_event_id(&response)?;
            Ok(format_calendar_created("google", &id))
        }
        WriteKind::Update(id) => {
            let response = exchange(AccountProvider::Google, connection, |token| {
                patch_json(
                    &google_event_patch_url(id),
                    token,
                    &google_event_write_body(fields),
                )
            })?;
            let written = parse_written_event_id(&response)?;
            Ok(format_calendar_updated("google", &written))
        }
        WriteKind::Delete(id) => {
            exchange(AccountProvider::Google, connection, |token| {
                delete_bearer(&google_event_delete_url(id), token)
            })?;
            Ok(format_calendar_deleted("google", id.trim()))
        }
    }
}

#[cfg(feature = "live-http")]
fn graph_write(
    connection: &AccountConnection,
    fields: &CalendarEventWrite,
    kind: &WriteKind,
) -> Result<String, String> {
    match kind {
        WriteKind::Create => {
            let response = exchange(AccountProvider::Microsoft, connection, |token| {
                post_json(
                    &graph_event_create_url(),
                    token,
                    &graph_event_write_body(fields),
                )
            })?;
            let id = parse_written_event_id(&response)?;
            Ok(format_calendar_created("graph", &id))
        }
        WriteKind::Update(id) => {
            let response = exchange(AccountProvider::Microsoft, connection, |token| {
                patch_json(
                    &graph_event_patch_url(id),
                    token,
                    &graph_event_write_body(fields),
                )
            })?;
            let written = parse_written_event_id(&response)?;
            Ok(format_calendar_updated("graph", &written))
        }
        WriteKind::Delete(id) => {
            exchange(AccountProvider::Microsoft, connection, |token| {
                delete_bearer(&graph_event_delete_url(id), token)
            })?;
            Ok(format_calendar_deleted("graph", id.trim()))
        }
    }
}

#[cfg(feature = "live-http")]
fn exchange(
    provider: AccountProvider,
    connection: &AccountConnection,
    call: impl FnOnce(&str) -> Result<String, String>,
) -> Result<String, String> {
    call(connection.access_token.as_str())
        .map_err(|error| scope_failure_hint(provider, &connection.scope, error))
}

#[cfg(test)]
mod tests {
    #[cfg(not(feature = "live-http"))]
    use super::{run_calendar_create, run_calendar_delete, run_calendar_update};
    use super::{scope_allows_calendar_write, scope_failure_hint};
    use softwake_providers::AccountProvider;
    #[cfg(not(feature = "live-http"))]
    use softwake_tools::{CalendarCreateArgs, CalendarDeleteArgs, CalendarUpdateArgs};

    #[test]
    fn calendar_write_scope_table() {
        let google = AccountProvider::Google;
        let microsoft = AccountProvider::Microsoft;
        assert!(scope_allows_calendar_write(google, ""));
        assert!(scope_allows_calendar_write(google, "   "));
        assert!(scope_allows_calendar_write(
            google,
            "https://www.googleapis.com/auth/calendar.events"
        ));
        assert!(scope_allows_calendar_write(
            google,
            "openid https://www.googleapis.com/auth/calendar.readonly https://www.googleapis.com/auth/calendar.events"
        ));
        assert!(!scope_allows_calendar_write(
            google,
            "https://www.googleapis.com/auth/calendar.readonly"
        ));
        assert!(!scope_allows_calendar_write(
            google,
            "https://www.googleapis.com/auth/gmail.send"
        ));
        assert!(scope_allows_calendar_write(microsoft, ""));
        assert!(scope_allows_calendar_write(
            microsoft,
            "Calendars.ReadWrite"
        ));
        assert!(scope_allows_calendar_write(
            microsoft,
            "Mail.Read Calendars.ReadWrite"
        ));
        assert!(!scope_allows_calendar_write(microsoft, "Calendars.Read"));
        assert!(!scope_allows_calendar_write(microsoft, "Mail.ReadWrite"));
    }

    #[test]
    fn calendar_write_scope_hint_appends_on_403_when_grant_lacks_write() {
        let denied = "cloud API HTTP 403: denied".to_owned();
        let hinted = scope_failure_hint(
            AccountProvider::Google,
            "https://www.googleapis.com/auth/calendar.readonly",
            denied.clone(),
        );
        assert!(hinted.contains("stored scope lacks calendar.events / Calendars.ReadWrite"));
        assert!(
            hinted.contains("Settings → Email → Accounts → Remove that account → Connect again")
        );
        assert!(hinted.starts_with(&denied));
        assert_eq!(
            scope_failure_hint(
                AccountProvider::Google,
                "https://www.googleapis.com/auth/calendar.events",
                denied.clone(),
            ),
            denied
        );
        assert_eq!(
            scope_failure_hint(AccountProvider::Microsoft, "", denied.clone()),
            denied
        );
        let network = "cloud API network transport failed".to_owned();
        assert_eq!(
            scope_failure_hint(
                AccountProvider::Microsoft,
                "Calendars.Read",
                network.clone()
            ),
            network
        );
        let microsoft = scope_failure_hint(AccountProvider::Microsoft, "Calendars.Read", denied);
        assert!(microsoft.contains("Calendars.ReadWrite"));
    }

    #[cfg(not(feature = "live-http"))]
    #[test]
    fn calendar_write_without_live_http_returns_live_required() {
        let create = CalendarCreateArgs {
            title: "Stand-up".to_owned(),
            start: "2026-09-28T09:00:00Z".to_owned(),
            end: "2026-09-28T09:15:00Z".to_owned(),
            location: None,
            description: None,
            account: None,
        };
        assert_eq!(
            run_calendar_create(&create).expect_err("offline create"),
            crate::cloud_tools::LIVE_REQUIRED
        );
        let update = CalendarUpdateArgs {
            id: "evt-1".to_owned(),
            title: Some("Moved".to_owned()),
            start: None,
            end: None,
            location: None,
            description: None,
            account: Some("ada@example.com".to_owned()),
        };
        assert_eq!(
            run_calendar_update(&update).expect_err("offline update"),
            crate::cloud_tools::LIVE_REQUIRED
        );
        let delete = CalendarDeleteArgs {
            id: "evt-1".to_owned(),
            account: None,
        };
        assert_eq!(
            run_calendar_delete(&delete).expect_err("offline delete"),
            crate::cloud_tools::LIVE_REQUIRED
        );
    }
}
