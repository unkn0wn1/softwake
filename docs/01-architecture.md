# Architecture

## Process model

```
┌─────────────────────────────────────────────────────────┐
│  softwake-ui (Tauri)                                   │
│  settings · status · hibernate/wake · soul reload       │
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
                   (soul.md, user.md, rules.md, glossary.md)
```

## Crates (planned monorepo)

Keep crates small and single-purpose. Exact names can shift; responsibilities should not.

| Crate | Responsibility |
|-------|----------------|
| `softwake-daemon` | Binary: state machine, IPC server, wiring |
| `softwake-state` | Sleep / awake / hibernate transitions and invariants |
| `softwake-audio` | Capture trait, mock backend, PipeWire stub. `pipewire-native` is off unless a developer opts in |
| `softwake-voice` | Awake STT/TTS boundary (mock default; sherpa stubs feature-gated; [ADR 0007](ADR-0007-awake-stt-tts.md)) |
| `softwake-wake` | Local wake/sleep phrases. Text table for the typed demo. PCM seam for sherpa-onnx keyword spotting ([ADR 0006](ADR-0006-on-device-wake.md)) |
| `softwake-session` | Text session for one awake period. Stores that rendered pack. Memory snippets are still not attached. No model client yet |
| `softwake-tools` | Tool registry with safe, confirm, and deny metadata. `echo` is safe, `notify` and `email_send` wait for confirmation, `shell` is denied |
| `softwake-connectors` | World I/O boundary. Email, Drive, and calendar traits with in-memory mocks. Registry is confirm or deny. No live cloud client in the default build. The daemon calls the email mock only ([ADR 0008](ADR-0008-connector-boundary.md)) |
| `softwake-policy` | Classifies tool names and connector pairs. Unknown subjects are denied. Overrides may only tighten. The daemon asks it before a tool runs ([ADR 0010](ADR-0010-policy-engine.md)) |
| `softwake-providers` | Model credentials, xAI device-code OAuth, Test probes, secret bag. Mock transport by default; `live-http` for real HTTPS ([ADR 0012](ADR-0012-model-providers.md)) |
| `softwake-memory` | Long-term memory boundary. `Memory` trait, in-memory mock, and opt-in JSON file. Off until enabled. The daemon does not call it ([ADR 0009](ADR-0009-long-term-memory.md)) |
| `softwake-soul` | Load four files (`soul.md`, `user.md`, `rules.md`, `glossary.md`), render instructions, and expand glossary aliases ([ADR 0011](ADR-0011-context-pack.md)) |
| `softwake-ipc` | Shared protocol: newline-delimited JSON over a Unix socket |
| `softwake-ui` | Tauri window (thin). Status and buttons call the daemon. Provider Settings call `softwake-providers` in-process |

Do not put PipeWire types into `softwake-soul`. Do not put HTTP clients into `softwake-state`.

## Trust boundaries

1. **UI is untrusted for action.** It may request hibernate/wake and edit config; the daemon enforces policy through `softwake-policy` ([ADR 0010](ADR-0010-policy-engine.md)).
2. **Tools run out-of-process** where practical, with explicit argv/env and timeouts. Phase 1's `echo` tool stays in-process because it performs no I/O.
3. **Secrets** stay in OS keychain / env; never in soul markdown committed to git.
4. **Network** only from session and explicitly allowed tools — not from the wake engine. A connector backend may use the network only in a future opt-in feature. The default mock does not open a socket. The memory mock does not use the network.

## Audio path

The typed demo and `softwaked serve` use `MockAudioCapture` plus the text phrase table ([ADR 0002](ADR-0002-wake-engine-spike.md)). The production wake engine is sherpa-onnx keyword spotting ([ADR 0006](ADR-0006-on-device-wake.md)). Weights are not in the repo. `NullDetector` is the PCM stand-in and returns no hit. Each captured frame is still passed to `WakeDetector::push_samples`.

`AudioFormat::WAKE` is 16 kHz mono `i16`. `AudioCapture::poll_frame` is how both the mock and a future native backend hand over one `AudioFrame`.

`PipeWireCapture` implements the capture trait. The default `pipewire` feature does not pull a system library: `start` returns `PipeWireError::NotLinked`, and `poll_frame` returns `PipeWireError::NotRunning` so the stub is not an idle microphone. `pipewire-native` is the opt-in feature. It does not link `libpipewire` yet (`PipeWireError::StreamUnwired`) and CI does not enable it. A developer can compile that feature with `cargo test -p softwake-audio --features pipewire-native`. A later build that links the library needs `libpipewire-0.3-dev` and must stay out of the default CI job.

- Capture via PipeWire (native bindings behind the trait, feature-gated). The default path is the mock.
- **Hibernate:** tear down capture; no frames to the wake engine.
- **Sleep:** capture + wake engine only; no tool dispatch; no model “acting” channel.
- **Awake:** capture may feed both wake-for-sleep-phrase and the active voice/session path (must not miss the sleep phrase). The typed demo still applies the text detector's hit.

Chromium/Electron/Capacitor WebView audio is **out of scope** for the daemon. Do not put mic capture in a webview — the daemon owns the ear.

## IPC

Unix domain socket and newline-delimited JSON, protocol version 1. The path, framing, and hello handshake are in [ADR 0003](ADR-0003-ipc-transport.md).

- The UI and `softwaked ctl` may request an action. The daemon owns the state machine and accepts or rejects it.
- The client and daemon exchange a hello before any command. A version mismatch closes the connection.
- Commands: `get_status`, `hibernate`, `wake_from_ui`, `sleep`, `reload_soul`.
- `tool_request` runs one safe tool, or stages a confirm-gated tool (`id`, `name`, `args`). `args` may be omitted. The daemon answers with the same response shape as a command. A safe run also broadcasts `tool_started` and `tool_finished`. A confirm-gated tool broadcasts `tool_confirm_pending` and does not run. `confirm_tool` / `cancel_tool` carry the pending id. Protocol generation stays 1. See [ADR 0003](ADR-0003-ipc-transport.md), [ADR 0004](ADR-0004-first-safe-tool.md), and [ADR 0005](ADR-0005-tool-confirmation.md).
- `set_config` is intentionally absent until the daemon can validate a configuration document.
- Events: `state_changed`, `partial_transcript` (awake only; not emitted yet), `tool_started`, `tool_finished`, `tool_confirm_pending`, `tool_confirm_resolved`, `error`.
- A rejected command is an error response. `state_changed` is broadcast only when the voice state changes.
- `reload_soul` re-reads `soul.md`, `user.md`, `rules.md`, and `glossary.md` from disk. The new text applies on the next awake, not in the middle of an awake session. A missing or invalid pack, including an unparseable glossary, refuses awake; hibernate, sleep, and UI resume still run.
- Status may include `soul`: `{ "ok": true }` or `{ "ok": false, "reason": "..." }`. The field is optional on the wire so older payloads still decode. Protocol generation stays 1.
- Soul directory, first match: `--soul-dir`, `SOFTWAKE_SOUL_DIR`, `$XDG_CONFIG_HOME/softwake/soul`, `~/.config/softwake/soul`.
- `softwaked serve` listens. `softwaked ctl` and the Tauri window connect to it. Default path: `$XDG_RUNTIME_DIR/softwake/softwaked.sock`, or `/tmp/softwake-$UID/softwaked.sock` when `XDG_RUNTIME_DIR` is unset.

## Tool bus

- Tools are named and carry a risk: safe, confirm, or deny ([ADR 0005](ADR-0005-tool-confirmation.md)). Phase 1's only tool was `echo` ([ADR 0004](ADR-0004-first-safe-tool.md)).
- `echo` is safe. With no arguments it returns `pong`. With arguments it returns `echo:` plus those arguments joined by spaces. It does not touch a shell, the filesystem, the clipboard, or an audio device.
- `notify` is confirm-gated. It appends one line to an in-memory sink only after `confirm_tool`. `shell` is denied and never runs.
- `email_send` is confirm-gated. Arguments are a recipient, a subject, and a body. It appends one message to an in-memory outbox only after `confirm_tool` ([ADR 0008](ADR-0008-connector-boundary.md)).
- The daemon calls `permit_tool_dispatch` first. Sleep and hibernate refuse every tool and clear a pending confirmation. An unknown name is refused while awake. One confirmation may be pending; a second confirm-gated request is rejected.
- Entering awake opens a text session with the rendered soul instructions. Sleep, and hibernate from awake, close that session.
- A real shell and deleting files stay denied until a later ADR gives them a confirm path. `email_send` is the confirm path for one in-memory message ([ADR 0008](ADR-0008-connector-boundary.md)). Confirmation here does not make `shell` runnable.
- “Full device control” is a product vision, not an architecture excuse to skip the registry.

## Connectors

World I/O is a library boundary in `softwake-connectors` ([ADR 0008](ADR-0008-connector-boundary.md)). The daemon calls that crate from `Hands` when a confirmed `email_send` runs: `authorize_confirmed` for `email` / `send`, then `EmailConnector::send` on a `MockEmail`. The registry is confirm or deny: `email` / `send`, `drive` / `list`, and `calendar` / `list` are confirm; `email` / `delete`, `drive` / `delete`, and `calendar` / `delete` are deny. `MockEmail` appends to an in-memory outbox when the caller sends after authorization. `MockDrive::list` and `MockCalendar::list` return what that value stores. The daemon does not call those list mocks. The registry methods themselves do not send or list. Before the email send, `softwake-policy` must evaluate `email` / `send` as confirm. A live backend, when one exists, is an opt-in feature that CI does not enable. The default mocks do not open a socket. Protocol generation stays 1; connector actions are not socket commands.

## Model providers

Credentials and Settings Test live in `softwake-providers` ([ADR 0012](ADR-0012-model-providers.md)). Three kinds ship: xAI device-code OAuth, xAI API key, and OpenAI API key. Secrets are a plaintext-at-rest bag under `$XDG_STATE_HOME/softwake/secrets.json` (mode `0600`) with a documented warning. Non-secret selection and the Test model cache are `$XDG_CONFIG_HOME/softwake/providers.json`. The model dropdown stays empty until Test succeeds. Default crate tests use `MockTransport` and do not open a socket. The `live-http` feature enables `ureq`. The window Settings panel talks to this crate through Tauri commands. IPC protocol generation stays 1. The daemon and text session do not call a chat model yet.

## Policy

Tool and connector classification goes through `softwake-policy` ([ADR 0010](ADR-0010-policy-engine.md)). `PolicyEngine::evaluate` reads the static tool registry and the static connector registry. It does not copy those rows. An unknown tool name or connector pair is deny. A connector evaluation is confirm or deny. An override, when one is supplied, may only raise risk. The daemon's engine has an empty override map, so the registered rows are unchanged.

`Hands::request` branches on that evaluation while awake. Sleep and hibernate still refuse every tool before the engine runs. `echo` still runs immediately. `notify` and `email_send` still wait for `confirm_tool`. `shell` is still denied. An unknown tool name is still reported as unknown. Protocol generation stays 1.

## Memory

Long-term memory is a library boundary in `softwake-memory` ([ADR 0009](ADR-0009-long-term-memory.md)). `Memory` is the trait (`remember`, `recall`, `forget`). `MockMemory` is the default backend. It stores snippets on the value only after that value is enabled, and it does not open a socket or write a file. `FileMemory` is the opt-in file backend. It stays off until `open_enabled`, then writes `memory.json` under `$XDG_STATE_HOME/softwake` when that variable is set and non-blank, and under `~/.local/state/softwake` otherwise. The directory is created on the first successful write. The daemon and `softwake-session` do not call the crate. Soul rendering stays instruction-only. Honcho is not a dependency.

## Config layout (draft)

```
~/.config/softwake/
  config.toml
  soul/
    soul.md
    user.md
    rules.md
    glossary.md
~/.local/state/softwake/
  runtime.json          # last state, pid hints
  memory.json           # snippets, only after an enabled FileMemory writes
~/.local/share/softwake/
  logs/
```

Project-local override optional later (`./.softwake/`) with the same trust caveats as other local config systems.
