# Softwake

Voice-first local conductor: asleep until hailed, awake with tools, hibernate when you want silence.

Rust end-to-end (daemon + Tauri UI). Phase 1 nails reliable **wake / sleep / hibernate** and a single safe tool loop. Personality and rules live in a **soul pack** (`soul.md`, `user.md`). Long-term memory (e.g. Honcho as an optional later example) is planned later, not required for phase 1.

## Status

Cargo workspace on stable Rust (edition 2024). `softwake-state` implements sleep / awake / hibernate, including rejected transitions. The other crates are compile-clean boundaries: an audio capture trait (no PipeWire backend), wake, session, tools, soul paths, and IPC types. The Tauri app (`softwake-ui`) is not a workspace member yet.

## Build and test

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace --all-targets
```

`cargo run -p softwake-daemon` builds `softwaked`, prints the initial voice state (`sleep`), and exits.

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
