# ADR 0052 — Agent team / local power (homes, rooms, peer DM, goal loop)

- **Status:** Accepted (implementing 2026-10-03)
- **Date:** 2026-10-03
- **Related:** [ADR 0017](ADR-0017-profiles.md), [ADR 0018](ADR-0018-tools-settings-shell.md), [ADR 0024](ADR-0024-timers-cron.md), [ADR 0025](ADR-0025-api-tool-calling.md), [ADR 0036](ADR-0036-agent-task-cron.md), [ADR 0041](ADR-0041-hud-profile-rail.md), [ADR 0051](ADR-0051-shell-redesign.md)

## Context

Softwake already has multi-profile packs ([ADR 0017](ADR-0017-profiles.md)), confirm-gated shell ([ADR 0018](ADR-0018-tools-settings-shell.md)), per-profile timers that tick every profile ([ADR 0024](ADR-0024-timers-cron.md)), agent-task cron ([ADR 0036](ADR-0036-agent-task-cron.md)), and HUD profile switching ([ADR 0041](ADR-0041-hud-profile-rail.md)). Gaps against the **agent team / local power** vision:

1. Shell runs in softwaked’s process environment — no per-agent home to build/run in.
2. Tools permissions are process-global (`tools.json`); there is no **allow-all** trust mode per profile.
3. There is no multi-agent **room**, turn-taking, or **peer DM** that wakes another profile.
4. `agent_task` fires use the **active** applied soul, not the schedule’s `profile_id` pack.
5. There is no first-class **goal-oriented outer loop** (goal + acceptance → plan/execute/verify until happy or stop) that any profile or room can invoke; coding-agent work is not wired to local `grok` CLI.

Spencer locked (2026-10-03): full machine capability with per-agent homes; ask before risky with an **allow-all** option; rooms where any member may speak after another with Softwake light turn-taking; peer DM wakes the target; independent profiles with timers; coding profiles use local grok plan↔revise↔execute; agents may compile/run in homes and ask to install software. Steering add: the goal-oriented loop is a **reusable Softwake capability** (tool/flow), not only a coding-profile habit — define goal + acceptance, iterate until happy or stop, with iteration caps, human gate on risky steps, and logged progress for rooms/PM.

### Code reality (coupling notes)

| Area | Today | Implication |
|------|--------|-------------|
| Profiles | `profiles/<id>/` pack + `profile.json` | Extend meta; add `homes/<id>/` under XDG data |
| Shell | `run_shell` → `/bin/sh -c`, no cwd/HOME pin | Pin cwd + `HOME` (+ `SOFTWAKE_AGENT_HOME`) to profile home |
| Tools | Global `tools.json` Always allow / Ask / Deny | Add profile `allow_all` (and optional global); risky install still Ask unless allow-all |
| Schedules | Tick all profiles; `agent_task` uses active soul | Load target profile pack for oneshot agent_task / peer DM |
| Sessions | One open session (active profile) | Peer/room/goal turns are **oneshot** (or short dedicated sessions) keyed by profile; HUD active profile unchanged unless operator switches |
| Chat tools | OpenAI tool advertise via Hands | New tools: `agent_message`, `goal_run` (and helpers); PROTOCOL stays **1** |
| UI | Settings panes + HUD rail | Rooms pane; profile Allow-all; optional goal progress in HUD |
| grok CLI | External binary on PATH | Coding backends spawn `grok` with plan/execute flags; Softwake owns the outer goal loop |

## Decision

### 1. Per-agent homes (full local power)

1. Each profile owns a **home directory**:
   `$XDG_DATA_HOME/softwake/homes/<profile_id>/` when `XDG_DATA_HOME` is set, else `~/.local/share/softwake/homes/<profile_id>/`.
2. Softwake creates the home on profile create and on first shell/goal use (idempotent `create_dir_all`).
3. Confirmed `shell` runs with:
   - process **cwd** = that home
   - env `HOME` = that home
   - env `SOFTWAKE_AGENT_HOME` = that home
   - env `SOFTWAKE_PROFILE_ID` = profile id  
   Other env (PATH, etc.) inherits from softwaked so compilers and language runtimes already on the machine remain usable.
4. Agents may compile and run any language available on the host **inside** the home. Paths outside the home are allowed when the operator has enabled shell (full machine capability) but remain subject to confirm / allow-all (below).
5. Public docs never cite private shop paths — only the XDG layout above.

### 2. Ask before risky + allow-all

1. Default remains: registry floors + operator Always allow / Ask / Deny in `tools.json` ([ADR 0018](ADR-0018-tools-settings-shell.md)).
2. Each `profile.json` gains `allow_all` (bool, default **false**). When true, for turns owned by that profile Softwake treats Ask-floor tools as **Always allow** (no pending card). Registry **Deny** stays deny (cannot lift).
3. Optional Settings → Tools **Allow all for active profile** checkbox writes that flag (and shows a clear trust warning).
4. **Software install** is a first-class confirm-gated tool `software_install` (risk Confirm; default Ask). Args: package manager hint + package list / command summary. Even with `allow_all`, Softwake still stages confirm for `software_install` unless the operator sets that tool to Always allow. Shell lines that look like `apt install`, `dnf install`, `pacman -S`, `brew install`, `cargo install` (host), `pip install`, `npm i -g` are routed to propose `software_install` instead of quiet shell when detected (heuristic; operator can still Always-allow shell).
5. Human gate: any Confirm decision still surfaces HUD Approve/Deny; goal-loop risky steps use the same path.

### 3. Rooms (multi-agent) + light turn-taking

1. A **room** is Softwake-owned state under  
   `$XDG_CONFIG_HOME/softwake/rooms/<room_id>.json` (else `~/.config/softwake/rooms/…`):
   ```json
   {
     "version": 1,
     "id": "standup",
     "title": "Standup",
     "members": ["default", "sally"],
     "created_ms": 0,
     "updated_ms": 0
   }
   ```
   Spencer (operator) adds/removes which profiles join via Settings → Rooms (or tools).
2. Room transcript / progress log:  
   `$XDG_STATE_HOME/softwake/rooms/<room_id>/log.jsonl` (else `~/.local/state/softwake/rooms/…`) — append-only JSON lines `{ts_ms, profile_id, name, kind, text}` where `kind` is `say` | `dm` | `goal_progress` | `system`.
3. **Turn-taking (light):** Softwake does **not** rigid round-robin. After a member finishes a turn, Softwake sets a short **cool-down** (default 1500 ms, code const) during which other members’ auto-replies are queued; the next speaker is either (a) the operator-addressed member, (b) the peer-DM target, or (c) the first queued member. Members never “pile on” the same stimulus in parallel — one room turn runs at a time (mutex / single flight per room_id).
4. Any member **may** speak after another once cool-down clears (no forced silence forever).
5. HUD: when a room is focused (Settings or later HUD chrome), appends from room log fan into a room view; v1 minimum is Settings → Rooms list + log tail + “Post as profile” for operator-driven kicks. Auto multi-agent chatter is driven by peer DM / goal_run / operator ask into the room.

### 4. Peer DM (wake another agent)

1. New confirm-gated tool **`agent_message`** (default Ask; description: send a message to another Softwake profile and wake it to reply).
   Args: `to` (profile id or name), `text`, optional `room_id`.
2. Softwake resolves the target profile, loads **that** profile’s soul pack (respecting `use_global_*`), runs a **oneshot** chat+tool-loop as that profile (HOME pinned to its home, `allow_all` of **target** applies), writes the exchange to the target’s HUD chat and optional room log, and returns the reply text to the caller.
3. Does **not** change `active_profile` or the operator HUD session unless the operator is already viewing that profile.
4. Messaging another agent is enough to wake it for that oneshot (no voice-state change required). If softwaked is hibernating, the tool returns an honest error (same class as other ask failures).

### 5. Independent profiles + timers

1. Schedule tick already scans all profiles — keep that.
2. **`agent_task` fires** load the schedule row’s `profile_id` soul + that profile’s `allow_all` / home for the oneshot (fix the active-soul coupling from ADR 0036 v1). Delivery channels stay keyed by `profile_id` ([ADR 0024](ADR-0024-timers-cron.md)).
3. Profiles remain independently schedulable; no requirement that the HUD active profile match the firing profile.

### 6. First-class goal-oriented loop (reusable capability)

This is a Softwake **capability**, not a coding-profile-only habit. Any profile, room, or operator flow may invoke it.

1. New tool **`goal_run`** (Confirm; default Ask):
   - `goal` (string) — what success looks like in prose
   - `acceptance` (string) — measurable checks (tests to run, files that must exist, ADR/changelog lines, commands that must exit 0, etc.)
   - optional `max_iterations` (default **8**, hard cap **32**)
   - optional `backend`: `softwake` (default) | `grok_cli`
   - optional `room_id` — progress lines append to that room log
   - optional `profile_id` — owner profile (default: active / calling profile)
2. **Outer loop** (owned by softwaked, logged):
   ```
   define (goal, acceptance, cap)
   for i in 1..=cap:
     plan     → produce/revise a short plan (model or grok --permission-mode plan)
     human_gate → if plan step is risky / needs Confirm, stage HUD confirm; abort on Deny
     execute  → run steps via tools/shell in profile home (or grok execute)
     verify   → run acceptance checks (shell commands / file predicates)
     log      → append goal_progress to room log + HUD note
     if acceptance satisfied → success stop
     if human stop / cancel → stop
     else revise plan with failure evidence
   stop with exhausted | success | cancelled | failed
   ```
3. **Measurable acceptance:** Softwake treats `acceptance` as an operator/agent-authored checklist. v1 verification runs newline-separated shell checks in the profile home (exit 0 = pass). Empty acceptance is invalid.
4. **Iteration caps:** `max_iterations` default 8; clamp 1..=32. Exhausted iterations return a structured summary (last plan, last verify output) without hanging forever.
5. **Human gate on risky:** uses existing Confirm path; `allow_all` may auto-approve non-`software_install` steps for that profile.
6. **Logged progress:** each iteration writes `goal_progress` with iteration index, phase, and short summary — room log when `room_id` set, else the owning profile’s HUD chat as a system/note turn.
7. **Coding profiles:** `profile.json` may set `"role": "coding"` (optional; default `"general"`). When `backend` is omitted and role is `coding`, Softwake prefers `grok_cli` if `grok` is on PATH; otherwise `softwake`. The grok backend invokes local `grok -m grok-4.7 --effort xhigh` plan then execute flavors Softwake already uses operationally — Softwake still owns the outer loop, caps, gates, and logs.
8. Rooms may invoke `goal_run` with `room_id` so PM-style progress is visible to members’ logs.

### 7. PROTOCOL and IPC

- PROTOCOL generation stays **1**.
- New behavior is tools + daemon-side oneshots + Settings/disk files. No new `ClientMessage` required for v1 (HUD confirm reuses pending tool cards). Additive Status fields only if unavoidable; prefer tool results + HUD notes.

### 8. Phased delivery (ship to completion)

| Phase | Scope | Exit |
|-------|--------|------|
| **P0** | ADR + milestones/changelog; profile home paths; shell cwd/HOME pin; `allow_all`; `software_install` tool + heuristic; tests | Unit tests green; docs honest |
| **P1** | Profile-scoped `agent_task` oneshot; `agent_message` peer DM wake; room store + log + turn cool-down + Settings → Rooms minimal UI | Tests + Settings list/create/members/log tail |
| **P2** | `goal_run` outer loop (`softwake` backend); progress logging; coding role → `grok_cli` backend when available; wire advertise/schema | Tests with mock verify; grok backend skipped in CI if no binary |
| **P3** | Polish: HUD room/goal notes, README operator section (public paths only), cargo install | CI green; merge |

P0–P2 may land as one PR or sequential PRs; all are required for “complete.”

## Alternatives

- **Coding-only implicit loop** (no tool). Rejected: Spencer required a reusable capability agents/profiles/rooms can invoke.
- **Full multi-session voice FSM per profile.** Rejected for v1: oneshots + single flight keep the daemon changeable; independent timers already work.
- **Containers / VMs per agent.** Rejected: homes + host toolchain match “full machine capability” with less machinery.
- **Raise PROTOCOL.** Rejected: tools + disk files suffice.
- **Separate coding-agent repo.** Deferred ideas already mentioned a boring harness elsewhere; this ADR keeps Softwake as the orchestrator and optionally shells out to `grok`.

## Consequences

- Operators who enable shell + `allow_all` grant a profile broad local power inside Softwake’s confirm model; Deny floors and `software_install` defaults remain seatbelts.
- Peer DM and rooms make Softwake a small agent team without requiring the HUD to juggle concurrent voice sessions.
- Goal loop gives measurable, capped, logged autonomy; coding profiles are one backend choice, not a special-case product.
- CI stays key-free / no live grok: unit tests mock verify and skip `grok_cli` when absent.
- softwaked/UI restart remains the operator’s job after `cargo install`.

## Non-goals (this ADR)

- Changing Telegram sticky ownership or Remote Agent pairing.
- Multi-conductor routing beyond rooms/peer DM/goal_run.
- Cloud sandbox or remote build farm.
- Force-push, secrets in repo, or documenting private host paths.
