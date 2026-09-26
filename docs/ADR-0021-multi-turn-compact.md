# ADR 0021 — Multi-turn awake chat and Hermes-style compaction

- **Status:** Accepted
- **Date:** 2026-09-26
- **Relates to:** [ADR 0013](ADR-0013-session-provider.md) (amends the one-shot ask wire shape)

## Decision

While Softwake is awake, each `ask` / `chat` turn sends:

1. **system** — the rendered soul pack plus the budgeted memory appendix (unchanged from ADR 0013 / ADR 0011). Rules and runtime policy stay in system and are never dropped by compaction.
2. **messages** — prior `{role: user|assistant, content}` turns for this awake session, then the new user line.

The session stores both user and assistant text for the awake period. Sleep and hibernate still close the session and clear history.

### Context limit

Softwake’s model cache remains ids-only. Context window size is resolved in this order:

1. Settings override `context_limit_tokens` when set to a positive value.
2. Else a built-in defaults map for known Grok / OpenAI-family ids (substring / exact matches documented in code).
3. Else fallback **128000** tokens.

Completion `max_tokens` (`CHAT_MAX_TOKENS`) stays separate from the context window. This slice does not special-case o-series rejection of `max_tokens` (same as ADR 0013).

### Compaction threshold

Settings `compact_at_percent` defaults to **70**. Before each ask, Softwake estimates usage of `system + prior messages + new user + reply headroom` (`CHAT_MAX_TOKENS`). When that estimate is ≥ `compact_at_percent` percent of the resolved limit, Softwake compacts older turns:

- Summarize the compacted prefix with one short provider completion (same selected model), or a local extractive fallback if that call fails (fail-open).
- Replace the compacted prefix with one user-role message: `Session summary: …`.
- Keep the recent tail raw (`keep_recent_turns`, default **8** message entries).
- Surface on Status that compaction ran, and show `context ~used / limit (percent%)` while awake.

### Estimator (v1)

Character-based: `ceil(char_count / 4)` (UTF-8 Unicode scalars). Optional later: provider response `usage`. Documented here so operators know the figure is approximate.

### Settings keys (providers.json, document version 1, additive)

| Key | Default | Meaning |
|-----|---------|---------|
| `context_limit_tokens` | `0` (unset → map / 128000) | Positive override for the context window. |
| `compact_at_percent` | `70` | Compaction trigger as percent of the limit. |
| `keep_recent_turns` | `8` | Recent raw message entries retained after compact. |

UI: Providers pane — “Context limit (tokens)” and “Compact at (%)”. Status pane shows context usage when awake.

## Context

ADR 0013 shipped one completion per ask as system + this user line only. User lines accumulated on the session but were not replayed; assistant text was not stored. Operators need multi-turn continuity in one awake session without silently overflowing the model window.

## Alternatives

- Amend ADR 0013 in place only. Rejected as the primary record; this ADR names the multi-turn + compact product locks.
- Put compaction inside the session crate with an HTTP client. Rejected: session stays free of provider I/O (ADR 0013).
- Raise `CHAT_MAX_TOKENS` to match the window. Rejected: completion budget stays separate; o-series notes in ADR 0013 still apply.
- Drop system sections during compact. Rejected: soul / rules / runtime policy must remain in system.

## Consequences

- Provider POST messages are `system` then the session history (including the new user turn).
- Fixture and e2e tests that assumed a two-message body after the first turn still hold; later turns include prior user and assistant content.
- Compaction adds at most one extra completion when the threshold is crossed.
- Protocol generation stays 1; Status fields for context are additive and omitted when asleep.
