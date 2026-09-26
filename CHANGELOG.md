# Changelog

## Unreleased

### Added

- Seed-on-wake: after sleep → awake, Softwake replays a budgeted suffix of per-profile HUD chat into the new model session (plaintext `hud-chat.json` in softwaked; encrypted vaults via UI `SeedChat`). `/clear` / `/halve` / `/compact` still clear or shrink the model session only.

- HUD polish: larger default expanded panel (520×620), resizable corner grip with persisted size, pin-to-stay-open (skips idle collapse), multiline ask box with larger type, and chat-log bottom padding so the last bubble clears the thinking line. Prefs: `hud_pinned`, `hud_expanded_w`, `hud_expanded_h` in `ui-prefs.json`.
- Email OAuth publisher clients: `softwake-ui` loads `~/.config/softwake/oauth-clients.env` at startup (and accepts MeetRec `MEETREC_*` aliases) so Connect works without exporting shell env every launch.

- Expanded HUD **context fullness meter** (used / limit / % + auto-compact threshold) while awake. Slash commands `/clear`, `/halve` (or `/reduce`), `/compact` (and clear-typed `clear context` / `halve context` / `compact context`) manage the model session without a chat completion. Default auto-compact threshold is **80%** (was 70); Settings overrides unchanged. Auto-compact still runs before ask when over threshold. HUD display history is seeded into the model on wake (budgeted; see ADR 0026 amendment). See [ADR 0027](docs/ADR-0027-context-meter-slash.md).

- Per-profile HUD chat history (`profiles/<id>/hud-chat.json`) with optional softwake-ui passphrase unlock (Argon2id + ChaCha20-Poly1305) and OS keyring wrap (`softwake` / `ui-data-key`). See [ADR 0026](docs/ADR-0026-hud-chat-unlock.md).
- Chat API tool-calling: non-deny Tools Settings tools are advertised on
  `/chat/completions`; the daemon runs a capped tool loop through Hands so
  natural asks can execute shell (and other tools) without saying `run …`
  ([ADR-0025](docs/ADR-0025-api-tool-calling.md)).


- Each ask/chat turn attaches a live **Tools permissions** appendix (from `tools.json`) so the model sees current always_allow / ask / deny modes. When shell is always_allow or ask, the appendix says shell is available; when deny, unavailable. The model is told not to claim a tool is denied against that list, and to propose shell as a `run …` / `shell …` command for the operator rather than inventing output. Static Runtime policy stub defers to this appendix for availability.

- Settings left-nav pane switching works again. Email OAuth (#69) used Google/Microsoft Connect controls in `app.js` without declaring their DOM refs, so a top-level `ReferenceError` aborted the script before nav click handlers registered (tabs highlighted on focus but stayed on Status).
- Per-profile Softwake schedules (`schedules.json`): once / daily / cron subset, confirm-gated `schedule` tool (Ask default), Settings → Timers, daemon tick with notify + fixed TTS fire (any voice state while softwaked runs). See [ADR-0024](docs/ADR-0024-timers-cron.md).


- Settings → **Email** can Connect / Disconnect **Google** and **Microsoft** (PKCE + loopback). Tokens live in the existing secret bag / OS keyring. Scopes cover mail, calendar, and drive. Status shows the connected account. Live send/list stay draft-gated; SMTP opt-in remains. Publisher client ids come from `SOFTWAKE_GOOGLE_CLIENT_ID` / optional `SOFTWAKE_GOOGLE_CLIENT_SECRET` / `SOFTWAKE_MICROSOFT_CLIENT_ID` (not Settings). See [ADR 0023](docs/ADR-0023-email-oauth.md).
- Settings → **Skills** lists user-added and agent-learned Markdown skills (`procedure` / `pitfalls` / `verify`) under the XDG data skills directory. Add, edit, and delete in the pane. Confirm-gated `skill_save` (Tools permission Ask by default) writes an agent skill after Approve; ask phrases like “make a skill …” / “make a skill from this” stage it. Soul rules still beat skills. Tools permission UX unchanged. Webhook wake and Email OAuth are not in this change.
- Settings → Tools lists every registered tool as Always allow, Ask, or Deny (`tools.json` version 2). Defaults match the previous floors: echo always allow, notify and email_send ask, shell deny. Shell confirm policy (`always`, `mutating_only`, `allowlisted_quiet`) applies when shell is Ask. Always allow skips the prompt; shell still expands glossary aliases. A pending tool expands the HUD and keeps it open until Approve or Deny. After Approve, the HUD can offer “Always allow this tool”; the Tools page is what persists. Soul policy overrides can still only tighten. Skills and email sign-in are unchanged.
- Settings → General can set the max spoken reply / TTS playback reaper (default 60 s, range 30–300 s). The key is `tts_playback_timeout_ms` in softwake.json (30000–300000). `SOFTWAKE_TTS_PLAYBACK_TIMEOUT_MS` wins over the file. The next speak uses the new deadline without restarting softwaked (`reload_playback` / `softwaked ctl reload-playback` confirms the running daemon). An in-flight clip keeps the deadline it started with. The written reply is not cut. Chat HTTP stays 120 s. Free-speech silence, keyword spotting, HUD idle collapse, and the half-duplex mute grace are unchanged.
- Live chat, speech-to-text, and speech-synthesis HTTP calls now wait up to 120 seconds (`CHAT_TIMEOUT`) instead of 30. A slow provider read was aborting the completion, so long replies never finished. The HUD and Settings status show the full reply text. Spoken audio uses the playback reaper (default 60 s, Settings → General, 30–300 s).
- The HUD collapses to a square bloom (no composer, chat, or hint). Click expands a panel of chat bubbles labeled You or the active profile name, each with a timestamp, plus Send and a mic icon (aria-label “Hold to talk”). It collapses again after the pointer has been outside for N seconds. A non-empty ask draft keeps it open. Settings → General sets N from 1 to 30 seconds (default 3). The key is `hud_idle_collapse_ms` in ui-prefs.json (1000–30000). The HUD re-reads that file; softwaked does not need a restart. Press-to-talk still ignores keys typed in the ask field. Free-speech silence, keyword spotting, and natural-language confirms are unchanged.

- Settings → General can tune free-speech end-of-utterance silence (default 2.0 s, range 0.5–4.0 s). The key is `free_speech_end_silence_ms` in softwake.json (500–4000). `SOFTWAKE_FREE_SPEECH_END_SILENCE_MS` wins over the file. Changes apply live (`reload_utterance` / `softwaked ctl reload-utterance`) without restarting softwaked and without rebuilding the keyword spotter. Press-to-talk is unchanged.
- Natural-language sleep phrases with soft closers (`go to sleep for a little while`, `I'm going to sleep`, `sleep for a few minutes`) now arm `Sleep now?` before the chat model, so Softwake no longer invents an “okay I’ll sleep” reply while staying awake.
- Free-speech end-of-utterance silence hangover raised to ~2.0 s (200 × 10 ms capture frames) so a mid-thought pause is less likely to cut the STT utterance. Press-to-talk is unchanged.

- Natural-language sleep and hibernate while awake ask a short confirm (`Sleep now?` / `Hibernate now?`) before changing state. A clear yes applies the transition and the existing state voice. No, or 15 seconds of silence, stays awake.
- While asleep, a below-threshold wake-word near-miss (not bare `hi`) or a typed wake attempt asks `Were you trying to wake me?` at most once every 45 seconds. Yes wakes. No or silence stays asleep.
- Hard keyword hits (`hi`, the profile name, `sleep`, `deep sleep`, and the other configured phrases) stay immediate and do not ask. Fire thresholds are unchanged. The near-miss probe runs during sleep so fuzzy wake can hear those hits; it does not lower the fire threshold.
