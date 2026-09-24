# ADR 0001 — Name and scope

- **Status:** Accepted
- **Date:** 2026-09-24

## Decision

Project name is **Softwake** (renamed from working title Stillroom). Phase 1 scope is voice-state reliability (sleep / awake / hibernate), soul pack (`soul.md`, `user.md`), Rust daemon + Tauri UI, and one safe tool. Long-term memory and world connectors are deferred.

## Context

Earlier prototypes tried webview-based audio and a Python conductor. Those approaches taught useful lessons (especially: do not put mic capture in Electron/Capacitor/webview), but Softwake needs a clean OSS-ready Rust tree. Stillroom was a temporary scaffold name; Softwake better matches the wake/sleep product metaphor.

## Consequences

- Docs lead the repo before runtime code.
- Prior prototypes are references for ideas only, not dependencies or vendored trees.
- Paths, crates, and config use `softwake` (`~/.config/softwake/`, `softwake-daemon`, etc.).
