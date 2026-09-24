# Milestones

## Phase 1 — Nail the ear (current)

**Goal:** Reliable sleep / awake / hibernate with soul pack and one safe tool.

1. [x] Workspace + empty crates + CI skeleton
2. [x] `softwake-state` with tested transitions
3. [x] `softwake-audio` mock + PipeWire capture behind the trait (native I/O is still a stub)
4. [x] `softwake-wake` spike: local wake + sleep phrases ([ADR 0002](ADR-0002-wake-engine-spike.md))
5. [x] Daemon IPC + minimal Tauri UI: show state, hibernate, wake-from-hibernate, sleep, reload soul
6. [x] `softwake-soul` loads `soul.md` + `user.md` (missing or invalid pack refuses awake; `reload_soul` re-reads disk and applies on the next awake)
7. [x] Awake session stub (even text-only first) + **one** safe tool (`echo`, [ADR 0004](ADR-0004-first-safe-tool.md))
8. [x] Manual gate checklist in [05-quality-gates.md](05-quality-gates.md)

**Exit:** Someone else can clone, configure phrases, and demo wake → tool → sleep → hibernate without babysitting.

## Phase 2 — Better voice and safer hands

- Realtime or higher-quality STT/TTS path
- Confirmation UX for risky tools
- Richer tool registry + logging
- Packaging polish

## Phase 3 — World connectors + memory

- Email / Drive / calendar via explicit connectors
- Long-term memory module (Honcho or smaller local store) — decision ADR
- Stricter policy engine

## Deferred ideas (do not pull into phase 1)

- Meeting memory / transcript integration (separate product track; may feed Softwake later)
- Boring coding-agent harness (separate repo)
- Multi-conductor / named worker routing (out of scope for Softwake)
