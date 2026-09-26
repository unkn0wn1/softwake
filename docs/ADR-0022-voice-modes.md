# ADR 0022 — Voice phrases, state voice, and voice test mode

- **Status:** Accepted
- **Date:** 2026-09-26
- **Relates to:** [ADR 0006](ADR-0006-on-device-wake.md), [ADR 0007](ADR-0007-awake-stt-tts.md), [ADR 0021](ADR-0021-multi-turn-compact.md)

## Decision

Keyword lists gain three phrases and do not drop the existing ones:

- bare `hi` → wake
- bare `sleep` → sleep
- `deep sleep` → hibernate

`deep sleep` enters hibernate from sleep or from awake and stops capture. It does not leave hibernate. `ctl resume` and UI Resume still land in sleep. `ctl wake` and UI Wake still enter awake from sleep when the soul pack is valid.

sherpa-onnx keyword spotting has no grammar. Softwake does not pretend otherwise. Short words are a best effort:

1. Single words of at most eight characters keep the existing per-keyword threshold `#0.15`. `hi` is not lowered further.
2. About 400 ms of silence resets the online stream, in addition to the existing three-second sample budget.
3. While awake, a short single-word hit is ignored once free speech or press-to-talk has already buffered about 1.5 seconds. Multi-word phrases (`deep sleep`, `hey <name>`, `go to sleep`) are not dropped by that gate.

A state change speaks one short line. The line is a one-shot completion: the loaded soul as system, and one fixed instruction as the only user turn, then the existing TTS path. That turn is not appended to the awake session. If the completion cannot run, TTS speaks a fixed fallback (`I'm awake.`, `Listening for wake.`, `Deep sleep.`) when a voice is configured.

Voice test mode defaults off. `softwaked serve --voice-test`, `softwaked ctl voice-test on|off`, and Settings → General set it on the running daemon. It does not persist. Phrases and the state line still run. Microphone speech is not sent to the chat model. Typed `ctl ask` still works.

Verbose (`-v` / `-vv`) KWS lines include the loaded profile name and, when it differs, the profile directory id. Ask lines log tokens about to be sent and the estimate from before compaction (`char/4`, ADR 0021). A compact line logs before, after, and the real compact percent.

## Non-goals

- A sentence parser or grammar.
- Leaving hibernate by voice.
- A protocol generation bump. `set_voice_test` and `Status.voice_test` are additive on generation 1.
- Persisting voice test mode in Settings JSON.

## Consequences

Bare `hi` can false-wake from ordinary speech while asleep. The 1.5 second gate does not run in sleep. Silence reset only helps after a pause.

If the spotter emits the short tag `sleep` for a longer phrase, longest-match cannot recover the longer phrase. A short command spoken after 1.5 seconds of continuous awake speech is ignored on purpose.

A profile whose name is exactly `sleep` or `hi` ties with that bare word and loses the tie-break (hibernate, then sleep, then wake). `hey <name>` still wakes.

Announcement speech can lag the HUD by one completion plus TTS. Rapid transitions are serialized on a playback lock and can speak a line after the state has already moved on.
