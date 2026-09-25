# ADR 0003 — IPC transport

- **Status:** Accepted
- **Date:** 2026-09-24
- **Amended:** 2026-09-25 — `reload_soul` re-reads the four-file pack from [ADR 0011](ADR-0011-context-pack.md). Clients may send `ask`. Protocol generation stays 1.

## Decision

The daemon and its clients speak **newline-delimited JSON** (one JSON value per line, `\n` terminated) over a **Unix domain socket**. Protocol generation is `1`. The first frame is a hello. A different version is rejected and the connection closes.

Socket path, first match wins:

1. `--socket PATH`
2. `SOFTWAKE_SOCKET`
3. `$XDG_RUNTIME_DIR/softwake/softwaked.sock` when `XDG_RUNTIME_DIR` is set and not blank
4. `/tmp/softwake-$UID/softwaked.sock` when that variable is unset

The uid is the owner of `/proc/self` (Linux). If that cannot be read and `XDG_RUNTIME_DIR` is unset, startup fails and asks for `SOFTWAKE_SOCKET`. macOS and Windows socket paths are out of scope for this phase.

`softwaked serve` removes a socket file whose peer refuses a connection. If a peer accepts, or does not answer a short probe, startup fails and leaves the file in place. The parent directory is mode `0700` when serve creates it. The socket is mode `0600`.

A client read waits at most 5 seconds for a frame. Serve accepts more than one client. A state change is broadcast as `state_changed`. A rejected command is a `response` with `status: err` and does not emit `state_changed`. `event: error` is reserved for a failure that is not the reply to one request.

`reload_soul` re-reads `soul.md` and `user.md` from disk and sets `soul_reload_pending`. The new text applies on the next transition into awake; an in-flight awake session keeps the instructions it already applied. The flag clears only after that apply succeeds. A missing or invalid pack refuses awake and leaves the flag set. `set_config` stays absent.

Status gained an optional `soul` object (`ok`, and `reason` when the pack is not valid) with `serde` default. Omitted on older peers, ignored by older clients. That did not change the meaning of existing fields, so the protocol generation stays `1`. The hello handshake is unchanged. This replaces the earlier note that `reload_soul` only recorded a request and did not parse the pack.

Clients may also send `tool_request` (`id`, `name`, and `args`). `args` defaults to an empty list when it is omitted. The daemon runs a safe tool only while awake, then broadcasts `tool_started` and `tool_finished` before the response. A confirm-gated tool does not run: the response is `status: ok` with `pending_tool`, and the daemon broadcasts `tool_confirm_pending`. A refusal is a `response` with `status: err` and `kind: tool_forbidden`, `unknown_tool`, `tool_denied`, or `confirmation_pending`.

`confirm_tool` and `cancel_tool` carry `id`, `pending_id`, and an optional `name`. Confirm runs the pending tool once while awake and broadcasts `tool_confirm_resolved` (`accepted: true`), then `tool_started` and `tool_finished`. Cancel clears it in any voice state (`accepted: false`) and does not run the tool. A missing id is `unknown_pending`. A name that does not match is `pending_mismatch`. Confirm while not awake, when the record is still there, is `confirm_forbidden`. Sleep and hibernate clear a pending confirmation first, so a later confirm for that id is `unknown_pending`.

Status may include `pending_tool` and `last_tool`. Both are omitted when absent, so older payloads still decode. These messages are additive. The protocol generation stays `1`. See [ADR 0004](ADR-0004-first-safe-tool.md) and [ADR 0005](ADR-0005-tool-confirmation.md).

Clients may send `ask` (`id`, `text`). The daemon completes one chat turn only while awake. Success is `status: ok` and `message` holds the assistant text. Refusal is `status: err` and `kind: chat_rejected`. No event is broadcast. The bearer is not on the wire. `ctl chat` uses this same message. The hello handshake is unchanged. See [ADR 0013](ADR-0013-session-provider.md).

Lines longer than 1 MiB, including the newline, are rejected.

## Context

The UI has to show sleep, awake, and hibernate, and it has to send hibernate, wake-from-hibernate, sleep, and reload-soul without owning the state machine. A TCP port would be reachable beyond the user. A length-prefixed frame is unambiguous but harder to read in a log. These messages are small.

Phase 1 does not take a tokio runtime. Blocking std threads are enough for a handful of local clients.

## Alternatives

- Length-prefixed frames (`u32` little-endian plus a JSON body). Slightly stricter for binary payloads. Rejected for phase 1 because a line is enough and can be read with ordinary tools.
- JSON-RPC over a TCP port on localhost. Easy to point a browser at, and also easy to expose by mistake. Rejected.
- Embedding the window in the daemon process. The UI would share the state machine. Rejected. The UI stays a client.

## Consequences

- `softwaked serve` is the long-running process. `softwaked ctl` and `softwake-ui` are clients. They do not apply transitions themselves.
- `softwaked` with no arguments, `demo`, and `--help` stay short-lived.
- A crashed serve can leave the socket file. The next serve removes it when the probe is refused.
- A client that stops reading can fill a small outbound queue. Further events for that client are dropped until it reads again. `get_status` still reports the current state.
- The Tauri window polls `get_status`. It can also be a long-lived subscriber later. The broadcast is already there.
- `soul` on status is additive. Clients that do not know the field still parse the rest of a status payload.
