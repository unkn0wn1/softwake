# softwake-node

Softwake **Remote Agent** companion binary (ADR-0039 / ADR-0040 / ADR-0042 / ADR-0043).

## Endpoints

| Method | Path | Auth | Notes |
|---|---|---|---|
| GET | `/health` | no | `{ok,role,version,presence_grace_ms}` |
| GET/POST | `/v1/presence` | Bearer | Heartbeat + effective state (90s grace → offline) |
| POST | `/v1/leases` | Bearer | Fire lease claim (409 if held) |
| GET/POST | `/v1/outbox` | Bearer | Durable per-profile away results |
| PUT | `/v1/schedules/{profile_id}` | Bearer | Mirror companion/auto rows |
| PUT | `/v1/vault/telegram` | Bearer | Mirrored bot token |
| PUT | `/v1/vault/llm` | Bearer | Optional mirrored xAI key |
| PUT | `/v1/profiles/{id}/messengers` | Bearer | Per-profile messengers |
| PUT | `/v1/profiles/{id}/soul` | Bearer | Mirrored soul pack (4 markdown files) |
| PUT | `/v1/tools` | Bearer | Mirrored `tools.json` permissions |
| PUT | `/v1/skills` | Bearer | Mirrored skills catalog |
| GET/POST | `/v1/telegram/ownership` | Bearer | Sticky ownership (ADR-0042) |

## Env

- `SOFTWAKE_NODE_LISTEN` — default `127.0.0.1:8790` (use Tailscale IP in production)
- `SOFTWAKE_NODE_PAIRING_SECRET` — required for auth endpoints
- `SOFTWAKE_NODE_DATA` / `XDG_DATA_HOME/softwake-node` — durable outbox/leases/schedules/soul/skills/tools/vault
- `SOFTWAKE_NODE_XAI_API_KEY` — preferred LLM key for `agent_task` + Telegram oneshot (else mirrored vault)
- `SOFTWAKE_NODE_MODEL` — optional model id (default `grok-4-fast-non-reasoning`)

## Companion `agent_task`

When a mirrored schedule with `action: agent_task` and `run_on: companion|auto` fires while the laptop is away, the node loads that profile’s mirrored soul + tools + skills and runs a bounded tool loop (max 6 rounds). Ask tools become pending text (never silent Always-allow). OAuth tools refuse. Results go to the outbox and Telegram when messengers want timer push. See ADR-0043.
