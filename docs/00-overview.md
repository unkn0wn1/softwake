# Overview

## One sentence

Softwake is a **local, voice-first conductor**: it listens quietly for a wake phrase, becomes an acting agent with tools while awake, and can fully stop listening in hibernate until the UI wakes it.

## Why it exists

Chat UIs are not how you want to drive a machine when your hands are busy. Softwake aims for the smallest useful slice of an “AI OS”: reliable presence (wake / sleep / hibernate), a soul pack for personality and rules, and a thin allowlisted tool surface. Not a desktop rewrite.

## Principles

1. **Voice reliability before features.** If wake/sleep/hibernate is flaky, nothing else matters.
2. **Daemon owns the ear and the hands.** The UI configures; it does not own mic lifecycle or tool execution.
3. **Asleep ≠ deaf.** Sleep stops *acting*; local wake-word listening continues. Hibernate stops the mic.
4. **Soul on disk.** Personality and user profile are files you can diff and version — not buried in a cloud prompt.
5. **Least privilege.** Tools are allowlisted; dangerous actions need confirmation. “Full control” is a destination with gates, not day-one scope.
6. **Steal ideas, not codebases.** Prior webview-mic and Python-conductor experiments are lessons only; Softwake is a clean Rust tree.

## Non-goals (phase 1)

- Multi-agent orchestration / desk routing
- Email, Drive, calendar connectors
- Full desktop automation (window managers, arbitrary shell)
- Cloud-only wake word (ambient cloud VAD is not “sleep”)
- Electron / Capacitor / webview as the audio runtime
- Long-term memory productization. Phase 1 did not ship it. The store is an opt-in file ([ADR 0009](ADR-0009-long-term-memory.md)); the daemon still does not load it

## Phases (summary)

| Phase | Outcome |
|-------|---------|
| **1** | Local wake word → awake session → one safe tool → sleep phrase; hibernate from UI; soul.md + user.md loaded into session |
| **2** | Better TTS/STT or realtime voice; confirm flows; richer tool bus |
| **3** | Connectors (boundary first; live backends still open) + long-term memory (local trait and opt-in durable file; daemon unwired) + policy engine ([ADR 0010](ADR-0010-policy-engine.md)) |

See [06-milestones.md](06-milestones.md).

## Stack (intent)

| Layer | Choice |
|-------|--------|
| Daemon | Rust |
| UI | Tauri 2 (Rust + small web front-end) |
| Audio | PipeWire on Linux first; abstract traits for later platforms |
| Wake word | sherpa-onnx keyword spotting, weights not vendored ([ADR 0006](ADR-0006-on-device-wake.md)). The typed demo still uses the phrase-table spike ([ADR 0002](ADR-0002-wake-engine-spike.md)) |
| LLM / voice | Pluggable; realtime APIs are candidates, not hard dependencies in docs |
| Config / soul | Markdown + TOML on disk under XDG paths |
