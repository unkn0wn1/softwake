# ADR 0037 — Agent memory write tools

- **Status:** Accepted
- **Date:** 2026-09-28
- **Depends on:** [ADR 0009](ADR-0009-long-term-memory.md), [ADR 0018](ADR-0018-tools-settings-shell.md), [ADR 0025](ADR-0025-api-tool-calling.md)

## Decision

Softwake exposes two confirm-gated chat tools so the agent can persist and remove long-term memory facts from voice, Telegram, and HUD turns:

| Tool | Default permission | Effect |
|---|---|---|
| `remember` | **Always allow** | Store snippet text; create `memory.json` on first success |
| `forget` | **Ask** | Drop one id, or `forget all` (bulk wipe) |

Registry risk is Confirm for both (same floor as `skill_save` / `notify`). Always allow skips the HUD prompt; Ask stages one pending confirmation. Deny omits the tool from the OpenAI advertise list.

### Why these defaults

- Softwake list/read tools default to Always allow; destructive disk writes (`skill_save`, `schedule`, calendar write, `forget`) default to Ask.
- `remember` is additive operator data. Voice and Telegram turns should be able to store a fact without interrupting every call. Operators who want a prompt can set Ask in Settings → Tools.
- `forget` and especially `forget all` remove durable facts → Ask.

### Storage

Unchanged from ADR 0009: process-global `memory.json` under `$XDG_STATE_HOME/softwake` (or `~/.local/state/softwake`). **Not** per-profile. Softwake rejected a Honcho / graph / vector store for this path.

`FileMemory::open_enabled` still treats a missing file as an empty enabled store. The first successful `remember` creates the directory (`0700`) and file (`0600`). Ask/chat recall still opens the file **only when it already exists** (fail-open). After the first remember, the next turn’s budgeted recall can see the new fact.

`FileMemory::forget_all` clears every snippet in one atomic write and leaves `next_id` unchanged (empty list with non-zero `next_id` remains valid).

### Arguments

- `remember <text…>` — joined argv / JSON `{ "text": "…" }`. Empty or whitespace-only rejected.
- `forget <id>` — decimal id ≥ 1 / JSON `{ "id": "3" }`.
- `forget all` — JSON `{ "all": true }`. Ask-gated.

No `recall` tool in this ADR: budgeted substring recall on ask/chat already exists.

### Protocol

Protocol generation stays `1`. No new IPC status fields. Tools appear in Settings → Tools and in the live permissions appendix when registered.

## Consequences

- Agents can create and maintain `memory.json` without hand-editing.
- Always-allow remember can grow the file up to the 1 MiB / 8 KiB caps if the model spam-calls; operators can Deny or Ask.
- Forget-all after one Approve has no undo beyond backups.
- Profile switch does not isolate memory (global file by design).
