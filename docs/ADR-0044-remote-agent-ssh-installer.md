# ADR-0044 — Remote Agent Tailscale-only SSH installer

- **Status:** Accepted
- **Date:** 2026-09-28
- **Depends on:** [ADR-0039](ADR-0039-remote-agent.md), [ADR-0040](ADR-0040-remote-agent-presence-outbox.md), [ADR-0043](ADR-0043-companion-agent-task-llm.md)

## Context

Operators (e.g. a laptop Softwake plus an always-on Proxmox CT on a VPS/OVH-style host joined to Tailnet) need a first-class way to **probe** and **install** `softwake-node` without manual cargo/env/systemd copy-paste. Slice 1 left “Test on Tailnet” and SSH install as stubs.

## Decision

| Lock | Choice |
|---|---|
| Network | **Tailscale only** — MagicDNS or `100.64.0.0/10`. Refuse public/WAN IPv4 and URL schemes on Save / Test / Install |
| Entry points | Settings → **Test on Tailnet** + **Install companion**; `softwaked ctl remote-agent test\|install [id]` (local-disk, no IPC) |
| SSH | Configured `ssh_user` (BatchMode keys); prefer create system user **`softwake`**, env `0640 root:softwake`, unit runs as `softwake` |
| Binary | Prefer ship **laptop-built** `softwake-node` (`SOFTWAKE_NODE_BIN` → `PATH` → `cargo build --release -p softwake-node` in workspace). Do not prefer cargo-on-CT |
| Listen | `SOFTWAKE_NODE_LISTEN=<ts-ip>:8790` — never generate `0.0.0.0` |
| Secrets | Pairing secret from bag only; env file on CT; never plaintext in `remote-agents.json` / softwake.json beyond existing bag patterns |
| Post-install | `systemctl enable --now softwake-node` + `GET /health` over Tailscale |
| Reinstall | Same Install path (overwrite binary + rewrite env + restart) |

### Probe sequence

1. Host validation (`is_tailscale_host`)
2. Optional `tailscale ping` if CLI present
3. SSH BatchMode `true`
4. `GET /health`
5. Optional `GET /v1/presence` with bag secret

### Out of scope

- Opt-in OAuth mirror
- Public ingress / non-TS SSH
- Automatic GitHub release asset download
- Changing companion `agent_task` LLM logic

## Consequences

- Operators can go from pairing form → Test → Install without leaving Settings.
- Non-root SSH without passwordless sudo cannot bootstrap; documented residual risk.
- Health ok does not imply LLM-ready (still needs `SOFTWAKE_NODE_XAI_API_KEY` or mirrored vault key).

## See also

- [remote-agent-pairing.md](remote-agent-pairing.md)
- [ADR-0039](ADR-0039-remote-agent.md)
