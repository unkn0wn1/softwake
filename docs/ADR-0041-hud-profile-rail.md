# ADR-0041 — HUD left-rail profile switch (+ `/refresh`)

- **Status:** Accepted
- **Date:** 2026-09-28

## Decision

When the Softwake HUD capsule is **expanded**, a narrow **left rail** lists every profile (agent name / id). The **active** profile is highlighted. Clicking a profile:

1. Persists the current profile’s HUD chat (when the vault is unlocked).
2. Writes `softwake.json` `active_profile` via `set_active_profile`.
3. Asks the daemon for **`/refresh`** (same path as ADR-0031: reload/retarget soul from the new active pack, clear model session + HUD reseed when awake, rediscover MCP). Does **not** force-wake from sleep/hibernate.
4. Loads that profile’s `hud-chat.json` into the HUD log (vault-aware).

Collapsed bloom stays **120×120** with no rail chrome. Settings → Profiles set-active remains disk + `reload_soul` (next awake); the live switch surface is the HUD rail (and `/profile` + `/refresh`).

Tauri command: `hud_switch_profile`. PROTOCOL stays 1 (reuse Ask).

## Context

Operators run multiple profiles (e.g. Sally vs Softwake). Settings already edits packs; Remote Agent is per-profile. The HUD needed an obvious, one-click way to talk as another agent without opening Settings. `/refresh` already clears session + MCP; pairing set-active with that path avoids a second reload pipeline.

## Alternatives

- Settings-only switch. Rejected: HUD is the daily surface; rail makes the active agent obvious.
- `/profile` alone without `/refresh`. Rejected: skips MCP rediscover (ADR-0031).
- New IPC `SwitchProfile` wire command. Rejected for this slice: Ask + `/refresh` is enough.
- Show rail while collapsed. Rejected: would break the compact bloom.

## Consequences

- `/refresh` and `/profile` slash commands are allowed while sleep/hibernate so disk-first set-active + Ask(`/refresh`) can retarget without waking (bodies already returned an honest not-awake note).
- Encrypted vaults still gate history load after switch.
- Telegram sticky ownership, companion LLM `agent_task`, SSH installer, and OAuth mirror remain out of scope.
