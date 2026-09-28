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
| Companion `agent_task` full LLM | **Stub summary** (slice 3) |
| Test on Tailnet button / SSH installer | **Stub** |
| Telegram sticky owner / OAuth mirror / HUD left-rail profiles | **Slice 3+** |

## Pairing flow

1. Bring a Proxmox CT onto your Tailnet.
2. Softwake Settings → **Remote Agent** → fill MagicDNS/`100.x`, SSH user, **pairing secret** → Save → Enabled.
3. On the CT:

```bash
cargo install --path crates/softwake-node --locked --force
export SOFTWAKE_NODE_PAIRING_SECRET='same-as-settings'
SOFTWAKE_NODE_LISTEN=100.x.y.z:8790 softwake-node
```

4. Laptop softwaked heartbeats `/v1/presence` about every 30s.
5. Create a timer with **Run on** `companion` or `auto`. Put the laptop to Sleep (or stop softwaked): companion should fire and write the outbox. Wake Softwake: HUD gains “While you were away: …”.

### Presence probe

```bash
curl -s -H "Authorization: Bearer $SOFTWAKE_NODE_PAIRING_SECRET" \
  http://100.x.y.z:8790/v1/presence
```

## Security notes

- Do not bind softwake-node on a public WAN interface.
- Never log the pairing secret.
- OAuth remains laptop-local by default.
