# Changelog

## Unreleased

### Fixed

- **Room chat freeze:** `RoomPost` writes the operator line and returns immediately; member oneshots / cool-down / `NO_REPLY` fan-out run on a background thread and append bubbles as they finish. Composer clears on Enter/Send. Room log uses speech bubbles with a stable color per participant (operator + each profile) and the name on the bubble.

- **HUD room chat:** clicking a room replaces the single chat pane (history, one composer, one Send), the same way switching profiles refreshes that pane. The profile log and Send no longer stay visible underneath, so a second composer does not stack in the top half.

- **Room fan-out:** an operator post wakes every member to decide (reply or `NO_REPLY`), sequentially with cool-down and single-flight. Member tokens resolve from profile id or display name, and the first speaker does not cancel the rest.
- **Rooms (ADR-0052):** Settings list shows persisted rooms after Save Settings list shows persisted rooms after Save (create-on-first-use / upsert); New is an action, not a fake top room; missing room reads return a clear `not found` instead of raw ENOENT; operator room chat pane posts with member fan-out (optional reply, cool-down + single-flight); `goal_run` logs plan/revise/gate/execute to room + HUD.

### Changed

- **HUD left rail:** **Profiles** stays at the top. **Rooms** sits halfway down the same rail and opens room history, compose, and member replies. Settings → Rooms remains for create/members admin.

### Added

- **ADR-0052 agent team / local power:** per-profile agent homes (`$XDG_DATA_HOME/softwake/homes/<id>/`), profile `allow_all` + `role`, confirm-gated `software_install`, rooms with light turn cool-down, peer DM `agent_message`, profile-scoped schedule `agent_task`, and first-class `goal_run` goal loop (plan/execute/verify with caps, human gate, progress log; softwake or local `grok` CLI backend). Settings → Profiles (Allow all / Role) and Settings → Rooms. PROTOCOL stays 1.

## Unreleased

### Added

- **Shell (ADR 0051):** the main window opens expanded and stays open until the orb is clicked. Idle auto-collapse and the pin control are retired (`hud_idle_collapse_ms` and `hud_pinned` are ignored). Shrunk size is `hud_shrunk_px` (96–280, default 120). Bloom intensity is `hud_bloom_intensity` (25–200%, default 100) on top of capture level. Gear on the main window and the shrunk orb opens Settings. The 2-way control turns listening on (awake) or off (**sleep**, not hibernate, so the wake word still works). When Voice Agent S2S owns the mic, 2-way does not also arm free-speech. The voice chevron writes `selected_tts_voice`; a profile switch clears it to Default. Settings → **Global** edits the main profile’s user, glossary, and rules. Each other profile has Use global user / glossary / rules (default on, `use_global_*`). Checked is a read-only global preview; unchecked edits that profile’s file and seeds a commented scaffold when the file is empty. Main (`default`, or the first profile id) always owns its own files. See [ADR 0051](docs/ADR-0051-shell-redesign.md).

- **Speech speed (xAI Providers):** Settings presets 0.5×–2× on Providers (xAI only). Wired into unary/streaming TTS (`speed`) and Voice Agent (`audio.output.speed`). Softwake clamps to the xAI API range 0.7–1.5.

### Changed

- **Voice Agent S2S toggle** moved from General onto **Providers → xAI** only (no fake S2S controls on OpenAI / OpenRouter / compatible).


### Fixed
- **HUD / chat:** intermediate tool-loop narration is no longer duplicated into dozens of identical assistant bubbles mid-ask (Soulwright PDF/text research). Live streaming revives the in-flight bubble in place, disk reload waits until the turn settles, and identical `on_delta` live text is not re-pushed.


- **Voice Agent S2S barge mashup:** on barge-in / cancel / new response, hard-kill prior `PcmPipePlayer` (SIGKILL + drain) and always `interrupt` before opening a new player so a `finish()`-orphaned ffplay/mpv cannot overlap the next sentence.

- **Voice Agent first-audio latency:** open/flush `PcmPipePlayer` on the realtime worker at the first `response.output_audio.delta` (no longer wait for HUD `GetStatus` → `pump()`, often ~900ms). ffplay gets `-fflags nobuffer` / low_delay / small probesize; stdin flushed after each PCM chunk.

- **Voice Agent S2S audio plays on FFmpeg 8:** `PcmPipePlayer` uses `-ch_layout mono` (FFmpeg 8 removed ffplay `-ac`, which caused `ffplay exited immediately (exit status: 1)` and silent replies). Legacy `-ac 1` remains a fallback. Idle `response.cancel` no longer spams HUD; Speaking phase is set only while PCM is playing.


### Added

- **Voice Agent continuous S2S (opt-in)** — Settings → General / `softwake.json` `voice_agent_s2s` / `softwaked ctl voice-agent on|off`. Streams mic PCM to xAI `wss://…/v1/realtime` and plays PCM replies with server VAD barge-in ([ADR 0050](docs/ADR-0050-voice-agent-s2s.md)). Softwake Hands stay on text ask. Default off.


### Added

- **Soft duplex barge-in** while Eve speaks: elevated mic energy cancels TTS + ask and returns to listen (not AEC; ADR-0049).
- **Streaming TTS** via xAI WebSocket (`audio.delta` → ffplay/mpv stdin) for lower time-to-first-audio; unary `POST /v1/tts` fallback.
- CancelAsk / Escape **stops voice instantly** (interrupt player + clear mute).

### Fixed


### Fixed

- Early TTS no longer cuts mid-sentence: clips queue (wait for prior playback) instead of `interrupt_playback` on every Spawn; tool-round spoken offset clamps so the final remainder is not dropped; sentence split ignores bare EOS periods and short abbreviations; Speaking no longer blocks on empty/overlap remainder synth.
- Chat asks with tools advertised (Soulwright) now **SSE-stream** into the HUD (`phase=streaming`) instead of sitting on Thinking until the full reply; early sentence-boundary TTS cuts dead air before final speak; HUD status poll speeds up to ~150ms during ask/stream.
- Softwake UI/Settings General and HUD show a **build stamp** (crate version · git sha · built-at); daemon reports `Status.build` for the same check.

### Added

- Chat **token streaming** into the HUD bubble (`phase=streaming`) on text-only / finalize rounds; **CancelAsk** / Escape / Cancel aborts in-flight ask (ADR-0048).

### Fixed

- HUD thinking/live phase always clears when ask completes, soft-finalizes, errors, rejects, or TTS settles (incl. Softwright multi-tool / last-round soft-finalize); `talkPending` cannot stick.

### Added

- Settings General **HUD transparency** slider (`hud_opacity` in ui-prefs.json) for the expanded capsule.
- HUD top-left **mic mute** toggle (text ask stays; wake/KWS/PTT/free-speech off); additive `SetMicMute` / `Status.mic_muted`.
- HUD turn **phases** on `Status.phase` (Listening / Thinking / Calling tools… / Speaking / Waiting for approve).
- ADR-0046 documenting HUD UX thread safety, phases, transparency, and mic mute.

### Fixed

- Tool loop: final round omits advertised tools and soft-finalizes instead of `tool loop reached the 6-round cap without a final reply` (Soulwright / tool thrash; ADR-0047).
- HUD strip: thinking/phase, context meter, and composer no longer mash or jump after the first message (`.strip-main` is flex; `#log` owns the grow — hidden siblings cannot steal it).
- Assistant reply is shown once: pre-TTS status poll and post-TTS ask/talk settle no longer append a duplicate bubble for the same text.
- Long Settings actions (**Test on Tailnet**, **Install companion**, Providers Test/Save) run off the UI thread with immediate status (`Testing…` / `Starting…` / `Saving…`).
- Assistant text appears as soon as the reply is ready (before TTS synthesize finishes); thinking no longer sticks when a turn rejects or Telegram lands.
- Chat/voice model select shows **None** when unset instead of the first catalog entry; Test no longer auto-picks the first model.
- Select/Delete chrome no longer overlaps the chat log (`#log` remains the sole flex grower; toolbar stays `flex-shrink: 0`).

## Unreleased

### Added
- Opt-in OAuth mirror to the companion (ADR-0045): Settings checkbox (default off), `PUT /v1/vault/oauth`, companion email/calendar/Drive when mirrored.

- Remote Agent **Test on Tailnet** (real ping/SSH/health) and **Install companion** / `softwaked ctl remote-agent install` — Tailscale-only SSH installer for softwake-node (ADR-0044).

- Remote Agent companion **`agent_task` LLM** (ADR-0043): softwake-node runs a bounded tool loop with mirrored soul pack, skills, and `tools.json` permissions; Ask → pending text (no silent Always-allow); OAuth tools refuse laptop-local; results go to outbox + Telegram timer fan-out when messengers say so. Env: `SOFTWAKE_NODE_XAI_API_KEY` / mirrored vault llm + optional `SOFTWAKE_NODE_MODEL`.


- **Telegram sticky ownership:** only one surface long-polls Telegram. Laptop softwaked owns while companion presence is `present`; when sleeping/hibernated/offline (90s grace), softwake-node may take over if the bot token is mirrored to the node vault. Per-profile messengers stay honored; TTS skipped on the node. See [ADR-0042](docs/ADR-0042-telegram-sticky-ownership.md).


- **HUD left-rail profile switch:** expanded HUD lists profiles on the left; click sets the active profile and runs `/refresh` (soul reload, clear/reseed session, MCP rediscover), then loads that profile’s HUD chat history. Collapsed bloom unchanged. See [ADR-0041](docs/ADR-0041-hud-profile-rail.md).

- **Remote Agent (slice 2):** presence heartbeats (90s grace), `run_on` dispatch with fire leases, per-profile schedule mirror + softwake-node tick, durable outbox → HUD “While you were away” (ADR-0040). Companion `agent_task` LLM shipped in ADR-0043; SSH install / OAuth mirror remain later. Telegram sticky ownership shipped in ADR-0042; HUD profile rail in ADR-0041.

## Unreleased

### Added

- **Remote Agent (slice 1):** always-on Tailscale companion pairing — ADR-0039, Settings → Remote Agent (name / MagicDNS / SSH user / roles / conflict policy stub / pairing secret), `remote-agents.json` + bag secrets, per-timer `run_on` (`local`\|`companion`\|`auto`, default local; fire still local), and a minimal `softwake-node` health + empty outbox stub. Presence, outbox→HUD, Telegram sticky owner deferred. See [ADR-0039](docs/ADR-0039-remote-agent.md) and [pairing docs](docs/remote-agent-pairing.md).

- **Authenticated webhook wake:** opt-in local `POST /v1/wake` on `127.0.0.1` (default port 8787) with Bearer / `X-Softwake-Webhook-Token` shared secret stored in the secret bag. Valid requests wake from sleep (same soul gate as `ctl wake`); optional JSON `message` runs a bounded inbound ask; hibernate returns HTTP 409 (no auto-resume). Configure with `softwaked ctl webhook` / `webhook-secret`. See [ADR-0038](docs/ADR-0038-webhook-wake.md).

- **Memory write tools:** confirm-gated `remember` (default Always allow) and `forget` (default Ask, including `forget all`) so the agent can persist and remove long-term memory facts from voice/Telegram/HUD. First successful `remember` creates `memory.json` under the Softwake state directory; ask/chat budgeted recall then sees new facts. No Honcho / vector DB; store stays the simple disk file ([ADR 0009](docs/ADR-0009-long-term-memory.md)). See [ADR-0037](docs/ADR-0037-memory-write-tools.md).

- **Agent-task cron:** schedule rows may set `action: agent_task` so a fire runs a bounded agent turn (prompt in `message`, tools under live Ask/Deny) and delivers the summary via HUD + Telegram timer fan-out / TTS. Fixed `notify` schedules unchanged. Create via Settings → Timers (Action = Agent task) or `schedule create agent_task daily 07:30 …`. See [ADR-0036](docs/ADR-0036-agent-task-cron.md).


- **Drive read scope widen:** Connect replaces Google `drive.file` with `drive.readonly` and Microsoft `Files.ReadWrite.AppFolder` with `Files.Read`. Graph `drive_list` / `drive_search` use `/me/drive/root/children` and `/me/drive/root/search` (not App Folder). Existing `drive_get` unchanged. Token refresh does not enlarge a grant: Settings → Email → Accounts → Remove that account → Connect again unless stored `scope` already contains `drive.readonly` / `Files.Read`. No Drive create/upload in this slice. See [ADR-0035](docs/ADR-0035-drive-read-scope.md).

- **Calendar write:** confirm-gated `calendar_create`, `calendar_update`, and `calendar_delete` (Ask by default) on the primary calendar through the connected Email account. Google Calendar `events` and Microsoft Graph `/me/events`. Optional `account` uses the same routing as inbox and `email_send`. Connect keeps Google `calendar.readonly` and adds `calendar.events` (not the full calendar ACL scope). Microsoft `Calendars.Read` is replaced by `Calendars.ReadWrite`. Token refresh does not enlarge a grant: Settings → Email → Accounts → Remove that account → Connect again, unless stored `scope` already contains `calendar.events` or `Calendars.ReadWrite`. See [ADR-0034](docs/ADR-0034-calendar-write.md).
- **Multiple Email accounts:** Settings → Email → Accounts lists every Google and Microsoft mailbox. Add account runs Connect again without replacing the others. Remove drops one row. Set active marks the account tools use for that provider. Inbox, calendar, Drive, and confirmed `email_send` take an optional `account` (connection id or email substring). With no `account`, one connected account is that account; if several are connected and any Google account is usable, tools use the active Google account. See [ADR-0033](docs/ADR-0033-multi-account-oauth.md).
- **OAuth email send:** confirmed `email_send` delivers via the Gmail API (`users.messages.send`) and Microsoft Graph (`/me/sendMail`) when Email OAuth is connected and the daemon is built with `live-http`. Google is preferred when both are connected. Without a usable account, the mock outbox or a local draft remains. SMTP send stays unwired. Reconnect only if the stored scope lacks `gmail.send` / `Mail.Send`. See [ADR-0023](docs/ADR-0023-email-oauth.md) and [ADR-0030](docs/ADR-0030-inbox-calendar-drive-tools.md).
- **Reasoning effort:** `/reasoning` / `/reasoning list` and `/reasoning <mode>` (`low`|`medium`|`high`|`xhigh`, or `default` to omit). Softwake ctl `softwake_list_reasoning` (Always allow) + `softwake_set_reasoning` (Ask). Stored in `providers.json` as `reasoning_effort`; forwarded on chat completions when set. See [ADR-0032](docs/ADR-0032-reasoning-effort.md).
- **MCP Settings** (Messengers-like pane): add stdio/url servers, enable/disable, group Always/Ask/Deny, auth secret in the bag (`mcp_secrets`). Discovered tools advertise as `mcp_<server>_<tool>` when not Deny. Rediscover on serve start and `/refresh`. See [ADR-0031](docs/ADR-0031-mcp-settings-ctl-tools.md).
- **Softwake ctl tools** (agent-callable slash mirrors): `softwake_status`, `softwake_list_models`, `softwake_list_voices`, `softwake_list_profiles` (Always allow); `softwake_set_model`, `softwake_set_voice`, `softwake_set_profile`, `softwake_sleep`, `softwake_hibernate`, `softwake_resume`, `softwake_new_session`, `softwake_refresh` (Ask). OpenAI tools + live appendix.
- **`/refresh`** and `softwake_refresh`: reload active profile soul pack, **clear the model session**, reseed from plaintext HUD history, rediscover MCP. Pair with `/profile` / `softwake_set_profile` when switching agents. Does not restart softwaked.


### Added

- Inbox / calendar / Drive read tools over Email OAuth (Google preferred, else Microsoft Graph): `email_list`, `email_search`, `email_get`, `calendar_list`, `calendar_get`, `drive_list`, `drive_search`, `drive_get`. Defaults: list Always allow; search/get Ask. Live HTTPS under daemon `live-http`. Scope-honest: Google `drive.readonly` / Microsoft `Files.Read` for user Drive / OneDrive (see ADR-0035; earlier builds used drive.file / AppFolder). See [ADR-0030](docs/ADR-0030-inbox-calendar-drive-tools.md).
- Skill read tools: `skill_list` (Always allow) and `skill_get` (Ask) so agents can see existing skills, not only `skill_save`. Advertised via OpenAI tools + live permissions appendix when not Deny.

- Settings left-nav **expandable sublists** for Timers and Skills (same chip pattern as Profiles). New **Messengers** pane with Telegram as the first channel. Per-profile `messengers.json` holds Default / Receive all / Voice flags for desktop HUD and Telegram; bot token lives in the secret bag. softwaked long-polls Telegram (`live-http`), shares `hud-chat.json` history with the HUD, fans timer fires to Receive-all + Default, and sends TTS audio when Voice is on. Dual-login TTS rules in [ADR-0029](docs/ADR-0029-messengers-telegram.md). PROTOCOL stays 1.

- Expanded awake slash commands: `/help`, `/status`, `/model` (`ai`/`voice`), `/voice`, `/new`, `/profile`, `/sleep`, `/hibernate`, `/resume` (plus clear-typed aliases where natural). Keeps `/clear` `/halve|/reduce` `/compact`. See [ADR 0028](docs/ADR-0028-slash-hud-self-sleep.md).
- HUD multi-select + Delete for chat bubbles; rewrites `hud-chat.json`; best-effort `DropChatTurns` trims matching model-session messages (`/clear` still drops all).
- Free-speech ambient self-sleep latch: three near-identical short free-speech replies within 90s auto-sleep (no TTS on the Nth). NL self-sleep markers (`going to sleep`, `mic off`, …) also sleep after one free-speech reply.


### Fixed

- Inbox `email_list` HTTP 403 while calendar/Drive work is **not** a Softwake scope-drop bug when Connect already granted `gmail.readonly`/`gmail.send`: the MeetRec-shared Google Cloud project must **Enable Gmail API** (project `577210165352`). Softwake persists granted `scope` in the secret bag and sends `Authorization: Bearer` to `gmail.googleapis.com`. No Sally `/refresh` needed. Docs/Settings copy + clearer live 403 hints. See [oauth-clients.md](docs/oauth-clients.md).

- HUD pin control: click listener and resize grip were nested inside `refreshHudPrefs`'s catch (only registered when prefs load failed), so pin did nothing when prefs succeeded. Listener is top-level again; pressed/on state uses `aria-pressed`, filled pin icon, stronger highlight, and an updated tooltip.
- Agents (including Telegram inbound oneshot asks) now get the same live Tools permissions appendix and OpenAI `tools` advertise list as desktop ask: non-deny tools (`email_send`, `skill_save`, `schedule`, …) are in the system prompt and tools array when Always allow or Ask; Deny is omitted from advertise. Appendix states Email OAuth connected/not-connected clearly so agents do not claim they lack email tools after OAuth connect.

- Seed-on-wake for **encrypted** HUD vaults: UI `SeedChat` now runs once when awake + unlocked + turns loaded (not only on the woke edge), and the daemon prepends budgeted history even if an early post-wake ask already landed.

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


- Settings → **Email** can Connect / Disconnect **Google** and **Microsoft** (PKCE + loopback). Tokens live in the existing secret bag / OS keyring. Scopes cover mail, calendar, and drive. Status shows the connected account. SMTP opt-in remains for non-OAuth setups. Publisher client ids come from `SOFTWAKE_GOOGLE_CLIENT_ID` / optional `SOFTWAKE_GOOGLE_CLIENT_SECRET` / `SOFTWAKE_MICROSOFT_CLIENT_ID` (not Settings). See [ADR 0023](docs/ADR-0023-email-oauth.md).
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
