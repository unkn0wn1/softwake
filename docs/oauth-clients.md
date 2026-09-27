# OAuth clients (publisher setup)

End users only press **Connect** / **Disconnect** in Settings → Email. They never
paste a client id or secret.

Spencer registers the Google and Microsoft desktop clients once. Softwake reads
publisher ids from process env or `$XDG_CONFIG_HOME/softwake/oauth-clients.env`
(mode `0600`, never committed). MeetRec `MEETREC_*` aliases are accepted on a
shared maintainer machine ([ADR-0023](ADR-0023-email-oauth.md)).

## Env vars

| Variable                        | Required                    | Notes |
| ------------------------------- | --------------------------- | ----- |
| `SOFTWAKE_GOOGLE_CLIENT_ID`     | For Google mail/calendar/Drive | Desktop OAuth client id |
| `SOFTWAKE_GOOGLE_CLIENT_SECRET` | Optional                    | Send on token exchange when Google needs it |
| `SOFTWAKE_MICROSOFT_CLIENT_ID`  | For Microsoft mail/calendar/OneDrive | Public client. No secret |

Aliases when Softwake keys are unset: `MEETREC_GOOGLE_CLIENT_ID`,
`MEETREC_GOOGLE_CLIENT_SECRET`, `MEETREC_MICROSOFT_CLIENT_ID`.

Missing ids → **This build has no OAuth client configured**.

## Softwake vs MeetRec scopes

MeetRec Connect asks only for calendar + optional Drive upload. Softwake Email
Connect asks for **mail as well** in one consent:

| Provider | MeetRec (calendar app) | Softwake Email Connect |
| -------- | ---------------------- | ---------------------- |
| Google | `openid` `email` `calendar.readonly` (+ `drive.file` when upload) | `openid` `email` `calendar.readonly` `drive.file` **`gmail.readonly` `gmail.send`** |
| Microsoft | `Calendars.Read` (+ `Files.ReadWrite.AppFolder` when upload) | identity + `Calendars.Read` `Files.ReadWrite.AppFolder` **`Mail.Read` `Mail.Send`** |

Reusing a MeetRec Google Cloud project for Softwake is fine **only if** that
project also enables the Gmail API and lists the Gmail scopes on the consent
screen. Calendar/Drive working while `email_list` returns HTTP 403 usually means
Gmail API is still disabled on the project (scopes may already be on the token).

## Google Cloud

1. [Google Cloud Console](https://console.cloud.google.com/) → project (e.g. shared `meetrec` / Softwake publisher project).
2. Enable **Gmail API**, **Google Calendar API**, and **Google Drive API**.
   Inbox tools call `gmail.googleapis.com`. If Gmail API is off, Connect can
   still succeed (calendar/Drive work) and inbox tools return **HTTP 403** with
   Google’s “Gmail API has not been used in project … or it is disabled” body.
3. OAuth consent screen → External → Testing. Add test users. Scopes must include
   Softwake’s set: `openid`, `email`, `calendar.readonly`, `drive.file`,
   `gmail.readonly`, `gmail.send` (full URLs under `https://www.googleapis.com/auth/…`).
4. Credentials → OAuth client ID → **Desktop app**.
5. Put the client id (and secret if present) in `oauth-clients.env` / CI secrets.
   Do not create a Web client. Do not put the secret in git, an issue, or a log.
6. Testing mode: Google may expire the refresh token about every 7 days until
   the consent screen is verified.

Redirect used by Softwake: `http://127.0.0.1:<port>/callback`.

### Exact click-path — Enable Gmail API (MeetRec-shared Softwake project)

Softwake’s local `oauth-clients.env` copies MeetRec’s Google desktop client into
`SOFTWAKE_GOOGLE_*` (no secrets in git). Live diagnosis against that client
shows inbox 403 from **project number `577210165352`** with reason
`accessNotConfigured` / “Gmail API has not been used … or it is disabled”, while
Calendar and Drive return 200 with the same bearer. Softwake is not dropping
scopes: the secret-bag connection persists `scope` including `gmail.readonly` and
`gmail.send`, and live HTTPS sends `Authorization: Bearer <access_token>` to
`https://gmail.googleapis.com/gmail/v1/users/me/messages`.

Do this once in Cloud Console (signed in as the project owner):

1. Open the enable link Google returns (or paste it):
   https://console.developers.google.com/apis/api/gmail.googleapis.com/overview?project=577210165352
2. Confirm the project picker shows the MeetRec / Softwake publisher project
   (number `577210165352`).
3. Click **Enable**.
4. Wait 1–5 minutes for propagation.
5. Retry `email_list` (awake ask / tool). **No Softwake reconnect**, no Sally
   `/refresh`, and no softwaked restart — the daemon reloads the live secret bag
   per call and refreshes the access token when near expiry.

Also confirm APIs & Services → Library (or Enabled APIs) lists **Gmail API**,
**Google Calendar API**, and **Google Drive API**. Consent screen → Edit app →
Scopes should list Softwake’s mail scopes (already granted on Spencer’s
reconnect).

### After changing scopes or enabling Gmail API

- **Gmail API newly enabled:** wait a few minutes for propagation, then retry
  `email_list`. No reconnect is required if the stored token already lists
  `gmail.readonly` / `gmail.send`. Sally does **not** need `/refresh` for OAuth
  tokens; Softwake reads the live secret bag on each tool call.
- **Consent scopes newly added:** Settings → Email → **Disconnect** Google, then
  **Connect** again so the authorize URL re-requests the full Softwake set
  (`prompt=consent`). Token refresh alone does not enlarge the grant.

## Microsoft Entra

1. [Microsoft Entra admin center](https://entra.microsoft.com/) → App registrations.
2. Supported accounts: any org directory and personal Microsoft accounts (`common`).
3. Authentication → **Mobile and desktop applications**. Custom redirect URI
   `http://localhost` (Entra ignores the port; Softwake still uses
   `http://localhost:<port>/callback`).
4. Allow public client flows: **Yes**. Do not create a client secret.
5. API permissions → Microsoft Graph delegated: Softwake’s
   `MICROSOFT_EMAIL_SCOPES` (`Mail.Read`, `Mail.Send`, `Calendars.Read`,
   `Files.ReadWrite.AppFolder`, plus identity / `offline_access`).
6. Put the Application (client) id in env / `oauth-clients.env`.

## What Softwake stores

| Value | Where |
| ----- | ----- |
| Publisher client ids | Process env or `oauth-clients.env` (not Settings UI) |
| Google client secret | Same, when set |
| Refresh and access tokens + granted `scope` | Secret bag / OS keyring (`google_connections` / `microsoft_connections`, one account each) |

Constants: `GOOGLE_EMAIL_SCOPES` / `MICROSOFT_EMAIL_SCOPES` in
`crates/softwake-providers/src/account_oauth.rs`.
