# ADR-0034 — Calendar create, update, and delete

- **Status:** Accepted
- **Date:** 2026-09-28

## Context

[ADR-0030](ADR-0030-inbox-calendar-drive-tools.md) ships `calendar_list` and
`calendar_get` over Email OAuth. Connect asked Google `calendar.readonly` and
Microsoft `Calendars.Read`. Operators who want Softwake to schedule, move, or
cancel events need write scopes and confirm-gated write tools. Account choice
stays [ADR-0033](ADR-0033-multi-account-oauth.md). Confirmed `email_send`
([ADR-0023](ADR-0023-email-oauth.md)) is unchanged.

## Decision

1. **Tools** (OpenAI advertise + Hands). Default Ask. Registry floor is confirm.

   | Tool | Purpose |
   |---|---|
   | `calendar_create` | Title, RFC3339 start, RFC3339 end, optional location and description |
   | `calendar_update` | Patch one event by id (`title`, `start`, `end`, `location`, `description`) |
   | `calendar_delete` | Delete one event by id |

   `calendar_list` and `calendar_get` stay as in ADR-0030. Each new tool takes
   optional `account` (connection id or email substring) and uses the same
   `with_account` / `resolve_account` path as inbox and `email_send`. Deny omits
   the tool from the chat `tools` array. Operator Always allow / Ask / Deny still
   apply.

2. **Connector actions.** `calendar` / `create`, `calendar` / `update`, and
   `calendar` / `delete` are confirm. Event delete only: this does not deregister
   the connector. `email` / `delete` and `drive` / `delete` stay deny.

3. **Primary calendar only (v1).** Google
   `POST` / `PATCH` / `DELETE`
   `https://www.googleapis.com/calendar/v3/calendars/primary/events`.
   Microsoft Graph `POST` / `PATCH` / `DELETE`
   `https://graph.microsoft.com/v1.0/me/events`. No calendar picker, attendees,
   or ACL.

4. **Times.** Callers pass RFC3339. Google `start` / `end` are
   `{ "dateTime": <caller string> }` with no separate time zone (the offset is
   in the string). Graph `start` / `end` are
   `{ "dateTime": <caller string>, "timeZone": "UTC" }`.

5. **Details.** `created google <id>`, `created graph <id>`,
   `updated google <id>`, `updated graph <id>`, `deleted google <id>`,
   `deleted graph <id>`. Delete uses the request id. Success is often HTTP 204
   with an empty body.

6. **Scopes on Connect.** Google keeps `calendar.readonly` and adds
   `https://www.googleapis.com/auth/calendar.events`. The full `calendar` ACL
   scope is not requested. `calendar.events` is event read and write.
   `calendar.readonly` stays so list/get keep working on grants that do not yet
   include events, and so list/get do not depend on the write scope alone.
   Microsoft replaces `Calendars.Read` with `Calendars.ReadWrite` (read is
   included). MeetRec client reuse is unchanged.

7. **Reconnect.** Token refresh does not enlarge a grant. For each account that
   lacks the write scope: Settings → Email → Accounts → **Remove** that row →
   **Connect** (or Add account) again. If stored `scope` already contains
   `calendar.events` (Google) or `Calendars.ReadWrite` (Microsoft), reconnect is
   not required. An empty stored scope still attempts the call. A non-empty
   scope that lacks write, on HTTP 403, appends a short reconnect hint. Provider
   errors stay token-free.

8. **Builders** are network-free in `softwake-connectors`. Live HTTPS is daemon
   `live-http` only, with the same timeouts and error dialect as inbox GET. A
   build without `live-http` returns the same live-required error as the other
   cloud tools and does not read the secret bag.

9. **PROTOCOL_VERSION** stays 1.

## Out of scope

Drive scope widen, Drive upload or trash, agent cron, memory write, webhook
wake, per-profile account bind, attendees, a multi-calendar picker, and
calendar ACL.

## Consequences

- Accounts connected before this slice keep list and get. Write fails with a
  reconnect hint when the stored scope lacks the write grant.
- CI stays offline. Tests do not perform live OAuth.
- `email_send` and multi-account routing are unchanged.

## Demo

```bash
cargo test -p softwake-connectors -p softwake-tools -p softwake-policy -p softwake-daemon
cargo check -p softwake-daemon --features live-http
```

Manual: for an account whose stored scope lacks write, Settings → Email →
Accounts → Remove → Connect, then ask to create an event and Approve in the HUD.
