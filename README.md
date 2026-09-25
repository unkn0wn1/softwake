# Softwake

Voice-first local conductor: asleep until hailed, awake with tools, hibernate when you want silence.

Rust end-to-end (daemon + Tauri UI). Phase 1 nails reliable **wake / sleep / hibernate** and a single safe tool loop. Phase 2 adds a confirmation gate for one risky tool. Personality and rules live in a **soul pack** (`soul.md`, `user.md`). Long-term memory (e.g. Honcho as an optional later example) is planned later, not required to run the daemon.

## Status

Cargo workspace on stable Rust (edition 2024). `softwake-state` implements sleep / awake / hibernate, including rejected transitions and phrase cooldowns. `softwake-audio` has a mock capture backend (`stop` ends frame delivery) and a trait-shaped PipeWire stub (default `pipewire` feature, no native library). `softwake-wake` scores configured phrases with a local text matcher ([ADR 0002](docs/ADR-0002-wake-engine-spike.md)). The production on-device wake engine is sherpa-onnx keyword spotting ([ADR 0006](docs/ADR-0006-on-device-wake.md)). While awake, STT/TTS use a local streaming boundary ([ADR 0007](docs/ADR-0007-awake-stt-tts.md)): mock inject/record by default, sherpa stubs behind features. Weights are not in the repo, and the demo stays typed. `softwake-soul` loads `soul.md` and `user.md`, checks them, and renders system instructions. `softwaked serve` speaks newline-delimited JSON on a Unix socket ([ADR 0003](docs/ADR-0003-ipc-transport.md)). `softwaked ctl` and the Tauri window `softwake-ui` are clients of that socket. Entering awake opens a text session with the rendered soul instructions. `echo` runs immediately while awake ([ADR 0004](docs/ADR-0004-first-safe-tool.md)). `notify` waits for confirmation and then appends a line to an in-memory sink. `shell` is denied ([ADR 0005](docs/ADR-0005-tool-confirmation.md)). Phase 3 wires the connector boundary to the tool bus ([ADR 0008](docs/ADR-0008-connector-boundary.md)): `email_send` waits for confirmation, and confirming it appends one message to an in-memory outbox. Other connector actions are denied. The default build has no live cloud client. A model client and further tools are later work. A missing soul pack refuses awake; `reload-soul` re-reads the files and applies on the next awake.

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
cp soul/soul.md soul/user.md ~/.config/softwake/soul/
cargo run -p softwake-daemon -- demo
```

```text
softwaked demo
typed commands only — mock capture; native PipeWire is feature-gated
state: sleep
capture: running
soul: ok
commands: wake, sleep, hibernate, resume, status, reload-soul, tool, confirm, cancel, hear, say, quit
>
```

Without those files the status line is `soul: missing` and `wake` stays in sleep. `hibernate`, `resume`, and `sleep` still run. `reload-soul` reads the directory again. `tool` is refused until a wake succeeds.

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

`softwaked demo --verbose` and `softwaked demo -v` (also `--demo -v`) print extra `verbose:` lines for each command: the raw input, the parsed command, and for `wake` / `sleep` the phrase, the detector hit, and whether the transition succeeded or why it was rejected. `SOFTWAKE_LOG=debug` enables that same detail. `softwaked --help` prints usage.

Type one command per line. `sleep` in the 800 ms after `wake` stays awake. `wake` in the 800 ms after `sleep` or `resume` stays asleep. `hibernate` is a UI command and applies on the next line.

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
cargo run -p softwake-daemon -- ctl resume    # back to sleep, not awake
cargo run -p softwake-daemon -- ctl sleep     # rejected while already asleep
cargo run -p softwake-daemon -- ctl reload-soul
cargo run -p softwake-daemon -- ctl tool echo hello
cargo run -p softwake-daemon -- ctl tool notify hello
cargo run -p softwake-daemon -- ctl confirm-tool 1
cargo run -p softwake-daemon -- ctl cancel-tool 1
```

`ctl` prints `state`, `capture`, `soul` (`ok` or `missing`), and `soul reload`, and exits non-zero when the daemon rejects the command or cannot be reached. `resume` is wake-from-hibernate and lands in sleep. `reload-soul` re-reads the soul pack from disk. The new text applies on the next awake session, not in the middle of one that is already awake. `ctl tool` runs one safe tool, or stages a confirm-gated tool. The daemon starts in sleep, so `ctl tool echo hello` is refused until the daemon is awake. The typed demo is the path that enters awake. A successful `echo` prints its result on the line after the status lines (`echo: hello`, or `pong` when `echo` has no arguments). `ctl tool notify hello` prints the pending id and does not append. `ctl tool email_send ada@example.com hello body` does the same, and `ctl confirm-tool <id>` appends one in-memory message. `ctl cancel-tool <id>` drops the pending call.

The socket path is the first match of `--socket PATH`, `SOFTWAKE_SOCKET`, `$XDG_RUNTIME_DIR/softwake/softwaked.sock`, and `/tmp/softwake-$UID/softwaked.sock` when `XDG_RUNTIME_DIR` is unset.

The soul directory is the first match of `--soul-dir PATH` (on `serve` and `demo`), `SOFTWAKE_SOUL_DIR`, `$XDG_CONFIG_HOME/softwake/soul`, and `~/.config/softwake/soul` when `XDG_CONFIG_HOME` is unset. See [docs/03-soul-pack.md](docs/03-soul-pack.md).

## Window

`softwake-ui` is a small Tauri window: the current state, whether the soul pack is `ok` or `missing`, the latest tool line, and buttons for Hibernate, Wake (leave hibernate into sleep), Sleep, and Reload soul. When a confirm-gated tool is waiting, the window shows that text and enables Confirm and Cancel. It only talks to the socket. Start `softwaked serve` first.

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
| [docs/03-soul-pack.md](docs/03-soul-pack.md) | soul.md, user.md, memory later |
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
| [docs/ADR-0008-connector-boundary.md](docs/ADR-0008-connector-boundary.md) | Connector boundary: mock email, confirm or deny |
