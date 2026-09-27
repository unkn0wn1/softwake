# ADR 0038 — Authenticated webhook wake

- **Status:** Accepted
- **Date:** 2026-09-28
- **Depends on:** [ADR 0014](ADR-0014-skills-hub.md), [ADR 0003](ADR-0003-ipc-transport.md), [ADR 0012](ADR-0012-model-providers.md)

## Decision

Softwake exposes an **authenticated local HTTP webhook** that can wake the daemon from sleep without the microphone, and optionally deliver a bounded message that runs an agent turn (same inbound path as Telegram oneshot ask).

| | Choice |
|---|---|
| Bind | `127.0.0.1` only (never `0.0.0.0` / `::` in this slice) |
| Path | `POST /v1/wake` |
| Auth | Bearer shared secret: `Authorization: Bearer <secret>` or `X-Softwake-Webhook-Token: <secret>` |
| Secret storage | `SecretBag.webhook_secret` in the OS keyring / secrets bag (never `softwake.json`, never logged) |
| Enable | `webhook_enabled` in `softwake.json` (default `false`). Listener starts only when enabled **and** secret is non-empty |
| Port | `webhook_port` in `softwake.json` (default `8787`). Env `SOFTWAKE_WEBHOOK_PORT` wins when set to a non-zero port |
| Sleep | Valid request → `wake_phrase()` (same soul gate as `ctl wake`) |
| Awake | No state change; optional message → `messenger_ask` |
| Hibernate | **HTTP 409** — no auto-resume. Operator uses `ctl resume` / UI Resume, then retries |
| Message | Optional JSON `{"message":"<text>"}` (or `text/plain`); max **2000** Unicode scalars |
| PROTOCOL | Stays **1** (HTTP is a side channel, not IPC) |

HMAC signing is out of scope for v1; Bearer shared-secret is enough for loopback and for a reverse proxy that forwards the Authorization header.

### Operator surface

```text
softwaked ctl webhook status|enable|disable|port <n>
softwaked ctl webhook-secret set <token>|generate|clear
```

These commands edit local config / the secret bag and do **not** require a running daemon socket. After enable + secret, a running `softwaked serve` picks up enable/secret on its accept loop; **port changes may need a softwaked restart** to rebind.

### Example

```bash
softwaked ctl webhook-secret generate
softwaked ctl webhook enable
# after softwaked serve is running with this build:
curl -sS -X POST "http://127.0.0.1:8787/v1/wake" \
  -H "Authorization: Bearer $SOFTWAKE_WEBHOOK_SECRET" \
  -H "Content-Type: application/json" \
  -d '{"message":"Ping from webhook"}'
```

### Reverse proxy

To expose Softwake beyond the machine, terminate TLS on a reverse proxy, require the same Bearer secret (or an upstream auth gate), and forward to `127.0.0.1:<port>`. Do not bind Softwake on a public interface. Softwake does not ship a public open-internet webhook.

### Non-goals

- Hermes gateway / multi-platform bots / new messengers
- Open `0.0.0.0` default
- Auto-resume from hibernate
- Auto-send email / silent Always-allow elevation
- Changing PROTOCOL_VERSION

## Consequences

- External automations (n8n, cron elsewhere, email rules) can wake Softwake without the mic when the operator opts in.
- Hibernate remains a hard “leave me alone” mode for webhooks as well as voice.
- Skills and tool policy still apply to any message-driven turn ([ADR 0014](ADR-0014-skills-hub.md), [ADR 0010](ADR-0010-policy-engine.md)).
