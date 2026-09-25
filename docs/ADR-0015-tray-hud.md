# ADR 0015 — System tray and always-on-top HUD

- **Status:** Accepted
- **Date:** 2026-09-25

## Decision

`softwake-ui` always presents Softwake in the **system tray** and as a small **always-on-top HUD capsule**. The full Settings window (left nav: General, Providers, Email, Status) stays a separate window. Protocol generation stays **1**.

### Tray

- A tray icon is present whenever the UI process is running.
- The icon reflects voice state: **sleep**, **awake**, or **hibernate**.
- Menu entries: a Status label (current state), **Settings** (show or focus the Settings window), and **Quit**.
- Closing the Settings window **hides** it; it does not quit Softwake. Quit leaves from the tray.

### HUD capsule

- A compact, undecorated, always-on-top corner window sits above other apps without dominating the screen.
- While capture is running (sleep or awake listening), the capsule shows **blooming multi-colour particles** (not stars). Density and brightness scale with a **level** input. Quiet is sparse; louder is denser and brighter. Sleep prefers a cooler palette; awake prefers a warmer one when both are easy to distinguish.
- Clicking the capsule slides out a **chat / type strip**. After idle, the strip fades so only the listening particles remain.
- Submitting a line uses the existing awake `ask` path (`Client::call_ask`). From sleep, the HUD may send `wake` then `ask` when the soul pack allows awake. Hibernate must be left first (`wake_from_ui` / Settings Wake).

### Level input (v1)

v1 shipped a **mock level** (sine while capture is running, near-zero otherwise) so particles could ship without a protocol field. **Capture RMS on `Status::capture_level` is [ADR 0016](ADR-0016-capture-level-hud.md)**; the HUD prefers that value and keeps the sine only as fallback.

### Non-goals

- Live PipeWire capture or sherpa wake/STT/TTS in this change.
- Skills hub implementation ([ADR 0014](ADR-0014-skills-hub.md)).
- Live email / Drive / calendar clients.
- Secret-bag encryption changes.
- Replacing the Settings left-nav shell.

## Context

Operators need Softwake present while they work in other apps: a tray for status and quit, and a thin capsule for glanceable listening feedback plus a typed ask without opening Settings. Particles tied to level make “is it hearing?” visible without a large window.

## Alternatives

1. **Settings-only window** — rejected; Softwake disappears behind other apps and has no ambient listening cue.
2. **Single window that morphs** — rejected; Settings (editors, providers, confirm) needs space; the capsule must stay small and always-on-top.
3. **Protocol field for RMS in the tray slice** — deferred then; [ADR 0016](ADR-0016-capture-level-hud.md) adds the additive `capture_level` field with the audio spike.

## Consequences

- `softwake-ui` enables Tauri’s `tray-icon` feature and creates a second webview (`hud`).
- Linux CI already installs AppIndicator packages for the tray.
- README documents how to run the UI with tray + HUD (`softwaked serve`, then `cargo run -p softwake-ui`).
