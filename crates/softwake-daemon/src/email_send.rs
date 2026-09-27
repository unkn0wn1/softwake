//! Confirmed `email_send` over Gmail and Microsoft Graph.
//!
//! Builders live in `softwake-connectors` and do not open a socket. This module
//! posts them when `live-http` is on, through [`crate::cloud_tools::with_account`].
//! SMTP send stays unwired.

use softwake_connectors::OutboundEmail;
#[cfg(feature = "live-http")]
use softwake_connectors::{
    gmail_send_body, gmail_send_url, graph_send_mail_body, graph_send_mail_url, parse_gmail_send_id,
};
#[cfg(feature = "live-http")]
use softwake_providers::AccountConnection;
#[cfg(any(test, feature = "live-http"))]
use softwake_providers::AccountProvider;

#[cfg(feature = "live-http")]
use crate::cloud_tools::{cloud_http_error, with_account};

/// Where a confirmed `email_send` goes after the policy gates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EmailTransport {
    /// Mock outbox, live draft, or the SMTP scaffold.
    Backend,
    /// Gmail `users.messages.send` or Graph `sendMail`.
    OAuth,
}

/// OAuth when a usable account was already selected; otherwise the email backend.
#[must_use]
pub(crate) fn choose_email_transport(oauth_usable: bool) -> EmailTransport {
    if oauth_usable {
        EmailTransport::OAuth
    } else {
        EmailTransport::Backend
    }
}

/// Post `message` with the preferred Email OAuth account.
///
/// Without `live-http` this returns [`crate::cloud_tools::LIVE_REQUIRED`] and
/// does not read the secret bag. Callers must not treat that as a reason to
/// skip a local probe: [`crate::cloud_tools::oauth_send_available`] is the probe.
///
/// # Errors
///
/// Refresh failure, HTTP failure, or a Gmail response without an id. A failure
/// here is the operator-facing result; callers must not fall through to SMTP.
pub(crate) fn run_email_send(message: &OutboundEmail) -> Result<String, String> {
    #[cfg(not(feature = "live-http"))]
    {
        let _ = message;
        Err(crate::cloud_tools::LIVE_REQUIRED.to_owned())
    }
    #[cfg(feature = "live-http")]
    {
        send_live(message)
    }
}

/// Empty `scope` is unknown and still allows the POST.
#[cfg(any(test, feature = "live-http"))]
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

/// Append the reconnect clause only when a stored scope is present and lacks send.
#[cfg(any(test, feature = "live-http"))]
fn scope_failure_hint(provider: AccountProvider, scope: &str, error: String) -> String {
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

#[cfg(feature = "live-http")]
fn send_live(message: &OutboundEmail) -> Result<String, String> {
    with_account(|provider, connection| match provider {
        AccountProvider::Google => {
            let from = from_address(connection);
            let payload = gmail_send_body(&message.to, &message.subject, &message.body, from);
            let response = post_for_send(provider, connection, &gmail_send_url(), &payload)?;
            let id = parse_gmail_send_id(&response)?;
            Ok(format!("sent gmail {id}"))
        }
        AccountProvider::Microsoft => {
            let payload = graph_send_mail_body(&message.to, &message.subject, &message.body);
            post_for_send(provider, connection, &graph_send_mail_url(), &payload)?;
            Ok("sent graph".to_owned())
        }
    })
}

#[cfg(feature = "live-http")]
fn from_address(connection: &AccountConnection) -> Option<&str> {
    connection
        .account_email
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

#[cfg(feature = "live-http")]
fn post_for_send(
    provider: AccountProvider,
    connection: &AccountConnection,
    url: &str,
    payload: &str,
) -> Result<String, String> {
    post_json(url, &connection.access_token, payload)
        .map_err(|error| scope_failure_hint(provider, &connection.scope, error))
}

/// POST JSON with the same timeouts and error dialect as cloud GET.
///
/// Accepts HTTP 200..299, including Graph 202 with an empty body.
#[cfg(feature = "live-http")]
fn post_json(url: &str, bearer: &str, payload: &str) -> Result<String, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(10))
        .timeout_read(std::time::Duration::from_secs(30))
        .timeout(std::time::Duration::from_secs(30))
        .build();
    let response = match agent
        .post(url)
        .set("Authorization", &format!("Bearer {bearer}"))
        .set("Content-Type", "application/json")
        .send_string(payload)
    {
        Ok(response) | Err(ureq::Error::Status(_, response)) => response,
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

#[cfg(test)]
mod tests {
    #[cfg(not(feature = "live-http"))]
    use super::run_email_send;
    use super::{EmailTransport, choose_email_transport, scope_allows_send, scope_failure_hint};
    #[cfg(not(feature = "live-http"))]
    use softwake_connectors::OutboundEmail;
    use softwake_providers::AccountProvider;

    #[test]
    fn email_send_choose_transport_prefers_oauth_when_usable() {
        assert_eq!(choose_email_transport(false), EmailTransport::Backend);
        assert_eq!(choose_email_transport(true), EmailTransport::OAuth);
    }

    #[test]
    fn email_send_scope_allows_send_table() {
        let google = AccountProvider::Google;
        let microsoft = AccountProvider::Microsoft;
        assert!(scope_allows_send(google, ""));
        assert!(scope_allows_send(google, "   "));
        assert!(scope_allows_send(
            google,
            "https://www.googleapis.com/auth/gmail.send"
        ));
        assert!(scope_allows_send(
            google,
            "openid https://www.googleapis.com/auth/gmail.send"
        ));
        assert!(scope_allows_send(google, "gmail.send"));
        assert!(!scope_allows_send(
            google,
            "https://www.googleapis.com/auth/gmail.readonly"
        ));
        assert!(scope_allows_send(microsoft, ""));
        assert!(scope_allows_send(microsoft, "Mail.Send"));
        assert!(scope_allows_send(microsoft, "Mail.Read Mail.Send"));
        assert!(!scope_allows_send(microsoft, "Mail.Read"));
        assert!(!scope_allows_send(microsoft, "Mail.ReadWrite"));
    }

    #[test]
    fn email_send_scope_hint_appends_when_grant_lacks_send() {
        let denied = "cloud API HTTP 403: denied".to_owned();
        let hinted = scope_failure_hint(
            AccountProvider::Google,
            "https://www.googleapis.com/auth/gmail.readonly",
            denied.clone(),
        );
        assert!(hinted.contains("stored scope lacks gmail.send / Mail.Send"));
        assert!(hinted.contains("Disconnect and Connect in Settings → Email"));
        assert!(hinted.starts_with(&denied));
        assert_eq!(
            scope_failure_hint(
                AccountProvider::Google,
                "https://www.googleapis.com/auth/gmail.send",
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
            scope_failure_hint(AccountProvider::Microsoft, "Mail.Read", network.clone()),
            network
        );
        let microsoft = scope_failure_hint(AccountProvider::Microsoft, "Mail.Read", denied);
        assert!(microsoft.contains("Mail.Send"));
    }

    #[cfg(not(feature = "live-http"))]
    #[test]
    fn email_send_without_live_http_returns_live_required() {
        let message = OutboundEmail {
            to: "ada@example.com".to_owned(),
            subject: "hello".to_owned(),
            body: "a short note".to_owned(),
        };
        let err = run_email_send(&message).expect_err("offline");
        assert_eq!(err, crate::cloud_tools::LIVE_REQUIRED);
    }
}
