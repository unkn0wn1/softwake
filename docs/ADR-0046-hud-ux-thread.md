# ADR-0046: HUD UX thread safety, phases, transparency, mic mute

**Status:** Accepted  
**Date:** 2026-09-28

## Context

Settings actions such as Test on Tailnet / Install companion blocked the Tauri UI thread (“Softwake UI is not responding”). The HUD stayed on bare `thinking…` until TTS synthesize finished because `finish_ask_reply` spoke before publishing the reply, and `GetStatus` under a held runtime lock only saw the thinking cache. Chat model select fell back to the first catalog entry when unset, looking selected while wake still logged `No chat model is selected`. Operators also wanted expanded HUD opacity, a mic mute that keeps text ask, and clearer turn phases.

## Decision

1. **Off-thread long work** — `remote_agent_test` / `remote_agent_install` / `provider_test` / `provider_set_key` run via `spawn_blocking`. Settings JS shows immediate `Testing…` / `Starting…` / `Saving…`.
2. **Reply before TTS** — Runtime shares serve’s `last_status` cache. `finish_ask_reply` publishes the assistant text and `phase=speaking` **before** `speak_if_configured`. Polls clear thinking and show the bubble while speech prepares.
3. **Turn phases** — Additive `Status.phase`: `listening`, `thinking`, `calling_tools`, `speaking`, `awaiting_approve`. HUD maps these to live labels. Reject paths clear a stuck thinking cache.
4. **Phase must clear on every turn terminal** — `ask` publishes `thinking` up front; every exit (reply / soft-finalize / TTS settle / ChatRejected / intent short-circuit / confirm / cancel / sleep ReleaseActingResources) clears `turn_phase` (or sets the intentional next phase). Retained `thinking…` placeholders are dropped on clear so `GetStatus` cannot resurrect Thinking after Err. HUD `refresh` passes `phase` into `considerStatus`; ask/talk settle via `endTurnUi` so `talkPending` / live Thinking cannot stick after Softwright multi-tool or last-round soft-finalize.
5. **Transparency** — `ui-prefs.json` `hud_opacity` (35–100, default 55) applied as `--hud-bg-alpha` on the expanded capsule.
6. **Mic mute** — Additive `ClientMessage::SetMicMute` + `Status.mic_muted`; HUD top-left toggle; KWS/PTT/free-speech off; typed ask remains. Preference also in `ui-prefs.json`.
7. **Model None** — Select shows `None` when `selected_model` is empty; never auto-pick `models[0]` on load. Providers Test no longer writes the first catalog id when unset. Empty select clears the saved model.
8. **Strip layout** — Expanded `.strip-main` is a **column flex** with `#log { flex: 1 1 0 }`. Fixed N-track CSS grid mis-assigned the grow row when vault/toolbar/live/meter were `[hidden]` (`display: none` drops tracks), so thinking, context meter, and composer mashed. Live / context / composer stay `flex-shrink: 0` bands in HTML order.
9. **Single assistant bubble** — HUD dedupes consecutive identical assistant text across the pre-TTS status poll and the post-TTS ask/talk settle (no short time gate). Detail/note may refresh on the same bubble; a later user turn that happens to get the same words still appends a new bubble. Mid-ask streaming also revives the in-flight assistant bubble when `streaming` was cleared (disk reload / soft-finalize), so identical tool narration is not appended on each status poll (see ADR-0048).

## Consequences

- PROTOCOL_VERSION stays 1 (additive fields only).
- Operator must still pick a chat model after Test for wake→awake state voice.
- `talkPending` still serializes one in-flight ask/talk turn (intentional).
