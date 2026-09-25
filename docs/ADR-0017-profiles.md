# ADR 0017 — Multi-profile packs and agent name

- **Status:** Accepted
- **Date:** 2026-09-25

## Decision

Softwake stores one or more **profiles** under the config root:

```text
~/.config/softwake/
  softwake.json                 # { "version": 1, "active_profile": "<id>" }
  profiles/<id>/profile.json    # { "id": "<id>", "name": "<agent name>" }
  profiles/<id>/{soul,user,rules,glossary}.md
  soul/                         # legacy pack; migration source only
```

Default profile id is `default`. Default agent name is `Softwake`.

Path resolution for the soul pack is:

1. `--soul-dir` (raw pack directory; skips the profile registry)
2. `SOFTWAKE_SOUL_DIR` (same)
3. Active profile pack under `profiles/<active_profile>/` after an idempotent migrate

First run with a legacy `soul/` directory **copies** the four markdown files into `profiles/default/`, writes `profile.json` with name Softwake, and writes `softwake.json` with `active_profile: default`. Legacy `soul/` is left in place and is not deleted.

`SoulPack::render_instructions_as(name)` inserts a code-owned Identity lead-in: `Your name is {name}.` Profile rename updates `profile.json` only. Phrase / keyword wake on that name is out of scope (separate KWS work).

Settings gains a **Profiles** left-nav pane: list / create / rename / set active, plus the four pack editors for the **selected** profile. Pack editors are removed from General. Protocol generation stays `1`. No new IPC status fields; the UI resolves profiles in-process. `reload_soul` still applies a valid pack on the next awake.

## Context

ADR 0011 defined the four-file pack in a single soul directory. Operators want more than one pack (work vs personal) and a stable agent name for typed asks. In-window editors already lived under General; they move to Profiles so General stays free of pack chrome.

## Alternatives

- Keep a single soul directory and only add a name field. Rejected: Spencer asked for multiple profiles.
- Put active profile id on the Status IPC object. Rejected for this slice: protocol stays 1; UI and daemon share path resolution.
- Move (not copy) legacy `soul/` on migrate. Rejected: copy is safer and leaves a rollback path.
- Wire the agent name into Sherpa / KWS wake phrases. Rejected: out of scope; name is stored for a later slice.

## Consequences

- Fresh installs create `profiles/default` on first resolve.
- Existing `~/.config/softwake/soul/` installs migrate without data loss.
- Flag / env overrides still point at a raw pack dir (optional `profile.json` for name).
- Public docs describe the layout without host-specific paths.
