# ADR-0040 — Remote Agent presence, leases, and outbox (slice 2)

- **Status:** Accepted
- **Date:** 2026-09-28
- **Depends on:** [ADR-0039](ADR-0039-remote-agent.md), [ADR-0024](ADR-0024-timers-cron.md)

## Context

Slice 1 (#94) shipped pairing Settings, `run_on`, and a stub `softwake-node`. Operators need real **presence**, **dispatch without double-fire**, and **while-you-were-away** HUD pull while the laptop sleeps — still Tailscale-only, laptop-primary, and **per-profile**.

## Decision

### Per-profile lock

Schedules, outbox items, leases, and mirrored soul/skills/messengers on the companion are keyed by **`profile_id`**. There is no global remote chat. OAuth tokens stay laptop-local (no mirror).

### Presence

| Item | Choice |
|---|---|
| Transport | `POST /v1/presence` on softwake-node (Bearer pairing secret) |
| States | `present` / `sleeping` / `hibernated` / `offline` |
| Grace | **`PRESENCE_GRACE_MS = 90_000`** — no heartbeat within 90s → `offline` |
| Laptop mapping | Awake→present, Sleep→sleeping, Hibernate→hibernated |
| Hard OS suspend | No heartbeat → grace → offline |

### Fire leases

| Item | Choice |
|---|---|
| Key | `(profile_id, schedule_id, fire_ms)` |
| Endpoint | `POST /v1/leases` — 200 claimed / 409 held |
| Default TTL | 120s |
| `run_on=local` | Laptop only |
| `run_on=companion` | Node only (laptop mirrors schedules; does not local-fire) |
| `run_on=auto` | Laptop if presence `present` (claims lease); else companion tick |

### Schedule mirror + node tick

Laptop periodically `PUT /v1/schedules/{profile_id}` with enabled `companion`/`auto` rows. softwake-node ticks those rows every ~15s so fires continue while the laptop is asleep. `agent_task` on the node records an outbox **summary stub** in slice 2 (full companion LLM is slice 3).

### Outbox → HUD

Node persists fire acks / agent-task summaries under `$XDG_DATA_HOME/softwake-node/` (override `SOFTWAKE_NODE_DATA`). Laptop pulls on wake + ~60s periodic sync and appends `While you were away: …` into that **profile’s** HUD via `hud_chat_write`.

### Auth

Mutating / outbox / presence / lease / schedule endpoints require `Authorization: Bearer <pairing_secret>` (or `X-Softwake-Remote-Token`). `GET /health` stays open. Secret from `SOFTWAKE_NODE_PAIRING_SECRET` on the node; laptop uses `SecretBag.remote_agent_pairing_secrets[agent_id]`.

## Out of scope (slice 3+)

- Full Telegram sticky ownership takeover  
- SSH installer of softwake-node  
- OAuth token mirror / vault bidirectional sync  
- HUD left-rail profile list (click → switch active profile + `/refresh`) — UI slice later  
- Full companion LLM for `agent_task`

## Consequences

- Timers with `run_on=companion|auto` finally dispatch for real.  
- Operators must set the same pairing secret on laptop Settings and `SOFTWAKE_NODE_PAIRING_SECRET`.  
- Companion `agent_task` summaries are honest stubs until slice 3.
