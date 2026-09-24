# ADR 0001 — Name and scope

- **Status:** Accepted
- **Date:** 2026-09-24

## Decision

Project name is **Softwake** (renamed from working title Stillroom). Phase 1 scope is voice-state reliability (sleep / awake / hibernate), soul pack (`soul.md`, `user.md`), Rust daemon + Tauri UI, and one safe tool. Long-term memory and world connectors are deferred.

## Context

Prior attempts: `/www/xai-voice` (webview audio), `/www/agent-desk` (Python conductor). Need a clean OSS-ready Rust tree and an unused name for domain + GitHub. Stillroom was a temporary scaffold name; Softwake better matches the wake/sleep product metaphor.

## Consequences

- Docs lead the repo before runtime code.
- agent-desk remains a reference, not a dependency.
- Paths, crates, and config use `softwake` (`~/.config/softwake/`, `softwake-daemon`, etc.).
- Domain: prefer checking `softwake.app` / similar if `softwake.dev` is unavailable.
