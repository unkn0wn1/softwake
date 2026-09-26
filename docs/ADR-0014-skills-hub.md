# ADR 0014 — Skills hub, refine loop, and webhook wake

- **Status:** Accepted
- **Date:** 2026-09-25

## Decision

Softwake gains a **skills hub** and an **inbound webhook wake** path as product direction. This ADR locks the shape. It does not ship a crate, a socket message, or a listener in this change.

### Skills on disk

A skill is Markdown the operator can read and diff. Authored skills live under `$XDG_DATA_HOME/softwake/skills/` when that variable is set and non-blank, otherwise under `~/.local/share/softwake/skills/`. Optional runtime or refine scratch may live under `$XDG_STATE_HOME/softwake/skills/` when that variable is set and non-blank, otherwise under `~/.local/state/softwake/skills/`. Skills are not soul-pack files and are not loaded by `softwake-soul`.

Each skill uses three sections:

| Section | Role |
|---------|------|
| `procedure` | Steps Softwake should follow for this workflow |
| `pitfalls` | Known failure modes and what not to do |
| `verify` | How to check the outcome before claiming success |

The pattern is Hermes-like (disk skills, refine on correction, deepen profile memory). Softwake is not a Hermes clone and does not vendor a foreign skills runtime.

### Refine loop and “make a skill from this”

Refinement is **opt-in**. Softwake may update a skill when the operator corrects a run, or when an audited self-eval path is enabled. It must not spam a refine write every N tool calls without an audit trail.

After a successful workflow, the operator may ask Softwake to **create a skill from that turn**. Creation is operator-requested. Softwake does not invent a marketplace install from that ask.

Successful runs may also deepen `user.md` or `FileMemory` snippets where the fact belongs in profile or durable recall ([ADR 0009](ADR-0009-long-term-memory.md), [ADR 0011](ADR-0011-context-pack.md)). Skills stay procedures. Memory stays retrieved text. The soul pack stays instructions.

### Precedence and policy

Soul **rules beat skills**. A skill cannot loosen `softwake-policy`, cannot turn a deny row into confirm, and cannot auto-send email. Connector and tool risk stay on [ADR 0008](ADR-0008-connector-boundary.md), [ADR 0005](ADR-0005-tool-confirmation.md), and [ADR 0010](ADR-0010-policy-engine.md). Email send remains gated; the default product stance is **drafts to the operator, no auto-send**.

### Webhook wake

An inbound webhook may wake Softwake and run against skills and tool policy. Example tone: Softwake notices an important email and offers to forward a draft — it does not silently send.

Ingress is either:

- a **local listener** (default posture), or
- an authenticated HTTP endpoint protected by a **shared secret**

There is **no open-internet default**. An unauthenticated public bind is out of scope for the first implementation.

While asleep, a valid webhook may transition sleep→awake (same soul gate as other awake entry) or **queue** work until awake. Hibernate still means the mic is down; webhook handling while hibernated is a later detail and must not imply ambient listening. Invocations still pass policy before any confirm-gated tool runs.

### Non-goals

- Honcho (or any dialectic memory product) as Softwake core or a default dependency. Optional external memory later implements `Memory` only ([ADR 0009](ADR-0009-long-term-memory.md)).
- Auto-email-send.
- A cloud skill marketplace.
- Voice / STT / TTS model pickers (separate PR after this stub).

## Context

Phase 4 already loads a four-file context pack, talks to a Settings provider, and attaches budgeted `FileMemory` recall. Operators still lack a durable place for repeatable workflows, a safe way to capture “do it like that again,” and a way for an external event to wake Softwake without a microphone. Hermes-style disk skills and a locked-down webhook are the smallest direction that fits Softwake’s local, rules-first posture without pulling a foreign memory core into the daemon.

## Alternatives

- Clone Hermes or Honcho into Softwake core. Rejected. Softwake takes the pattern (disk skills, refine on correction, deepen local profile/memory) and keeps its own crates and policy.
- Put skills inside the soul pack (`rules.md` or a fifth file). Rejected. Rules are standing law. Skills are procedures. Rules beat skills.
- Auto-refine every N tool calls. Rejected. Too noisy and hard to audit. Opt-in / on correction / operator-requested create only.
- Open webhook on `0.0.0.0` without a secret. Rejected. Local listener or shared-secret auth; no open-internet default.
- Auto-send email from a skill or webhook. Rejected. Drafts to the operator; send stays confirm-gated.
- Ship the Rust listener and skills crate in this change. Rejected. This ADR is the direction stub. Implementation is a later slice.
- Fold voice model pickers into this PR. Rejected. Spencer ordered ADR first; pickers are the next PR.

## How to demo

This change is documentation only:

```bash
test -f docs/ADR-0014-skills-hub.md
rg -n 'ADR 0014|ADR-0014' README.md docs/06-milestones.md docs/00-overview.md docs/01-architecture.md
```

No new binary behavior. `cargo test --workspace` stays green because no Rust changed.

## Consequences

- Direction is locked: disk skills, opt-in refine, operator-requested skill creation, authenticated webhook wake, rules over skills, no Honcho core, no auto-send, no marketplace.
- Implementation (crate layout, on-disk schema version, listener bind, secret storage, IPC if any) comes in later PRs and must amend this ADR when those details are chosen.
- Protocol generation stays 1 until an implementation ADR or slice adds a message on purpose.
- Voice model pickers remain a separate follow-up and are not blocked on shipping skill code.
- Clients and CI see no runtime change from this stub alone.


## Amendment (2026-09-26) — Settings Skills page and `skill_save`

- Crate `softwake-skills` stores one Markdown file per skill under `$XDG_DATA_HOME/softwake/skills/` (else `~/.local/share/softwake/skills/`). Front matter: `version`, `title`, `source` (`user`|`agent`), `updated`. Body headings: Procedure, Pitfalls, Verify.
- Settings → Skills lists both sources and supports add/edit/remove (Tauri only; protocol generation stays 1).
- Registry tool `skill_save` is confirm-gated. Default Tools permission is Ask. After confirm the daemon writes `source: agent`. Ask heuristics (`make a skill …`, `make a skill from this`) stage the tool like shell intent.
- Webhook wake, auto-refine spam, and marketplace remain later. Soul rules still beat skills.
