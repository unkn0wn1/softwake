# ADR-0042 — Telegram sticky ownership (laptop ↔ companion)

- **Status:** Accepted
- **Date:** 2026-09-28
- **Depends on:** [ADR-0029](ADR-0029-messengers-telegram.md), [ADR-0039](ADR-0039-remote-agent.md), [ADR-0040](ADR-0040-remote-agent-presence-outbox.md)

## Context

Slice 2 shipped presence (90s grace), fire leases, and outbox. Telegram still long-polled **only** on laptop softwaked, so Sleep / hard suspend left inbound chats unanswered. Operators need a **single sticky owner** for `getUpdates` so the companion can answer while the laptop is away — without double replies when the laptop returns.

## Decision

### Single owner

Exactly **one** surface owns the Telegram Bot API long-poll at a time.

| Companion effective presence | Telegram owner |
|---|---|
| `present` | **Laptop** softwaked |
| `sleeping` / `hibernated` / `offline` | **Companion** softwake-node (iff mirrored bot token exists) |

Grace aligns with ADR-0040: **`PRESENCE_GRACE_MS = 90_000`**. Missing heartbeat → `offline` → companion eligible.

### Transfer

1. **Relinquish:** Laptop voice → Sleep/Hibernate (or hard suspend). Laptop **stops** issuing `getUpdates`. Heartbeat carries non-present (or grace → offline). Node sets owner=`companion` and starts long-poll with the mirrored token.
2. **Resume:** Laptop Awake → heartbeat `present`. Node sets owner=`laptop` and **stops** polling (in-flight long-poll is not processed if ownership flipped). Laptop resumes `getUpdates`.

Laptop gate is local (voice + “companion enabled?”) so Sleep stops the poll immediately without waiting on Tailnet RTT. Node gate is presence + vault token.

Explicit endpoints (Bearer pairing secret):

- `GET /v1/telegram/ownership`
- `POST /v1/telegram/ownership` `{"claimer":"laptop"|"companion"}` → 200 or 409

### What must be mirrored for the node to answer Telegram

| Item | Required? |
|---|---|
| `telegram_bot_token` → node vault (`PUT /v1/vault/telegram`, file mode 0600) | **Yes** — without it the node never polls |
| Per-profile `messengers.json` (`PUT /v1/profiles/{id}/messengers`) | **Yes** for chat_id / Default · Receive-all · Voice flags |
| `xai_api_key` vault or `SOFTWAKE_NODE_XAI_API_KEY` | Optional — enables bounded oneshot replies; else honest stub text |
| OAuth / Google / Microsoft tokens | **No** |
| Full soul pack / skills tool-loop | **No** this ADR (companion `agent_task` remains stub) |

Laptop mirrors token + messengers (+ optional xAI key) on the existing ~45s mirror loop. Never log secrets.

### Companion inbound behavior

- Long-poll only when desired owner is `companion`.
- Resolve profile via mirrored `chat_id`; bind on first inbound if needed.
- Reply via bounded xAI oneshot when a key is present; otherwise stub: laptop-away honesty.
- Push `telegram_inbound` outbox summaries for laptop HUD “while away” pull.
- **TTS skipped** on the node (no audio stack); Voice flag is recorded but not synthesized on CT.

### Failure modes

| Failure | Behavior |
|---|---|
| Race both poll | Presence gates + owner record; Telegram serializes `getUpdates`; flipped owner discards mid-poll batch |
| No mirrored token | Node does not poll; messages wait at Telegram until laptop owns again |
| Companion down, laptop sleeping | Nobody polls until laptop Awake or node recovers |
| Hard OS suspend | Up to 90s grace before companion owns |
| Token rotate on laptop | Next mirror overwrites node vault |
| No LLM key on node | Stub reply + outbox |

### Solo laptop

When no enabled Remote Agent is configured, softwaked always owns Telegram (unchanged).

## Consequences

- Sleeping Softwake no longer abandons Telegram if a paired companion has a mirrored token.
- Operators must keep the bot token on the laptop secret bag so mirror can run; optional node LLM key for real away-replies.
- Full companion agent_task tool-loop, SSH installer, and OAuth mirror remain later slices.

## See also

- [remote-agent-pairing.md](remote-agent-pairing.md)
- [ADR-0040](ADR-0040-remote-agent-presence-outbox.md)

## Amendment — OAuth mirror (ADR-0045)

Inbound Telegram on the companion stays a no-tools oneshot unless the OAuth
vault has a usable connection, in which case it uses `run_agent_turn`. Sticky
ownership is unchanged. Opt-in mirror:
[ADR-0045](ADR-0045-oauth-mirror.md).

