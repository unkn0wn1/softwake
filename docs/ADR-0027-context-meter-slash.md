# ADR-0027 — Expanded HUD context meter and slash context commands

- **Status:** Accepted
- **Date:** 2026-09-27
- **Relates to:** [ADR 0021](ADR-0021-multi-turn-compact.md) (amends default compact threshold), [ADR 0026](ADR-0026-hud-chat-unlock.md) (HUD display history stays separate)

## Decision

1. **Expanded HUD meter.** While awake, the expanded HUD shows estimated model-context fullness: `used / limit (percent%)` plus the auto-compact threshold. Status already carried `context_used` / `context_limit` / `context_compacted`; this ADR adds additive `context_compact_at` and surfaces the same meter on the HUD (not only the Settings Status pane).

2. **Auto-compact default 80%.** Settings `compact_at_percent` default moves from 70 to **80** (`DEFAULT_COMPACT_AT_PERCENT`). Existing positive overrides in `providers.json` are unchanged. Auto-compact still runs **before** each ask when estimated usage meets the threshold ([ADR 0021](ADR-0021-multi-turn-compact.md)).

3. **Slash / clear-typed commands** while awake (typed HUD ask, `ctl ask` / `ctl chat`, demo `ask`/`chat`). They manage **model session** context only — not HUD display history (ADR 0026):

   | Input | Effect |
   |-------|--------|
   | `/clear`, `clear context` | Drop all session turns; keep session open + system pack |
   | `/halve`, `/reduce`, `halve context`, `reduce context` | Keep newest ~half by character mass |
   | `/compact`, `compact context` | Force Hermes-style compaction (`keep_recent_turns`); no-op when nothing to compact |

   Matching is case-insensitive on the trimmed line. Commands do not call the chat model (compact may use the existing compact completion / extractive fallback). No TTS for the command reply. PROTOCOL generation stays **1**.

4. **Seed-on-wake (amended 2026-09-27).** Waking opens a fresh model session from the soul pack, then replays a budgeted suffix of per-profile HUD history (see [ADR 0026](ADR-0026-hud-chat-unlock.md) amendment). Slash commands still affect the model session only.

## Context

Operators need to see how full the awake window is without opening Settings, and to reclaim budget mid-session without sleeping. ADR 0021 already compacted before ask and showed usage on Status; the expanded HUD and slash commands make that operable in the capsule.

## Alternatives

- New `ClientMessage` variants for clear/compact/halve. Rejected for this slice: intercepting `Ask` text keeps PROTOCOL gen 1 and works from HUD/`ctl` without new wire shapes.
- Seed model context from HUD history on wake. Accepted in the 2026-09-27 HUD polish amendment (budgeted; plaintext daemon load + UI `SeedChat` for encrypted vaults).
- Change only the HUD copy without a Status threshold field. Rejected: meter and Status should share one source (`context_compact_at`).

## Consequences

- Status may include `context_compact_at` while awake (omitted when asleep).
- GetStatus refreshes `context_used` from the open session so the HUD meter stays live between asks.
- Docs/UI defaults for compact-at show 80.
