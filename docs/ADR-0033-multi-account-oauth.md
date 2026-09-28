# ADR-0033 — Multiple Google and Microsoft Email accounts

- **Status:** Accepted
- **Date:** 2026-09-27

## Context

[ADR-0023](ADR-0023-email-oauth.md) stores Email OAuth tokens in
`google_connections` and `microsoft_connections` and shipped one Connect per
provider. [ADR-0030](ADR-0030-inbox-calendar-drive-tools.md) and the ADR-0023
OAuth-send amendment route inbox, calendar, Drive, and confirmed `email_send`
through the first row. Connect and token refresh replaced that provider’s whole
vec, so a second sign-in dropped the first account.

## Decision

1. **Bag version stays 2.** Add optional `active_google_connection_id` and
   `active_microsoft_connection_id`. Missing keys load as unset. A missing or
   dangling id means the first row of that provider. The keyring pointer file
   does not gain these fields. `PROTOCOL_VERSION` stays 1.
2. **Upsert by `AccountConnection.id`.** Connect and refresh replace the
   matching row, or append when the id is new. An email match replaces a row
   only when at least one id is empty. The first account in an empty list
   becomes active. A later account does not steal active.
3. **Remove one row** by connection id or full email. If the active id no
   longer names a remaining row, the first remaining id becomes active. Google
   revoke sends only the removed refresh token.
4. **Settings → Email → Accounts** lists every account, adds another with the
   same Connect flow, removes one row, and sets active per provider. Legacy
   snapshot scalars (`google_connected`, `google_email`, and the Microsoft
   twins) stay active-or-first.
5. **Tool routing.** Optional `account` is a connection id or email substring
   on inbox, calendar, Drive, and `email_send`. With no `account`, Softwake
   uses the only connected account if there is exactly one. If more than one
   account is connected and any Google account is usable, it uses the active
   Google account (the first Google account when none is marked). Otherwise it
   uses the active Microsoft account. `account` picks a different account,
   including Microsoft when Google is also connected. A hint that matches
   nothing on a `live-http` send is an error and does not fall through to SMTP.
6. **Same publisher client.** `SOFTWAKE_*` / `MEETREC_*` ids and
   `oauth-clients.env` are unchanged. A second account is another grant on that
   desktop client.

## Consequences

- An existing one-account bag keeps working. Until Set active or a write that
  records the id, tools and the Accounts badge use the first row.
- Operators add a second mailbox from Settings → Email → Add account, then Set
  active on the row that provider should prefer.
- Gmail `users.messages.send` and Graph `sendMail` are unchanged. Builds
  without `live-http` still use the mock outbox or local draft.

## Amendment — calendar write uses the same account (2026-09-28)

`calendar_create`, `calendar_update`, and `calendar_delete`
([ADR-0034](ADR-0034-calendar-write.md)) take the same optional `account` and
the same active / Google-preferred rules as inbox and `email_send`. There is
still no per-profile account bind.

## Out of scope

Per-profile account bind, voice account picking, wider Drive scopes, agent
cron, memory writes, and webhook wake. Calendar event write is
[ADR-0034](ADR-0034-calendar-write.md).

## Demo

```bash
cargo test -p softwake-providers -p softwake-tools -p softwake-ui -p softwake-daemon
cargo check -p softwake-daemon --features live-http
cargo check -p softwake-ui --features live-http
```

Manual: Settings → Email → Add account for a second mailbox, Set active, then
ask for the inbox. Pass `account` with the other email or connection id to use
that mailbox instead. Manual sign-in is not part of CI.

## Amendment — OAuth mirror (ADR-0045)

When the operator enables Mirror OAuth tokens on a Remote Agent, the companion
receives a copy of the same Google/Microsoft connection rows and uses the same
`account=` / active-or-first / Google-preferred rules. There is still no
per-profile account bind.

