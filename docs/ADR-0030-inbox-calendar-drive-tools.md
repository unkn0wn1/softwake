# ADR-0030 — Inbox, calendar, Drive, and skill read tools

- **Status:** Accepted
- **Date:** 2026-09-27

## Context

Email OAuth ([ADR-0023](ADR-0023-email-oauth.md)) stores Google and Microsoft
tokens with mail, calendar, and Drive scopes. Softwake still only exposes
confirm-gated `email_send` on the tool bus. Agents (Ask / Telegram) correctly
report that they lack inbox list/read tools. Operators also need upcoming
calendar events and Drive file list/metadata through the same connected
accounts, advertised like other non-deny tools ([ADR-0025](ADR-0025-api-tool-calling.md)).

## Decision

1. **Tools (OpenAI advertise + Hands):**

   | Tool | Purpose | Default permission |
   |---|---|---|
   | `email_list` | Recent inbox messages | Always allow |
   | `email_search` | Gmail `q` / Graph `$search` | Ask |
   | `email_get` | One message by id | Ask |
   | `calendar_list` | Upcoming events (default 7 days) | Always allow |
   | `calendar_get` | One event by id | Ask |
   | `drive_list` | Files visible under OAuth scopes | Always allow |
   | `drive_search` | Name / query search | Ask |
   | `drive_get` | Metadata; optional cheap text body | Ask |
   | `skill_list` | List saved skills (id + title) | Always allow |
   | `skill_get` | Full skill by id | Ask |

   Registry floor is confirm (world I/O). Operator Always allow / Ask / Deny
   still apply via Tools Settings and the live permissions appendix. Deny is
   omitted from the chat `tools` array.

2. **Backends:** Prefer Google when connected, else Microsoft. Live HTTPS only
   under daemon `live-http` (same as Telegram / chat). Pure URL builders and
   JSON parsers live in `softwake-connectors` (network-free; MeetRec-shaped).
   Tokens stay in the secret bag; refresh in-process when near expiry; never
   log tokens.

3. **Connector registry:** Add confirm actions `email`/`list`, `email`/`search`,
   `email`/`get`, `calendar`/`get`, `drive`/`search`, `drive`/`get`. Existing
   `drive`/`list` and `calendar`/`list` stay confirm. Email and Drive deletes
   stay deny. Calendar event delete is confirm in
   [ADR-0034](ADR-0034-calendar-write.md) (event delete only, not a connector
   deregister).

4. **Scope honesty:** Google Connect uses `gmail.readonly`,
   `calendar.readonly`, `calendar.events`, and `drive.readonly`. Microsoft uses
   `Mail.Read`, `Calendars.ReadWrite`, and `Files.Read`. Drive list/search/get
   therefore see the user’s Drive / OneDrive (not only Softwake-touched files or
   App Folder children). See [ADR-0035](ADR-0035-drive-read-scope.md). Earlier
   builds used `drive.file` / `Files.ReadWrite.AppFolder`; document the widen and
   the Remove → Connect requirement when stored `scope` lacks the new grants.
5. **PROTOCOL_VERSION** stays 1. No new socket messages; tools ride the existing
   Hands / API tool-calling path.

6. **Out of scope:** SMTP client (`LiveEmailMode::Send` stays unwired), Drive
   upload/trash, CSAM/content scanning. Calendar event create, update, and
   delete are in scope via [ADR-0034](ADR-0034-calendar-write.md). Confirmed `email_send`
   over Gmail `users.messages.send` and Graph `sendMail` is in scope on a
   `live-http` daemon when a usable Email OAuth account is connected (amendment
   below).

## Consequences

- Agents stop claiming they lack inbox tools once OAuth is connected and the
  new tools are not Deny. Agents can list/read saved skills without only seeing
  `skill_save`.
- CI stays offline: parsers and registry tests use fixtures; live HTTP is
  feature-gated.
- Operators may set list tools to Always allow for low-friction “what’s in my
  inbox / calendar” asks; get/search stay Ask by default.

## Demo

```bash
cargo test -p softwake-connectors -p softwake-tools -p softwake-policy -p softwake-daemon
cargo check -p softwake-daemon --features live-http
```

Manual: Connect Google (or Microsoft) in Settings → Email, set tool permissions,
awake ask “list my recent email” / “what’s on my calendar”.

## Amendment — Gmail API 403 vs missing scopes (2026-09-27)

`email_list` / `email_search` / `email_get` call Gmail over the Email OAuth
bearer. Two different failures look similar in the HUD:

1. **Gmail API disabled** on the publisher GCP project (common when Softwake
   reuses a MeetRec project that only enabled Calendar + Drive). Symptom:
   calendar/Drive tools work; inbox returns HTTP 403; Google’s JSON body says
   enable `gmail.googleapis.com`. Fix: enable Gmail API ([oauth-clients.md](oauth-clients.md));
   reconnect not required if `scope` already has `gmail.readonly`. Sally `/refresh` is unrelated — Softwake reads the live secret bag (and refreshes the access token near expiry) on each tool call.
2. **Mail scopes not on the token** (connected before Softwake requested mail,
   or consent skipped mail). Symptom: `scope` lacks `gmail.readonly`. Fix:
   Settings → Email → Disconnect Google → Connect again.

Daemon live-http errors now surface a short, token-free hint from the provider
error body (and a Softwake reconnect / Gmail API note on HTTP 403) instead of
only `cloud API HTTP 403`.

## Amendment — OAuth `email_send` (2026-09-27)

Confirmed `email_send` delivers through the connected Email OAuth account when
the daemon is built with `live-http`: Gmail `POST users.messages.send` or
Microsoft Graph `POST /me/sendMail` (HTTP 202, empty body, no message id).
Google is preferred when both providers are connected. The usable-account probe
is local (secret bag). The POST is the only network step.

A build without `live-http` does not take the OAuth send path, even if a secret
bag exists on disk — the same rule as inbox tools. “OAuth connected” for send
implies that live daemon feature.

No usable account falls through to the mock outbox, live draft, or SMTP
`TransportNotWired`. An error after an account is selected (refresh, HTTP 403,
network) is returned and does not fall through to SMTP.

Reconnect is required only when the stored `scope` lacks `gmail.send` (Google)
or `Mail.Send` (Microsoft). Enabling the Gmail API stays required, same 403
class as inbox. Token refresh does not enlarge the grant.

## Amendment — multiple Email accounts (2026-09-27)

Inbox, calendar, Drive, and confirmed `email_send` choose an account with
`resolve_account` ([ADR-0033](ADR-0033-multi-account-oauth.md)). With no
`account` argument, one usable account is that account. If more than one
account is usable and any Google account is usable, tools use the active Google
account (the first Google row when none is marked). Otherwise they use the
active Microsoft account. `account` (connection id or email substring) selects
a different row, including Microsoft when Google is also connected. Refresh
updates that row in place.

The SMTP client and Drive upload/trash stay out of scope. Calendar event
create, update, and delete are [ADR-0034](ADR-0034-calendar-write.md).

## Amendment — calendar write scopes and tools (2026-09-28)

`calendar_create`, `calendar_update`, and `calendar_delete` are confirm-gated
(Ask by default) on the primary calendar. Google Connect keeps
`calendar.readonly` and adds `calendar.events`. Microsoft Connect uses
`Calendars.ReadWrite` instead of `Calendars.Read`. Token refresh does not
enlarge an existing grant: Settings → Email → Accounts → Remove that account →
Connect again, unless stored `scope` already lists the write scope. See
[ADR-0034](ADR-0034-calendar-write.md). Drive upload/trash stay out of scope.

## Amendment — Drive read scope widen (2026-09-28)

Connect now requests Google `drive.readonly` and Microsoft `Files.Read`. Graph
list/search use `/me/drive/root/…` instead of App Folder. See
[ADR-0035](ADR-0035-drive-read-scope.md). Token refresh does not enlarge an
older grant: Settings → Email → Accounts → Remove → Connect.
