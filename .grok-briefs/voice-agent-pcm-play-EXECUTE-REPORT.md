# Voice Agent PCM play fix — execute report

## Root cause
FFmpeg **8.0.1** on spencenb removed ffplay `-ac`. Softwake `PcmPipePlayer` passed `-ac 1` → `Failed to set value '1' for option 'ac': Option not found` → exit 1 within the 40ms alive check → HUD `ffplay exited immediately (exit status: 1)`, text/transcript only, model could claim "text mode".

Secondary: every `pump()` note forced `phase=speaking` (including `listening…` and cancel errors). `SpeechStarted` always sent `response.cancel`, producing `Cancellation failed: no active response found` spam that also aborted players on Error.

## Fix
1. Prefer `-ch_layout mono`; legacy `-ac 1` fallback; continue on immediate exit.
2. Cancel only when `response_active` / local player; suppress idle-cancel errors.
3. `VaHudPhase`: Speaking only while PCM writes succeed; Listening on barge / audio done / transcript done.

## Ops
`cargo install --force` softwaked (+ UI if desired). No restart in this change — Spencer restarts when ready.
