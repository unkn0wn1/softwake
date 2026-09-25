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

1. [x] Realtime or higher-quality STT/TTS path
   - [x] On-device wake engine chosen ([ADR 0006](ADR-0006-on-device-wake.md))
   - [x] Capture + PCM → WakeDetector plumbing (mock default; native feature-gated; CI mic-free)
   - [x] Streaming STT / TTS path ([ADR 0007](ADR-0007-awake-stt-tts.md); mock default; sherpa stubs feature-gated)
2. [x] Confirmation UX for risky tools ([ADR 0005](ADR-0005-tool-confirmation.md))
3. [x] Richer tool registry + logging
4. [x] Packaging polish

## Phase 3 — World connectors + memory

1. [ ] Email / Drive / calendar via explicit connectors
   - [x] Connector boundary ([ADR 0008](ADR-0008-connector-boundary.md)): `softwake-connectors`, `EmailConnector`, `MockEmail`, confirm/deny registry, no live cloud client in the default build
   - [x] Confirm-gated tool `email_send` ([ADR 0008](ADR-0008-connector-boundary.md)): daemon `Hands` holds `MockEmail`; confirm calls `authorize_confirmed` then `EmailConnector::send`. No live client.
   - [ ] Live email backend (opt-in, not in CI)
   - [x] Drive backend stub ([ADR 0008](ADR-0008-connector-boundary.md)): `DriveConnector`, `MockDrive`, `drive` / `list` confirm. No live client. The daemon does not call it.
   - [ ] Live Drive backend (opt-in, not in CI)
   - [x] Calendar backend stub ([ADR 0008](ADR-0008-connector-boundary.md)): `CalendarConnector`, `MockCalendar`, `calendar` / `list` confirm. No live client. The daemon does not call it.
   - [ ] Live Calendar backend (opt-in, not in CI)
2. [ ] Long-term memory
   - [x] Decision ([ADR 0009](ADR-0009-long-term-memory.md)): thin local store behind a `Memory` trait in `softwake-memory`. `MockMemory` is in-process and off until enabled. Honcho is not the default and is not a dependency.
   - [x] Productized durable store ([ADR 0009](ADR-0009-long-term-memory.md)): `FileMemory` writes `memory.json` under `$XDG_STATE_HOME/softwake` when that variable is set and non-blank, otherwise under `~/.local/state/softwake`. The handle stays off until `open_enabled`. `MockMemory` stays the default. The daemon does not call it.
3. [x] Stricter policy engine
   - [x] Connector actions are confirm or deny; unknown pairs fail closed ([ADR 0008](ADR-0008-connector-boundary.md)).
   - [x] Policy beyond the connector registry ([ADR 0010](ADR-0010-policy-engine.md)): `softwake-policy` evaluates tool names and connector pairs. Unknown subjects are denied. Overrides may only tighten. The daemon classifies through that engine. No live cloud client.

## Deferred ideas (do not pull into phase 1)

- Meeting memory / transcript integration (separate product track; may feed Softwake later)
- Boring coding-agent harness (separate repo)
- Multi-conductor / named worker routing (out of scope for Softwake)
