# ADR 0011 — Context pack and confirm-echo

- **Status:** Accepted
- **Date:** 2026-09-25

## Decision

The soul directory holds four required files.

| File | Role |
|------|------|
| `soul.md` | Personality. It may be chatty. It cannot override rules. |
| `user.md` | Who the operator is. |
| `rules.md` | Hard constraints. It is rendered text. It is never a way to loosen the tool registry. |
| `glossary.md` | Short alias map. Personal paths live in the operator's copy, not in the repo template. |

Validation matches the existing file checks: present, non-empty (whitespace-only fails), valid UTF-8, at most 1 MiB. Check order is `soul.md`, then `user.md`, then `rules.md`, then `glossary.md`. The first failure wins. A missing directory is still a missing `soul.md`. After `glossary.md` passes those checks, the alias map is parsed. A bad map refuses the pack. Failure refuses awake. Hibernate, sleep, and UI resume stay available. `reload_soul` still re-reads now, sets the pending flag even when the read is invalid, and applies on the next successful awake. A refused wake leaves the flag set.

`SoulPack::render_instructions` order is:

1. Identity (`soul.md`)
2. User profile (`user.md`)
3. Rules, after a code-owned sentence: rules in this section override the identity section, and personality cannot loosen them
4. Glossary, after a code-owned sentence: an alias expands to the path on the right and does not grant a tool
5. Runtime policy stub, still last, including a code-owned sentence that rules override identity and a glossary alias does not change tool risk

Those two lead-in sentences and the policy block are constants in code. Deleting them from a markdown file does not remove them. Rules win over soul by position and by those sentences. The stub wins over files for tool names.

`PolicyEngine` does not read these files. `shell` stays deny. `email_send` stays confirm and in-memory. A line in `rules.md` that says shell is allowed does not change the registry.

Glossary expand is one pass over whole tokens. It does not expand `~`, environment variables, or a second alias. It does not touch the filesystem. Quotes are not special.

Confirm-echo: mutating or path-sensitive command text is read back before a later runner would start. `requires_readback` is true when the first token is a mutating verb (`rm`, `mv`, `cp`, `mkdir`, `rmdir`, `touch`, `chmod`, `chown`, `ln`, `dd`, `truncate`, `unlink`, `delete`, `send`, `write`, ASCII case-insensitive), when any original token is an alias, or when any expanded token is `~`, starts with `~/`, or starts with `/`. Otherwise a safe read can stay quiet. The readback string is still built. Accept replies are `yes`, `execute`, and `continue` (trimmed, ASCII case-insensitive). Any other non-empty reply is a correction in that beat, not a silent run. An empty reply is refused. This slice ships `Glossary::expand`, `Glossary::confirm_echo`, `SoulPack::confirm_echo`, and `classify_echo_reply`, with unit tests. The daemon confirm path for `notify` and `email_send` is still [ADR 0005](ADR-0005-tool-confirmation.md). No shell process. Echo text does not authorize anything. [ADR 0010](ADR-0010-policy-engine.md)'s empty override map stays empty.

Protocol generation stays `1`. No new command, event, or status field. `SoulReport` stays `{ ok, reason }`. A failed read can name `rules.md`, `glossary.md`, or an unparseable map in `reason`.

A later session may append memory snippets after the rendered pack. This ADR does not wire `FileMemory` into the prompt. The soul crate does not call `softwake-memory`.

## Context

Phase 1 shipped two files and a policy stub. Phase 3 shipped policy, connectors, and opt-in file memory, with the daemon still not calling memory. Phase 4 needs the constraint file and the alias map before provider OAuth.

The acting session stores one string: the rendered pack. There is no separate system-prompt file. Memory snippets stay out of this render. [ADR 0009](ADR-0009-long-term-memory.md) already says the session layer attaches them later.

## Alternatives

- Optional rules or glossary, warn and stay awake. Rejected. Awake would proceed with no rules file.
- Parse `rules.md` into `PolicyOverrides`. Rejected for this slice. [ADR 0010](ADR-0010-policy-engine.md) already says a future file may only tighten a known row. Prose that loosens `shell` must not become a row.
- Move the runtime policy stub above Identity. Rejected. The stub stays last so editable glossary text is not the last word on tool risk.
- New IPC for echo text, or a protocol bump. Rejected. Nothing on the socket needs a new field.
- In-window editors. Rejected for this slice. Reload already re-reads the directory. Editors stay an open Phase 4 item.
- A real shell behind confirm-echo. Rejected. [ADR 0005](ADR-0005-tool-confirmation.md) still denies `shell`. Echo text is not a process spawn.
- Recursive alias expansion and `~` / env expansion. Rejected. One pass and literal targets, so a cycle cannot loop and the readback shows the path the file actually contains.

## How to demo

```bash
mkdir -p ~/.config/softwake/soul
cp soul/*.md ~/.config/softwake/soul/
cargo test -p softwake-soul
cargo run -p softwake-daemon -- demo
```

`wake` reaches awake when all four files are valid. Remove `rules.md` and `wake` stays in sleep. `reload-soul` after restoring the file applies on the next awake. Expand and echo are covered by `cargo test -p softwake-soul`, not by a demo command.

A directory that only has the old two files refuses awake until `rules.md` and `glossary.md` are copied as well.

## Consequences

- Two-file installs refuse awake until `rules.md` and `glossary.md` are copied.
- Clients that speak protocol 1 see the same status shape. A failed read can now name `rules.md`, `glossary.md`, or a bad alias map in `soul.reason`.
- The next path-sensitive tool should call `SoulPack::confirm_echo` before it stages work, and should treat `classify_echo_reply` as the accept check. That tool is not in this change. `Hands` does not call these functions.
- UI editing of the four files remains open. The window reloads. It does not edit the files.
- Provider OAuth and live connectors stay later slices.
