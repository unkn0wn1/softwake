# Softwake HUD UX + UI-thread safety — approved plan

**Date:** 2026-09-28 (Asia/Bangkok)
**Tip base:** `3a5b9c3` (origin/main, #100 OAuth mirror)
**Branch / worktree:** `feat/hud-ux-thread` @ `/tmp/softwake-hud-ux-thread`
**ADR:** 0046
**Model:** grok-4.7 `--effort xhigh`

## Scope

1. Settings General **HUD transparency** slider → persist `hud_opacity` in `ui-prefs.json`; apply to expanded capsule.
2. HUD **mic mute** toggle (top-left) → text Ask stays; wake/KWS/PTT/free-speech off; visible state; daemon-enforced.
3. **Layout gap** — `#chat-toolbar` must not steal the `1fr` row from `#log` (overlap/crush when Select is on).
4. **UI threading** — `remote_agent_test` / `remote_agent_install` (and similar long Settings commands if found) → `async` + `spawn_blocking`; JS immediate “Testing…” / “Starting…”; clear stale status.
5. **Thinking stuck** — clear on turn complete / reject / Telegram outbound status / missing model; surface `No chat model is selected…` in HUD; daemon must not leave `status.message = thinking…` after reject.
6. **Input freeze** — keep ask/talk on spawn_blocking; no sync block on webview thread; talkPending serializes one turn (intentional residual).

## Non-goals

- Restart softwaked / UI; force-push; production deploy; secret exfil
- PROTOCOL_VERSION bump
- Auto-select chat model after Test
- Bloom particle redesign; Telegram sticky; OAuth mirror changes

## Implementation sketch

### A. Opacity (`ui_prefs` + Settings + HUD)

- `UiPrefs` / `UiPrefsSnapshot`: add `hud_opacity: u8` (percent 35..=100, default **55** matching current `oklch(… / 0.55)`).
- `clamp_hud_opacity`, `normalize`, `ui_prefs_set_hud_opacity`.
- Register command in `lib.rs`.
- Settings General (`index.html` + `app.js`): range input + hint; save on change like idle-collapse.
- `hud.js` / `hud.css`: on prefs load, set `--hud-bg-alpha` (or inline style) on `.capsule.expanded` background alpha. Collapsed bloom may keep fixed alpha unless trivial to share.

### B. Mic mute (IPC additive + daemon + HUD)

- `Status.mic_muted: bool` additive (`serde(default, skip_serializing_if = "is_false")`).
- `ClientMessage::SetMicMute { id, muted: bool }` additive (mirror SeedChat pattern).
- Daemon `Runtime`: `user_mic_muted: bool`; on SetMicMute persist in-memory (and mirror to `ui-prefs` **or** persist only via UI prefs + push to daemon on HUD load — prefer: **ui-prefs `hud_mic_muted` + daemon latch via SetMicMute** so restart restores from HUD prefs refresh).
- When muted: treat like input mute for KWS/free-speech/PTT (`fuzzy_listen_active`, `auto_listening_active`, TalkStart → TalkRejected “mic muted — type to ask”); do **not** block `Ask` / slash.
- Optional: also OR with `softwake_voice::input_muted()` checks rather than hijacking half-duplex TTS mute.
- HUD: top-left `#mic-mute` button (near pin / corner); `aria-pressed`; muted icon; call `hud_set_mic_mute` Tauri cmd → IPC; apply from Status on refresh.
- Persist `hud_mic_muted` in ui-prefs; on HUD boot push SetMicMute to daemon.

### C. Layout gap (`hud.css`)

- Change `.strip-main` from single `grid-template-rows: minmax(0, 1fr)` to an explicit plan so **log always owns the flex row**:
  - Prefer: `grid-template-rows: auto auto minmax(0, 1fr) auto auto auto auto auto;` wired to vault-gate, chat-toolbar, log, live, pending, allow, context-meter, ask-form — **or** simpler: wrap toolbar+log so log is the only `1fr` child.
- Add `margin-bottom` / increase `gap` under `.chat-toolbar` (≥8px) so select/delete chrome never overlaps first bubble.
- Ensure `[hidden]` children stay `display: none` (UA + existing rules).

### D. UI threading

- `remote_agent.rs`: change `remote_agent_test` / `remote_agent_install` to `pub async fn` wrapping body in `tauri::async_runtime::spawn_blocking`.
- Audit: any other Settings command that does network/SSH/long IO sync (provider Test if sync — fix same way if it blocks).
- `app.js` remote-agent handlers: set `remoteAgentStatus` to **“Testing…”** / **“Starting…”** immediately (clear prior `testStatus`); disable buttons while in flight; re-enable in `finally`; on result apply snapshot with real summary.

### E. Thinking clear + missing model

- Daemon `serve.rs` / ask+talk paths: after `Outcome` settle (ok **or** ChatRejected/TalkRejected), ensure published status cache is the final message (not leftover `thinking…`). Add `clear_thinking` or always overwrite in the response publisher.
- HUD `hud.js`:
  - On ask/talk `.then` / `.catch` / `.finally`: if message is thinking, **do not** early-return forever — either clear live after settle or keep polling until non-thinking (cap ~2s) then clear.
  - In `considerStatus` / `refresh`: if `talkPending` and snap.message is a real reply (not thinking), clear thinking and `pushAssistant` if not already keyed.
  - On ChatRejected text containing “No chat model is selected”, show as error bubble + live error.
- State voice skip stays log-only (announce.rs) — HUD surfaces the same sentence when Ask fails for missing model.

### F. Input freeze

- Verify no remaining sync path on first ask (hud_ask already spawn_blocking).
- Do not disable textarea during thinking; `talkPending` only gates duplicate submit / PTT (document as residual).
- Ensure Settings Test/Install cannot freeze the Settings window (D).

### Docs

- `docs/ADR-0046-hud-ux-thread.md` — opacity, mic mute, off-thread Settings actions, thinking clear.
- CHANGELOG Unreleased; `docs/06-milestones.md` checkbox; light Settings/README hint.
- EXECUTE report must include: **Spencer must select a chat model after Providers Test** for wake→awake state voice.

## Tests

- `ui_prefs`: opacity clamp + default 55; mic_muted round-trip if stored there.
- IPC: SetMicMute serde round-trip; Status.mic_muted default false.
- Daemon unit: muted → TalkStart rejected; Ask still works; thinking cache cleared after ChatRejected.
- Compile-check async remote_agent commands.
- No browser E2E.

## Quality gates

`cargo fmt`, `clippy -D warnings` on touched crates, targeted `cargo test -p softwake-ui -p softwake-ipc -p softwake-daemon --lib` (avoid hanging e2e/daemon integration if known flaky — prefer unit). Commit, push, PR, CI green, merge.

## Reinstall

```bash
cd /www/softwake && git pull --ff-only
cargo build -p softwake-daemon --release --features live-http,sherpa-kws,pipewire-capture
cargo build -p softwake-ui --release
# operator restarts softwaked + Softwake UI — agent must NOT
```


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


## Spencer follow-up 2 (2026-09-28): reply before TTS + richer phases

9. **Show assistant text before speech ends**
   - Root cause: `finish_ask_reply` calls `speak_if_configured` (TTS synthesize blocks) **before** `retain_status_text(reply)` / Outcome; serve holds runtime lock so GetStatus returns cached `thinking…` for the whole TTS wait.
   - Fix: publish reply into retained status **and** shared `last_status` cache **before** TTS synthesize/play; then speak. HUD must paint the bubble as soon as polls see the reply (or ask promise resolves early if we also split return — prefer cache publish so polls work while lock held).
   - Do not wait for player exit to clear thinking.

10. **Richer turn phases** (live status, not bare "thinking")
    - Phases: Listening, Thinking, Calling tools…, Speaking, Waiting for approve.
    - Prefer additive `Status.phase` (snake_case wire) + HUD live labels; keep `message` for operator/reply text once known.
    - Wire: talk_start → listening; ask start → thinking; tool invoke in chat loop → calling_tools; reply ready / TTS start → speaking (with message=reply); pending_tool → awaiting_approve; settle → phase cleared.
    - HUD `considerStatus` / live line: map phase → human label; push assistant when `message` is real reply (not a phase token).
