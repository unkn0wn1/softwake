# ADR 0005 — Tool confirmation

- **Status:** Accepted
- **Date:** 2026-09-25

## Decision

The tool surface is a registry of metadata, not a single allowlisted name. Each entry has a name, a risk, and a short description.

| Risk | Meaning |
|------|---------|
| `safe` | Run immediately while awake. |
| `confirm` | Do not run. Stage one pending confirmation. Run only after `confirm_tool`. |
| `deny` | Never run, including while awake and after any confirm attempt. |

Registered names:

| Name | Risk | Effect |
|------|------|--------|
| `echo` | safe | Unchanged from [ADR 0004](ADR-0004-first-safe-tool.md). No arguments returns `pong`. Arguments return `echo:` plus those arguments joined by spaces. |
| `notify` | confirm | Arguments joined by spaces become one notification line. The daemon appends that line to an in-memory sink only after confirm. |
| `shell` | deny | Registered so a test can prove deny. It does not start a process. |

Gate order for a tool request:

1. Voice state. `permit_tool_dispatch` runs first. Sleep and hibernate refuse every name (`tool_forbidden`) and do not consult the registry. They also clear a pending confirmation.
2. Registry lookup. An unknown name is `unknown_tool`. A deny name is `tool_denied`.
3. `safe` runs immediately and emits `tool_started`, then `tool_finished`.
4. `confirm` does not run. The daemon stores one pending record (decimal id, name, args, description), emits `tool_confirm_pending`, and returns `pending_tool` on status. The sink does not change.

One pending confirmation at a time. A second confirm-gated request is `confirmation_pending` and leaves the first record in place. A safe tool may still run while a confirmation is waiting.

`confirm_tool` carries the pending id and an optional name. The name, when present, must match or the call is `pending_mismatch` and the record stays. Confirm runs the tool once, appends the `notify` line, emits `tool_confirm_resolved` with `accepted: true`, then `tool_started` and `tool_finished`, and clears the record. A second confirm of the same id is `unknown_pending`.

Confirm is refused when the daemon is not awake (`confirm_forbidden`) if the record is still present. The record stays so cancel can clear it.

`cancel_tool` clears a matching id in any voice state, emits `tool_confirm_resolved` with `accepted: false`, and does not touch the sink.

Sleep and hibernate clear a pending confirmation as part of leaving the acting session (`accepted: false`, log outcome `cancelled`). A later confirm or cancel for that id is `unknown_pending`, because the record is already gone.

The tool log is an in-memory ring of 64 entries. Each entry has a monotonic counter, the tool name, the risk when the call was classified, an outcome (`ran`, `pending`, `confirmed`, `cancelled`, `denied`, `forbidden`, `unknown`), and an optional detail. Status carries `last_tool` as one line, such as `notify confirm pending`. The notification sink keeps the newest 64 lines. A file under `$XDG_STATE_HOME/softwake/tool.log` (or `~/.local/state/softwake/tool.log`) is not written in this slice.

Wire names are `confirm_tool` and `cancel_tool` on the client, and `tool_confirm_pending` / `tool_confirm_resolved` on events. Fields use snake_case. `pending_tool` and `last_tool` on status are optional and omitted when absent. Protocol generation stays `1`. [ADR 0003](ADR-0003-ipc-transport.md) records the frame.

The runtime policy names `echo` (safe), `notify` (confirm), and `shell` (deny), and says that `notify` runs only after `confirm_tool`.

## Context

Phase 1 proved a binary allowlist with a pure tool. The next risky tool needs a confirmation path before it can change anything. A microphone is not required to prove that path. `notify` mutates a daemon-owned `Vec` (a ring of lines) and nothing else, so CI can assert "pending does not append" and "confirm appends once".

`shell` is on the registry as deny so an awake session cannot talk the daemon into running it, and so a confirm attempt has nothing to confirm.

## Alternatives

- Run confirm-gated tools immediately while awake. Rejected. That is the phase-1 rule, and it is the wrong default once a tool has a side effect.
- A real shell, clipboard, volume, or notification daemon behind confirm. Rejected for this slice. Confirm has to be proven before those side effects exist. `notify` only appends to memory.
- Auto-confirm inside the daemon. Rejected. The operator has to send `confirm_tool` or the demo command `confirm`.
- Cloud-hosted confirmation. Rejected. The decision stays on this machine.
- Replace the pending record when a second confirm-gated tool is requested. Rejected. Dropping an unseen confirmation is easier to miss than a clear error.
- Bump the protocol generation. Rejected. The new messages and status fields are additive, with serde defaults for the optional ones.

## How to demo

Copy the soul templates, then run the typed demo:

```bash
mkdir -p ~/.config/softwake/soul
cp soul/soul.md soul/user.md ~/.config/softwake/soul/
cargo run -p softwake-daemon -- demo
```

```text
> wake
> tool echo hi
> tool notify hello
> confirm
> tool notify later
> cancel
> sleep
> hibernate
```

`tool notify hello` prints a pending id and `waiting for confirm`. The notification line appears only after `confirm` (or `confirm-tool 1`). `cancel` (or `cancel-tool 1`) prints a cancellation and does not append. `tool shell` is denied. `tool volume` is unknown. The same calls while asleep or hibernating are refused.

`softwaked ctl tool notify hello` returns the pending id on status. `softwaked ctl confirm-tool <id>` runs it. `softwaked ctl cancel-tool <id>` drops it. The daemon starts in sleep, and serve still has no microphone path into awake, so those ctl commands are refused until something else has entered awake.

The window polls status. When `pending_tool` is set it shows the pending text and enables Confirm and Cancel. Those buttons call `confirm_tool` and `cancel_tool`.

## Consequences

- `echo` stays safe. Blind `invoke` on the registry does not run `notify` or `shell`.
- A real side-effect tool should be `confirm` or `deny` until its own ADR says otherwise. `shell` stays deny until that ADR exists.
- The in-memory log and sink reset when the process exits. A state-directory file can be added later without changing the gate.
- Clients that do not send `confirm_tool` still speak protocol generation 1. They will see a pending `notify` as a successful status with `pending_tool`, not as a finished tool.
