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

The runtime policy stub names `echo` (safe), `notify` (confirm), `email_send` (confirm), and `shell` (deny). `email_send` runs only after `confirm_tool` ([ADR 0008](ADR-0008-connector-boundary.md)).

When memory lands, it must be a **separate module** with clear read/write APIs. Do not stuff memory retrieval into `softwake-soul` parsing. Soul pack renders instructions; memory supplies retrieved snippets the session layer attaches.

## Loading rules

1. Read from the configured soul directory.
2. Both files must be present, non-empty (whitespace-only counts as empty), and valid UTF-8.
3. Each file is capped at 1 MiB so a multi-megabyte paste is rejected before it is decoded.
4. Render one system instruction document with three sections:
   - Identity (`soul.md`)
   - User profile (`user.md`)
   - Runtime policy stub (state: awake; `echo` safe, `notify` and `email_send` confirm, `shell` deny; `notify` and `email_send` run only after `confirm_tool`)
5. A missing or invalid pack **refuses awake**. Hibernate, sleep, and UI resume still run. The machine is not left half-awake.

## Directory

First match wins:

1. `--soul-dir PATH` on `softwaked serve` and `softwaked demo`
2. `SOFTWAKE_SOUL_DIR`
3. `$XDG_CONFIG_HOME/softwake/soul` when `XDG_CONFIG_HOME` is set and not blank
4. `~/.config/softwake/soul/` otherwise (`$HOME/.config/softwake/soul`)

The directory does not have to exist at startup. Serve and the demo still start; status reports the pack as missing until the files are in place and re-read.

## Reload

`reload_soul` (ctl `reload-soul`, the demo command `reload-soul`, and the UI button) **re-reads the files from disk now** and reports whether that read is valid. The new text **applies on the next awake**. It does not replace instructions in an awake session that is already running.

- Startup reads the directory once and does not set the pending flag.
- `reload_soul` sets the pending flag, including when the new read is invalid.
- The flag clears only after a **successful** transition into awake applies the last good read.
- A refused wake leaves the flag set and leaves the voice state unchanged.
- Editing the files without `reload_soul` does not change what the next wake applies. The daemon uses the last startup or reload read, not a silent re-read at wake time.

Status carries an optional `soul` object: `{ "ok": true }` or `{ "ok": false, "reason": "..." }`. Peers that predate the field still decode. Protocol generation stays `1`.

## Repo templates

This repository ships examples under `soul/`:

- `soul/soul.md` — starter agent soul
- `soul/user.md` — starter user profile skeleton

Operators copy these into their config dir and edit. Do not commit personal secrets into `user.md` in git.

## Style note

Keep Softwake’s soul shorter than a general architect-bot bible; voice agents need tight instructions.
