# ADR-0023 — Email OAuth (Google and Microsoft)

- **Status:** Accepted
- **Date:** 2026-09-26

## Context

Settings → Email already stores optional SMTP fields and an SMTP password in the
provider secret bag. Softwake still uses the confirm-gated `email_send` tool and
draft-only live scaffold. Operators also need Google and Microsoft sign-in so
mail, calendar, and drive can share one connected account without pasting
client secrets into the window.

## Decision

1. **Grant:** Authorization code + PKCE S256 + ephemeral loopback callback
   (`127.0.0.1` for Google, `localhost` host string for Microsoft). Not
   device-code (that stays xAI-only on Providers).
2. **UI:** Settings → Email **Accounts** offers Connect / Disconnect per
   provider. Status shows the connected account email. No client-id paste
   fields.
3. **Publisher clients:** Process env only —
   `SOFTWAKE_GOOGLE_CLIENT_ID`, optional `SOFTWAKE_GOOGLE_CLIENT_SECRET`,
   `SOFTWAKE_MICROSOFT_CLIENT_ID`. Missing id →
   `This build has no OAuth client configured`.
4. **Tokens:** Stored in the existing secret bag / OS keyring as
   `google_connections` / `microsoft_connections` (one account each for this
   pane). Never logged, never shown in the window script.
   **Superseded for multiple accounts (2026-09-27):** [ADR-0033](ADR-0033-multi-account-oauth.md)
   keeps the same vecs and adds a per-provider active connection id. Connect
   upserts by account id instead of replacing the vec.
5. **Scopes (one Connect):**

   | Provider | Scopes |
   |---|---|
   | Google | `openid email` + `calendar.readonly` + `calendar.events` + `drive.file` + `gmail.send` + `gmail.readonly` |
   | Microsoft | `openid profile email offline_access User.Read Calendars.ReadWrite Files.ReadWrite.AppFolder Mail.Send Mail.Read` |

   Mail scopes are intentional for Softwake Email. Google keeps
   `calendar.readonly` and adds `calendar.events` (event read and write). The
   full `calendar` ACL scope is not requested. Microsoft uses
   `Calendars.ReadWrite` instead of `Calendars.Read`. Drive stays `drive.file` /
   AppFolder. **Calendar write (2026-09-28):**
   [ADR-0034](ADR-0034-calendar-write.md).
6. **Live I/O:** This ADR ships connect, disconnect, and token storage. Live
   Gmail / Graph send and list stay later. Confirm-gated `email_send` and
   draft-only mode remain. **Superseded (2026-09-27):** list shipped in
   [ADR-0030](ADR-0030-inbox-calendar-drive-tools.md); confirmed `email_send`
   over Gmail and Graph ships in the amendment below. SMTP Send mode stays unwired.
7. **PROTOCOL_VERSION** stays 1. OAuth is in-process Tauri (no new socket
   messages).
8. **Timers / cron** are out of scope.

## Consequences

- Softwake-ui needs `live-http` for real Connect (same as xAI device-code).
- Maintainers must register desktop OAuth clients and set env vars before
  Connect works.
- A later slice can bind connector backends to these tokens without changing
  the bag shape.

## Demo

```bash
cargo test -p softwake-providers
cargo test -p softwake-ui
```

Manual Connect requires publisher env vars and a browser.

## Amendment — publisher client file + MeetRec aliases (2026-09-27)

Maintainers may place publisher client ids in
`$XDG_CONFIG_HOME/softwake/oauth-clients.env` (mode `0600`, never committed).
`publisher_*_client_id` helpers read that file when process env is unset.
Process env still wins. `softwake-ui` warms the lookup once at startup. Allowed keys:

- `SOFTWAKE_GOOGLE_CLIENT_ID`
- `SOFTWAKE_GOOGLE_CLIENT_SECRET` (optional)
- `SOFTWAKE_MICROSOFT_CLIENT_ID`
- MeetRec aliases on a shared maintainer machine: `MEETREC_GOOGLE_CLIENT_ID`,
  `MEETREC_GOOGLE_CLIENT_SECRET`, `MEETREC_MICROSOFT_CLIENT_ID`

Missing Softwake and MeetRec ids still surfaces
`This build has no OAuth client configured`. Never log or paste client secrets.


## Amendment — Gmail API + MeetRec scope delta + reconnect (2026-09-27)

Softwake Google Connect already requests `gmail.readonly` and `gmail.send` in
addition to MeetRec’s `calendar.readonly` / `drive.file` (see
[oauth-clients.md](oauth-clients.md)). Microsoft likewise requests `Mail.Read` /
`Mail.Send` beyond calendar / AppFolder.

Publisher Google Cloud projects that reuse a MeetRec client **must enable the
Gmail API**. Calendar and Drive APIs alone are not enough: Connect and
calendar/Drive tools can succeed while inbox tools (`email_list` / search / get)
return HTTP 403 with Google’s “Gmail API has not been used in project … or it is
disabled” message. Enable Gmail API on the same project as the OAuth client,
then retry; reconnect is unnecessary when the stored token’s `scope` already
includes `gmail.readonly`.

When Softwake **adds** mail (or other) scopes to `GOOGLE_EMAIL_SCOPES`, operators
must **Disconnect** then **Connect** Google in Settings → Email so consent runs
again (`prompt=consent`). Token refresh does not enlarge the grant. Settings
copy states this; the authorize URL keeps `access_type=offline`,
`prompt=consent`, and `include_granted_scopes=true`.

## Amendment — OAuth `email_send` (2026-09-27)

This supersedes §6 for live send. List already shipped in
[ADR-0030](ADR-0030-inbox-calendar-drive-tools.md). Confirmed `email_send`
delivers through the connected Email OAuth account when the daemon is built
with `live-http`:

- Google: `POST https://gmail.googleapis.com/gmail/v1/users/me/messages/send`
  with a base64url RFC 2822 `raw` body. Detail is `sent gmail {id}`.
- Microsoft Graph: `POST https://graph.microsoft.com/v1.0/me/sendMail`
  (HTTP 202, empty body, no message id). Detail is `sent graph`.

Google is preferred when both providers are connected, same as inbox. There is
no account picker in this amendment. **Multiple accounts (2026-09-27):**
[ADR-0033](ADR-0033-multi-account-oauth.md) adds per-provider active ids and an
optional `account` tool argument. A build without `live-http`, or with no usable account,
keeps the mock outbox, live draft, or SMTP `TransportNotWired` path. An error
after an account is selected (refresh, HTTP 403, network) is returned and does
not fall through to SMTP.

Reconnect only when the stored `scope` lacks `gmail.send` (Google) or
`Mail.Send` (Microsoft). An operator whose stored scope already lists those
send scopes does not Disconnect/Connect for this slice. Token refresh does not
enlarge the grant. The Gmail API must still be enabled on the publisher GCP
project (same 403 class as inbox). SMTP Send mode remains `TransportNotWired`.

## Amendment — calendar write scopes (2026-09-28)

Connect now requests Google `https://www.googleapis.com/auth/calendar.events`
in addition to `calendar.readonly`, and Microsoft `Calendars.ReadWrite` instead
of `Calendars.Read`. `calendar.readonly` stays so list/get do not depend on the
write scope, and so older grants can still list events. The full Google
`calendar` ACL scope is not requested.

Token refresh does not enlarge a grant. For each account whose stored `scope`
lacks `calendar.events` or `Calendars.ReadWrite`: Settings → Email → Accounts →
**Remove** that row → **Connect** again. If the stored scope already contains
the write scope, reconnect is not required. See
[ADR-0034](ADR-0034-calendar-write.md).
