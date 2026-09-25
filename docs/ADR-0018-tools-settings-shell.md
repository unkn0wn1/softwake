# ADR-0018 — Tools Settings and gated shell

- **Status:** Accepted
- **Date:** 2026-09-25

## Context

Softwake already had a deny-listed `shell` tool, glossary expand, and confirm-echo
readback ([ADR-0005](ADR-0005-tool-confirmation.md), [ADR-0011](ADR-0011-context-pack.md)).
Operators want a Settings surface to opt into shell (including ssh via glossary
host aliases) without turning Softwake into a silent full-machine agent.

## Decision

1. **Settings → Tools** is a left-nav pane. Softwake tools stay **off** until
   enabled there. v1 ships one switch: **shell**.
2. Non-secret flags persist in `$XDG_CONFIG_HOME/softwake/tools.json`
   (`shell_enabled`, `confirm_policy`). Default: shell off, policy `always`.
3. Registry risk for `shell` becomes **confirm** (not deny). The daemon still
   **denies** shell while `shell_enabled` is false.
4. Confirm policy (KISS):
   - `always` (default): every shell run stages confirm-echo.
   - `mutating_only`: quiet only when confirm-echo does not require readback.
   - `allowlisted_quiet`: reserved; same gate as `mutating_only` in v1.
5. Before stage/run, Hands expands the command with the active soul **glossary**
   (`aau → ssh -l root aau`). Aliases do not change tool risk.
6. Confirm uses the existing pending-tool / Status confirm path (HUD confirm
   when present). After accept, the daemon runs `/bin/sh -c` with a wall-clock
   timeout and stdout/stderr caps. It does not log the command line.
7. Ask path: when shell is enabled, clear `run` / `shell` / `ssh` / `ssh to … and …`
   lines become a gated shell proposal instead of a provider chat turn.
8. CI must not open live ssh. Unit tests cover expand, confirm gate, and local
   `echo` only.

## Consequences

- Softwake is still not “access everything”. Filesystem browser, email send
  changes, and a skills refine loop stay out of scope.
- Policy floor for `shell` is confirm; overrides may only tighten to deny.
- Operators must enable Tools → shell, add glossary aliases, and confirm before
  remote commands run.
