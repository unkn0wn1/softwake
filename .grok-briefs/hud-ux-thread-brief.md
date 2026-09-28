# Softwake job: HUD UX + UI-thread safety (transparency, mic mute, layout, thinking)

**Branch:** `feat/hud-ux-thread`  
**Worktree:** `/tmp/softwake-hud-ux-thread` (from `origin/main` @ `3a5b9c3` — includes #100 OAuth mirror / ADR-0045)  
**Repo:** unkn0wn1/softwake  
**Model:** grok-4.7, `--effort xhigh`  
**Daemon features when building:** `live-http,sherpa-kws,pipewire-capture` (local verify only; do **not** restart softwaked/UI)

Also copy this brief to `.grok-briefs/hud-ux-thread-brief.md` in the worktree.

## Goal (this PR ONLY — ship real behavior)

One coherent PR fixing Spencer’s HUD/Settings UX batch:

1. **HUD transparency slider** (Settings General) — persist opacity for the **expanded** HUD; document.
2. **HUD mic mute toggle (top-left)** — mute mic listening while keeping text input; wake/KWS/PTT off when muted; visible state.
3. **Layout gap** — space between multi-select/delete chrome and chat start (they overlap today).
4. **UI threading / “Softwake UI is not responding”** — Test on Tailnet, Install companion, and other long Settings/HUD actions MUST run off the UI thread; show immediate status (“Testing…” / “Starting…”); clear stale last message; never block egui/GTK/Tauri main loop.
5. **Thinking stuck** — HUD shows thinking with no progression; Telegram reply can arrive while HUD still thinking. Clear thinking when turn completes / Telegram outbound lands / error. Wake log `state voice skipped: No chat model is selected` — ensure thinking doesn’t hang forever when model missing; **surface that error in HUD**.
6. **Input freeze on first thinking** — don’t lock input/buttons on UI thread during model call (investigate; fix if a sync path remains).

Also note for Spencer in report: he needs a **chat model selected in Settings after Test** for wake→awake state voice.

## Context (do not re-litigate)

- `ui-prefs.json` already persists text size, HUD idle collapse, pin, expanded size (`crates/softwake-ui/src/ui_prefs.rs`, ADR-0020 / HUD polish).
- Half-duplex TTS mute exists (`softwake_voice::input_muted`) — that is **not** the user mic mute. User mute is a separate latch: text Ask stays live; KWS wake/sleep, free-speech, and PTT must not fire while muted.
- `hud_ask` / `hud_talk_stop` already use `tauri::async_runtime::spawn_blocking` — keep that pattern. **`remote_agent_test` / `remote_agent_install` are sync `#[tauri::command]`** and block the UI during SSH/network (root cause of “not responding”).
- Additive Status / ClientMessage fields on PROTOCOL_VERSION 1 are the Softwake pattern (see `auto_listening`, `SeedChat`). Prefer additive; **do not bump PROTOCOL_VERSION** unless unavoidable.
- Thinking is published via `publish_thinking` in `serve.rs` before long lock holds; HUD `setLive("thinking…")` + `lastStatusMessage`; form submit / talk stop can leave thinking if status still says thinking or if a parallel Telegram turn completes without clearing the local live line.
- `No chat model is selected. Choose one in Settings after Test.` comes from `softwake_providers` / `load_disk_chat` — state voice logs skip; Ask should ChatRejected with that sentence and HUD must show it (not hang on thinking).

## IN (must ship)

### 1. Transparency
- Settings General: slider (or range) for **expanded HUD opacity** (sensible clamp, e.g. 0.35–1.0; default ≈ current ~0.55).
- Persist in `ui-prefs.json` (e.g. `hud_opacity` as percent 35–100 or f32).
- HUD applies opacity to expanded capsule background (not collapsed bloom chromium unless trivial); reload on prefs refresh.
- Document in ADR + Settings hint + CHANGELOG.

### 2. Mic mute (top-left)
- Toggle control top-left on HUD (expanded at minimum; collapsed if it fits without breaking 120×120 bloom — prefer expanded + visible muted chrome on bloom if easy).
- When muted: no KWS wake/sleep scoring consumption that would transition; no free-speech auto listen; PTT `TalkStart` refused with clear sentence; **typed Ask / slash still work**.
- Persist muted preference (ui-prefs and/or daemon Additive `mic_muted` on Status + `SetMicMute` ClientMessage — pick the smallest Softwake-shaped design; daemon must enforce voice path off).
- Visible pressed/muted state (aria-pressed, icon slash, live hint “mic muted”).

### 3. Layout gap
- Fix `.strip-main` grid so `#chat-toolbar` does not steal the `1fr` row from `#log` when Select is on (today `grid-template-rows: minmax(0,1fr)` + auto-rows → first visible child gets 1fr → overlap/crush).
- Explicit row plan: toolbar `auto`, log `minmax(0,1fr)`, rest auto; add a few px gap/margin so select/delete chrome never overlaps first bubble.

### 4. UI threading
- Convert long-running Tauri commands to `async` + `spawn_blocking` (at least `remote_agent_test`, `remote_agent_install`; audit similar sync SSH/network/provider Test if they block the same way).
- Settings JS: on click, **immediately** set status to “Testing…” / “Starting…” (clear stale `testStatus`), disable double-submit, then await invoke; on settle, show real summary.
- Never `std::thread::sleep` or blocking SSH on the command thread that serves the webview.

### 5. Thinking race
- Clear HUD thinking when: ask/talk promise settles (success **or** error); status poll shows a non-thinking message / new assistant-visible reply; Telegram/outbound completion updates status; ChatRejected (incl. no model).
- If daemon returns `message: thinking…` after ask completes, treat as incomplete: keep polling briefly then clear — do **not** leave thinking forever.
- When model missing: surface `No chat model is selected…` in HUD live/error bubble; do not leave bloom/capsule in thinking class.
- Daemon: ensure reject paths clear published thinking cache (status.message not left as thinking… after ChatRejected / TalkRejected / missing model).

### 6. Input freeze
- Confirm ask/talk stay on spawn_blocking; ensure Settings long actions match.
- Do not disable `#ask-input` / Send for the whole model call unless already required for talkPending race; if talkPending is the only gate, keep it but never block the rAF bloom / main loop.
- Document residual: talkPending still serializes one in-flight voice/ask turn (intentional).

### Docs / quality
- Short **ADR-0046** (next free after 0045) + CHANGELOG + milestones honesty + light README/Settings hint.
- Tests where natural: `ui_prefs` opacity clamp; remote_agent command is async (compile); thinking-clear / mic-mute unit if daemon helper is pure; layout is CSS (no browser E2E harness).

## OUT (reject scope creep)

- Force-push, production deploy, restart softwaked/UI, secret exfil
- OAuth / Telegram sticky / companion LLM / PROTOCOL_VERSION bump
- Redesign bloom particles or Settings left-nav
- Changing default chat model automatically (operator must pick after Test — call out in report)

## Real paths (do NOT invent crates)

- `crates/softwake-ui/src/ui_prefs.rs`, `ui/index.html`, `ui/app.js`, `ui/hud.html`, `ui/hud.js`, `ui/hud.css`, `src/remote_agent.rs`, `src/commands.rs`, `src/lib.rs`
- `crates/softwake-daemon/src/serve.rs` (`publish_thinking`), `runtime.rs` (ask/talk reject, auto_listening, KWS mute), `announce.rs` (state voice skip)
- `crates/softwake-voice` (half-duplex mute — extend carefully or add parallel user-mute flag)
- `crates/softwake-ipc/src/types.rs` (additive Status / ClientMessage only)
- Docs: `docs/ADR-0046-*.md`, CHANGELOG, `docs/06-milestones.md`

## UX locks

| | Choice |
|---|---|
| Opacity | Expanded HUD; Settings General slider; persist ui-prefs |
| Mic mute | Top-left toggle; text input stays; wake/KWS/PTT off |
| Layout | Toolbar auto row; log keeps 1fr; visible gap |
| Long actions | spawn_blocking + immediate “Testing…” / “Starting…” |
| Thinking | Always clear on turn end / reject / Telegram land |
| Model missing | Surface error in HUD; no infinite thinking |

## Plan-then-execute

1. Plan mode only → write `.grok-briefs/hud-ux-thread-APPROVED-PLAN.md` (+ copy `/tmp/softwake-hud-ux-thread-APPROVED-PLAN.md`)
2. Execute against that plan on `feat/hud-ux-thread` in this worktree
3. `cargo fmt` / clippy / targeted tests; commit; push; PR; wait CI green; `gh pr merge --merge`
4. Write `.grok-briefs/hud-ux-thread-EXECUTE-REPORT.md` with PR#, merge SHA, what fixed, reinstall, residual risks, **model-selection note for Spencer**

## Reinstall (document in PR / report)

```bash
cd /www/softwake && git pull --ff-only
cargo build -p softwake-daemon --release --features live-http,sherpa-kws,pipewire-capture
cargo build -p softwake-ui --release
# operator restarts softwaked + Softwake UI themselves — agent must NOT restart
```

## Operator note (must appear in report)

After Providers **Test**, Spencer must **select a chat model** in Settings. Without it, wake→awake state voice logs `state voice skipped: No chat model is selected` and will not speak.


## Spencer follow-up (2026-09-28, same PR)

7. **Chat model select: unset vs first option**
   - Bug: `renderProviders` sets `modelSelect.value = … : models[0]` when `selected_model` empty/missing → UI shows grok-4.2 (or first listed) while disk stays unset → wake logs `No chat model is selected`.
   - Fix: always prepend a blank/`None` option (`value=""` label `None`). If `selected_model` empty or not in list, select the blank option — **never** fall back to `models[0]` on load.
   - Same pattern for voice model if it has the same `voiceModels[0]` fallback.
   - Do **not** silently persist first model on snapshot/load. Operator must choose (change event already calls `provider_set_model`).
   - UI load path must distinguish unset even if Test historically suggested a first model.

8. **Settings Save status**
   - Same as Test/Install: immediate **Saving…** status, clear stale line, success/error when done; long save work off UI thread (`spawn_blocking` if sync today).
   - At minimum Providers Save key / Save base URL / Save context; Remote Agent Save if it blocks.
