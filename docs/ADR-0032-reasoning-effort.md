# ADR-0032: Reasoning effort slash + Softwake ctl

## Status

Accepted

## Context

xAI Grok (and some OpenAI-compatible chat APIs) accept a top-level
`reasoning_effort` on Chat Completions (`low` / `medium` / `high` / `xhigh`).
Softwake had no operator knob for that. Awake slash already covers model, voice,
and profile; Softwake ctl tools mirror those for the agent (ADR-0031).

## Decision

1. **`providers.json` field** `reasoning_effort` (string, default empty). Empty
   means Softwake **omits** the field on the wire (provider default). Document
   version stays 1 (older files omit the field).
2. **Modes:** `low`, `medium`, `high`, `xhigh`. Slash/tool input `default` /
   `off` / `none` clears the override.
3. **Slash:** `/reasoning` and `/reasoning list` show current + modes;
   `/reasoning <mode>` sets (or clears). Same awake-session gate as `/model`.
4. **Tools:** `softwake_list_reasoning` (Always allow) and
   `softwake_set_reasoning` (Ask). Advertised via OpenAI tools when not Deny.
5. **Wire:** `prepare_chat` copies the setting onto `PreparedChat`;
   `complete_chat` / `complete_chat_turn` insert `reasoning_effort` when
   non-empty. Applies to every provider Softwake POSTs to; unsupported backends
   may ignore or error — operator chooses a mode that their model accepts.

## Consequences

- Next ask after set uses the new effort (disk read on prepare; no daemon
  restart).
- Non-reasoning models may reject the field; clear with `/reasoning default`.
- Settings UI picker is out of scope for this ADR (slash + tools suffice).
