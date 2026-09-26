# ADR-0025 — Chat API tool-calling

- **Status:** Accepted
- **Date:** 2026-09-27

## Context

Ask heuristics (`run` / `shell` / `ssh`) stage shell without a model round-trip
([ADR-0018](ADR-0018-tools-settings-shell.md)). Natural asks such as “please
check swapspace on aau” still went through chat with **no** `tools` field, so
the model could only propose `run …` in prose ([live Tools permissions
appendix](ADR-0018-tools-settings-shell.md)). Operators want the model to call
enabled tools when it needs real results.

## Decision

1. Softwake stays on legacy `/chat/completions` (not Responses API this slice).
2. Each ask advertises OpenAI-nested function tools for every registered tool
   whose Tools Settings permission is not **Deny**.
3. `complete_chat_turn` may return assistant text or `tool_calls`. The daemon
   runs a multi-turn loop: tool_calls → `Hands::request` → tool role results →
   continue, capped at six rounds.
4. **Always allow** runs immediately (glossary expand still on shell spawn).
   **Ask** stages HUD confirm and stops the loop with a pending sentence.
   **Deny** is omitted from `tools`.
5. Ask heuristics remain a fast path before the model. Never invent command
   output — only report Hands detail / stdout.
6. Session transcript stores final assistant text (or the pending sentence),
   not ephemeral tool wire messages.

## Consequences

- Natural language can trigger shell under Always allow without saying `run`.
- Ask-mode API calls share the existing pending-tool HUD path.
- Providers that ignore `tools` still work when the list is empty (all deny).
- Migrating to the Responses API is a later ADR if Softwake needs it.
