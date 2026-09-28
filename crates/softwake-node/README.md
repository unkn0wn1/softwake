# softwake-node

Softwake **Remote Agent** companion binary (ADR-0039 / ADR-0040).

## Endpoints

| Method | Path | Auth | Notes |
|---|---|---|---|
| GET | `/health` | no | `{ok,role,version,presence_grace_ms}` |
| GET/POST | `/v1/presence` | Bearer | Heartbeat + effective state (90s grace → offline) |
| POST | `/v1/leases` | Bearer | Fire lease claim (409 if held) |
| GET/POST | `/v1/outbox` | Bearer | Durable per-profile away results |
| PUT | `/v1/schedules/{profile_id}` | Bearer | Mirror companion/auto rows |

## Env

- `SOFTWAKE_NODE_LISTEN` — default `127.0.0.1:8790` (use Tailscale IP in production)
- `SOFTWAKE_NODE_PAIRING_SECRET` — required for auth endpoints
- `SOFTWAKE_NODE_DATA` / `XDG_DATA_HOME/softwake-node` — durable outbox/leases/schedules
