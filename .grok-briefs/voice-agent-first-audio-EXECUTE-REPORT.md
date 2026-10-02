# Voice Agent first-audio latency — execute report

## Root cause
Assistant PCM was opened/written only in `VoiceAgentBridge::pump()`, which runs from `drain_pcm` on client `GetStatus`. HUD adaptive poll is **~900ms** when not text-streaming, so `output_audio.delta` chunks sat in the mpsc until the next poll. ffplay also lacked low-latency flags and stdin was not flushed — audible start lagged a couple of status/stream entries after the first transcript paint.

## Fix
1. Voice Agent **worker** opens `PcmPipePlayer`, writes, and flushes on the first `response.output_audio.delta` (HUD `pump` only updates transcript/phase).
2. `write_chunk` flushes stdin after every PCM/MP3 write.
3. ffplay: `-fflags nobuffer -flags low_delay -probesize 32 -analyzeduration 0 -infbuf`; mpv: `--cache=no --audio-buffer=0`.
4. Cancel / SpeechStarted / done / stop abort or finish the in-thread player.

## Ops
`cargo install --force` softwaked (+ UI if desired). **No restart** in this change — Spencer restarts when ready.
