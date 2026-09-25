# Softwake

Voice-first local conductor: asleep until hailed, awake with tools, hibernate when you want silence.

Rust end-to-end (daemon + Tauri UI). Phase 1 nails reliable **wake / sleep / hibernate** and a single safe tool loop. Phase 2 adds a confirmation gate for one risky tool. Personality and rules live in a **soul pack** (`soul.md`, `user.md`, `rules.md`, `glossary.md`). Long-term memory is a separate local trait ([ADR 0009](docs/ADR-0009-long-term-memory.md)), off by default, and not required to run the daemon.

## Status

Cargo workspace on stable Rust (edition 2024). `softwake-state` implements sleep / awake / hibernate, including rejected transitions and phrase cooldowns. `softwake-audio` has a mock capture backend (`stop` ends frame delivery) and a trait-shaped PipeWire stub (default `pipewire` feature, no native library). `softwake-wake` scores configured phrases with a local text matcher ([ADR 0002](docs/ADR-0002-wake-engine-spike.md)). The production on-device wake engine is sherpa-onnx keyword spotting ([ADR 0006](docs/ADR-0006-on-device-wake.md)). While awake, STT/TTS use a local streaming boundary ([ADR 0007](docs/ADR-0007-awake-stt-tts.md)): mock inject/record by default, sherpa stubs behind features. Weights are not in the repo, and the demo stays typed. `softwake-soul` loads `soul.md`, `user.md`, `rules.md`, and `glossary.md`, checks them, and renders system instructions ([ADR 0011](docs/ADR-0011-context-pack.md)). `softwaked serve` speaks newline-delimited JSON on a Unix socket ([ADR 0003](docs/ADR-0003-ipc-transport.md)). `softwaked ctl` and the Tauri window `softwake-ui` are clients of that socket. Entering awake opens a text session with the rendered soul instructions. `echo` runs immediately while awake ([ADR 0004](docs/ADR-0004-first-safe-tool.md)). `notify` waits for confirmation and then appends a line to an in-memory sink. `shell` is denied ([ADR 0005](docs/ADR-0005-tool-confirmation.md)). Phase 3 wires the connector boundary to the tool bus ([ADR 0008](docs/ADR-0008-connector-boundary.md)): `email_send` waits for confirmation, and confirming it appends one message to an in-memory outbox. `MockDrive` and `MockCalendar` list files and events stored on that value. In the connector registry, `drive` / `list` and `calendar` / `list` are confirm, and delete actions are denied. Those list mocks are not tools on the bus. The default build has no live cloud client. `softwake-memory` is a `Memory` trait, an in-memory `MockMemory`, and an opt-in `FileMemory` that writes `memory.json` only after `open_enabled`. Both stay off until that value is enabled. When `memory.json` exists under the Softwake state directory, awake `ask` / `chat` attach a budgeted recall appendix after the rendered pack ([ADR 0009](docs/ADR-0009-long-term-memory.md), [ADR 0013](docs/ADR-0013-session-provider.md)). Missing or disabled memory is fail-open (no snippets). `softwake-policy` classifies the existing tool and connector allowlists. Unknown names are denied. The daemon uses that classification, and its override map is empty ([ADR 0010](docs/ADR-0010-policy-engine.md)). `softwake-providers` holds xAI device-code OAuth, xAI API key, OpenAI API key, OpenRouter API key, and OpenAI-compatible (key + base URL) Settings ([ADR 0012](docs/ADR-0012-model-providers.md)). Secrets stay in an XDG state bag (plaintext v1 with a warning). The window Settings panel runs Test, then fills the model picker. Mock transport keeps default tests offline; `live-http` enables real HTTPS. While awake, typed `ask` and `chat` in `softwaked demo` send the rendered context pack and the user line to the selected provider ([ADR 0013](docs/ADR-0013-session-provider.md)). Default tests use `MockTransport`. The daemon `live-http` feature performs the real call and is off by default. `softwaked ctl ask` and `ctl chat` send that same turn to a running `softwaked serve`. `softwaked ctl wake` enters awake on a running `softwaked serve` when the four-file pack is valid. `ctl resume` still lands in sleep. Serve still has no microphone wake. Protocol generation stays 1. OpenRouter and OpenAI-compatible base URL are available in Settings. Budgeted memory snippets on ask/chat are shipped. A missing or invalid four-file pack refuses awake; `reload-soul` re-reads that pack and applies on the next awake.

## Build and test

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace --all-targets
```

See [Cargo features](#cargo-features) for `pipewire`, `pipewire-native`, and `sherpa-kws`.

## Cargo features

| Crate | Feature | Default | What it compiles |
|-------|---------|---------|------------------|
| `softwake-audio` | `pipewire` | yes | Capture stub. Does not link `libpipewire`. |
| `softwake-audio` | `pipewire-native` | no | Same stub. The stream stays unwired. Not enabled in CI. |
| `softwake-wake` | `sherpa-kws` | no | PCM detector stub. No weights and no ONNX download. Not enabled in CI. |
| `softwake-voice` | `sherpa-asr` | no | Streaming ASR stub. No weights and no ONNX download. Not enabled in CI. |
| `softwake-voice` | `sherpa-tts` | no | TTS stub. No weights and no synthesizer download. Not enabled in CI. |
| `softwake-providers` | `live-http` | no | Real HTTPS via `ureq` for OAuth and Test. Unit tests use `MockTransport`. Not required for `cargo test -p softwake-providers`. |
| `softwake-daemon` | `live-http` | no | Enables `softwake-providers/live-http` so typed `ask` / `chat` can call the selected provider. The default build rejects that call and does not open a socket. |
| `softwake-ui` | `live-http` | yes | Enables `softwake-providers/live-http` so Settings Test and xAI sign-in can reach the network. |

`--no-default-features` on `softwake-audio` omits the `pipewire` stub. The sherpa-onnx keyword-spotting choice is [ADR 0006](docs/ADR-0006-on-device-wake.md).

```bash
cargo test -p softwake-audio --features pipewire-native
cargo test -p softwake-wake --features sherpa-kws
cargo test -p softwake-voice --features sherpa-asr,sherpa-tts
```

A later native PipeWire stream will also need `libpipewire-0.3-dev`. This build does not link that library.

## Install

From the repository root, using the workspace `Cargo.lock`:

```bash
cargo install --path crates/softwake-daemon --locked
```

That installs the `softwaked` binary. The window binary is `softwake-ui`. From a checkout, start `softwaked serve`, then:

```bash
cargo run -p softwake-ui
```

The same lockfile install for the window:

```bash
cargo install --path crates/softwake-ui --locked
```

Linux packages for the window are listed under [Window](#window). Socket and soul paths are under [Serve and ctl](#serve-and-ctl).

[`packaging/softwake.desktop`](packaging/softwake.desktop) is a sample launcher. `Exec` is `softwake-ui` and `Icon` is the theme name `softwake`. After `softwake-ui` is on `PATH`, copy the file to `~/.local/share/applications/` and set `Icon=` to an icon you provide.

## Demo

`softwaked` with no arguments prints the initial voice state and exits:

```bash
cargo run -p softwake-daemon
```

```text
softwaked state: sleep
```

The interactive demo is typed commands only. The microphone is not opened. Mock capture is the default, and native PipeWire is an optional feature that is not linked in the default build. It starts in sleep with mock capture running. `wake` and `sleep` submit the configured phrases to the text detector, then apply the voice-state machine, including the 800 ms phrase cooldown. `hibernate` stops capture. A voice command is rejected until `resume`, which returns to sleep and starts capture again. `wake` also requires a valid soul pack. Copy the repo templates into the config directory first (or pass `--soul-dir`):

```bash
mkdir -p ~/.config/softwake/soul
cp soul/*.md ~/.config/softwake/soul/
cargo run -p softwake-daemon -- demo
```

```text
softwaked demo
typed commands only — mock capture; native PipeWire is feature-gated
state: sleep
capture: running
soul: ok
commands: wake, sleep, hibernate, resume, status, reload-soul, tool, confirm, cancel, hear, say, ask, chat, quit
>
```

A directory that only has `soul.md` and `user.md` refuses awake until `rules.md` and `glossary.md` are copied too. Without those files the status line is `soul: missing` and `wake` stays in sleep. `hibernate`, `resume`, and `sleep` still run. `reload-soul` reads the directory again. `tool` is refused until a wake succeeds.

Wake, run the tool, sleep, then hibernate:

```text
> wake
heard: "hey softwake" -> wake
transition sleep -> awake (wake phrase)
effect: open session
state: awake
capture: running
soul: ok
> tool echo hello
tool echo: echo: hello
state: awake
capture: running
soul: ok
last tool: echo safe ran
> tool echo
tool echo: pong
state: awake
capture: running
soul: ok
last tool: echo safe ran
> sleep
heard: "softwake sleep" -> sleep
transition awake -> sleep (sleep phrase)
effect: release acting resources
state: sleep
capture: running
soul: ok
last tool: echo safe ran
> hibernate
transition sleep -> hibernate (UI hibernate)
effect: stop capture
state: hibernate
capture: stopped
soul: ok
last tool: echo safe ran
```

`sleep` in the 800 ms after `wake` stays awake, so a fast paste of `wake` then `sleep` will not leave awake. `hibernate` from awake does not use that cooldown. It closes the session and stops capture. `tool volume` is rejected. `tool shell` is denied. `tool` while asleep or hibernating is rejected.

While awake, inject a mock STT transcript or record mock TTS (no microphone, no model download — [ADR 0007](docs/ADR-0007-awake-stt-tts.md)):

```text
> hear hello there
partial transcript: "hello there"
final transcript: "hello there"
state: awake
capture: running
soul: ok
> say hello
said: "hello"
state: awake
capture: running
soul: ok
last said: hello
```

`hear` and `say` are refused while asleep or hibernating. Sleep and hibernate drop any queued mock STT and do not speak.

While awake, `ask` and `chat` send the rendered context pack and the typed line to the provider chosen in Settings ([ADR 0013](docs/ADR-0013-session-provider.md)):

```text
> ask hello
assistant: <model text>
state: awake
capture: running
soul: ok
```

`chat` is the same command. The same turn against a running daemon is `softwaked ctl ask` or `softwaked ctl chat` once that `softwaked serve` process is awake. `softwaked ctl wake` enters awake on that serve when the four-file pack is valid, and `ctl resume` still lands in sleep. Serve still has no microphone wake. See [Serve and ctl](#serve-and-ctl). Without the daemon `live-http` feature, a fully configured Settings file still gets `Live HTTP is not enabled in this build. Re-run with the live-http feature to call the provider.` and does not open a provider socket. With the feature:

```bash
cargo run -p softwake-daemon --features live-http -- demo
```

Then `wake`, then `ask hello`.

Confirm a notification, or cancel one:

```text
> wake
> tool notify hello
pending 1: notify — Append a notification to the in-memory sink.
waiting for confirm
state: awake
capture: running
soul: ok
pending: 1 notify hello
last tool: notify confirm pending
> confirm
confirmed 1: tool notify: hello
state: awake
capture: running
soul: ok
last tool: notify confirm confirmed
notification: hello
> tool notify later
> cancel
cancelled 2: notify
> sleep
```

`confirm-tool 1` is the same confirm with an explicit id. `cancel-tool 1` is the same cancel. The notification line is appended only after confirm. Sleep and hibernate drop a pending confirmation. `sleep` in the 800 ms after `wake` stays awake, including in this confirm example.

Send one in-memory message the same way:

```text
> wake
> tool email_send ada@example.com hello a short note
pending 1: email_send — Append one message to the in-memory outbox.
waiting for confirm
state: awake
capture: running
soul: ok
pending: 1 email_send ada@example.com hello a short note
last tool: email_send confirm pending
> confirm
confirmed 1: tool email_send: sent 1
state: awake
capture: running
soul: ok
last tool: email_send confirm confirmed
email: ada@example.com | hello | a short note
```

The outbox line appears only after `confirm`. `cancel` prints `cancelled 1: email_send` and adds no `email:` line. A call with no body is rejected and does not wait for confirm.

A `> ` prompt is printed before each line is read. The same path accepts a pipe (`printf 'wake\nstatus\nquit\n' | cargo run -p softwake-daemon -- demo`). `softwaked --demo` is the same mode.

`softwaked demo --verbose` and `softwaked demo -v` (also `--demo -v`) print extra `verbose:` lines for each command: the raw input, the parsed command, for `wake` / `sleep` the phrase, the detector hit, and whether the transition succeeded or why it was rejected, and for `ask` / `chat` the provider id and model id. The bearer is not printed. `SOFTWAKE_LOG=debug` enables that same detail. `softwaked --help` prints usage.

Type one command per line. `sleep` in the 800 ms after `wake` stays awake. `wake` in the 800 ms after `sleep` or `resume` stays asleep. `hibernate` is a UI command and applies on the next line.

## Model providers

Settings in `softwake-ui` configure one acting provider ([ADR 0012](docs/ADR-0012-model-providers.md)). That panel is the Providers pane in the window:

1. Choose **xAI sign-in**, **xAI API key**, **OpenAI**, **OpenRouter**, or **OpenAI-compatible**.
2. For a key provider, paste the key and press **Save key**. For **OpenAI-compatible**, also set the **Base URL** (for example `http://127.0.0.1:11434/v1`) and press **Save base URL**. For xAI sign-in, press **Start sign-in**. Softwake opens the verification page in the default browser and shows that address as a link next to the user code. Enter the code on that page, then **Poll** (or wait for the automatic poll). If the browser does not open, use the link in Settings.
3. Press **Test**. On success, the **Chat model** and **Voice model** dropdowns fill from `GET /v1/models` (chat vs speech-to-text split, with a registry seed fallback). Both stay empty until Test succeeds.
4. Pick a chat model (the acting session) and a voice / STT model (stored for a later audio path; Test does not run live STT).

Secrets are stored under `$XDG_STATE_HOME/softwake/secrets.json` (or `~/.local/state/softwake/secrets.json`), mode `0600`. When the OS keyring answers (Linux Secret Service; macOS Keychain and Windows Credential Manager through the same crate), that file is a version-2 pointer and the bag is one keyring item (`softwake` / `secret-bag`). Plaintext is an opt-in fallback (`SOFTWAKE_SECRET_BACKEND=plaintext`, or the Settings button when the keyring is unavailable) and still shows a warning. An existing version-1 file migrates on the first resolved load when the keyring probe succeeds. A pointer is never rewritten as plaintext. `SOFTWAKE_SECRET_BACKEND=keyring` fails closed when the service is down. Non-secret selection and the model cache are `$XDG_CONFIG_HOME/softwake/providers.json`. The public xAI device-code client id is safe to commit; refresh tokens and API keys are not. Environment fallbacks: `XAI_API_KEY`, `OPENAI_API_KEY`, `OPENROUTER_API_KEY`, `OPENAI_COMPATIBLE_API_KEY` when no key is saved.

```bash
cargo test -p softwake-providers
```

Default workspace tests do not call the network. Live HTTPS is the `live-http` feature.

## Serve and ctl

`softwaked serve` keeps the voice-state machine and mock capture running and listens for clients. `softwaked --serve` is the same mode. A stale socket file is removed on startup. If another serve is already listening, startup fails and leaves that socket in place.

```bash
cargo run -p softwake-daemon -- serve
```

In another terminal:

```bash
cargo run -p softwake-daemon -- ctl status
cargo run -p softwake-daemon -- ctl hibernate
cargo run -p softwake-daemon -- ctl status    # hibernate, capture stopped
cargo run -p softwake-daemon -- ctl resume    # back to sleep; does not enter awake
cargo run -p softwake-daemon -- ctl wake      # sleep -> awake; needs a valid soul pack
cargo run -p softwake-daemon -- ctl sleep     # rejected while already asleep
cargo run -p softwake-daemon -- ctl reload-soul
cargo run -p softwake-daemon -- ctl tool echo hello
cargo run -p softwake-daemon -- ctl tool notify hello
cargo run -p softwake-daemon -- ctl confirm-tool 1
cargo run -p softwake-daemon -- ctl cancel-tool 1
# after that same daemon is awake:
cargo run -p softwake-daemon -- ctl ask hello
cargo run -p softwake-daemon -- ctl chat hello there
```

A live answer is opt-in and is not what CI runs:

```bash
cargo run -p softwake-daemon --features live-http -- serve
```

`ctl` prints `state`, `capture`, `soul` (`ok` or `missing`), and `soul reload`, and exits non-zero when the daemon rejects the command or cannot be reached. `ctl ask` and `ctl chat` send one line to that daemon. The reply is the line after the status lines. `ctl wake` enters awake from sleep when `soul.md`, `user.md`, `rules.md`, and `glossary.md` are valid. A missing or invalid file refuses `ctl wake` and leaves the voice state unchanged. Hibernate, sleep, and resume still run. After `ctl sleep` or `ctl resume`, a wake phrase waits out the 800 ms cooldown. `ctl resume` still lands in sleep. Serve still has no microphone wake. `ctl ask` still requires that same process to be awake. Without `live-http`, a ready Settings file still gets `Live HTTP is not enabled in this build. Re-run with the live-http feature to call the provider.` `resume` is wake-from-hibernate and lands in sleep. `reload-soul` re-reads the four-file pack from disk. The new text applies on the next awake, not in the middle of a session that is already awake. `ctl tool` runs one safe tool, or stages a confirm-gated tool. The daemon starts in sleep, so `ctl tool echo hello` is refused until the daemon is awake. Serve has no microphone path into awake. A successful `echo` prints its result on the line after the status lines (`echo: hello`, or `pong` when `echo` has no arguments). `ctl tool notify hello` prints the pending id and does not append. `ctl tool email_send ada@example.com hello body` does the same, and `ctl confirm-tool <id>` appends one in-memory message. `ctl cancel-tool <id>` drops the pending call.

The socket path is the first match of `--socket PATH`, `SOFTWAKE_SOCKET`, `$XDG_RUNTIME_DIR/softwake/softwaked.sock`, and `/tmp/softwake-$UID/softwaked.sock` when `XDG_RUNTIME_DIR` is unset.

The soul directory is the first match of `--soul-dir PATH` (on `serve` and `demo`), `SOFTWAKE_SOUL_DIR`, `$XDG_CONFIG_HOME/softwake/soul`, and `~/.config/softwake/soul` when `XDG_CONFIG_HOME` is unset. See [docs/03-soul-pack.md](docs/03-soul-pack.md).

## Window

`softwake-ui` is a resizable settings window, 860 by 680. A left nav has four panes. Status is selected when the window opens.

**Status** shows the daemon state, whether capture is running, whether the soul pack is `ok` or `missing` (and the reason when the daemon sent one), whether a soul reload is pending, the latest tool line, and a confirm-gated tool when one is waiting. Buttons are Hibernate, Wake (leave hibernate into sleep), Sleep, Reload soul, Confirm, and Cancel. The Status Wake button is `wake_from_ui` / `ctl resume` (hibernate → sleep). `softwaked ctl wake` is the separate command that enters awake from sleep and requires a valid soul pack. Reload reads `soul.md`, `user.md`, `rules.md`, and `glossary.md`. The new text applies on the next awake. The status snapshot does not include the socket path. The window uses the same default socket as `softwaked ctl`. Start `softwaked serve` first. Provider commands are not socket commands.

**Providers** is the model Settings panel ([ADR 0012](docs/ADR-0012-model-providers.md)): choose a provider, save a key or sign in, press Test, then pick a chat model and a voice (STT) model. Both lists stay empty until Test succeeds.

**General** edits `soul.md`, `user.md`, `rules.md`, and `glossary.md` in the resolved soul directory (`SOFTWAKE_SOUL_DIR`, or the XDG default). Save writes the four files. Reload soul applies a valid pack on the next awake.

**Email** says live email is coming later. The window does not send mail.

```bash
cargo run -p softwake-ui
```

On Linux the window links WebKitGTK. The packages used in CI are `libwebkit2gtk-4.1-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`, `patchelf`, `libxdo-dev`, and `libssl-dev`.

## Docs

| Doc | Purpose |
|-----|---------|
| [docs/00-overview.md](docs/00-overview.md) | What it is, non-goals, phases |
| [docs/01-architecture.md](docs/01-architecture.md) | Processes, IPC, audio, tools |
| [docs/02-voice-states.md](docs/02-voice-states.md) | Sleep / wake / hibernate |
| [docs/03-soul-pack.md](docs/03-soul-pack.md) | Four-file context pack; memory is [ADR 0009](docs/ADR-0009-long-term-memory.md) |
| [docs/04-coding-style.md](docs/04-coding-style.md) | KISS, DRY, SRP, Rust rules |
| [docs/05-quality-gates.md](docs/05-quality-gates.md) | CI and definition of done |
| [docs/06-milestones.md](docs/06-milestones.md) | Phase checklist |
| [docs/ADR-0001-name-and-scope.md](docs/ADR-0001-name-and-scope.md) | Name and phase-1 scope |
| [docs/ADR-0002-wake-engine-spike.md](docs/ADR-0002-wake-engine-spike.md) | Text phrase table for the wake spike |
| [docs/ADR-0003-ipc-transport.md](docs/ADR-0003-ipc-transport.md) | Unix socket and newline-delimited JSON |
| [docs/ADR-0004-first-safe-tool.md](docs/ADR-0004-first-safe-tool.md) | Why the first tool is `echo` |
| [docs/ADR-0005-tool-confirmation.md](docs/ADR-0005-tool-confirmation.md) | Safe, confirm, and deny tools |
| [docs/ADR-0006-on-device-wake.md](docs/ADR-0006-on-device-wake.md) | On-device wake engine (sherpa-onnx keyword spotting) |
| [docs/ADR-0007-awake-stt-tts.md](docs/ADR-0007-awake-stt-tts.md) | Awake speech-to-text and text-to-speech |
| [docs/ADR-0008-connector-boundary.md](docs/ADR-0008-connector-boundary.md) | Connector boundary: in-memory email, Drive, and calendar; confirm or deny |
| [docs/ADR-0009-long-term-memory.md](docs/ADR-0009-long-term-memory.md) | Long-term memory: local trait, in-memory mock, opt-in JSON file |
| [docs/ADR-0010-policy-engine.md](docs/ADR-0010-policy-engine.md) | Policy engine: one evaluation path, default deny, tighten-only overrides |
| [docs/ADR-0011-context-pack.md](docs/ADR-0011-context-pack.md) | Context pack and confirm-echo foundation |
| [docs/ADR-0012-model-providers.md](docs/ADR-0012-model-providers.md) | Provider Settings: xAI sign-in, API keys, Test, chat and voice/STT pickers |
| [docs/ADR-0013-session-provider.md](docs/ADR-0013-session-provider.md) | Awake session chat to the selected provider |
| [docs/ADR-0014-skills-hub.md](docs/ADR-0014-skills-hub.md) | Skills hub, refine loop, and webhook wake (direction) |
