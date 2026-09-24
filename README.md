# Softwake

Voice-first local conductor: asleep until hailed, awake with tools, hibernate when you want silence.

Rust end-to-end (daemon + Tauri UI). Phase 1 nails reliable **wake / sleep / hibernate** and a single safe tool loop. Personality and rules live in a **soul pack** (`soul.md`, `user.md`). Long-term memory (e.g. Honcho) is planned later, not required for phase 1.

> Name check (2026-09-24): `/www/softwake` was free; crates.io `softwake` unused. Prefer `softwake.app` / similar if `softwake.dev` is taken. Not a fork of `agent-desk` or `xai-voice` — those are design references only.

## Status

Docs-first scaffold. No runtime yet.

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

## Related (reference only)

- `/www/agent-desk` — earlier Python conductor ideas (PipeWire, wake/sleep, Voice 2.0)
- `/www/xai-voice` — abandoned Ionic/Capacitor scaffold (wrong audio path)
- `/www/architect-soul-example` — soul.md / AGENTS.md patterns
- `/www/honcho` — candidate long-term memory later
