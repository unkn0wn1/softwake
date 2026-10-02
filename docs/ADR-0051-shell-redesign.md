# ADR 0051 — Shell redesign: main window / HUD orb, 2-way chat, user profile inheritance

- **Status:** Accepted (docs / phased plan only — implementation waits for explicit slice approval)
- **Date:** 2026-10-02
- **Related:** [ADR 0015](ADR-0015-tray-hud.md), [ADR 0017](ADR-0017-profiles.md), [ADR 0022](ADR-0022-voice-modes.md), [ADR 0041](ADR-0041-hud-profile-rail.md), [ADR 0046](ADR-0046-hud-ux-thread.md), [ADR 0049](ADR-0049-duplex-barge-stream-tts.md), [ADR 0050](ADR-0050-voice-agent-s2s.md)

## Context

Softwake’s daily surface is still the **HUD capsule** from [ADR 0015](ADR-0015-tray-hud.md): a collapsed **120×120 bloom**, single-click expand to a resizable chat strip (~520×620 default), idle auto-collapse (default 3 s unless pinned), and Settings as a **separate** tray-opened window. There is no separate “main chat window” today — expanded HUD *is* the chat shell.

Operators now want a Grok-Bot-like presence model:

1. **Expanded main window** by default (on start / after sleep), with the bloom acting as an **orb** that single-clicks to shrink / expand — stays open until the orb is clicked (not idle collapse, not double-click).
2. A clear **2-way chat** control plus a **voice chevron** on the main composer, with a **minimal** 2-way + voice strip when shrunk.
3. Voice stickiness that remembers the last voice until a **profile switch**, then falls back to **Default** (Eve / resolver default) — not a required per-profile voice field.
4. Per-profile **Use main user profile** for `user.md` (default checked → show main user.md **disabled**; unchecked → editable, seeded with commented recommendations).
5. Settings stays a **separate window**; gear on main window and shrunk HUD opens it. Profile rail and reload-on-switch stay ([ADR 0041](ADR-0041-hud-profile-rail.md)).
6. New Settings: **shrunk HUD size** + **bloom intensity**.
7. **2-way ON** starts **awake** (listening). Unchecking 2-way is how the operator hibernates / sleeps listening. If Voice Agent S2S is the active path ([ADR 0050](ADR-0050-voice-agent-s2s.md)), 2-way **is** S2S; otherwise soft-duplex STT → chat → TTS ([ADR 0049](ADR-0049-duplex-barge-stream-tts.md)).

This ADR locks those product decisions and a **shippable phased plan**. It does **not** implement UI or daemon behaviour.

### Code reality (coupling notes)

| Area | Today | Implication |
|------|--------|-------------|
| Windows | Tauri: `hud` webview + Settings webview + tray | “Main window” = expanded HUD; “HUD orb” = collapsed bloom. No third chat window. |
| Expand / collapse | `hud.js` `setExpanded` + `hud_set_layout`; starts **collapsed**; capsule click / Space / Enter toggles | Default-expanded + “stay open until orb click” must retire or repurpose idle auto-collapse + pin semantics. |
| Collapsed size | Hard-coded `HUD_COLLAPSED_W/H = 120` in `softwake-ui` | Shrunk-size Settings needs prefs + layout path (expanded size already in `ui-prefs.json`). |
| Idle collapse | `hud_idle_collapse_ms` (Settings General) + pin | Conflicts with locked “stays open until orb clicked”. |
| 2-way control | No named toggle; closest are wake / sleep / hibernate, `auto_listening`, mic mute, Voice Agent S2S | New composer control must map cleanly onto voice-state machine without duplicating tray Resume paths. |
| Voice pick | Global `providers.json` `selected_tts_voice`; `/voice` slash + Providers pane | Stickiness-until-profile-switch is a UI/session latch; do not invent a required pack field. |
| `user.md` | One file per profile pack ([ADR 0017](ADR-0017-profiles.md)); editors always editable when a profile is selected | Inheritance needs a profile flag + resolve path that can point at main profile’s `user.md`. |
| Settings open | Tray menu → `show_settings` only | Gear on main + orb is new chrome; reuse the same show/focus path. |
| Bloom | Particle density/brightness track capture level; opacity is `hud_opacity` | Intensity knob is new; keep level-driven bloom as the base signal. |

## Decision

### 1. Shell vocabulary

- **Main window** — the expanded Softwake chat shell (today’s expanded HUD strip + profile rail + composer + log).
- **HUD / orb** — the shrunk bloom-only window (today’s collapsed capsule).
- Softwake still uses **one** undecorated always-on-top window that morphs size; Settings remains a **second** window ([ADR 0015](ADR-0015-tray-hud.md) alternative #2 is accepted only for chat↔orb, not for Settings).

### 2. Expand / shrink interaction (locked)

1. On UI start and when returning from sleep (voice state sleep → awake, or UI reopen while daemon is awake), the shell opens **expanded** (main window).
2. **Single-click the orb** (bloom region / dedicated orb affordance) **shrinks** main → HUD, or **expands** HUD → main. Not double-click.
3. Main stays open until the operator shrinks via the orb (or Quit). **Idle auto-collapse is removed or disabled by default** under this model; the existing pin control becomes redundant for “keep open” and may be removed or demoted in implementation (see Non-goals / P0).
4. Space / Enter on the focused orb may still toggle expand for accessibility; keys in the ask field are never stolen (unchanged).
5. Drag, always-on-top, and saved position behaviour from ADR 0015 amendments stay unless a slice explicitly changes them.
6. Expanded size remains operator-resizable with `hud_expanded_w` / `hud_expanded_h`. **Shrunk HUD size** becomes a Settings preference (clamped), replacing the hard-coded 120×120 default.

### 3. Composer chrome

| Surface | Controls |
|---------|----------|
| **Main window** | Full composer: text ask, Send, Cancel when in flight, **2-way chat toggle**, **voice chevron** (roster + current), existing mic/PTT as needed, profile rail, gear → Settings. |
| **Shrunk HUD** | Minimal strip: **2-way** + **voice** (compact), bloom, gear → Settings. No full log / rail unless a later slice expands scope. |

### 4. 2-way chat semantics

1. **2-way ON** → Softwake is **awake** and listening (free-speech / soft-duplex path, or Voice Agent S2S when that mode is active).
2. **2-way OFF** → operator intent to stop two-way listening: transition to **sleep** or **hibernate** per the existing voice-state machine and Settings/KWS conventions (implementation picks the honest mapping; prefer sleep when KWS wake should remain, hibernate when capture should fully stop — document the choice in the implementing PR).
3. If `voice_agent_s2s` is enabled and the live path is xAI Voice Agent ([ADR 0050](ADR-0050-voice-agent-s2s.md)), **2-way ON is S2S**; Softwake does not also run free-speech STT→ask→TTS on the same mic (same bypass rule as ADR 0050).
4. Otherwise 2-way ON uses soft-duplex / STT → chat tool-loop → TTS ([ADR 0049](ADR-0049-duplex-barge-stream-tts.md), [ADR 0007](ADR-0007-awake-stt-tts.md)).
5. Typed ask / HUD text ask remains available while awake regardless of S2S (Hands tool-loop unchanged).
6. Mic mute ([ADR 0046](ADR-0046-hud-ux-thread.md)) remains: mute stops listening paths but keeps typed ask; it is not a substitute for 2-way OFF.

### 5. Voice stickiness

1. Remember the **last selected TTS / Voice Agent voice** across the Softwake UI session.
2. On **profile switch** (HUD rail, `/profile`, Settings set-active that triggers live refresh), fall back to **Default** (empty / Eve via existing `resolve_tts_voice`).
3. Do **not** require a `voice` field on `profile.json`. Optional later enrichment is out of scope.
4. Providers pane and `/voice` remain valid ways to change the global `selected_tts_voice`; the chevron is the daily control and must stay consistent with that store (or a thin session overlay that writes through — implementing PR chooses one source of truth and documents it).

### 6. `user.md` — Use main user profile

1. Each profile gains a boolean preference (suggested wire name: `use_main_user_profile`, default **true**).
2. **Checked (default):** Settings → Profiles → User tab shows the **main** profile’s `user.md` contents **disabled** (read-only preview). Runtime pack resolve for that profile uses main’s `user.md` body when rendering instructions.
3. **Unchecked:** User tab is editable for that profile’s own `user.md`. First uncheck **seeds** the editor with **commented rules / recommendations** for a user profile (scaffold), without silently overwriting an existing non-empty custom file unless the operator confirms.
4. **Main** means the designated primary profile (default id `default` unless Softwake later adds an explicit main marker — implementing PR must define resolution and document it). A profile cannot “use main” in a way that creates a cycle (main always uses its own file).
5. Soul / rules / glossary stay per-profile (unchanged). Only `user.md` inherits under this checkbox.

### 7. Settings

1. Settings remains a **separate window** (tray + gear). Closing Settings hides it; Quit is tray-only ([ADR 0015](ADR-0015-tray-hud.md)).
2. **Gear** on main window and on shrunk HUD focuses/shows Settings (same path as tray Settings).
3. New General (or HUD) prefs: **shrunk HUD size**, **bloom intensity**. Existing expanded opacity / text size stay.
4. Profile rail on the main window stays; every profile switch still runs the live reload / `/refresh` effects already defined in [ADR 0041](ADR-0041-hud-profile-rail.md).

## Consequences

- Operators get a persistent expanded chat shell with an orb shrink, closer to Grok Bot, without merging Settings into the capsule.
- Idle-collapse and pin semantics from ADR 0015 amendments are superseded for the default path; CHANGELOG / ADR 0015 should gain a short amendment pointer when P0 lands.
- 2-way becomes the primary listening affordance; sleep / hibernate / mic mute / Voice Agent S2S must be explained relative to it so operators are not surprised.
- `user.md` inheritance reduces copy-paste across profiles but adds resolve complexity and a clear “main” definition.
- PROTOCOL generation stays **1** unless a slice proves an additive Status / ClientMessage field is required (prefer reuse of wake / sleep / hibernate / existing prefs + Ask).

## Non-goals

- Implementing UI or daemon code in the ADR PR (docs only).
- A third always-on-top chat window separate from the HUD webview.
- Merging Settings into the main window.
- Required per-profile voice fields in `profile.json`.
- Softwake Hands → Voice Agent `function` tools bridge (still future; ADR 0050 residual).
- AEC / hardware echo cancellation.
- Changing tray Quit / state icons beyond what 2-way state mapping needs.
- Reworking vault / HUD chat encryption ([ADR 0026](ADR-0026-hud-chat-unlock.md)).
- Production deploy, force-push, or service restart as part of docs work.

## Phased implementation plan

Each phase is intended to be **independently shippable** after Spencer’s explicit go. Do not start a later phase in the same PR without approval. Prefer small PRs: ADR already accepted; code PRs amend this ADR only if decisions change.

### P0 — Orb expand / shrink + default expanded + shrunk size

**Goal:** Shell presence model only. No 2-way button yet.

**Slices (approve one PR at a time if needed):**

1. **Default expanded** on UI start; keep expanded across ordinary idle. Remove or default-off `hud_idle_collapse_ms` behaviour so the shell does not auto-shrink.
2. **Orb single-click** shrinks expanded → bloom HUD; click expands back. Clarify bloom hit-target vs composer chrome so clicks on buttons do not toggle. Drop any double-click expectation (none in code today; keep it that way).
3. **Shrunk HUD size** Setting (clamped prefs in `ui-prefs.json`; replace hard-coded 120×120). Expanded resize prefs unchanged.
4. Demote or remove **pin** if idle collapse is gone (honest UX: pin without idle collapse is confusing). Document in CHANGELOG.
5. Optional thin **gear** on main (and orb if cheap) wired to existing `show_settings` — or defer gear chrome to P2 if it risks scope creep; tray Settings remains enough to reach the new size pref.

**Verify:** UI-only / prefs tests; manual expand↔shrink; no daemon feature flags required; no softwaked restart required for review builds unless prefs live-apply needs it.

**Out of P0:** 2-way button, voice chevron, user.md inheritance, bloom intensity, S2S wiring changes.

### P1 — 2-way button + voice chevron + listening path wiring

**Goal:** Daily talk controls on main + minimal HUD strip; honest awake / sleep-or-hibernate mapping; S2S when enabled.

**Slices:**

1. **Main composer:** 2-way toggle + voice chevron (full controls). Persist 2-way intent appropriately (session and/or prefs — document).
2. **Shrunk HUD:** minimal 2-way + voice strip (no full rail/log).
3. **Semantics:** 2-way ON → awake + listening; 2-way OFF → sleep or hibernate (pick and document; keep KWS wake workable if sleep is chosen). Align tray Resume / `/sleep` / `/hibernate` copy so operators are not stuck.
4. **Path selection:** if Voice Agent S2S active → 2-way is S2S (ADR 0050 bypass); else soft-duplex STT→chat→TTS.
5. **Voice stickiness:** remember last voice; on profile switch (rail / `/profile` / live set-active) reset to Default; chevron stays consistent with `selected_tts_voice` / `/voice`.

**Verify:** daemon + UI tests for state transitions; S2S on/off matrix; profile switch resets voice; no PROTOCOL bump unless additive Status is clearly needed for HUD sync.

**Out of P1:** user.md Use-main UX, bloom intensity, large Settings IA redesign.

### P2 — user.md Use-main + bloom intensity + Settings gear polish

**Goal:** Profile inheritance UX and presence polish.

**Slices:**

1. **Use main user profile** checkbox (default on) in Profiles → User; disabled preview of main `user.md`; uncheck → editable + commented scaffold seed rules; resolve path for render_instructions.
2. **Bloom intensity** Setting (multiplier on particle density/brightness; keep capture-level as base input).
3. **Gear discoverability** on main + shrunk HUD if not done in P0; ensure Settings hide-on-close and always-on-top rules still hold.
4. Docs/README touch for the new shell vocabulary; amend ADR 0015 / 0017 briefly when behaviour lands.

**Verify:** pack load tests for inheritance / cycle guard; UI prefs clamps; no regression on profile rail `/refresh`.

**Out of P2:** Hands-on-S2S, AEC, per-profile required voice fields.

### Suggested approval order

```text
P0 (shell) → Spencer go → ship
P1 (2-way + voice) → Spencer go → ship
P2 (user.md + bloom + gear polish) → Spencer go → ship
```

If codebase coupling forces a merge (e.g. voice chevron needs chrome that only exists after P0 layout), keep the **user-visible** scope of each PR inside one phase and note the dependency in the PR body.

## Open design risks (for implementers)

1. **Idle collapse vs “stay open”** — `hud_idle_collapse_ms` and pin are first-class in Settings General and `ui-prefs.json`. P0 must deliberately supersede them or operators will see conflicting knobs.
2. **2-way OFF → sleep vs hibernate** — Hibernate refuses HUD ask until Settings Resume; sleep keeps KWS. Wrong default will feel like Softwake “died.” Prefer documenting the choice in the P1 PR description before coding.
3. **S2S vs soft-duplex** — ADR 0050 already bypasses free-speech while S2S owns the mic. The 2-way toggle must not arm both paths.
4. **“Main” profile for user.md** — today every profile has its own `user.md` with no inherit flag. Defining main as `default` is simplest; operators who renamed/removed `default` need a fallback story.
5. **Voice source of truth** — global `selected_tts_voice` vs session sticky overlay vs write-through on chevron; pick one in P1 to avoid Providers / chevron / `/voice` drift.
6. **Shrunk chrome density** — minimal 2-way + voice + gear on a small orb risks hit-target collisions with bloom click-to-expand; P0/P1 should reserve an orb drag/click zone distinct from controls.
7. **After-sleep expanded** — “after sleep” can mean daemon voice sleep or UI process restart; P0 should define both so reopen behaviour is predictable.

## Alternatives considered

1. **Keep idle collapse + pin as the only “stay open” path** — rejected; Spencer locked orb click as the shrink control.
2. **Separate always-on-top main window + orb** — rejected for v1; one morphing HUD webview is enough and matches the current Tauri layout.
3. **Required per-profile TTS voice in `profile.json`** — rejected; stickiness until switch + Default is enough.
4. **Fold Settings into the main window** — rejected; Settings stays separate ([ADR 0015](ADR-0015-tray-hud.md)).
5. **2-way is only Voice Agent S2S** — rejected; S2S stays opt-in; soft-duplex remains the default talk path when S2S is off.
