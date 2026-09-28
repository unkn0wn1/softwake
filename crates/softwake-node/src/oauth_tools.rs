//! Companion OAuth cloud tools (ADR-0045).
//!
//! Uses mirrored `vault/oauth.json` + softwake-connectors URL builders +
//! softwake-providers [`Transport`]. Does not touch the laptop keyring.

use softwake_connectors::{
    CalendarEventWrite, clamp_calendar_days, clamp_calendar_max, clamp_drive_max, clamp_inbox_max,
    format_calendar_created, format_calendar_deleted, format_calendar_event, format_calendar_list,
    format_calendar_updated, format_drive_file, format_drive_list, format_inbox_list,
    format_inbox_message, gmail_get_url, gmail_list_url, gmail_send_body, gmail_send_url,
    google_drive_export_text_url, google_drive_get_url, google_drive_list_url,
    google_drive_media_url, google_event_create_url, google_event_delete_url, google_event_get_url,
    google_event_patch_url, google_event_write_body, google_events_url, graph_calendar_view_url,
    graph_drive_content_url, graph_drive_item_url, graph_drive_root_children_url,
    graph_drive_root_search_url, graph_event_create_url, graph_event_delete_url,
    graph_event_get_url, graph_event_patch_url, graph_event_write_body, graph_get_url,
    graph_list_url, graph_send_mail_body, graph_send_mail_url, is_cheap_text_mime,
    parse_gmail_list, parse_gmail_message, parse_gmail_send_id, parse_google_drive_file,
    parse_google_drive_list, parse_google_event, parse_google_events, parse_graph_drive_file,
    parse_graph_drive_list, parse_graph_event, parse_graph_events, parse_graph_list,
    parse_graph_message, parse_written_event_id, truncate_drive_text,
};
use softwake_providers::{
    AccountConnection, AccountProvider, OauthMirrorDocument, SecretBag, Transport,
    ensure_fresh_account,
};
use softwake_tools::{
    CALENDAR_CREATE_TOOL, CALENDAR_DELETE_TOOL, CALENDAR_GET_TOOL, CALENDAR_LIST_TOOL,
    CALENDAR_UPDATE_TOOL, DRIVE_GET_TOOL, DRIVE_LIST_TOOL, DRIVE_SEARCH_TOOL, EMAIL_GET_TOOL,
    EMAIL_LIST_TOOL, EMAIL_SEARCH_TOOL, EMAIL_SEND_TOOL, parse_calendar_create_args,
    parse_calendar_delete_args, parse_calendar_get_args, parse_calendar_list_args,
    parse_calendar_update_args, parse_drive_get_args, parse_drive_list_args,
    parse_drive_search_args, parse_email_get_args, parse_email_list_args, parse_email_search_args,
    parse_email_send_args,
};

use crate::state::NodeState;

/// Same words as daemon `LIVE_REQUIRED`.
#[allow(dead_code, reason = "used when live-http is off")]
pub const LIVE_REQUIRED: &str =
    "live-http is required for inbox/calendar/Drive tools in this build";

/// Enable-hint when the node vault is empty (ADR-0045).
#[must_use]
pub fn oauth_enable_hint(name: &str) -> String {
    format!(
        "tool `{name}` needs OAuth tokens mirrored to the companion. Enable “Mirror OAuth tokens to companion” in Settings → Remote Agent (default off)."
    )
}

/// Run one OAuth-backed companion tool against the mirrored vault.
pub fn run_oauth_tool(
    state: &NodeState,
    name: &str,
    args: &[String],
    transport: &dyn Transport,
) -> String {
    match run_oauth_tool_inner(state, name, args, transport) {
        Ok(text) => text,
        Err(err) => err,
    }
}

fn run_oauth_tool_inner(
    state: &NodeState,
    name: &str,
    args: &[String],
    transport: &dyn Transport,
) -> Result<String, String> {
    let doc = state.load_oauth_document();
    let (provider, mut connection, hint) = resolve_from_doc(&doc, name, args)?;
    let now = NodeState::now_ms();
    let fresh = ensure_fresh_account(provider, transport, &connection, now)?;
    if fresh.access_token != connection.access_token
        || fresh.expires_at_ms != connection.expires_at_ms
        || fresh.refresh_token != connection.refresh_token
    {
        let mut next = doc;
        upsert_connection(&mut next, provider, fresh.clone());
        let _ = state.put_oauth_document(&next);
        connection = fresh;
    }
    dispatch(
        name,
        args,
        provider,
        &connection,
        hint.as_deref(),
        transport,
    )
}

fn resolve_from_doc(
    doc: &OauthMirrorDocument,
    name: &str,
    args: &[String],
) -> Result<(AccountProvider, AccountConnection, Option<String>), String> {
    let hint = account_hint(name, args)?;
    let mut bag = SecretBag::empty();
    bag.google_connections.clone_from(&doc.google_connections);
    bag.microsoft_connections
        .clone_from(&doc.microsoft_connections);
    bag.active_google_connection_id
        .clone_from(&doc.active_google_connection_id);
    bag.active_microsoft_connection_id
        .clone_from(&doc.active_microsoft_connection_id);
    let (provider, connection) = bag.resolve_account(hint.as_deref())?;
    Ok((provider, connection, hint))
}

fn account_hint(name: &str, args: &[String]) -> Result<Option<String>, String> {
    let hint = match name {
        EMAIL_LIST_TOOL => parse_email_list_args(args)
            .map(|a| a.account)
            .map_err(|e| e.to_string())?,
        EMAIL_SEARCH_TOOL => parse_email_search_args(args)
            .map(|a| a.account)
            .map_err(|e| e.to_string())?,
        EMAIL_GET_TOOL => parse_email_get_args(args)
            .map(|a| a.account)
            .map_err(|e| e.to_string())?,
        EMAIL_SEND_TOOL => parse_email_send_args(args)
            .map(|a| a.account)
            .map_err(|e| e.to_string())?,
        CALENDAR_LIST_TOOL => parse_calendar_list_args(args)
            .map(|a| a.account)
            .map_err(|e| e.to_string())?,
        CALENDAR_GET_TOOL => parse_calendar_get_args(args)
            .map(|a| a.account)
            .map_err(|e| e.to_string())?,
        CALENDAR_CREATE_TOOL => parse_calendar_create_args(args)
            .map(|a| a.account)
            .map_err(|e| e.to_string())?,
        CALENDAR_UPDATE_TOOL => parse_calendar_update_args(args)
            .map(|a| a.account)
            .map_err(|e| e.to_string())?,
        CALENDAR_DELETE_TOOL => parse_calendar_delete_args(args)
            .map(|a| a.account)
            .map_err(|e| e.to_string())?,
        DRIVE_LIST_TOOL => parse_drive_list_args(args)
            .map(|a| a.account)
            .map_err(|e| e.to_string())?,
        DRIVE_SEARCH_TOOL => parse_drive_search_args(args)
            .map(|a| a.account)
            .map_err(|e| e.to_string())?,
        DRIVE_GET_TOOL => parse_drive_get_args(args)
            .map(|a| a.account)
            .map_err(|e| e.to_string())?,
        other => return Err(format!("unknown oauth tool `{other}`")),
    };
    Ok(hint)
}

fn upsert_connection(
    doc: &mut OauthMirrorDocument,
    provider: AccountProvider,
    connection: AccountConnection,
) {
    let list = match provider {
        AccountProvider::Google => &mut doc.google_connections,
        AccountProvider::Microsoft => &mut doc.microsoft_connections,
    };
    if let Some(existing) = list.iter_mut().find(|c| c.id == connection.id) {
        *existing = connection;
    } else {
        list.push(connection);
    }
}

fn dispatch(
    name: &str,
    args: &[String],
    provider: AccountProvider,
    connection: &AccountConnection,
    _hint: Option<&str>,
    transport: &dyn Transport,
) -> Result<String, String> {
    match name {
        EMAIL_LIST_TOOL => {
            let parsed = parse_email_list_args(args).map_err(|e| e.to_string())?;
            email_list(provider, connection, &parsed, transport)
        }
        EMAIL_SEARCH_TOOL => {
            let parsed = parse_email_search_args(args).map_err(|e| e.to_string())?;
            email_search(provider, connection, &parsed, transport)
        }
        EMAIL_GET_TOOL => {
            let parsed = parse_email_get_args(args).map_err(|e| e.to_string())?;
            email_get(provider, connection, &parsed, transport)
        }
        EMAIL_SEND_TOOL => {
            let parsed = parse_email_send_args(args).map_err(|e| e.to_string())?;
            email_send(provider, connection, &parsed, transport)
        }
        CALENDAR_LIST_TOOL => {
            let parsed = parse_calendar_list_args(args).map_err(|e| e.to_string())?;
            calendar_list(provider, connection, &parsed, transport)
        }
        CALENDAR_GET_TOOL => {
            let parsed = parse_calendar_get_args(args).map_err(|e| e.to_string())?;
            calendar_get(provider, connection, &parsed, transport)
        }
        CALENDAR_CREATE_TOOL => {
            let parsed = parse_calendar_create_args(args).map_err(|e| e.to_string())?;
            calendar_create(provider, connection, &parsed, transport)
        }
        CALENDAR_UPDATE_TOOL => {
            let parsed = parse_calendar_update_args(args).map_err(|e| e.to_string())?;
            calendar_update(provider, connection, &parsed, transport)
        }
        CALENDAR_DELETE_TOOL => {
            let parsed = parse_calendar_delete_args(args).map_err(|e| e.to_string())?;
            calendar_delete(provider, connection, &parsed, transport)
        }
        DRIVE_LIST_TOOL => {
            let parsed = parse_drive_list_args(args).map_err(|e| e.to_string())?;
            drive_list(provider, connection, &parsed, transport)
        }
        DRIVE_SEARCH_TOOL => {
            let parsed = parse_drive_search_args(args).map_err(|e| e.to_string())?;
            drive_search(provider, connection, &parsed, transport)
        }
        DRIVE_GET_TOOL => {
            let parsed = parse_drive_get_args(args).map_err(|e| e.to_string())?;
            drive_get(provider, connection, &parsed, transport)
        }
        other => Err(format!("unknown oauth tool `{other}`")),
    }
}

fn get_json(
    transport: &dyn Transport,
    url: &str,
    bearer: &str,
    headers: &[(&str, &str)],
) -> Result<String, String> {
    let response = if headers.is_empty() {
        transport.get_bearer(url, bearer)
    } else {
        transport.get_bearer_with_headers(url, bearer, headers)
    }
    .map_err(|e| e.to_string())?;
    if !(200..300).contains(&response.status) {
        return Err(cloud_http_error(response.status, &response.body, bearer));
    }
    Ok(response.body)
}

fn post_json(
    transport: &dyn Transport,
    url: &str,
    bearer: &str,
    body: &str,
) -> Result<String, String> {
    let response = transport
        .post_json_bearer(url, bearer, body)
        .map_err(|e| e.to_string())?;
    if !(200..300).contains(&response.status) {
        return Err(cloud_http_error(response.status, &response.body, bearer));
    }
    Ok(response.body)
}

fn patch_json(
    transport: &dyn Transport,
    url: &str,
    bearer: &str,
    body: &str,
) -> Result<String, String> {
    let response = transport
        .patch_json_bearer(url, bearer, body)
        .map_err(|e| e.to_string())?;
    if !(200..300).contains(&response.status) {
        return Err(cloud_http_error(response.status, &response.body, bearer));
    }
    Ok(response.body)
}

fn delete_req(transport: &dyn Transport, url: &str, bearer: &str) -> Result<String, String> {
    let response = transport
        .delete_bearer(url, bearer)
        .map_err(|e| e.to_string())?;
    if !(200..300).contains(&response.status) {
        return Err(cloud_http_error(response.status, &response.body, bearer));
    }
    Ok(response.body)
}

fn cloud_http_error(status: u16, body: &str, bearer: &str) -> String {
    let mut message = format!("cloud API HTTP {status}");
    if let Some(detail) = cloud_error_detail(body) {
        let safe = if !bearer.is_empty() && detail.contains(bearer) {
            detail.replace(bearer, "<redacted>")
        } else {
            detail
        };
        message.push_str(": ");
        message.push_str(&safe);
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
            .and_then(serde_json::Value::as_str)
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
    let mut out = String::new();
    for (i, ch) in detail.chars().enumerate() {
        if i >= MAX {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

fn email_list(
    provider: AccountProvider,
    connection: &AccountConnection,
    args: &softwake_tools::EmailListArgs,
    transport: &dyn Transport,
) -> Result<String, String> {
    let max = clamp_inbox_max(args.max_results);
    let token = connection.access_token.as_str();
    match provider {
        AccountProvider::Google => {
            let url = gmail_list_url(max, None);
            let body = get_json(transport, &url, token, &[])?;
            let stubs = parse_gmail_list(&body)?;
            let mut messages = Vec::new();
            for stub in stubs.into_iter().take(max as usize) {
                let get_url = gmail_get_url(&stub.id);
                if let Ok(raw) = get_json(transport, &get_url, token, &[]) {
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
            let body = get_json(transport, &url, token, &[])?;
            let messages = parse_graph_list(&body)?;
            Ok(format_inbox_list("graph", &messages))
        }
    }
}

fn email_search(
    provider: AccountProvider,
    connection: &AccountConnection,
    args: &softwake_tools::EmailSearchArgs,
    transport: &dyn Transport,
) -> Result<String, String> {
    let max = clamp_inbox_max(args.max_results);
    let query = args.query.trim();
    if query.is_empty() {
        return Err("email_search needs query".to_owned());
    }
    let token = connection.access_token.as_str();
    match provider {
        AccountProvider::Google => {
            let url = gmail_list_url(max, Some(query));
            let body = get_json(transport, &url, token, &[])?;
            let stubs = parse_gmail_list(&body)?;
            let mut messages = Vec::new();
            for stub in stubs.into_iter().take(max as usize) {
                let get_url = gmail_get_url(&stub.id);
                if let Ok(raw) = get_json(transport, &get_url, token, &[]) {
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
            let body = get_json(transport, &url, token, &[("ConsistencyLevel", "eventual")])?;
            let messages = parse_graph_list(&body)?;
            Ok(format_inbox_list("graph", &messages))
        }
    }
}

fn email_get(
    provider: AccountProvider,
    connection: &AccountConnection,
    args: &softwake_tools::EmailGetArgs,
    transport: &dyn Transport,
) -> Result<String, String> {
    let id = args.id.trim();
    if id.is_empty() {
        return Err("email_get needs id".to_owned());
    }
    let token = connection.access_token.as_str();
    match provider {
        AccountProvider::Google => {
            let url = gmail_get_url(id);
            let body = get_json(transport, &url, token, &[])?;
            let msg = parse_gmail_message(&body)?;
            Ok(format_inbox_message("gmail", &msg))
        }
        AccountProvider::Microsoft => {
            let url = graph_get_url(id);
            let body = get_json(transport, &url, token, &[])?;
            let msg = parse_graph_message(&body)?;
            Ok(format_inbox_message("graph", &msg))
        }
    }
}

fn scope_allows_send(provider: AccountProvider, scope: &str) -> bool {
    if scope.trim().is_empty() {
        return true;
    }
    match provider {
        AccountProvider::Google => {
            scope
                .split_whitespace()
                .any(|token| token == "https://www.googleapis.com/auth/gmail.send")
                || scope.contains("gmail.send")
        }
        AccountProvider::Microsoft => scope.split_whitespace().any(|token| token == "Mail.Send"),
    }
}

fn scope_send_hint(provider: AccountProvider, scope: &str, error: String) -> String {
    if error.contains("cloud API HTTP 403")
        && !scope.trim().is_empty()
        && !scope_allows_send(provider, scope)
    {
        format!(
            "{error} stored scope lacks gmail.send / Mail.Send; Disconnect and Connect in Settings → Email."
        )
    } else {
        error
    }
}

fn email_send(
    provider: AccountProvider,
    connection: &AccountConnection,
    args: &softwake_tools::EmailSendArgs,
    transport: &dyn Transport,
) -> Result<String, String> {
    let token = connection.access_token.as_str();
    match provider {
        AccountProvider::Google => {
            let from = connection
                .account_email
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty());
            let payload = gmail_send_body(&args.to, &args.subject, &args.body, from);
            let response = post_json(transport, &gmail_send_url(), token, &payload)
                .map_err(|e| scope_send_hint(provider, &connection.scope, e))?;
            let id = parse_gmail_send_id(&response)?;
            Ok(format!("sent gmail {id}"))
        }
        AccountProvider::Microsoft => {
            let payload = graph_send_mail_body(&args.to, &args.subject, &args.body);
            post_json(transport, &graph_send_mail_url(), token, &payload)
                .map_err(|e| scope_send_hint(provider, &connection.scope, e))?;
            Ok("sent graph".to_owned())
        }
    }
}

fn rfc3339_window(days: u32) -> (String, String) {
    let now = chrono::Utc::now();
    let end = now + chrono::Duration::days(i64::from(days));
    (
        now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        end.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    )
}

fn calendar_list(
    provider: AccountProvider,
    connection: &AccountConnection,
    args: &softwake_tools::CalendarListArgs,
    transport: &dyn Transport,
) -> Result<String, String> {
    let days = clamp_calendar_days(args.days);
    let max = clamp_calendar_max(args.max_results);
    let (time_min, time_max) = rfc3339_window(days);
    let token = connection.access_token.as_str();
    match provider {
        AccountProvider::Google => {
            let url = google_events_url(&time_min, &time_max, max);
            let body = get_json(transport, &url, token, &[])?;
            let events = parse_google_events(&body)?;
            Ok(format_calendar_list("google", &events))
        }
        AccountProvider::Microsoft => {
            let url = graph_calendar_view_url(&time_min, &time_max, max);
            let body = get_json(
                transport,
                &url,
                token,
                &[("Prefer", "outlook.timezone=\"UTC\"")],
            )?;
            let events = parse_graph_events(&body)?;
            Ok(format_calendar_list("graph", &events))
        }
    }
}

fn calendar_get(
    provider: AccountProvider,
    connection: &AccountConnection,
    args: &softwake_tools::CalendarGetArgs,
    transport: &dyn Transport,
) -> Result<String, String> {
    let id = args.id.trim();
    if id.is_empty() {
        return Err("calendar_get needs id".to_owned());
    }
    let token = connection.access_token.as_str();
    match provider {
        AccountProvider::Google => {
            let url = google_event_get_url(id);
            let body = get_json(transport, &url, token, &[])?;
            let event = parse_google_event(&body)?;
            Ok(format_calendar_event("google", &event))
        }
        AccountProvider::Microsoft => {
            let url = graph_event_get_url(id);
            let body = get_json(transport, &url, token, &[])?;
            let event = parse_graph_event(&body)?;
            Ok(format_calendar_event("graph", &event))
        }
    }
}

fn calendar_create(
    provider: AccountProvider,
    connection: &AccountConnection,
    args: &softwake_tools::CalendarCreateArgs,
    transport: &dyn Transport,
) -> Result<String, String> {
    let fields = CalendarEventWrite {
        title: Some(args.title.clone()),
        start: Some(args.start.clone()),
        end: Some(args.end.clone()),
        location: args.location.clone(),
        description: args.description.clone(),
    };
    let token = connection.access_token.as_str();
    match provider {
        AccountProvider::Google => {
            let body = post_json(
                transport,
                &google_event_create_url(),
                token,
                &google_event_write_body(&fields),
            )?;
            let id = parse_written_event_id(&body)?;
            Ok(format_calendar_created("google", &id))
        }
        AccountProvider::Microsoft => {
            let body = post_json(
                transport,
                &graph_event_create_url(),
                token,
                &graph_event_write_body(&fields),
            )?;
            let id = parse_written_event_id(&body)?;
            Ok(format_calendar_created("graph", &id))
        }
    }
}

fn calendar_update(
    provider: AccountProvider,
    connection: &AccountConnection,
    args: &softwake_tools::CalendarUpdateArgs,
    transport: &dyn Transport,
) -> Result<String, String> {
    let fields = CalendarEventWrite {
        title: args.title.clone(),
        start: args.start.clone(),
        end: args.end.clone(),
        location: args.location.clone(),
        description: args.description.clone(),
    };
    let token = connection.access_token.as_str();
    match provider {
        AccountProvider::Google => {
            let body = patch_json(
                transport,
                &google_event_patch_url(&args.id),
                token,
                &google_event_write_body(&fields),
            )?;
            let id = parse_written_event_id(&body)?;
            Ok(format_calendar_updated("google", &id))
        }
        AccountProvider::Microsoft => {
            let body = patch_json(
                transport,
                &graph_event_patch_url(&args.id),
                token,
                &graph_event_write_body(&fields),
            )?;
            let id = parse_written_event_id(&body)?;
            Ok(format_calendar_updated("graph", &id))
        }
    }
}

fn calendar_delete(
    provider: AccountProvider,
    connection: &AccountConnection,
    args: &softwake_tools::CalendarDeleteArgs,
    transport: &dyn Transport,
) -> Result<String, String> {
    let token = connection.access_token.as_str();
    match provider {
        AccountProvider::Google => {
            delete_req(transport, &google_event_delete_url(&args.id), token)?;
            Ok(format_calendar_deleted("google", args.id.trim()))
        }
        AccountProvider::Microsoft => {
            delete_req(transport, &graph_event_delete_url(&args.id), token)?;
            Ok(format_calendar_deleted("graph", args.id.trim()))
        }
    }
}

fn drive_list(
    provider: AccountProvider,
    connection: &AccountConnection,
    args: &softwake_tools::DriveListArgs,
    transport: &dyn Transport,
) -> Result<String, String> {
    let max = clamp_drive_max(args.max_results);
    let token = connection.access_token.as_str();
    match provider {
        AccountProvider::Google => {
            let url = google_drive_list_url(max, None);
            let body = get_json(transport, &url, token, &[])?;
            let files = parse_google_drive_list(&body)?;
            Ok(format_drive_list("google", &files))
        }
        AccountProvider::Microsoft => {
            let url = graph_drive_root_children_url(max);
            let body = get_json(transport, &url, token, &[])?;
            let files = parse_graph_drive_list(&body)?;
            Ok(format_drive_list("graph", &files))
        }
    }
}

fn drive_search(
    provider: AccountProvider,
    connection: &AccountConnection,
    args: &softwake_tools::DriveSearchArgs,
    transport: &dyn Transport,
) -> Result<String, String> {
    let max = clamp_drive_max(args.max_results);
    let query = args.query.trim();
    if query.is_empty() {
        return Err("drive_search needs query".to_owned());
    }
    let token = connection.access_token.as_str();
    match provider {
        AccountProvider::Google => {
            let url = google_drive_list_url(max, Some(query));
            let body = get_json(transport, &url, token, &[])?;
            let files = parse_google_drive_list(&body)?;
            Ok(format_drive_list("google", &files))
        }
        AccountProvider::Microsoft => {
            let url = graph_drive_root_search_url(query, max);
            let body = get_json(transport, &url, token, &[])?;
            let files = parse_graph_drive_list(&body)?;
            Ok(format_drive_list("graph", &files))
        }
    }
}

fn drive_get(
    provider: AccountProvider,
    connection: &AccountConnection,
    args: &softwake_tools::DriveGetArgs,
    transport: &dyn Transport,
) -> Result<String, String> {
    let id = args.id.trim();
    if id.is_empty() {
        return Err("drive_get needs id".to_owned());
    }
    let token = connection.access_token.as_str();
    match provider {
        AccountProvider::Google => {
            let url = google_drive_get_url(id);
            let body = get_json(transport, &url, token, &[])?;
            let mut file = parse_google_drive_file(&body)?;
            if args.read_text && is_cheap_text_mime(&file.mime_type) {
                let text_url = if file.mime_type == "application/vnd.google-apps.document" {
                    google_drive_export_text_url(id)
                } else {
                    google_drive_media_url(id)
                };
                if let Ok(raw) = get_json(transport, &text_url, token, &[]) {
                    file.text = truncate_drive_text(&raw);
                }
            }
            Ok(format_drive_file("google", &file))
        }
        AccountProvider::Microsoft => {
            let url = graph_drive_item_url(id);
            let body = get_json(transport, &url, token, &[])?;
            let mut file = parse_graph_drive_file(&body)?;
            if args.read_text && is_cheap_text_mime(&file.mime_type) {
                let content_url = graph_drive_content_url(id);
                if let Ok(raw) = get_json(transport, &content_url, token, &[]) {
                    file.text = truncate_drive_text(&raw);
                }
            }
            Ok(format_drive_file("graph", &file))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use softwake_providers::{HttpResponse, MockTransport};

    fn fixture_conn(id: &str, email: &str, access: &str) -> AccountConnection {
        AccountConnection {
            id: id.into(),
            access_token: access.into(),
            refresh_token: "refresh-keep".into(),
            expires_at_ms: 9_999_999_999_999,
            token_type: "Bearer".into(),
            scope: "https://www.googleapis.com/auth/gmail.readonly".into(),
            account_email: Some(email.into()),
        }
    }

    #[test]
    fn enable_hint_mentions_oauth() {
        let msg = oauth_enable_hint("email_list");
        assert!(msg.contains("OAuth"));
        assert!(msg.contains("Mirror OAuth"));
    }

    #[test]
    fn email_list_mock_formats_inbox_without_leaking_token() {
        let dir = tempfile_dir();
        let state = NodeState::open(dir);
        let access = "sentinel-access-token-xyz";
        let doc = OauthMirrorDocument {
            google_connections: vec![fixture_conn("g1", "ada@example.com", access)],
            ..OauthMirrorDocument::default()
        };
        state.put_oauth_document(&doc).expect("put");
        let list_url = gmail_list_url(10, None);
        let transport = MockTransport::new().with_get(
            list_url,
            HttpResponse {
                status: 200,
                body: r#"{"messages":[{"id":"m1","threadId":"t1"}]}"#.into(),
            },
        ).with_get(
            gmail_get_url("m1"),
            HttpResponse {
                status: 200,
                body: r#"{"id":"m1","threadId":"t1","snippet":"hi","payload":{"headers":[{"name":"Subject","value":"Hello"},{"name":"From","value":"bob@example.com"}]}}"#.into(),
            },
        );
        let out = run_oauth_tool(&state, EMAIL_LIST_TOOL, &[], &transport);
        assert!(
            out.contains("Hello") || out.contains("bob@example.com") || out.contains("m1"),
            "{out}"
        );
        assert!(!out.contains(access), "{out}");
    }

    #[test]
    fn account_hint_picks_second_google_row() {
        let dir = tempfile_dir();
        let state = NodeState::open(dir);
        let doc = OauthMirrorDocument {
            google_connections: vec![
                fixture_conn("g1", "ada@example.com", "tok-a"),
                fixture_conn("g2", "bea@example.com", "tok-b"),
            ],
            active_google_connection_id: Some("g1".into()),
            ..OauthMirrorDocument::default()
        };
        state.put_oauth_document(&doc).expect("put");
        let list_url = gmail_list_url(10, None);
        let transport = MockTransport::new().with_get(
            list_url,
            HttpResponse {
                status: 200,
                body: r#"{"messages":[]}"#.into(),
            },
        );
        let out = run_oauth_tool(&state, EMAIL_LIST_TOOL, &["account=g2".into()], &transport);
        // empty list still formats without error
        assert!(!out.contains("no connected account"), "{out}");
        assert!(!out.contains("tok-a"), "{out}");
        assert!(!out.contains("tok-b"), "{out}");
    }

    fn tempfile_dir() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "softwake-oauth-tools-{}-{}-{}",
            std::process::id(),
            NodeState::now_ms(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("tempdir");
        dir
    }
}
