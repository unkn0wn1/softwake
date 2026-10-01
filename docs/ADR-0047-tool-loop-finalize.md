# ADR-0047 — Tool-loop last-round finalize

- **Status:** Accepted
- **Date:** 2026-10-01
- **Depends on:** [ADR-0025](ADR-0025-api-tool-calling.md), [ADR-0043](ADR-0043-companion-agent-task-llm.md)

## Context

Operators talking to tool-light souls (e.g. HUD profile **Soulwright**, a literary
analyst that should rarely call tools) still hit Softwake’s global tool advertise
and permissions appendix. The model can emit `tool_calls` for every round of the
multi-turn loop. When that happened for all six rounds, the daemon and companion
returned a hard error:

`tool loop reached the 6-round cap without a final reply`

Raising `MAX_TOOL_ROUNDS` alone does not fix “never emits final text.” Soulwright’s
soul pack does not force tools; the runtime loop lacked a forced text turn.

## Decision

1. Keep **`MAX_TOOL_ROUNDS = 6`**. Semantics: up to **five** tool-bearing rounds,
   then a **sixth** text-only finalize round.
2. On the final allowed HTTP POST, pass an **empty** `tools` list (omit the field)
   so the model cannot request more `tool_calls`.
3. If the last round still yields empty content or unexpected `tool_calls`,
   **soft-finalize**: return operator-facing `Message` text from (a) last assistant
   prose, else (b) recent wire tool result snippets Softwake already pushed,
   else (c) a short “Tool budget reached…” sentence. Never invent command stdout.
4. Mirror the same policy on softwake-node companion `agent_task` (ADR-0043).
5. Optional appendix reminder: pure drafting / analysis may answer in text without
   tool thrash. Do not rewrite operator soul packs in this ADR.

## Consequences

- Soulwright and similar profiles get a speakable reply instead of the cap error
  when the model thrashing tools.
- Tool-heavy asks still get five tool hops before a forced reply.
- HUD / TTS see `ToolLoopOk::Message` more often; the old cap string is no longer
  the normal exhaustion path.
- Providers that ignore empty tools still receive a normal completion request.

## Non-goals

- Blindly raising the round cap.
- Per-profile tool advertise (may come later).
- OpenAI `tool_choice: "none"` (empty `tools` is enough this slice).
