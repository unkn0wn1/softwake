# softwake-node (Remote Agent companion)

Always-on companion stub for Softwake ([ADR-0039](../../docs/ADR-0039-remote-agent.md)).

## Slice 1

- `GET /health` → `{"ok":true,"role":"companion","version":"..."}`
- `GET /v1/outbox` → `{"items":[]}` (empty; real outbox is slice 2+)

Listen address: env `SOFTWAKE_NODE_LISTEN` (default `127.0.0.1:8790`).

**Production:** bind the node's **Tailscale IP** (or MagicDNS-reachable interface) only. Do not expose on a public WAN IP. Pairing uses Tailscale SSH from the laptop Softwake Settings → Remote Agent page — see [remote-agent-pairing.md](../../docs/remote-agent-pairing.md).

```bash
SOFTWAKE_NODE_LISTEN=100.x.y.z:8790 softwake-node
```
