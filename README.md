# Softwake

Voice-first local conductor: asleep until hailed, awake with tools, hibernate when you want silence.

Rust end-to-end (daemon + Tauri UI). Phase 1 nails reliable **wake / sleep / hibernate** and a single safe tool loop. Personality and rules live in a **soul pack** (`soul.md`, `user.md`). Long-term memory (e.g. Honcho as an optional later example) is planned later, not required for phase 1.

## Status

Cargo workspace on stable Rust (edition 2024). `softwake-state` implements sleep / awake / hibernate, including rejected transitions and phrase cooldowns. `softwake-audio` has a mock capture backend (`stop` ends frame delivery) and a trait-shaped PipeWire stub (default `pipewire` feature, no native library). `softwake-wake` scores configured phrases with a local text matcher ([ADR 0002](docs/ADR-0002-wake-engine-spike.md)). Session, tools, soul paths, and IPC types are compile-clean boundaries. The Tauri app (`softwake-ui`) is not a workspace member yet.

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

The interactive demo is typed commands only. The microphone is not open, and PipeWire is not wired yet. It starts in sleep with mock capture running. `wake` and `sleep` submit the configured phrases to the text detector, then apply the voice-state machine, including the 800 ms phrase cooldown. `hibernate` stops capture. A voice command is rejected until `resume`, which returns to sleep and starts capture again.

```bash
cargo run -p softwake-daemon -- demo
```

```text
softwaked demo
typed commands only — mic / PipeWire not wired yet
state: sleep
capture: running
commands: wake, sleep, hibernate, resume, status, quit
>
```

A `> ` prompt is printed before each line is read. The same path accepts a pipe (`printf 'wake\nstatus\nquit\n' | cargo run -p softwake-daemon -- demo`). `softwaked --demo` is the same mode.

`softwaked demo --verbose` and `softwaked demo -v` (also `--demo -v`) print extra `verbose:` lines for each command: the raw input, the parsed command, and for `wake` / `sleep` the phrase, the detector hit, and whether the transition succeeded or why it was rejected. `SOFTWAKE_LOG=debug` enables that same detail. `softwaked --help` prints usage.

Type one command per line. `sleep` in the 800 ms after `wake` stays awake. `wake` in the 800 ms after `sleep` or `resume` stays asleep. `hibernate` is a UI command and applies on the next line.

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
