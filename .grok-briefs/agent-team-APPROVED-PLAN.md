# Softwake — ADR-0052 agent team / local power: APPROVED PLAN

**Status:** approve-ready (executor-authored from locked decisions + code inspection; goal loop is first-class Softwake capability).  
**Branch:** `feat/agent-team-adr0052`  
**Worktree:** `/tmp/softwake-agent-team` @ origin/main (`bc0d777`)  
**Model:** grok-4.7 `--effort xhigh` plan-then-execute; Softwake recovery if stall.

## Product locks

1. Full machine capability; per-agent homes; ask before risky; **allow-all** per profile.
2. Rooms: operator picks members; any may speak after another; Softwake light turn-taking (cool-down + single-flight).
3. Peer DM wakes target profile (oneshot; no active_profile steal).
4. Profiles independent timers; agent_task uses **schedule profile** soul/home.
5. Coding profiles prefer local grok CLI backend inside Softwake-owned outer loop.
6. Goal-oriented loop = **reusable tool/flow** (`goal_run`) for any agent/profile/room — define goal+acceptance, iterate plan/execute/verify until happy or stop; caps; human gate; logged progress.
7. Compile/run in homes; `software_install` asks (unless that tool Always allow).
8. No softwaked/UI restart by agent; no force-push; no secrets; public paths only; PROTOCOL 1.

## Phases → files

### P0 — homes + allow_all + software_install
- `softwake-soul`: `ProfileMeta.allow_all`, `role` (`general`|`coding`); `home.rs` resolve/ensure under XDG_DATA_HOME/softwake/homes/<id>
- `create_profile` ensures home
- `softwake-tools/shell.rs`: `run_shell_in(command, cwd, extra_env)` ; keep `run_shell` as thin wrapper
- Hands `spawn_shell` uses active (or context) profile home
- `software_install` tool + heuristic from shell text
- Settings Tools: Allow-all checkbox for selected/active profile
- Unit tests: home path, allow_all effective permission, install heuristic

### P1 — peer DM + rooms + profile-scoped agent_task
- `rooms.rs`: CRUD rooms JSON + append log.jsonl + cool-down/single-flight helpers
- Tools: `agent_message`, maybe `room_post` / list helpers as needed
- Daemon: oneshot as target profile (load pack by id, pin home, apply allow_all)
- Fix `fire_schedule_agent_task` to use schedule `profile_id` pack
- Settings → Rooms minimal UI

### P2 — goal_run
- `goal.rs`: loop state machine, acceptance shell checks, progress events
- Tool `goal_run`; softwake backend (plan via oneshot model, execute via Hands tools, verify via shell checks)
- `grok_cli` backend: spawn grok when on PATH; CI skips
- Room/HUD progress logging
- Tests with mock verify (no live grok)

### P3 — docs + install
- milestones, changelog, README section
- `cargo install` softwaked + softwake-ui usual features
- PR, CI green, merge

## Acceptance (this ADR)

- [ ] ADR-0052 in docs/ + milestones/changelog updated
- [ ] Homes created; shell cwd/HOME pinned; tests
- [ ] allow_all + software_install wired; tests
- [ ] Rooms + peer DM + profile-scoped agent_task; tests
- [ ] goal_run loop with caps + progress log; softwake backend tests
- [ ] CI green; PR merged; cargo install done; restart notes for Spencer
