# ADR-0035 — Drive read scope widen (beyond drive.file / AppFolder)

- **Status:** Accepted
- **Date:** 2026-09-28

## Context

[ADR-0030](ADR-0030-inbox-calendar-drive-tools.md) ships `drive_list`,
`drive_search`, and `drive_get` over Email OAuth. Connect asked Google
`drive.file` and Microsoft `Files.ReadWrite.AppFolder`. List/search therefore
only saw Softwake-touched Google files or App Folder children — not the
operator’s real Drive / OneDrive. Account choice stays
[ADR-0033](ADR-0033-multi-account-oauth.md). Calendar write
([ADR-0034](ADR-0034-calendar-write.md)) and confirmed `email_send`
([ADR-0023](ADR-0023-email-oauth.md)) are unchanged.

## Decision

1. **Google Connect scopes.** Replace
   `https://www.googleapis.com/auth/drive.file` with
   `https://www.googleapis.com/auth/drive.readonly`. Keep
   `calendar.readonly`, `calendar.events`, `gmail.readonly`, and `gmail.send`.
   Do not request the full `drive` write/ACL scope in this slice.

2. **Microsoft Connect scopes.** Replace `Files.ReadWrite.AppFolder` with
   `Files.Read`. Keep `Calendars.ReadWrite`, `Mail.Read`, and `Mail.Send`.
   Do not request `Files.ReadWrite` or `Files.ReadWrite.All` here.

3. **Graph endpoints.** `drive_list` and `drive_search` for Microsoft use
   user drive root:
   - `GET /me/drive/root/children`
   - `GET /me/drive/root/search(q='…')`
   Get/content stay `/me/drive/items/{id}` (+ `/content`). Google Drive v3
   `files.list` / `files.get` builders are unchanged; the grant is what widens
   visibility.

4. **Tools.** No new Drive write tools (`drive_create` / upload / trash) in
   this PR. Existing list/search/get keep defaults and optional `account`.

5. **Reconnect.** Token refresh does not enlarge grants. Operators whose
   stored `scope` lacks `drive.readonly` (Google) or `Files.Read` (Microsoft)
   must Settings → Email → Accounts → Remove that account → Connect again.
   Reconnect is not required when those scopes are already on the token.

6. **PROTOCOL_VERSION** stays 1.

## Consequences

- After reconnect (or fresh Connect), agents can list/search/get real user
  Drive / OneDrive files under the same Hands / API tool-calling path.
- Broader read consent is intentional and documented; MeetRec-shared clients
  still apply.
- Drive create/upload remains a later slice if write scopes stay small.

## Demo

```bash
cargo test -p softwake-providers -p softwake-connectors -p softwake-tools -p softwake-daemon --lib
cargo check -p softwake-daemon --features live-http
```

Manual: Remove → Connect Google/Microsoft, then awake ask “list my Drive files”.
