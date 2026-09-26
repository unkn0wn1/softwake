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
5. **Scopes (one Connect):**

   | Provider | Scopes |
   |---|---|
   | Google | `openid email` + `calendar.readonly` + `drive.file` + `gmail.send` + `gmail.readonly` |
   | Microsoft | `openid profile email offline_access User.Read Calendars.Read Files.ReadWrite.AppFolder Mail.Send Mail.Read` |

   Mail scopes are intentional for Softwake Email. Calendar and drive match the
   usual desktop readonly / AppFolder pattern.
6. **Live I/O:** This ADR ships connect, disconnect, and token storage. Live
   Gmail / Graph send and list stay later. Confirm-gated `email_send` and
   draft-only mode remain.
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
