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
   - [x] Productized durable store ([ADR 0009](ADR-0009-long-term-memory.md)): `FileMemory` writes `memory.json` under `$XDG_STATE_HOME/softwake` when that variable is set and non-blank, otherwise under `~/.local/state/softwake`. The handle stays off until `open_enabled`. `MockMemory` stays the default. Awake ask/chat open it only when `memory.json` exists (budgeted recall).
3. [x] Stricter policy engine
   - [x] Connector actions are confirm or deny; unknown pairs fail closed ([ADR 0008](ADR-0008-connector-boundary.md)).
   - [x] Policy beyond the connector registry ([ADR 0010](ADR-0010-policy-engine.md)): `softwake-policy` evaluates tool names and connector pairs. Unknown subjects are denied. Overrides may only tighten. The daemon classifies through that engine. No live cloud client.

## Phase 4 — Context pack, then provider access

**Goal:** The acting session loads a four-file context pack. Provider sign-in fills a model picker after Test. Live connectors stay later slices.

1. [x] Context pack and confirm-echo foundation ([ADR 0011](ADR-0011-context-pack.md))
   - [x] `softwake-soul` loads `rules.md` and `glossary.md` with `soul.md` and `user.md`. All four are required. A missing or invalid pack, including an unparseable glossary, refuses awake. `reload_soul` re-reads the pack and applies on the next awake.
   - [x] Render order: Identity, User profile, Rules, Glossary, runtime policy stub. The rules section states that rules override soul. A glossary alias does not change tool risk.
   - [x] Repo templates `soul/rules.md` and `soul/glossary.md` use placeholder paths only.
   - [x] Alias expand and confirm-echo readback are library functions with unit tests. No shell. Protocol generation stays 1.
2. [x] Provider settings foundation ([ADR 0012](ADR-0012-model-providers.md))
   - [x] `softwake-providers`: xAI device-code OAuth, xAI API key, OpenAI API key, OpenRouter API key, OpenAI-compatible API key + base URL. Secret bag under XDG state (plaintext v1 warning). Settings JSON under XDG config (includes compatible base URL). Model picker empty until Test.
   - [x] Mock `Transport` for CI. `live-http` (ureq) is opt-in on the crate; `softwake-ui` enables it by default for Settings Test and sign-in.
   - [x] Thin Settings panel in `softwake-ui`. Protocol generation stays 1. That slice stopped at the `ProviderHandle` stub.
   - [x] OpenRouter and OpenAI-compatible base URL
   - [x] Wire awake session chat to the selected provider ([ADR 0013](ADR-0013-session-provider.md)). While awake, typed `ask` and `chat` send the rendered context pack and the user line to the Settings provider. Tests use `MockTransport`. The daemon `live-http` feature performs the real call. A missing Settings file, a Test that has not succeeded, a missing model, or a missing bearer returns a clear error. Protocol generation stays 1.
   - [x] Budgeted memory snippets after the rendered pack ([ADR 0009](ADR-0009-long-term-memory.md), [ADR 0013](ADR-0013-session-provider.md)). At most 4 snippets / 2048 UTF-8 bytes. Query is the user line. Opens `FileMemory` only when `memory.json` exists. Fail-open. Session takes an appendix string; daemon owns the memory crate. CI key-free / no Redis.
3. [ ] Live email, Drive, and calendar connectors (still open from phase 3)
4. [ ] In-window editors for `soul.md`, `user.md`, `rules.md`, and `glossary.md` (reload stays how a pack is applied)

## Deferred ideas (do not pull into phase 1)

- Meeting memory / transcript integration (separate product track; may feed Softwake later)
- Boring coding-agent harness (separate repo)
- Multi-conductor / named worker routing (out of scope for Softwake)
