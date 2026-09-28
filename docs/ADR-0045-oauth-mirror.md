# ADR-0045 — Opt-in OAuth mirror to the companion

- **Status:** Accepted
- **Date:** 2026-09-28

## Context

Companion `agent_task` and Telegram away turns (ADR-0043 / ADR-0042) could not run
email, calendar, or Drive tools because OAuth tokens stayed laptop-local.
Operators who already pair a Tailscale companion need an **opt-in** way to mirror
only the Google/Microsoft connection rows the laptop already has, so cloud tools
work while the laptop is away — without expanding scopes or opening public ingress.

## Decision

1. **Default OFF.** `RemoteAgentConfig.oauth_mirror: bool` (serde default false).
   Settings → Remote Agent shows a clear checkbox and danger copy that tokens
   land on the companion CT. Document version of `remote-agents.json` stays 1.
2. **Sync.** When the first enabled agent has the flag on, softwaked `PUT`s
   `OauthMirrorDocument` (google/microsoft connections + active ids only) to
   `PUT /v1/vault/oauth` on the existing ~45s Tailscale mirror, authenticated
   with the pairing-secret Bearer. Flag off `PUT`s an empty document; the node
   deletes `vault/oauth.json` (mode 0600 under `$SOFTWAKE_NODE_DATA/vault/`).
3. **Security model.** Tailscale encrypts transit; pairing secret authenticates.
   No second cipher layer. Do not bind a public interface. Never log tokens.
   `GET /v1/vault/oauth` returns counts and emails only.
4. **Companion use.** When the vault has a usable connection and the tool is
   AlwaysAllow, softwake-node runs email/calendar/Drive via softwake-connectors
   + providers refresh under feature `live-http` (default on). Empty vault
   returns an enable hint. Ask stays Ask (pending text). Telegram away stays a
   no-tools oneshot unless the vault is usable, then it uses `run_agent_turn`.
5. **Refresh.** Node refreshes with publisher client id/secret from env or
   `oauth-clients.env` under the softwake home on the CT. Persist the refreshed
   row. Missing client id when refresh is required fails closed with the existing
   “no OAuth client configured” message.
6. **Multi-account.** Mirror all bag rows; `account=` / active-or-first /
   Google-preferred rules match ADR-0033. No per-profile bind.
7. **Revoke.** Disable the checkbox (softwaked running) so a clear PUT lands,
   rotate the pairing secret, and delete `vault/oauth.json` if a clear cannot land.
8. **PROTOCOL_VERSION** stays 1.

## Consequences

- Laptop tools are unchanged when the flag is off.
- Operators must place publisher client ids on the CT for refresh while away.
- Reinstall preserves allowlisted env keys and does not delete the data dir
  (reinstall is not revoke).
- Residual risk: disabling the whole agent without a clear PUT leaves the file
  until SSH delete; a CT root can always read `0600` files.

## Links

- [ADR-0033](ADR-0033-multi-account-oauth.md), [ADR-0039](ADR-0039-remote-agent.md),
  [ADR-0040](ADR-0040-remote-agent-presence-outbox.md),
  [ADR-0043](ADR-0043-companion-agent-task-llm.md),
  [ADR-0044](ADR-0044-remote-agent-ssh-installer.md)
- [remote-agent-pairing.md](remote-agent-pairing.md), [oauth-clients.md](oauth-clients.md)

## Demo

```bash
cargo test -p softwake-tools -p softwake-providers -p softwake-daemon -p softwake-node -p softwake-ui --locked
cargo test -p softwake-node --locked --no-default-features
```
