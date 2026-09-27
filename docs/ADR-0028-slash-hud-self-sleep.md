# ADR-0028 — Operator slash commands, HUD message delete, free-speech self-sleep

- **Status:** Accepted
- **Date:** 2026-09-27
- **Relates to:** [ADR 0027](ADR-0027-context-meter-slash.md) (extends slash surface), [ADR 0026](ADR-0026-hud-chat-unlock.md) (HUD display history), [ADR 0022](ADR-0022-voice-modes.md) (sleep / hibernate)

## Decision

### 1. Expanded awake slash / clear-typed commands

While awake, trimmed ask text that matches an operator command is handled **without** a chat completion (same intercept pattern as ADR 0027). Existing context commands stay:

| Input | Effect |
|-------|--------|
| `/clear`, `clear context` | Drop session turns; keep session + system pack |
| `/halve`, `/reduce`, `halve context`, `reduce context` | Keep newest ~half by character mass |
| `/compact`, `compact context` | Force Hermes compaction |

New commands:

| Input | Effect |
|-------|--------|
| `/help`, `help` | List slash commands (no TTS) |
| `/status`, `status` | Short awake status line (state, profile, model, voice, context meter) |
| `/model` | List chat models + current selection |
| `/model ai <id>` | Set `selected_model` in `providers.json` (live for next ask) |
| `/model voice <id>` | Set `selected_voice_model` (STT) |
| `/voice`, `/voice list` | List TTS voices + current |
| `/voice <id>` | Set `selected_tts_voice` |
| `/new`, `new session`, `fresh session` | Re-read soul pack, re-apply Identity, open a **fresh** session (seed HUD history as on wake). Voice state stays awake. |
| `/profile` | List profiles (id + name; mark active) |
| `/profile <name-or-id>` | Set active profile, retarget soul dir (unless `--soul-dir` / `SOFTWAKE_SOUL_DIR`), rebuild KWS, fresh session if awake |
| `/sleep` | Immediate sleep (same as KWS / ctl sleep — **no** NL confirm) |
| `/hibernate` | Immediate hibernate |
| `/resume` | Hibernate → sleep only (`WakeFromUi`); rejected otherwise |

Matching is case-insensitive on the trimmed line. A leading `/` is optional for the clear-typed aliases listed above. Unknown `/…` still falls through to the model (except we may reply with a short “unknown command; try /help” for bare `/foo` with no args — see implementation: unknown slash with a leading `/` gets a help hint; bare words stay chat).

PROTOCOL generation stays **1**. No new `ClientMessage` for slash (still intercept `Ask`).

### 2. HUD multi-select delete

The expanded HUD can select one or more chat bubbles and delete them. Softwake-ui updates the in-memory ring and rewrites `hud-chat.json` (plaintext or vault). Best-effort: if the model session is open, softwake-ui sends additive `DropChatTurns` with `{role, text}` pairs; softwaked removes matching messages once each (exact text). Failures are quiet — `/clear` still drops the whole model session. PROTOCOL stays 1 (additive variant).

### 3. Free-speech ambient self-sleep latch

TV / ambient noise can STT into short free-speech asks; the model may reply with the same short dismissal (“Still the telly…”) forever. Softwake adds a **safety latch**:

1. Only replies that finish a **free-speech** (auto-utterance) ask count — not typed HUD asks or press-to-talk.
2. A reply is **short** when ≤ 120 Unicode scalars after trim.
3. **Near-identical** means equal after normalize (lowercase, collapse whitespace, strip most punctuation) or Levenshtein distance ≤ 2 on the normalized forms when both lengths ≤ 120.
4. After **N = 3** near-identical short free-speech replies inside a **90 s** window, softwaked **does not TTS** that Nth reply, clears the latch, and transitions to **sleep** (existing sleep announcement / state voice). Capture stays up (sleep, not hibernate). Half-duplex mute is not required; sleep already stops free-speech buffering.
5. **NL path:** a free-speech assistant reply whose normalized form is only a self-sleep marker (`going to sleep`, `i am going to sleep`, `mic off`, `sleeping now`, and close variants) also enters sleep after a single occurrence (speaks the reply once, then sleep). Typed `/sleep` remains the explicit operator path.

Constants are code-owned (not Settings) for this slice. Latch state clears on sleep / hibernate / `/new` / successful wake.

## Context

Operators need slash control of model, voice, profile, and session without opening Settings; HUD clutter from ambient loops needs delete; Sally looping “Still the telly. Not you. / Mic off.” on free speech must self-terminate.

## Alternatives

- New IPC per slash command. Rejected: Ask intercept matches ADR 0027.
- Model-only “please sleep” without a latch. Rejected: TV noise will not reliably call a tool.
- Hibernate on latch. Rejected: operator still wants voice wake; sleep is enough.
- Mute capture for minutes. Deferred; sleep stops free-speech asks.

## Consequences

- Softwaked writes `providers.json` and `softwake.json` (active profile) from slash paths.
- `/profile` retargets the loaded soul directory when not locked by `--soul-dir` / `SOFTWAKE_SOUL_DIR`.
- `/new` while awake applies the reloaded pack immediately (not only on next wake).
- HUD gains select / delete chrome; `DropChatTurns` is best-effort session hygiene.
