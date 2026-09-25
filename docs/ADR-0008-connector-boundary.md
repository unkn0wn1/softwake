# ADR 0008 — Connector boundary

- **Status:** Accepted
- **Date:** 2026-09-25
- **Amended:** 2026-09-25 (`email_send` on the tool bus); 2026-09-25 (Drive and calendar list mocks); 2026-09-25 (live email opt-in scaffold)

## Decision

World connectors live in `softwake-connectors`, separate from the tool bus in [ADR 0005](ADR-0005-tool-confirmation.md).

`EmailConnector` is the email trait. `MockEmail` is the default backend. `send` appends one message to an in-memory outbox on that value. The mock does not open a socket and does not read credentials.

`DriveConnector` is the Drive trait. `MockDrive` is the default backend. `list` returns the files stored on that value, oldest first. `insert` on the mock is test setup for that value. It is not a registry action. The mock does not open a socket and does not read credentials.

`CalendarConnector` is the calendar trait. `MockCalendar` is the default backend. `list` returns the events stored on that value, oldest first. `insert` on the mock is test setup for that value. It is not a registry action. Event `start` is stored unchanged and is not parsed as a date or a timezone. The mock does not open a socket and does not read credentials.

The registry is a static table. There is no method that inserts a row.

| Connector | Action | Risk | Effect |
|-----------|--------|------|--------|
| `email` | `send` | confirm | [`MockEmail::send`](../crates/softwake-connectors/src/mock.rs) appends after the caller has authorized the action. The registry method does not append. |
| `email` | `delete` | deny | No backend. Refused on the blind path and on the confirmed path. |
| `drive` | `list` | confirm | [`MockDrive::list`](../crates/softwake-connectors/src/mock.rs) returns stored files after the caller has authorized the action. The registry method does not list. |
| `drive` | `delete` | deny | No backend. Refused on the blind path and on the confirmed path. |
| `calendar` | `list` | confirm | [`MockCalendar::list`](../crates/softwake-connectors/src/mock.rs) returns stored events after the caller has authorized the action. The registry method does not list. |
| `calendar` | `delete` | deny | No backend. Refused on the blind path and on the confirmed path. |

`ConnectorRisk` has two variants, `Confirm` and `Deny`. It has no `safe` spelling. The tool enum `ToolRisk` stays in `softwake-tools` and is not reused here, so a safe send cannot be written down as a connector action.

`invoke` is the blind path. It does not take a connector value. A confirm action returns "requires confirmation". A deny action returns "denied". An unknown pair returns "unknown". No registered action returns success from `invoke`.

`authorize_confirmed` is the path after a confirmation the daemon already accepted. It does not check a token, matching `invoke_confirmed` in [ADR 0005](ADR-0005-tool-confirmation.md). It does not send and does not list. `Ok(())` means the caller may perform the action. The confirm pairs are `email` / `send`, `drive` / `list`, and `calendar` / `list`.

Lookup is the pair `(connector, action)`. Matching is case-sensitive. `gmail` / `send` is unknown: the registry names the capability `email`, and a later vendor client would implement `EmailConnector` under that same action.

The default build has no live Gmail, Drive, or Calendar client and no connector Cargo feature flag. Drive and calendar stay in-memory mocks. Live email is an **opt-in Settings scaffold** (off by default): non-secret fields in `email.json`, SMTP password in the provider secret bag, Test connection that does not open a socket, and a `LiveEmail` backend behind `EmailConnector`. CI does not set credentials and does not enable a network backend.

`email_send` is the confirm-gated tool on the bus from [ADR 0005](ADR-0005-tool-confirmation.md). Its arguments are `to`, `subject`, and `body`: the first argument, the second argument, and the rest joined by one space. Fewer than three arguments is refused while awake and does not stage a confirmation. The strings are stored unchanged.

The handler lives on daemon `Hands`, beside the notification sink. `Hands` holds an `EmailBackend` (`Mock` by default, or `Live` when Settings enable live email) and a `ConnectorRegistry`. Tests construct `Hands` with the mock. Serve and the typed demo load Settings from disk and fall back to the mock when live is off or load fails. `permit_tool_dispatch` runs first. A blind request stages one pending confirmation and does not send. After the operator accepts, the handler parses the stored arguments, calls `invoke_confirmed`, then `authorize_confirmed` for `email` / `send`, then `EmailBackend::send`. The pending record is cleared only after that append. Cancel, and leaving awake, clear the record and do not send. Live draft-only mode stores a local draft and reports `draft {id}`; mock mode still reports `sent {id}`. Live send mode still requires confirm and then returns a clear not-wired error until a later transport ADR. Softwake does not auto-send; Settings never sends without the Status confirm path.

`softwake-tools` parses those arguments and does not depend on `softwake-connectors`. `softwake-connectors` does not depend on `softwake-tools`. The email mock value stays in the daemon. The daemon does not construct `MockDrive` or `MockCalendar`.

IPC protocol generation stays `1`. `email_send` uses the existing tool messages. The outbox is not a status field. The window already shows a pending confirmation and the confirm detail. Drive and calendar actions are not tool names and are not socket commands.

The soul runtime policy names `email_send` as confirm, beside `notify`. `shell` stays deny. A deny row in the table above stays deny until a later change to this ADR. `email` / `send` is the connector pair. It is not the tool name, and it is not a shell.

## Context

Phase 3's connector milestone needs a boundary before policy can name world I/O. [ADR 0005](ADR-0005-tool-confirmation.md) already has safe, confirm, and deny for session tools, and its in-memory `notify` proved confirmation without leaving the process. Email needs its own trait so a later backend can replace the mock without growing `softwake-tools`.

The same rule as [ADR 0006](ADR-0006-on-device-wake.md) and [ADR 0007](ADR-0007-awake-stt-tts.md) applies: the default path is the mock, and the heavy backend stays out of CI.

`drive` / `list` and `calendar` / `list` were registered as deny name-only placeholders so a test could see the names. They had no trait and no mock. This amendment adds `DriveConnector`, `MockDrive`, `CalendarConnector`, and `MockCalendar`, and moves those list rows to confirm. `drive` / `delete` and `calendar` / `delete` are deny rows with no backend, matching `email` / `delete`.

## Alternatives

- A policy-engine-only rewrite of tool risks. Rejected. Those risks already exist. They do not name a mailbox.
- Register `email.send` on the tool bus in this slice. Rejected. That copies the `notify` path across IPC, the demo, and the window before a backend trait exists.
- Live Gmail on by default. Rejected. CI would need a token or a network. Same stance as `pipewire-native` and the sherpa features.
- Share `ToolRisk`. Rejected. Its safe variant would remain representable on a send.
- Vendor name `gmail` in the registry. Rejected. The action is the capability. The vendor is a backend.
- Honcho, or a memory ADR, in this slice. Rejected. Memory is a different boundary.
- A permit-token type inside this crate. Rejected. [ADR 0005](ADR-0005-tool-confirmation.md) keeps the token in the daemon. A second token scheme with no daemon caller is extra machinery.
- Call Gmail from `softwake-tools`. Rejected. The tool crate stays free of world I/O types.
- Put `MockEmail` inside `softwake-tools` when the tool is wired. Rejected. The notification sink already lives on the daemon, and a tools dependency on connectors would undo this boundary.
- A separate ADR number for this Drive and calendar amendment. Rejected. This ADR already named the update.
- `Safe` for list because the mock stays in-process. Rejected. The row is the capability. A later live backend would read a remote account.
- `drive_list` / `calendar_list` on the tool bus in this amendment. Rejected. That copies the `email_send` path through the soul pack, the demo, and the daemon.
- Deny rows for upload and create. Rejected. Unknown pairs already fail closed. `delete` is the destructive name, matching email.

## How to demo

Copy the soul templates, then run the typed demo:

```bash
mkdir -p ~/.config/softwake/soul
cp soul/*.md ~/.config/softwake/soul/
cargo run -p softwake-daemon -- demo
```

```text
> wake
> tool email_send ada@example.com hello a short note
> confirm
```

`tool email_send ada@example.com hello a short note` prints a pending id and `waiting for confirm`. The `email:` line appears only after `confirm`. `cancel` prints a cancellation and does not append. The same call while asleep or hibernating is refused.

Drive and calendar are proven without the daemon. The typed demo does not list files or events:

```bash
cargo test -p softwake-connectors
cargo test -p softwake-policy
```

## Consequences

- Callers can classify an action, store a fake outbound message, and read files or events kept on a mock value. They cannot reach a mailbox, a Drive account, or a calendar from this build.
- A live email scaffold exists as Settings opt-in (off by default, off in CI). It implements `EmailConnector` under the same `email` / `send` name. Real SMTP or Gmail OAuth is a later transport; this scaffold is config + Test + draft-only (or not-wired send).
- `email` / `delete`, `drive` / `delete`, and `calendar` / `delete` stay deny until a later change to this ADR adds a backend and changes the row on purpose.
- The daemon default and CI path call `MockEmail` via `EmailBackend::Mock`. An opted-in live Settings file selects `EmailBackend::Live`. The daemon does not construct `MockDrive` or `MockCalendar`.
- The in-memory outbox and the in-memory file and event lists die with the value. This slice does not write a mailbox or a Drive or calendar file.
- Confirming `email_send` appends one message on the daemon's `MockEmail`. A second confirm of that id does not append again.
- Clients that speak protocol generation 1 see no new message kinds. `email_send` is a name on the existing tool messages. Connector actions are not socket commands.

## Amendment — live email opt-in scaffold (2026-09-25)

### Decision

Add an opt-in live email **scaffold** without turning Softwake into a Gmail product and without giving CI a key:

1. **Off by default.** `EmailSettings.live_enabled` defaults to `false`. Missing `email.json` is the mock path. Workspace CI runs mock-only sends.
2. **Settings surface.** The Email pane offers enable toggle, SMTP host/port/username/from, mode (`draft_only` default, or `send`), password into the secret bag, Save, Clear password, and Test connection. The pane does not call `email_send` and does not confirm-send.
3. **Backend shape.** `LiveEmail` implements `EmailConnector`. Test and send check configuration only; they do not open a socket in this build. Draft-only appends to an in-process draft list after the existing confirm gate. Send mode returns a fixed not-wired error after confirm so the operator is never surprised by a silent network send.
4. **Secret bag.** `email_smtp_password` is a new field on the provider secret bag (keyring or opt-in plaintext). The connector only sees a boolean “password present”.
5. **No auto-send.** Rules and skills still cannot auto-send ([ADR 0014](ADR-0014-skills-hub.md)). Confirm stays mandatory. Spencer’s default product stance remains drafts / confirm-gated.
6. **Gmail OAuth / real SMTP** are explicitly later. This ADR does not add OAuth types for mail.

### Alternatives (this amendment)

- Live Gmail on by default. Rejected. CI would need a token or network.
- Settings Send button that bypasses confirm. Rejected. Confirm stays on the tool bus.
- Full SMTP client in this PR. Rejected. Scaffold first; transport needs its own review.
- Cargo feature flag instead of Settings toggle. Rejected for the operator path; Settings is how providers already opt into live HTTPS. A Cargo feature is unnecessary while the scaffold opens no socket.
