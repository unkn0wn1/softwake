# Changelog

## Unreleased

- Settings → General can tune free-speech end-of-utterance silence (default 2.0 s, range 0.5–4.0 s). The key is `free_speech_end_silence_ms` in softwake.json (500–4000). `SOFTWAKE_FREE_SPEECH_END_SILENCE_MS` wins over the file. Changes apply live (`reload_utterance` / `softwaked ctl reload-utterance`) without restarting softwaked and without rebuilding the keyword spotter. Press-to-talk is unchanged.
- Natural-language sleep phrases with soft closers (`go to sleep for a little while`, `I'm going to sleep`, `sleep for a few minutes`) now arm `Sleep now?` before the chat model, so Softwake no longer invents an “okay I’ll sleep” reply while staying awake.
- Free-speech end-of-utterance silence hangover raised to ~2.0 s (200 × 10 ms capture frames) so a mid-thought pause is less likely to cut the STT utterance. Press-to-talk is unchanged.

- Natural-language sleep and hibernate while awake ask a short confirm (`Sleep now?` / `Hibernate now?`) before changing state. A clear yes applies the transition and the existing state voice. No, or 15 seconds of silence, stays awake.
- While asleep, a below-threshold wake-word near-miss (not bare `hi`) or a typed wake attempt asks `Were you trying to wake me?` at most once every 45 seconds. Yes wakes. No or silence stays asleep.
- Hard keyword hits (`hi`, the profile name, `sleep`, `deep sleep`, and the other configured phrases) stay immediate and do not ask. Fire thresholds are unchanged. The near-miss probe runs during sleep so fuzzy wake can hear those hits; it does not lower the fire threshold.
