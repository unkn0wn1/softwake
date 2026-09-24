# Soul pack

The soul pack is the **versionable personality and context** Softwake loads into an acting session. It is markdown (and later optional structured sidecars), not a proprietary blob.

## Files (phase 1)

| File | Role |
|------|------|
| `soul.md` | Agent identity, voice, boundaries, how it should behave while awake |
| `user.md` | Who the operator is: preferences, timezone, hard nos, how to address them |

Both are required for a valid pack. Missing files → daemon refuses to enter **awake** (UI can still hibernate/sleep and edit files).

## Later (not phase 1)

| Piece | Role |
|-------|------|
| Long-term memory | Durable facts / conversation memory — **Honcho** is an optional example; could also be a thin local store |
| `tools.md` or policy TOML | Human-readable tool policy mirroring allowlists |
| `AGENTS.md`-style lane rules | Optional; keep out of phase 1 unless needed |

When memory lands, it must be a **separate module** with clear read/write APIs. Do not stuff memory retrieval into `softwake-soul` parsing. Soul pack renders instructions; memory supplies retrieved snippets the session layer attaches.

## Loading rules

1. Read from configured soul directory (default `~/.config/softwake/soul/`).
2. Validate UTF-8, max size caps (prevent multi‑MB accidents).
3. Render a single system instruction document with clear sections:
   - Identity (`soul.md`)
   - User profile (`user.md`)
   - Runtime policy (state: awake; tool allowlist summary; confirm rules)
4. `reload_soul` IPC re-reads from disk; in-flight awake session either hot-reloads safely or requires re-wake (pick one in implementation and document it — prefer “applies on next awake” for simplicity).

## Repo templates

This repository ships examples under `soul/`:

- `soul/soul.md` — starter agent soul
- `soul/user.md` — starter user profile skeleton

Operators copy these into their config dir and edit. Do not commit personal secrets into `user.md` in git.

## Style note

Keep Softwake’s soul shorter than a general architect-bot bible; voice agents need tight instructions.
