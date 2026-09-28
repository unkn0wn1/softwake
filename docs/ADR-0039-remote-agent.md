# ADR-0039 — Remote Agent (companion)

- **Status:** Accepted
- **Date:** 2026-09-28
- **Depends on:** [ADR-0024](ADR-0024-timers-cron.md), [ADR-0029](ADR-0029-messengers-telegram.md), [ADR-0038](ADR-0038-webhook-wake.md)

## Context

Operators want Softwake reachable while the laptop sleeps or is away, without turning Softwake into a remote-first server or thin client. The laptop remains the primary Softwake (softwaked + UI + KWS + keyring). An always-on **companion** on a Proxmox CT can hold timer leases / away results and later accept Tailnet webhook wake — but only over **Tailscale**.

## Decision

| Lock | Choice |
|---|---|
| Product name | **Remote Agent** = always-on **companion** (not remote core / thin client) |
| Network | **Tailscale only** — MagicDNS or `100.x`; no public IP, WAN SSH, or public ingress |
| Primary | Laptop softwaked + UI + KWS + keyring |
| Timers | Per-row `run_on`: `local` \| `companion` \| `auto` (prefer laptop if present). Default `local` |
| Outbox | Shared outbox on companion: fire leases + away results; laptop pulls into HUD on wake |
| Chat | **No** full dual chat sync; outbox → HUD seed only |
| Scope | **Per-profile** (`profile_id` on schedules/outbox/mirrored pack); OAuth stay laptop-local |
| Telegram | Sticky single owner — laptop if present, else companion when Tailscale laptop down (grace) |
| OAuth | Tokens stay laptop-local by default (mirror profiles/skills/timers/memory; not raw OAuth unless later explicit) |
| Webhook | May target companion on Tailnet later (bearer/HMAC); loopback-only bind on the node's Tailscale IP (see ADR-0038 note) |

### Slice matrix

| Area | Slice 1 (this ADR) | Slice 2+ |
|---|---|---|
| ADR + pairing docs | Live | — |
| `remote-agents.json` + Settings UI | Live | Installer / SSH install of softwake-node |
| Pairing secret in bag | Live (storage) | Used by installer / mutual auth |
| Test on Tailnet | Stub status string | `tailscale ping` / SSH BatchMode probe |
| Conflict policy | Persisted stub enum | Runtime resolver |
| `run_on` field + Timers UI | Live field; fire still **local** | **Slice 2 (ADR-0040):** honor run_on + leases |
| `softwake-node` health + empty outbox | Live stub binary | **Slice 2:** presence, leases, durable outbox, schedule mirror |
| Outbox → HUD | Docs only | **Slice 2:** pull → per-profile HUD while-away |
| Telegram sticky owner | Docs only | **Live (ADR-0042)** |
| OAuth mirror, SSH install | Docs only | Later |

### Config

- Non-secret: `$XDG_CONFIG_HOME/softwake/remote-agents.json`
- Secrets: `SecretBag.remote_agent_pairing_secrets` keyed by agent id
- Schedules: `ScheduleEntry.run_on` (serde default `local`)

### softwake-node

Companion binary exposes `GET /health` and `GET /v1/outbox` (empty in slice 1). Default listen `127.0.0.1:8790`; production binds the node's Tailscale IP via `SOFTWAKE_NODE_LISTEN`.

## Consequences

- Operators can save a companion pairing and set timer `run_on` preferences without waiting for presence/outbox.
- Slice 1 must not break local timers: companion dispatch is explicitly deferred.
- Pairing remains Tailscale-only by policy; Settings rejects URL-scheme hostnames.

## See also

- [remote-agent-pairing.md](remote-agent-pairing.md)

- [ADR-0042](ADR-0042-telegram-sticky-ownership.md) — Telegram sticky ownership
