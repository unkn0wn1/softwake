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
   `drive`/`list` and `calendar`/`list` stay confirm. Deletes stay deny.

4. **Scope honesty:** Google Connect still uses `gmail.readonly`,
   `calendar.readonly`, and `drive.file`. Drive list/search therefore only sees
   files the Softwake client created or the user opened with it — not the whole
   Drive. Microsoft uses `Mail.Read`, `Calendars.Read`, and
   `Files.ReadWrite.AppFolder` (App Folder children only). Document this in the
   appendix; do not claim full-mailbox/Drive access beyond those scopes.

5. **PROTOCOL_VERSION** stays 1. No new socket messages; tools ride the existing
   Hands / API tool-calling path.

6. **Out of scope:** Live SMTP/`email_send` transport, calendar writes, Drive
   upload/trash, CSAM/content scanning.

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
