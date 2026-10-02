# xAI speech speed + S2S on Providers — execute report

## Prior (#110)
FFmpeg 8 ffplay `-ac` → exit 1 fixed on main `cbf923f`; binaries installed; **no restart**.

## This change
1. **Speech speed** on Providers (xAI only): presets 0.5×–2× → `providers.json` `selected_tts_speed_milli`. Wired into unary TTS `speed`, streaming TTS query `speed`, Voice Agent `audio.output.speed`. Softwake clamps to xAI API **0.7–1.5**.
2. **Voice Agent S2S** checkbox moved off General onto Providers xAI panel only. OpenAI / OpenRouter / compatible never show S2S or speed (no fake toggles).
3. `tts_available` gates the whole xAI voice block (voice, speed, S2S panel).

## Ops
`cargo install --force` daemon + UI. No restart in this PR — Spencer restarts when ready to hear S2S + speed.
