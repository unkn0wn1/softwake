# ADR-0024 — Timers / cron (per-profile schedules)

- **Status:** Accepted
- **Date:** 2026-09-26

## Context

Operators want Softwake-owned reminders and recurring tasks the agent can
propose, without installing system crontab or systemd timers. Schedules should
follow Softwake profiles (work vs personal) and reuse the existing Tools confirm
gate and announce/notify paths.

## Decision

1. **Store** each profile’s schedules in
   `$XDG_CONFIG_HOME/softwake/profiles/<id>/schedules.json`
   (`version`, `timezone: "local"`, `entries`). Softwake never writes the host
   crontab.
2. **Kinds:** `once` (local `YYYY-MM-DDTHH:MM`), `daily` (`HH:MM`), and a small
   5-field `cron` subset (`min hour dom mon dow`, dow 0–6 Sun–Sat). Wall clock is
   host local (`chrono::Local`). Asia/Bangkok operators get Bangkok times when
   the host TZ is `Asia/Bangkok`.
3. **Tool:** confirm-gated `schedule` (create / edit / delete / list). Tools
   Settings default is **Ask**, same floor as `notify`.
4. **NL propose:** clear lines such as `remind me daily at 07:30 …` become a
   pending `schedule` tool when permission is not Deny.
5. **Scheduler:** daemon thread ticks every 15s, scans **all** profiles, fires
   while softwaked is running in **any** voice state (awake / sleep / hibernate).
   Fire path: notify sink line + `spawn_fixed_line` TTS + HUD status text.
   Catch-up grace: 15 minutes; older misses are skipped and advanced.
6. **OS suspend / power-off:** out of Softwake’s control; timers miss until
   softwaked runs again (then catch-up applies).
7. **Settings → Timers:** list / create / edit / delete for the active profile
   (optional “Show all profiles”). PROTOCOL stays 1; UI reads/writes the same
   JSON files the daemon uses (mtime reload on tick).

## Consequences

- Operators enable or Always-allow `schedule` on Tools if they want silent
  creates; Ask remains the default.
- Reinstall daemon + UI; Spencer restarts softwaked himself so the tick thread
  starts.

## Amendment (2026-09-28)

Schedules may set `action: agent_task` so a fire runs a bounded agent turn and
delivers the result (not only a fixed notify string). See
[ADR-0036](ADR-0036-agent-task-cron.md). Default `action` remains `notify`.

## Amendment (2026-09-28) — run_on

Schedules may set `run_on: local | companion | auto` (default `local`) so a row can prefer the laptop or a Remote Agent companion. See [ADR-0039](ADR-0039-remote-agent.md). Slice 1 persists the field and exposes Timers UI; companion dispatch is slice 2 (fires remain local).

