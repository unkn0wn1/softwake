# Remote Agent pairing (Tailscale-only)

Softwake's **Remote Agent** is an always-on **companion** on a Proxmox CT. The laptop Softwake stays primary. Pairing and traffic use **Tailscale only** (MagicDNS or `100.x`). No public IP, WAN SSH, or public ingress.

Remote work is **per-profile**: schedules, outbox items, and mirrored pack files are keyed by `profile_id`. OAuth tokens stay laptop-local.

See [ADR-0039](ADR-0039-remote-agent.md) and [ADR-0040](ADR-0040-remote-agent-presence-outbox.md).

## Live vs stub

| Area | Status |
|---|---|
| Settings pairing + `remote-agents.json` + pairing secret | **Live** |
| Presence heartbeats (`present`/`sleeping`/`hibernated`/`offline`, 90s grace) | **Live** |
| `run_on` local / companion / auto + fire leases | **Live** |
| Schedule mirror + node tick (fires while laptop asleep) | **Live** |
| Outbox → per-profile HUD “While you were away” | **Live** |
| Companion `agent_task` full LLM (soul/skills/tools mirror) | **Live** (ADR-0043) |
| Telegram bot token + messengers mirror to node vault | **Live** (required for companion Telegram) |
| Node LLM key (`SOFTWAKE_NODE_XAI_API_KEY` or mirrored xAI key) | **Required for real agent_task / Telegram LLM**; honest stub without it |
| Test on Tailnet button / SSH installer | **Live** (ADR-0044) |
| Telegram sticky ownership (laptop present / companion away) | **Live** (ADR-0042) |
| OAuth mirror | **Out** (opt-in later) |
| HUD left-rail profiles | **Live** (ADR-0041) |

## Pairing flow

Mental model: laptop Softwake stays primary; an always-on **Proxmox CT** (often on a VPS/OVH-style host) joins the same **Tailnet** and runs `softwake-node`. No public WAN SSH or ingress.

1. Bring a Proxmox CT onto your Tailnet. Authorize the laptop SSH key for the configured user (BatchMode).
2. Softwake Settings → **Remote Agent** → fill MagicDNS/`100.x`, SSH user, **pairing secret** → Save → Enabled.
3. Click **Test on Tailnet** (ping / SSH / `GET /health`). Pre-install, health may fail — SSH should pass.
4. Click **Install companion** (or `softwaked ctl remote-agent install [id]`). Softwake ships a laptop-built `softwake-node` via scp, creates user `softwake`, writes `/etc/softwake-node.env` (`LISTEN=<ts-ip>:8790` + pairing secret) and a systemd unit, then `enable --now` and re-checks `/health`.
5. Optional on the CT env: `SOFTWAKE_NODE_XAI_API_KEY` (or rely on mirrored vault) for real `agent_task` / Telegram LLM.
6. Laptop softwaked heartbeats `/v1/presence` about every 30s.
7. Create a timer with **Run on** `companion` or `auto`. Put the laptop to Sleep (or stop softwaked): companion should fire and write the outbox. Wake Softwake: HUD gains “While you were away: …”.

### Reinstall

Same **Install companion** button or `softwaked ctl remote-agent install [id]` — overwrites binary + env and restarts the unit.

### Manual fallback (CT)

```bash
# Prefer Install companion; manual only if needed:
cargo install --path crates/softwake-node --locked --force   # on laptop, then scp
# on CT:
export SOFTWAKE_NODE_PAIRING_SECRET='same-as-settings'
# Optional: SOFTWAKE_NODE_XAI_API_KEY=…
SOFTWAKE_NODE_LISTEN=100.x.y.z:8790 softwake-node
```

### ctl

```bash
softwaked ctl remote-agent test [id]
softwaked ctl remote-agent install [id]
```

### Presence probe

```bash
curl -s -H "Authorization: Bearer $SOFTWAKE_NODE_PAIRING_SECRET" \
  http://100.x.y.z:8790/v1/presence
```

## Security notes

- Do not bind softwake-node on a public WAN interface.
- Never log the pairing secret.
- OAuth remains laptop-local by default.


## What gets mirrored (laptop → node)

| Item | Endpoint | Notes |
|---|---|---|
| Schedules (`companion`/`auto`) | `PUT /v1/schedules/{profile_id}` | Live since #95 |
| Messengers | `PUT /v1/profiles/{id}/messengers` | Live since #97 |
| Telegram bot token | `PUT /v1/vault/telegram` | Live since #97 |
| xAI API key | `PUT /v1/vault/llm` | Optional; prefer `SOFTWAKE_NODE_XAI_API_KEY` |
| Soul pack (4 md) | `PUT /v1/profiles/{id}/soul` | ADR-0043 |
| Tools permissions | `PUT /v1/tools` | ADR-0043 |
| Skills catalog | `PUT /v1/skills` | ADR-0043 |
| OAuth tokens | — | **Not mirrored** |

