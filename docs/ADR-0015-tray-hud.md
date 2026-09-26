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
- Submitting a line uses the existing awake `ask` path (`Client::call_ask`). From sleep, the HUD may send `wake` then `ask` when the soul pack allows awake. Hibernate must be left first (`wake_from_ui` / Settings Resume).

### Level input (v1)

v1 shipped a **mock level** (sine while capture is running, near-zero otherwise) so particles could ship without a protocol field. **Capture RMS on `Status::capture_level` is [ADR 0016](ADR-0016-capture-level-hud.md)**; the HUD prefers that value and keeps the sine only as fallback.

### Non-goals

- Live PipeWire capture or sherpa wake/STT/TTS in this change.
- Skills hub implementation ([ADR 0014](ADR-0014-skills-hub.md)).
- Live email / Drive / calendar clients.
- Secret-bag encryption changes.
- Replacing the Settings left-nav shell.
- Settings text-size density ([ADR 0020](ADR-0020-ui-text-scale.md)); deferred here.

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


## Amendment — placement and collapsed size (2026-09-25)

Live multi-monitor feedback locked two placement rules:

1. The HUD anchors to the **bottom-right of the primary monitor work area** (not under the cursor, not top-right). Placement uses the monitor origin (`Monitor::position` / `work_area`) so a primary that is not at `(0, 0)` still lands on the main screen.
2. The capsule stays **tiny when collapsed** (bloom only). A click expands to a **fixed** larger window that shows the type strip and reply. Collapse re-anchors to the same primary bottom-right corner. Always-on-top is unchanged.

Space and Enter on the capsule toggle expand only when the capsule itself is focused. Keys typed in the ask field are never stolen.

## Amendment (2026-09-25) — placement, drag, z-order

- Default park remains **primary monitor bottom-right** (work area). A missing monitor API falls back to the first monitor, then a non-centered corner — never silent center.
- Operators may **drag** the undecorated capsule; Softwake persists logical top-left in `hud-position.json` under the Softwake config root. While that file exists, expand/collapse **resizes only** and does not re-anchor to bottom-right.
- The HUD re-asserts **always-on-top** after layout changes, when Settings is shown from the tray, and when Settings or the HUD gains focus, so the capsule stays above the Settings window on Linux.
- Press-to-talk release paints the idle mic and a **thinking…** line before the blocking STT/ask/TTS round trip (`spawn_blocking` in the UI commands) so the HUD chrome does not freeze for the duration of the call.

## Amendment (2026-09-25) — bloom during think/speak

After playback and HUD fire-and-forget ([ADR 0007](ADR-0007-awake-stt-tts.md)), bloom could still freeze mid-STT/ask because sync `hud_snapshot` blocked the Tauri main thread on a contended daemon `Mutex<Runtime>`.

- `hud_snapshot` runs on the blocking pool (`spawn_blocking`), same as talk/ask.
- Serve `GetStatus` uses `try_lock`: when ask/talk_stop holds the runtime lock, the server returns the last cached `Status` (with a thinking line published before the long hold) so HUD polls never wait on the pipeline.
- The HUD particle loop always uses last-known level; while `talkPending` / thinking it adds a local sine breath so the capsule stays lively even if capture RMS is briefly stale.

## Amendment — square collapse, bubbles, idle (2026-09-26)

- Collapsed default is a **square** bloom, 120×120 logical pixels. Particles are centered. The composer, chat log, and hint are hidden.
- A click expands to 400×480: a short bloom strip, scrollable bubbles, and a bottom composer (text, Send, mic icon). Each bubble is labeled `You` or the active profile name and has a timestamp. Bubble text is the full model reply (no line clamp). Recent turns live in a client-side ring buffer for the HUD session.
- The mic control is an icon only. Its accessible name stays “Hold to talk”. Space and Enter on that button are press-to-talk. Space typed in the ask field is not stolen. Space on the collapsed capsule still toggles expand.
- Idle collapse default is 3 seconds. The pointer inside the capsule, or a non-empty ask draft, keeps it open. Leaving starts the timer. Settings → General stores `hud_idle_collapse_ms` in `ui-prefs.json` (default 3000, clamp 1000–30000, shown as 1–30 seconds). The HUD re-reads the file. No daemon IPC.
- A saved drag keeps the bottom-right corner across expand and collapse instead of jumping to primary bottom-right. First launch with no saved position still parks at primary bottom-right. Always-on-top is unchanged.
