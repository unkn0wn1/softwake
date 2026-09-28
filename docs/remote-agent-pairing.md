# Remote Agent pairing (Tailscale-only)

Softwake's **Remote Agent** is an always-on **companion** on a Proxmox CT. The laptop Softwake stays primary. Pairing and traffic use **Tailscale only** (MagicDNS or `100.x`). No public IP, WAN SSH, or public ingress.

See [ADR-0039](ADR-0039-remote-agent.md) for product locks and the slice matrix.

## What slice 1 ships

**Live**

- Settings → **Remote Agent**: Name, Tailscale hostname, SSH user, roles checkboxes, conflict policy stub, pairing secret, enabled
- On-disk `remote-agents.json` + pairing secret in the secret bag
- Timers **Run on** (`local` / `companion` / `auto`) — companion/auto options unlock when an agent is enabled; **fires still run locally**
- `softwake-node` stub: `GET /health`, `GET /v1/outbox` → `{"items":[]}`

**Stub / next slice**

- Test on Tailnet button (honest “not implemented” status)
- SSH install of softwake-node onto the CT
- Presence / laptop-alive, companion timer dispatch, outbox → HUD seed, Telegram sticky owner, OAuth mirror

## Intended pairing flow (design-accurate)

1. Bring a Proxmox CT onto your Tailnet (MagicDNS name e.g. `softwake-ct`, or remember its `100.x` address).
2. From the laptop (also on Tailnet), open Softwake Settings → **Remote Agent** → **+ New**.
3. Fill **Name**, **Tailscale hostname** (`softwake-ct` or `100.x.y.z`), **SSH user**, roles, optional pairing secret → **Save** → **Enabled**.
4. On the CT (over Tailscale SSH, never public WAN):

   ```bash
   # on the companion CT (Tailscale SSH from the laptop)
   cargo install --path crates/softwake-node --locked --force
   SOFTWAKE_NODE_LISTEN=100.x.y.z:8790 softwake-node
   ```

5. Slice 2 will use the pairing secret + SSH to automate install/health checks. Slice 1's **Test on Tailnet** only validates that hostname/user look set.

## Security notes

- Do not bind softwake-node (or webhook) on a public WAN interface.
- OAuth tokens remain laptop-local by default.
- Telegram ownership handoff is sticky and deferred to slice 2.
