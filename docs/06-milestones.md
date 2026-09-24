# Milestones

## Phase 1 — Nail the ear (current)

**Goal:** Reliable sleep / awake / hibernate with soul pack and one safe tool.

1. Workspace + empty crates + CI skeleton
2. `softwake-state` with tested transitions
3. `softwake-audio` mock + PipeWire capture behind trait
4. `softwake-wake` spike: local wake + sleep phrases (engine choice documented in an ADR)
5. Daemon IPC + minimal Tauri UI: show state, hibernate, wake-from-hibernate, reload soul
6. `softwake-soul` loads `soul.md` + `user.md`
7. Awake session stub (even text-only first) + **one** safe tool
8. Manual gate checklist in [05-quality-gates.md](05-quality-gates.md)

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

- Meeting memory / qmd integration (separate product track; may feed Softwake later)
- Boring coding-agent harness (separate repo)
- Multi-conductor / named worker routing (agent-desk territory)
