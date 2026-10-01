# ADR-0043 — Companion `agent_task` LLM (Remote Agent slice 3)

- **Status:** Accepted
- **Date:** 2026-09-28
- **Depends on:** [ADR-0039](ADR-0039-remote-agent.md), [ADR-0040](ADR-0040-remote-agent-presence-outbox.md), [ADR-0036](ADR-0036-agent-task-cron.md), [ADR-0042](ADR-0042-telegram-sticky-ownership.md)

## Context

Slice 2 (#95) fired `run_on=companion|auto` schedules on softwake-node but recorded an **honest summary stub** for `action: agent_task`. Operators need a real bounded agent turn on the companion while the laptop is away — using that profile’s mirrored soul, skills, and tool permissions — without mirroring OAuth or changing Telegram sticky ownership.

## Decision

| Lock | Choice |
|---|---|
| Trigger | Node tick of mirrored `companion`/`auto` rows with `action: agent_task` after fire lease claim |
| Soul | Per-profile four-file pack mirrored via `PUT /v1/profiles/{id}/soul` |
| Tools | Global laptop `tools.json` mirrored via `PUT /v1/tools` |
| Skills | Authored skills mirrored via `PUT /v1/skills` (full replace, max 32) |
| API key | `SOFTWAKE_NODE_XAI_API_KEY` **or** mirrored vault `xai_api_key` (`PUT /v1/vault/llm`) |
| Model | `SOFTWAKE_NODE_MODEL` (default `grok-4-fast-non-reasoning`) |
| Bounds | `MAX_TOOL_ROUNDS = 6` (final round omits tools + soft-finalize, [ADR-0047](ADR-0047-tool-loop-finalize.md)); HTTP read 120s / connect 15s |
| Ask | Pending text into outbox delivery — **never** silent Always-allow |
| OAuth tools | Refuse with “OAuth stays laptop-local”; continue or stop with clear text |
| shell AlwaysAllow | Honor with softwake-tools timeout/output cap; Ask → pending |
| Delivery | Outbox `agent_task_result` + Telegram when `wants_timer_push` + bound chat_id + bot token; **no TTS** |
| OAuth mirror | **Out** (opt-in later) |

### Invoke policy on companion

| Permission / tool | Behavior |
|---|---|
| Deny | Not advertised |
| Ask | Do not run; return pending confirmation text |
| AlwaysAllow echo / notify / skill_list / skill_get / schedule list / softwake_status | Execute |
| AlwaysAllow shell | Execute with existing caps |
| email_* / calendar_* / drive_* | Refuse (OAuth laptop-local) |
| remember/forget, softwake set/sleep/…, skill_save, mcp_* | Refuse (not available on companion) |

### Env vars (node)

| Var | Role |
|---|---|
| `SOFTWAKE_NODE_LISTEN` | Bind address (Tailscale IP in prod) |
| `SOFTWAKE_NODE_PAIRING_SECRET` | Bearer auth |
| `SOFTWAKE_NODE_DATA` | Data dir override |
| `SOFTWAKE_NODE_XAI_API_KEY` | Preferred LLM key (else mirrored vault) |
| `SOFTWAKE_NODE_MODEL` | Chat model id |

## Consequences

- Companion `agent_task` finally runs a real LLM turn while the laptop sleeps.
- Operators must keep soul/tools/skills mirroring healthy (same ~45s cadence as schedules).
- Without an API key, the node still advances the schedule and writes an honest stub summary.
- OAuth-backed workflows stay laptop-only until a future opt-in mirror.

## See also

- [remote-agent-pairing.md](remote-agent-pairing.md)

## Amendment — OAuth mirror (ADR-0045)

OAuth-backed tools run on the companion when the opt-in vault has a usable
connection and the tool is AlwaysAllow. Empty vault returns an enable hint.
Ask permissions are unchanged. Mirror is default off
([ADR-0045](ADR-0045-oauth-mirror.md)).

