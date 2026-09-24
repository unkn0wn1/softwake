# Milestones

## Phase 1 — Nail the ear

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

1. [ ] Realtime or higher-quality STT/TTS path
   - [x] On-device wake engine chosen ([ADR 0006](ADR-0006-on-device-wake.md))
   - [x] Capture + PCM → WakeDetector plumbing (mock default; native feature-gated; CI mic-free)
   - [ ] Streaming STT / TTS path (later)
2. [x] Confirmation UX for risky tools ([ADR 0005](ADR-0005-tool-confirmation.md))
3. [x] Richer tool registry + logging
4. [ ] Packaging polish

## Phase 3 — World connectors + memory

- Email / Drive / calendar via explicit connectors
- Long-term memory module (Honcho or smaller local store) — decision ADR
- Stricter policy engine

## Deferred ideas (do not pull into phase 1)

- Meeting memory / transcript integration (separate product track; may feed Softwake later)
- Boring coding-agent harness (separate repo)
- Multi-conductor / named worker routing (out of scope for Softwake)
