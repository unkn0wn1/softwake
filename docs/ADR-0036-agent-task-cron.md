# ADR-0036 — Agent-task cron (Hermes-style prompt schedules)

- **Status:** Accepted
- **Date:** 2026-09-28

## Context

ADR-0024 ships Softwake-owned per-profile timers that fire a **fixed** notify /
TTS string (plus Telegram timer fan-out). Operators also want Hermes-style cron
rows that **run a prompt** (an agent turn with tools) and **deliver the result**
to messaging / HUD — without installing system crontab or pulling a Hermes
gateway into Softwake.

## Decision

1. **Action field** on each `ScheduleEntry`: `action` is `notify` (default) or
   `agent_task`. Missing JSON field deserializes as `notify` (no migration).
   Timing kinds stay `once` / `daily` / `cron`.
2. **Prompt storage:** reuse `message` as the agent prompt when
   `action == agent_task`; `title` remains the short label.
3. **Fire path:**
   - `notify` → existing `fire_schedule_reminder` (unchanged).
   - `agent_task` → bounded oneshot tool loop (same stack as messenger oneshot
     ask: soul + Tools appendix + advertise non-deny tools + `MAX_TOOL_ROUNDS`).
     Deliver via timer semantics: notify sink + `fanout_timer` (HUD + Telegram
     Default/Receive-all) + optional desktop TTS when timer voice is on.
4. **Safety:** Ask / Deny / Always-allow from Tools Settings still apply.
   Pending confirmations are **delivered as text**; cron never silently elevates
   to Always-allow. Do not raise `MAX_TOOL_ROUNDS` for cron.
5. **Soul / profile (v1):** the agent turn uses softwaked’s **current** applied
   soul and Tools settings (active profile). Delivery uses the schedule row’s
   `profile_id` channels. Prefer placing agent tasks on the active profile.
6. **Create paths:** `schedule create agent_task once|daily|cron <when> <prompt>`
   (tool + OpenAI `fire=agent_task`); Settings → Timers Action select; optional
   NL `agent task daily at HH:MM …`.
7. **Advance on error:** failed agent turns still advance / disable so a bad
   prompt cannot tight-loop every tick.
8. **Out of scope:** system crontab, memory write, webhook wake, multi-agent /
   Hermes gateway, PROTOCOL bump.

## Consequences

- Fixed-notify schedules keep working without file changes.
- Long agent tasks may hold the Runtime lock (same as Telegram inbound ask) and
  cause a skipped 15s tick via `try_lock`.
- Operators who Always-allow dangerous tools also allow those tools from
  scheduled fires — Tools Settings remain the control plane.
- Reinstall daemon + UI; Spencer restarts softwaked so the tick path picks up
  agent-task handling.


## Companion node

When `run_on` is `companion` or `auto` (laptop away), softwake-node runs the same family of bounded turn using mirrored soul / skills / tools ([ADR-0043](ADR-0043-companion-agent-task-llm.md)). Ask still never silently elevates; OAuth stays laptop-local.

## Amendment — OAuth mirror (ADR-0045)

OAuth stays laptop-local unless the operator enabled OAuth mirror
([ADR-0045](ADR-0045-oauth-mirror.md)).

