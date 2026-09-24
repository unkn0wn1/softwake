# Architecture

## Process model

```
┌─────────────────────────────────────────────────────────┐
│  softwake-ui (Tauri)                                   │
│  settings · status · hibernate/wake · soul editor       │
└───────────────────────────┬─────────────────────────────┘
                            │ localhost IPC (JSON-RPC or similar)
┌───────────────────────────▼─────────────────────────────┐
│  softwaked (Rust daemon)                               │
│  state machine · wake gate · session · tool dispatcher  │
├─────────────┬─────────────┬─────────────┬───────────────┤
│ audio I/O   │ wake engine │ model/voice │ tool runners  │
│ (PipeWire)  │ (local)     │ (pluggable) │ (subprocesses)│
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
| `softwake-session` | Build model session from soul pack; stream events |
| `softwake-tools` | Tool registry, allowlist, confirm policy, runners |
| `softwake-soul` | Load/validate soul pack; render system instructions |
| `softwake-ipc` | Shared protocol types (daemon ↔ UI) |
| `softwake-ui` | Tauri app (thin) |

Do not put PipeWire types into `softwake-soul`. Do not put HTTP clients into `softwake-state`.

## Trust boundaries

1. **UI is untrusted for action.** It may request hibernate/wake and edit config; the daemon enforces policy.
2. **Tools run out-of-process** where practical, with explicit argv/env and timeouts.
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

- Local socket (Unix domain) preferred over binding a LAN port.
- Version the protocol. UI and daemon negotiate version on connect.
- Events: `state_changed`, `partial_transcript` (awake only), `tool_started` / `tool_finished`, `error`.
- Commands: `get_status`, `hibernate`, `wake_from_ui`, `reload_soul`, `set_config` (validated).

## Tool bus

- Tools are named, documented, and allowlisted in config.
- Phase 1 ships **one** safe tool (e.g. set volume or summarize clipboard) to prove the loop.
- Dangerous tools (shell, send email, delete files) require explicit policy + confirmation — later phases.
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
