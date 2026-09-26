# Voice states

Three states. Names are fixed vocabulary for UI, logs, and docs.

| State | Mic / capture | Wake-word engine | Model acts / tools |
|-------|---------------|------------------|--------------------|
| **sleep** | On | On (listening for wake) | **Off** — ignore speech for actions |
| **awake** | On | On (listening for sleep phrase) | **On** — session + tools per policy |
| **hibernate** | **Off** | Off | Off — only UI (or equivalent local command) can leave hibernate |

## Sleep (default while “on”)

- Softwake is **present but passive**.
- Everything heard is scored only for the configured **wake phrase(s)**.
- No tool calls, no assistant speech, no “helpfulness” on ambient audio.
- Cloud VAD / realtime models must not be allowed to start acting while asleep. If a realtime session exists in sleep, it is a design smell for phase 1 — prefer local wake gate before opening a costly acting session.

## Awake

- Entered when local wake engine accepts a wake phrase (and optional confidence / cooldown rules) **and** the loaded soul pack is valid. `softwaked ctl wake` is the serve entry for that same wake phrase, including the soul-pack refusal. `ctl resume` still lands in sleep.
- A missing or invalid `soul.md`, `user.md`, `rules.md`, or `glossary.md` refuses the transition, including an unparseable glossary. The machine stays in sleep (capture still running). Hibernate, sleep, and UI resume are not blocked.
- Acting session starts. The text session stores the rendered context pack and, on typed `ask` / `chat`, sends it to the selected provider ([ADR 0013](ADR-0013-session-provider.md)). Sleep and hibernate still close the session and do not call the model.
- Streaming STT/TTS may act ([ADR 0007](ADR-0007-awake-stt-tts.md)). The default path is mock inject (`hear`) and mock record (`say`). Prefer silence for TTS outside awake. While TTS plays, mic input is gated (half-duplex) so speaker playback cannot loop into free speech / KWS.
- The soul pack last read at startup or by `reload_soul` is applied as system instructions for this session. A reload during awake waits for the next awake entry and does not replace the instructions already stored on the open session.
- Tools may run only while awake and only when the registry allows them. `echo` is safe and runs immediately. `notify` and `email_send` wait for `confirm_tool`. `shell` is denied and never runs ([ADR 0004](ADR-0004-first-safe-tool.md), [ADR 0005](ADR-0005-tool-confirmation.md), [ADR 0008](ADR-0008-connector-boundary.md)). Sleep and hibernate also drop a pending confirmation.
- Sleep phrase (or explicit UI control) returns to **sleep**: close the acting session; keep capture + wake engine.

## Hibernate

- Entered from UI (or a future explicit local hotkey that does not need the mic).
- **Stops all listening.** Release mic devices; stop wake engine; close any acting session. Hibernate from awake releases the session before capture stops.
- Cannot be woken by voice. Leaving hibernate is a conscious UI (or CLI) action → typically land in **sleep**, not directly awake (safer default).

## Invariants (test these)

1. In **sleep**, tool dispatcher receives zero invocations.
2. In **hibernate**, audio capture callback is not running (no frames).
3. Transition **awake → sleep** releases model/tool resources within a bounded time.
4. Transition **hibernate → sleep** does not auto-enter awake.
5. Cooldown after wake and after sleep to prevent phrase loops.

## Configuration (conceptual)

```toml
[voice]
wake_phrases = ["<profile name>", "hey <profile name>", "hey softwake", "softwake"]
sleep_phrases = ["go to sleep", "goodnight <profile name>", "<profile name> sleep", "goodnight softwake", "softwake sleep"]
# hibernate has no phrase by default — UI only
post_wake_cooldown_ms = 800
post_sleep_cooldown_ms = 800
```

Exact TOML shape lives with the implementation; this doc owns the semantics.
