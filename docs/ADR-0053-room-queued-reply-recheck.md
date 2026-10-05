# ADR 0053 - Room queued-reply re-check

- **Status:** Accepted
- **Date:** 2026-10-05
- **Related:** [ADR 0052](ADR-0052-agent-team-local-power.md)

## Context

Room members answer one operator post on parallel threads. Each prompt is a snapshot of peer lines already in the log. A finished reply is enqueued for one TTS player. The room log and profile-chat mirrors receive that line when it is about to play. Barge-in or a new operator post clears the speech queue and bumps a generation id. A line already appended stays in the log.

When several replies finish together, later ones wait while an earlier line plays. Those waiting authors have not heard the lines that will play ahead of them.

ADR 0052 described the room log as append-only and members as not speaking in parallel. The code since then already runs member turns together. This ADR is the contract for the re-check. The amendment records that the final line is appended once, at play time.

## Decision

1. A global `room_requeue_recheck` flag in `softwake.json` defaults **on** when the key is missing. Settings → Rooms has the checkbox. Off skips the model call and still appends the line when it plays, so playback speaks the draft. See the amendment.
2. Only a reply that is still waiting (not playing) is eligible. The clip that is already playing is left as spoken.
3. The check starts when that clip becomes the oldest waiting line, which is when the previous clip has been claimed and is playing. The model call overlaps that playback. The first clip in a round has nothing ahead of it and skips the call.
4. The check is one short prompt: the author's draft plus peer lines logged ahead of it, labeled by name, using the text those lines have after their own check. The model returns exactly one of `KEEP`, `NO_REPLY`, `PRIVATE Name: message`, or a replacement line.
5. At most one check per reply per fan-out. No revision loop.
6. If the check has not returned when it is that reply's turn, wait at most `ROOM_REQUEUE_RECHECK_WAIT` (1500 ms) from the start, then play the draft.
7. The public `say` and its profile-chat mirrors are appended when the clip is about to play, after this check, or immediately when the check is skipped. The draft is not logged at finish. Before TTS:
   - `KEEP`, timeout, or a model error appends the draft, then plays it.
   - A revision appends the new line only.
   - `NO_REPLY` appends nothing and plays nothing.
   - `PRIVATE Name: ...` appends no public say. The existing private-note path runs at this decision, then nothing is spoken.
8. Interrupt still clears the queue, including a check that has not finished, and bumps the generation. A late result is discarded and does not write the log. A line already playing is not revised. `clear` does not delete a row already appended at play time. A clip that was still waiting has no row.

## Consequences

- A member `say` appears when that line is about to be spoken. That row is appended once.
- Profile chats receive the final text once.
- The open text session is ingested at the end of the fan-out. The delta is lines appended at play time, plus private notes.
- Free-speech end silence, the speech-to-speech coalesce, and per-profile speech speed are unchanged. Room playback still uses each profile's `tts_voice` and does not write Settings `selected_tts_voice`.

## Amendment (2026-10-05): bubble at speak time

The first accepted text logged the draft when the member finished, then replaced or deleted that row before TTS. The room showed every finished reply at once. A bubble could change or disappear before that member spoke.

That timing is reversed:

- A public reply that goes through the speech queue is not appended at finish. The room log, profile-chat mirrors, and `job.history` get the line once, when that clip is about to play and the re-check has returned.
- `KEEP`, the 1500 ms timeout, and a model error append the draft, then TTS starts.
- A revision appends the new line only. There is no second row and no in-place replace.
- `NO_REPLY` appends nothing and plays nothing.
- `PRIVATE Name: ...` uses the existing private-note path at decision time and is not spoken. An initial private note, which never queues audio, is still published when the member reply returns.
- `room_requeue_recheck` off skips the model call and still waits until play time to append. Off speaks the draft. The bubble lines up with speech in both modes.
- Interrupt drops clips that were not appended. A generation bump still discards a late check. A line already appended at play time stays in the log.
- Open text-session ingest remains the post-fan-out history delta. That delta contains lines appended at play time, and private notes.
- Unchanged: one check per reply, skip when nothing is ahead, the 1500 ms budget, free-speech end silence, the speech-to-speech coalesce, per-profile speech speed, the single TTS player, and no write of Settings `selected_tts_voice` during a room line.

## Non-goals

No deploy step, no second re-check, no re-check of a line that is already playing, and no re-check of a private note that was never queued.
