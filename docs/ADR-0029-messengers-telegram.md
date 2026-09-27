# ADR-0029 — Expandable Settings nav + Messengers (Telegram)

- **Status:** Accepted
- **Date:** 2026-09-27

## Context

Settings already nests **Profiles** as left-nav chips that open a detail pane
([ADR-0017](ADR-0017-profiles.md)). Timers and Skills still use an in-pane
`<select>`, which does not scale and diverges from Profiles. Operators also want
outbound/inbound chat on messenger channels (first: Telegram) that share the
same per-profile HUD display history
([ADR-0026](ADR-0026-hud-chat-unlock.md)), with optional TTS on those channels,
without pasting bot tokens into git.

## Decision

### 1. Expandable left-nav sublists (universal pattern)

Any Settings item that needs a submenu uses the Profiles pattern:

1. Top-level nav button toggles a `.nav-sub` chip list under it.
2. Clicking a chip selects that row and shows its detail in the content pane.
3. Optional `+ New` / actions stay under the chip list.

**Timers**, **Skills**, and **Messengers** adopt this. Future panes that need a
submenu reuse the same CSS/JS helpers — no one-off layouts.

### 2. Messengers pane + per-profile channel file

Store non-secret channel bindings per profile:

```text
~/.config/softwake/profiles/<id>/messengers.json
```

Shape (version 1):

```json
{
  "version": 1,
  "desktop": { "default": true, "receive_all": true, "voice": true },
  "telegram": {
    "enabled": false,
    "default": false,
    "receive_all": false,
    "voice": false,
    "chat_id": null
  }
}
```

- **Default** — primary channel for that profile.
- **Receive all** — gets timer fires and agent pushes marked for fan-out.
- **Voice** — outbound to that channel includes TTS (see dual-login below).
- `desktop` is the local HUD surface (always “present” when softwake-ui is up).
- `telegram.chat_id` is bound on first inbound message to that bot for the
  profile (or set in Settings). One bot token is shared; chat ids are per profile.

### 3. Bot token in the secret bag

`telegram_bot_token` lives in the existing provider secret bag / OS keyring
(same path as SMTP password). Settings → Messengers → Telegram shows
has-token / clear / save. Never log or commit the token. PROTOCOL stays **1**;
the UI reads/writes bag + `messengers.json` in-process like Timers/Email.

### 4. Shared HUD chat history

Telegram inbound and Softwake outbound on Telegram append the same
`profiles/<id>/hud-chat.json` turns the HUD uses (`user` / `assistant`, cap 40).

- Softwaked appends when the file is **plaintext** (or missing).
- When the UI vault is **encrypted**, softwaked writes pending turns to
  `profiles/<id>/hud-chat-inbox.json`; softwake-ui merges them on the next
  unlock/load/save (daemon never sees the passphrase).
- Desktop login therefore sees Telegram traffic in the HUD log. Seed-on-wake
  ([ADR-0026](ADR-0026-hud-chat-unlock.md) amendment) continues to read plaintext
  only.

### 5. Telegram transport

Prefer **Bot API long-poll** (`getUpdates` with timeout) inside softwaked while
`serve` runs — suitable for a desktop daemon with no public webhook URL.
Requires the daemon `live-http` feature (same HTTPS stack as chat/TTS). Without
`live-http`, the poller is a no-op and Settings still edits config.

Inbound text → append user turn → run the normal ask path for that profile
(without requiring the voice state to be awake; voice state is unchanged) →
append assistant turn → reply on Telegram (text always; voice file when Voice).

### 6. Fan-out rules

| Event | Who receives |
|-------|----------------|
| Timer / schedule fire | Every channel with **Receive all**, plus the **Default** channel |
| Agent reply to a **Telegram** inbound | Telegram (always) + hud-chat; desktop Voice does not auto-speak unless desktop has Receive all / Default and the dual-login TTS rule says so |
| Agent reply to a **HUD / typed** ask | Channels with **Receive all** only (not every Default). Desktop HUD already shows the reply via Status; no duplicate desktop TTS beyond the normal `speak_if_configured` path |

### 7. Dual-login TTS rule (desktop + Telegram both active)

When both the desktop HUD and Telegram are configured for the profile:

1. **Text:** Telegram always gets the outbound text when it is a fan-out target.
   HUD history always gets the turns (shared file).
2. **Desktop speech:** At most **one** local TTS play per reply — the existing
   HUD/timer announce path. Do **not** stack a second desktop speak for the
   same event because Telegram also fired.
3. **Telegram voice:** If Telegram **Voice** is checked, softwaked synthesizes
   TTS (xAI mp3 when configured) and sends it as Telegram **audio** alongside
   the text. Independent of whether desktop already spoke.
4. If desktop **Voice** is off, skip local speak; Telegram Voice may still send
   audio.
5. Timer fires keep today’s notify + fixed-line desktop speak when desktop is
   Default or Receive-all with Voice; Telegram text (+ audio if Voice) follows
   the fan-out table.

### 8. Out of scope

- Pushing every desktop-only typed ask to Telegram unless **Receive all** is on.
- Webhook mode, multi-bot, non-Telegram channels (chips stay extensible).
- PROTOCOL bump (no new IPC messages).
- Restarting Spencer’s running softwaked/UI from this change.

## Consequences

- Reinstall daemon + UI with `live-http` (and existing wake features) so long-poll
  and Telegram HTTPS work; Spencer restarts softwaked himself.
- Operators create a Telegram bot via BotFather, paste the token in Messengers,
  message the bot once to bind `chat_id`, then set Default / Receive all / Voice.
- Encrypted vaults rely on inbox merge; plaintext vaults get direct appends.
