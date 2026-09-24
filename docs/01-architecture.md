# Architecture

## Process model

```
┌─────────────────────────────────────────────────────────┐
│  softwake-ui (Tauri)                                   │
│  settings · status · hibernate/wake · soul editor       │
└───────────────────────────┬─────────────────────────────┘
                            │ Unix socket, newline-delimited JSON
┌───────────────────────────▼─────────────────────────────┐
│  softwaked (Rust daemon)                               │
│  state machine · wake gate · session · tool dispatcher  │
├─────────────┬─────────────┬─────────────┬───────────────┤
│ audio I/O   │ wake engine │ model/voice │ tool runners  │
│ (PipeWire)  │ (local)     │ (pluggable) │ (allowlisted) │
└─────────────┴─────────────┴─────────────┴───────────────┘
                            │
                            ▼
                   soul pack on disk
                   (soul.md, user.md, …)
```

## Crates (planned monorepo)

Keep crates small and single-purpose. Exact names can shift; responsibilities should not.

| Crate | Responsibility |
|-------|----------------|
| `softwake-daemon` | Binary: state machine, IPC server, wiring |
| `softwake-state` | Sleep / awake / hibernate transitions and invariants |
| `softwake-audio` | Capture trait, mock backend, PipeWire stub |
| `softwake-wake` | Local wake/sleep phrases. Text table for the spike; PCM engine later |
| `softwake-session` | Text session for one awake period. Stores rendered soul instructions. No model client yet |
| `softwake-tools` | Tool registry and allowlist. Phase 1 registers `echo` only |
| `softwake-soul` | Load/validate soul pack; render system instructions |
| `softwake-ipc` | Shared protocol: newline-delimited JSON over a Unix socket |
| `softwake-ui` | Tauri window (thin). Status and buttons call the daemon |

Do not put PipeWire types into `softwake-soul`. Do not put HTTP clients into `softwake-state`.

## Trust boundaries

1. **UI is untrusted for action.** It may request hibernate/wake and edit config; the daemon enforces policy.
2. **Tools run out-of-process** where practical, with explicit argv/env and timeouts. Phase 1's `echo` tool stays in-process because it performs no I/O.
3. **Secrets** stay in OS keychain / env; never in soul markdown committed to git.
4. **Network** only from session and explicitly allowed tools — not from the wake engine.

## Audio path (phase 1)

Current spike: `MockAudioCapture` plus a text phrase table ([ADR 0002](ADR-0002-wake-engine-spike.md)). `PipeWireCapture` implements the capture trait and reports that native I/O is not linked yet. The `pipewire` feature does not pull a system library.

- Capture via PipeWire (`pw-record` or native bindings behind a trait).
- **Hibernate:** tear down capture; no frames to wake engine.
- **Sleep:** capture + wake engine only; no tool dispatch; no model “acting” channel.
- **Awake:** capture may feed both wake-for-sleep-phrase and the active voice/session path (design detail in spike — must not miss sleep phrase).

Chromium/Electron/Capacitor WebView audio is **out of scope** for the daemon. Do not put mic capture in a webview — the daemon owns the ear.

## IPC

Unix domain socket and newline-delimited JSON, protocol version 1. The path, framing, and hello handshake are in [ADR 0003](ADR-0003-ipc-transport.md).

- The UI and `softwaked ctl` may request an action. The daemon owns the state machine and accepts or rejects it.
- The client and daemon exchange a hello before any command. A version mismatch closes the connection.
- Commands: `get_status`, `hibernate`, `wake_from_ui`, `sleep`, `reload_soul`.
- `tool_request` runs one allowlisted tool (`id`, `name`, `args`). `args` may be omitted. The daemon answers with the same response shape as a command. A successful run also broadcasts `tool_started` and `tool_finished`. Protocol generation stays 1. See [ADR 0003](ADR-0003-ipc-transport.md) and [ADR 0004](ADR-0004-first-safe-tool.md).
- `set_config` is intentionally absent until the daemon can validate a configuration document.
- Events: `state_changed`, `partial_transcript` (awake only; not emitted yet), `tool_started`, `tool_finished` (emitted when `echo` runs), `error`.
- A rejected command is an error response. `state_changed` is broadcast only when the voice state changes.
- `reload_soul` re-reads `soul.md` and `user.md` from disk. The new text applies on the next awake, not in the middle of an awake session. A missing or invalid pack refuses awake; hibernate, sleep, and UI resume still run.
- Status may include `soul`: `{ "ok": true }` or `{ "ok": false, "reason": "..." }`. The field is optional on the wire so older payloads still decode. Protocol generation stays 1.
- Soul directory, first match: `--soul-dir`, `SOFTWAKE_SOUL_DIR`, `$XDG_CONFIG_HOME/softwake/soul`, `~/.config/softwake/soul`.
- `softwaked serve` listens. `softwaked ctl` and the Tauri window connect to it. Default path: `$XDG_RUNTIME_DIR/softwake/softwaked.sock`, or `/tmp/softwake-$UID/softwaked.sock` when `XDG_RUNTIME_DIR` is unset.

## Tool bus

- Tools are named and allowlisted. Phase 1's allowlist is exactly `echo` ([ADR 0004](ADR-0004-first-safe-tool.md)).
- `echo` is a pure function. With no arguments it returns `pong`. With arguments it returns `echo:` plus those arguments joined by spaces. It does not touch a shell, the filesystem, the clipboard, or an audio device.
- The daemon calls `permit_tool_dispatch` first. Sleep and hibernate refuse every tool. An unknown name is refused even while awake.
- Entering awake opens a text session with the rendered soul instructions. Sleep, and hibernate from awake, close that session.
- Dangerous tools (shell, send email, delete files) require explicit policy and confirmation — later phases.
- “Full device control” is a product vision, not an architecture excuse to skip allowlists.

## Config layout (draft)

```
~/.config/softwake/
  config.toml
  soul/
    soul.md
    user.md
~/.local/state/softwake/
  runtime.json          # last state, pid hints
~/.local/share/softwake/
  logs/
```

Project-local override optional later (`./.softwake/`) with the same trust caveats as other local config systems.
