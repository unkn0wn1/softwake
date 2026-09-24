# Softwake

Voice-first local conductor: asleep until hailed, awake with tools, hibernate when you want silence.

Rust end-to-end (daemon + Tauri UI). Phase 1 nails reliable **wake / sleep / hibernate** and a single safe tool loop. Personality and rules live in a **soul pack** (`soul.md`, `user.md`). Long-term memory (e.g. Honcho as an optional later example) is planned later, not required for phase 1.

## Status

Cargo workspace on stable Rust (edition 2024). `softwake-state` implements sleep / awake / hibernate, including rejected transitions and phrase cooldowns. `softwake-audio` has a mock capture backend (`stop` ends frame delivery) and a trait-shaped PipeWire stub (default `pipewire` feature, no native library). `softwake-wake` scores configured phrases with a local text matcher ([ADR 0002](docs/ADR-0002-wake-engine-spike.md)). `softwake-soul` loads `soul.md` and `user.md`, checks them, and renders system instructions. `softwaked serve` speaks newline-delimited JSON on a Unix socket ([ADR 0003](docs/ADR-0003-ipc-transport.md)). `softwaked ctl` and the Tauri window `softwake-ui` are clients of that socket. Entering awake opens a text session with the rendered soul instructions. The only allowlisted tool is `echo` ([ADR 0004](docs/ADR-0004-first-safe-tool.md)); it runs only while awake. A model client and further tools are later work. A missing soul pack refuses awake; `reload-soul` re-reads the files and applies on the next awake.

## Build and test

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace --all-targets
```

The default `pipewire` feature compiles the capture stub. It does not require a system PipeWire library. `--no-default-features` on `softwake-audio` omits the stub.

## Demo

`softwaked` with no arguments prints the initial voice state and exits:

```bash
cargo run -p softwake-daemon
```

```text
softwaked state: sleep
```

The interactive demo is typed commands only. The microphone is not open, and PipeWire is not wired yet. It starts in sleep with mock capture running. `wake` and `sleep` submit the configured phrases to the text detector, then apply the voice-state machine, including the 800 ms phrase cooldown. `hibernate` stops capture. A voice command is rejected until `resume`, which returns to sleep and starts capture again. `wake` also requires a valid soul pack. Copy the repo templates into the config directory first (or pass `--soul-dir`):

```bash
mkdir -p ~/.config/softwake/soul
cp soul/soul.md soul/user.md ~/.config/softwake/soul/
cargo run -p softwake-daemon -- demo
```

```text
softwaked demo
typed commands only — mic / PipeWire not wired yet
state: sleep
capture: running
soul: ok
commands: wake, sleep, hibernate, resume, status, reload-soul, tool, quit
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
> tool echo
tool echo: pong
state: awake
capture: running
soul: ok
> sleep
heard: "softwake sleep" -> sleep
transition awake -> sleep (sleep phrase)
effect: release acting resources
state: sleep
capture: running
soul: ok
> hibernate
transition sleep -> hibernate (UI hibernate)
effect: stop capture
state: hibernate
capture: stopped
soul: ok
```

`sleep` in the 800 ms after `wake` stays awake, so a fast paste of `wake` then `sleep` will not leave awake. `hibernate` from awake does not use that cooldown. It closes the session and stops capture. `tool volume` is rejected. `tool` while asleep or hibernating is rejected.

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
```

`ctl` prints `state`, `capture`, `soul` (`ok` or `missing`), and `soul reload`, and exits non-zero when the daemon rejects the command or cannot be reached. `resume` is wake-from-hibernate and lands in sleep. `reload-soul` re-reads the soul pack from disk. The new text applies on the next awake session, not in the middle of one that is already awake. `ctl tool` runs one allowlisted tool. The daemon starts in sleep, so `ctl tool echo hello` is refused until the daemon is awake. The typed demo is the path that enters awake. A successful tool prints its result on the line after `soul reload` (`echo: hello`, or `pong` when `echo` has no arguments).

The socket path is the first match of `--socket PATH`, `SOFTWAKE_SOCKET`, `$XDG_RUNTIME_DIR/softwake/softwaked.sock`, and `/tmp/softwake-$UID/softwaked.sock` when `XDG_RUNTIME_DIR` is unset.

The soul directory is the first match of `--soul-dir PATH` (on `serve` and `demo`), `SOFTWAKE_SOUL_DIR`, `$XDG_CONFIG_HOME/softwake/soul`, and `~/.config/softwake/soul` when `XDG_CONFIG_HOME` is unset. See [docs/03-soul-pack.md](docs/03-soul-pack.md).

## Window

`softwake-ui` is a small Tauri window: the current state, whether the soul pack is `ok` or `missing`, and buttons for Hibernate, Wake (leave hibernate into sleep), Sleep, and Reload soul. It only talks to the socket. Start `softwaked serve` first.

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
| [docs/06-milestones.md](docs/06-milestones.md) | Phase 1 vertical slice |
| [docs/ADR-0001-name-and-scope.md](docs/ADR-0001-name-and-scope.md) | Name and phase-1 scope |
| [docs/ADR-0002-wake-engine-spike.md](docs/ADR-0002-wake-engine-spike.md) | Text phrase table for the wake spike |
| [docs/ADR-0003-ipc-transport.md](docs/ADR-0003-ipc-transport.md) | Unix socket and newline-delimited JSON |
| [docs/ADR-0004-first-safe-tool.md](docs/ADR-0004-first-safe-tool.md) | Why the first tool is `echo` |
