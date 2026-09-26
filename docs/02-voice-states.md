# Voice states

Three states. Names are fixed vocabulary for UI, logs, and docs.

| State | Mic / capture | Wake-word engine | Model acts / tools |
|-------|---------------|------------------|--------------------|
| **sleep** | On | On (listening for wake) | **Off** — ignore speech for actions |
| **awake** | On | On (listening for sleep phrase) | **On** — session + tools per policy |
| **hibernate** | **Off** | Off | Off — only UI Resume or `ctl resume` can leave hibernate (lands in sleep) |

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
- Free-speech end-of-utterance silence defaults to 2.0 s (range 0.5–4.0 s). Settings → General writes `free_speech_end_silence_ms` (500–4000) in softwake.json. `SOFTWAKE_FREE_SPEECH_END_SILENCE_MS` wins over the file. `softwaked ctl reload-utterance` applies the hangover live. Press-to-talk and wake-word spotting keep their own timing.
- The soul pack last read at startup or by `reload_soul` is applied as system instructions for this session. A reload during awake waits for the next awake entry and does not replace the instructions already stored on the open session.
- Tools may run only while awake and only when the registry allows them. `echo` is safe and runs immediately. `notify` and `email_send` wait for `confirm_tool`. `shell` is denied and never runs ([ADR 0004](ADR-0004-first-safe-tool.md), [ADR 0005](ADR-0005-tool-confirmation.md), [ADR 0008](ADR-0008-connector-boundary.md)). Sleep and hibernate also drop a pending confirmation.
- Sleep phrase (or explicit UI control) returns to **sleep**: close the acting session; keep capture + wake engine.

## Hibernate

- Entered from the UI, from `ctl hibernate`, or from the voice phrase `deep sleep` (from sleep or from awake).
- **Stops all listening.** Release mic devices; stop wake engine; close any acting session. Hibernate from awake releases the session before capture stops.
- Cannot be woken by voice. `hi`, `sleep`, and `deep sleep` do nothing while hibernating. Leaving hibernate is UI Resume or `ctl resume`, which lands in **sleep**, not awake.

## Invariants (test these)

1. In **sleep**, tool dispatcher receives zero invocations.
2. In **hibernate**, audio capture callback is not running (no frames).
3. Transition **awake → sleep** releases model/tool resources within a bounded time.
4. Transition **hibernate → sleep** does not auto-enter awake.
5. Cooldown after wake and after sleep to prevent phrase loops.

## Configuration (conceptual)

```toml
[voice]
wake_phrases = ["<profile name>", "hey <profile name>", "hey softwake", "softwake", "hi"]
sleep_phrases = ["go to sleep", "goodnight <profile name>", "<profile name> sleep", "goodnight softwake", "softwake sleep", "sleep"]
hibernate_phrases = ["deep sleep"]
post_wake_cooldown_ms = 800
post_sleep_cooldown_ms = 800
```

`hi` and `sleep` are bare words added after the longer phrases. sherpa-onnx has no grammar. A short single word of at most eight characters uses `#0.10` so it can fire (global default `0.15`). Softwake does not ignore that hit during a long awake utterance, and a pause does not reset the keyword stream. The stream soft-refreshes after about five seconds of quiet audio (hard refresh at ten seconds). While asleep, bare `hi` can false-wake. If that is too noisy or too weak on a microphone, prefer `hey <name>` or the product phrases. Bare `sleep` should match again during awake chat; `go to sleep` and `<name> sleep` remain the more reliable multi-word options.

Entering awake, sleep, or hibernate speaks one short line through the profile voice. The line is a one-shot prompt, not a turn stored on the awake session. See [ADR 0022](ADR-0022-voice-modes.md).

## Confirm before natural language

Hard keyword hits stay immediate. A clean `hi`, profile name, `sleep`, `go to sleep`, or `deep sleep` from the spotter changes state with no question.

Text that arrives on the ask path (typed chat, free speech, or press-to-talk) and clearly means sleep or hibernate — including soft closers like “go to sleep for a little while” or “I’m going to sleep” — does not change state until the profile voice asks `Sleep now?` or `Hibernate now?`. The wait is 15 seconds from the question. Yes applies the transition and then the normal state line. No speaks `Okay, staying awake.` Timeout stays awake and says nothing. An unclear reply drops the question and is a normal ask.

While asleep, a probe near-miss of a wake phrase other than bare `hi` asks `Were you trying to wake me?` at most once every 45 seconds. Typed text that looks like a wake attempt asks the same question. Yes, or a wake phrase in the reply, wakes. No speaks `Okay, staying asleep.` Timeout and unclear speech stay asleep and do not ask again until the cooldown ends. Ordinary chat while asleep is still refused. Hibernate still ignores voice.

The probe stream is enabled during sleep. Its threshold stays below the fire threshold. Fire thresholds are not changed.

Voice test mode is off unless `softwaked serve --voice-test`, `softwaked ctl voice-test on`, or Settings → General turns it on. Phrases and the state line still run. Microphone speech is not sent to the chat model. The flag is not saved across a serve restart.

Half-duplex mute during TTS playback is unchanged.

Exact TOML shape lives with the implementation; this doc owns the semantics.
