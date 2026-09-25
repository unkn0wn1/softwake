# Soul pack

The soul pack is the **versionable personality and context** Softwake loads into an acting session. It is markdown, not a proprietary blob. [ADR 0011](ADR-0011-context-pack.md) is the decision for the four-file pack.

## Files

| File | Role |
|------|------|
| `soul.md` | Agent identity, voice, and boundaries. Personality cannot override `rules.md` |
| `user.md` | Who the operator is: preferences, timezone, hard nos, how to address them |
| `rules.md` | Hard constraints. Rendered text. It does not change the tool registry |
| `glossary.md` | Short alias map. A whole token expands to the path on the right. An alias is not a permission |

All four are required. A missing file, an empty file, invalid UTF-8, a file over 1 MiB, or an unparseable glossary refuses **awake**. Hibernate, sleep, and UI resume still run. A directory that only has `soul.md` and `user.md` refuses awake until `rules.md` and `glossary.md` are copied in.

Check order is `soul.md`, then `user.md`, then `rules.md`, then `glossary.md`. The first failure wins. A missing directory is a missing `soul.md`.

## Later (not this slice)

| Piece | Role |
|-------|------|
| Long-term memory | Separate crate `softwake-memory` ([ADR 0009](ADR-0009-long-term-memory.md)). Thin local store behind a trait. Off until the operator enables a backend. The durable backend is an opt-in JSON file. Honcho is not the default |
| `tools.md` or policy TOML | Human-readable tool policy mirroring allowlists. A later file may only raise a known row's risk, or be refused ([ADR 0010](ADR-0010-policy-engine.md)) |
| In-window editors | Still later. The General pane says the editors are coming. Status reloads the four files and does not edit them |

The runtime policy stub names `echo` (safe), `notify` (confirm), `email_send` (confirm), and `shell` (deny). `email_send` runs only after `confirm_tool` ([ADR 0008](ADR-0008-connector-boundary.md)). `rules.md` does not feed `PolicyEngine`. A sentence that allows `shell` does not make `shell` runnable.

Memory is a **separate module** ([ADR 0009](ADR-0009-long-term-memory.md)) with `remember`, `recall`, and `forget`. Do not stuff memory retrieval into `softwake-soul` parsing. Soul pack renders instructions. The session layer attaches retrieved snippets, and it does not attach them yet. The durable backend is `FileMemory`. This pack does not load `memory.json`.

## Loading rules

1. Read from the configured soul directory.
2. All four files must be present, non-empty (whitespace-only counts as empty), and valid UTF-8.
3. Each file is capped at 1 MiB so a multi-megabyte paste is rejected before it is decoded.
4. After `glossary.md` passes those checks, parse the alias map. A duplicate alias, a bad alias token, an empty target, or a target that contains a second arrow refuses the pack.
5. Render one instruction document with five sections:
   - Identity (`soul.md`)
   - User profile (`user.md`)
   - Rules (`rules.md`). A code-owned sentence above the file says rules override identity
   - Glossary (`glossary.md`). A code-owned sentence says an alias does not grant a tool
   - Runtime policy stub (state: awake; `echo` safe, `notify` and `email_send` confirm, `shell` deny; `notify` and `email_send` run only after `confirm_tool`). A code-owned sentence says rules override identity and an alias does not change tool risk
6. A missing or invalid pack **refuses awake**. Hibernate, sleep, and UI resume still run. The machine is not left half-awake.

The policy stub stays last. File text cannot move it.

## Glossary

Blank lines, and lines whose first non-whitespace character is `#`, are prose. Any other line without `→` or `->` is prose. A heading and no aliases is a valid file. The file itself must still be non-empty.

An alias line may start with `- ` or `* `. Split on the first arrow. The left side is one token matching `[A-Za-z_][A-Za-z0-9_-]{0,31}`. The right side is the rest of the line, trimmed and non-empty, and must not contain another arrow. Match is case-sensitive.

Expand splits on whitespace and replaces a token only when it equals an alias. One pass. No `~` expansion, no environment expansion, and no second pass. `docs/file` does not expand. Quotes are not special.

## Confirm-echo

`Glossary::confirm_echo` and `SoulPack::confirm_echo` build a readback for command text. They do not run a command. The daemon does not call them. `shell` stays deny.

`requires_readback` is true when the first token is a mutating verb, when any original token is an alias, or when any expanded token is `~`, starts with `~/`, or starts with `/`. A quiet safe read can skip showing the text. The string is still built.

Accept replies are `yes`, `execute`, and `continue`. Any other non-empty reply is a correction. An empty reply is refused. `classify_echo_reply` is that check. It does not authorize a tool.

## Directory

First match wins:

1. `--soul-dir PATH` on `softwaked serve` and `softwaked demo`
2. `SOFTWAKE_SOUL_DIR`
3. `$XDG_CONFIG_HOME/softwake/soul` when `XDG_CONFIG_HOME` is set and not blank
4. `~/.config/softwake/soul/` otherwise (`$HOME/.config/softwake/soul`)

The directory does not have to exist at startup. Serve and the demo still start; status reports the pack as missing until the files are in place and re-read.

## Reload

`reload_soul` (ctl `reload-soul`, the demo command `reload-soul`, and the UI button) **re-reads the four files from disk now** and reports whether that read is valid. The new text **applies on the next awake**. It does not replace instructions in an awake session that is already running.

- Startup reads the directory once and does not set the pending flag.
- `reload_soul` sets the pending flag, including when the new read is invalid.
- The flag clears only after a **successful** transition into awake applies the last good read.
- A refused wake leaves the flag set and leaves the voice state unchanged.
- Editing the files without `reload_soul` does not change what the next wake applies. The daemon uses the last startup or reload read, not a silent re-read at wake time.

Status carries an optional `soul` object: `{ "ok": true }` or `{ "ok": false, "reason": "..." }`. Peers that predate the field still decode. Protocol generation stays `1`. `reason` can name `rules.md`, `glossary.md`, or a bad alias map. No new field.

## Repo templates

This repository ships examples under `soul/`:

- `soul/soul.md` — starter agent soul
- `soul/user.md` — starter user profile skeleton
- `soul/rules.md` — starter constraints. They name "the operator"
- `soul/glossary.md` — starter alias map. Targets are placeholders such as `/path/to/docs`

Operators copy these into their config dir and edit:

```bash
mkdir -p ~/.config/softwake/soul
cp soul/*.md ~/.config/softwake/soul/
```

Put real paths in the config copy of `glossary.md`. Do not commit personal secrets into `user.md` in git.

## Style note

Keep Softwake’s soul shorter than a general architect-bot bible; voice agents need tight instructions.
