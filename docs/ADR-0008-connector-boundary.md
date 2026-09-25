# ADR 0008 — Connector boundary

- **Status:** Accepted
- **Date:** 2026-09-25
- **Amended:** 2026-09-25 (`email_send` on the tool bus)

## Decision

World connectors live in `softwake-connectors`, separate from the tool bus in [ADR 0005](ADR-0005-tool-confirmation.md).

`EmailConnector` is the email trait. `MockEmail` is the default backend. `send` appends one message to an in-memory outbox on that value. The mock does not open a socket and does not read credentials.

The registry is a static table. There is no method that inserts a row.

| Connector | Action | Risk | Effect |
|-----------|--------|------|--------|
| `email` | `send` | confirm | [`MockEmail::send`](../crates/softwake-connectors/src/mock.rs) appends after the caller has authorized the action. The registry method does not append. |
| `email` | `delete` | deny | No backend. Refused on the blind path and on the confirmed path. |
| `drive` | `list` | deny | Name only. No backend. |
| `calendar` | `list` | deny | Name only. No backend. |

`ConnectorRisk` has two variants, `Confirm` and `Deny`. It has no `safe` spelling. The tool enum `ToolRisk` stays in `softwake-tools` and is not reused here, so a safe send cannot be written down as a connector action.

`invoke` is the blind path. It does not take a connector value. A confirm action returns "requires confirmation". A deny action returns "denied". An unknown pair returns "unknown". No registered action returns success from `invoke`.

`authorize_confirmed` is the path after a confirmation the daemon already accepted. It does not check a token, matching `invoke_confirmed` in [ADR 0005](ADR-0005-tool-confirmation.md). It does not send. `Ok(())` means the caller may perform the action. Today that is only `email` / `send`.

Lookup is the pair `(connector, action)`. Matching is case-sensitive. `gmail` / `send` is unknown: the registry names the capability `email`, and a later vendor client would implement `EmailConnector` under that same action.

The default build has no live Gmail, Drive, or Calendar client, no OAuth types, and no connector feature flag. CI does not set credentials and does not enable a network backend.

`email_send` is the confirm-gated tool on the bus from [ADR 0005](ADR-0005-tool-confirmation.md). Its arguments are `to`, `subject`, and `body`: the first argument, the second argument, and the rest joined by one space. Fewer than three arguments is refused while awake and does not stage a confirmation. The strings are stored unchanged.

The handler lives on daemon `Hands`, beside the notification sink. `Hands` holds a `MockEmail` and a `ConnectorRegistry`. `permit_tool_dispatch` runs first. A blind request stages one pending confirmation and does not send. After the operator accepts, the handler parses the stored arguments, calls `invoke_confirmed`, then `authorize_confirmed` for `email` / `send`, then `EmailConnector::send`. The pending record is cleared only after that append. Cancel, and leaving awake, clear the record and do not send.

`softwake-tools` parses those arguments and does not depend on `softwake-connectors`. `softwake-connectors` does not depend on `softwake-tools`. The mock value stays in the daemon.

IPC protocol generation stays `1`. `email_send` uses the existing tool messages. The outbox is not a status field. The window already shows a pending confirmation and the confirm detail.

The soul runtime policy names `email_send` as confirm, beside `notify`. `shell` stays deny. A deny row in the table above stays deny until a later change to this ADR. `email` / `send` is the connector pair. It is not the tool name, and it is not a shell.

## Context

Phase 3's connector milestone needs a boundary before policy can name world I/O. [ADR 0005](ADR-0005-tool-confirmation.md) already has safe, confirm, and deny for session tools, and its in-memory `notify` proved confirmation without leaving the process. Email needs its own trait so a later backend can replace the mock without growing `softwake-tools`.

The same rule as [ADR 0006](ADR-0006-on-device-wake.md) and [ADR 0007](ADR-0007-awake-stt-tts.md) applies: the default path is the mock, and the heavy backend stays out of CI.

`drive` / `list` and `calendar` / `list` are registered as deny so a test can see the names. They have no trait and no mock. Moving either row to confirm waits on a backend and an update to this ADR.

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

## How to demo

Copy the soul templates, then run the typed demo:

```bash
mkdir -p ~/.config/softwake/soul
cp soul/soul.md soul/user.md ~/.config/softwake/soul/
cargo run -p softwake-daemon -- demo
```

```text
> wake
> tool email_send ada@example.com hello a short note
> confirm
```

`tool email_send ada@example.com hello a short note` prints a pending id and `waiting for confirm`. The `email:` line appears only after `confirm`. `cancel` prints a cancellation and does not append. The same call while asleep or hibernating is refused.

The registry itself is still proven without the daemon:

```bash
cargo test -p softwake-connectors
```

## Consequences

- Callers can classify a connector action and store a fake outbound message. They cannot reach a mailbox from this build.
- A live backend, when it exists, is a non-default feature, off in CI, behind a follow-up ADR, and it implements `EmailConnector` rather than a new registry vendor name.
- `drive` / `list` and `calendar` / `list` stay deny until that follow-up adds a backend and changes the row on purpose.
- The in-memory outbox dies with the value. This slice does not write a mailbox file.
- Confirming `email_send` appends one message on the daemon's `MockEmail`. A second confirm of that id does not append again.
- Clients that speak protocol generation 1 see no new message kinds. `email_send` is a name on the existing tool messages. Connector actions are not socket commands.
