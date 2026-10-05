# ADR 0053 - Room queued-reply re-check

- **Status:** Accepted
- **Date:** 2026-10-05
- **Related:** [ADR 0052](ADR-0052-agent-team-local-power.md)

## Context

Room members answer one operator post on parallel threads. Each prompt is a snapshot of peer lines already in the log. A finished reply is appended immediately, mirrored into profile chats, and enqueued for one TTS player. Barge-in or a new operator post clears the speech queue and bumps a generation id. Lines already in the log stay there.

When several replies finish together, later ones wait while an earlier line plays. Those waiting authors have not heard the lines that will play ahead of them.

ADR 0052 described the room log as append-only and members as not speaking in parallel. The code since then already runs member turns together. This ADR is the contract for the re-check and for rewriting one queued say.

## Decision

1. A global `room_requeue_recheck` flag in `softwake.json` defaults **on** when the key is missing. Settings → Rooms has the checkbox. Off keeps today's finish-time log and playback with no rewrite.
2. Only a reply that is still waiting (not playing) is eligible. The clip that is already playing is left as spoken.
3. The check starts when that clip becomes the oldest waiting line, which is when the previous clip has been claimed and is playing. The model call overlaps that playback. The first clip in a round has nothing ahead of it and skips the call.
4. The check is one short prompt: the author's draft plus peer lines logged ahead of it, labeled by name, using the text those lines have after their own check. The model returns exactly one of `KEEP`, `NO_REPLY`, `PRIVATE Name: message`, or a replacement line.
5. At most one check per reply per fan-out. No revision loop.
6. If the check has not returned when it is that reply's turn, wait at most `ROOM_REQUEUE_RECHECK_WAIT` (1500 ms) from the start, then play the draft.
7. The draft is still logged at finish time so the room is not blank while the queue plays. Before TTS:
   - `KEEP` or timeout or a model error plays the draft.
   - A revision replaces that one `say` row (same timestamp and profile) and the matching profile-chat turns. The log, the room UI, and the mirrors show one final line.
   - `NO_REPLY` deletes that row and those turns, and nothing plays.
   - `PRIVATE Name: ...` deletes the public draft, then uses the existing private-note path.
8. Interrupt still clears the queue, including a check that has not finished, and bumps the generation. A late result is discarded and does not rewrite the log. A line already playing is not revised. `clear` still does not delete rows that were already logged.

## Consequences

- A member `say` can be replaced or removed before it plays. The log is not append-only for that row.
- Profile chats that received the draft are updated to the final text, or the draft turn is removed.
- The open text session is ingested at the end of the fan-out, after the check, so it sees the final line when the check finished in time.
- Free-speech end silence, the speech-to-speech coalesce, and per-profile speech speed are unchanged. Room playback still uses each profile's `tts_voice` and does not write Settings `selected_tts_voice`.

## Non-goals

No deploy step, no second re-check, no re-check of a line that is already playing, and no re-check of a private note that was never queued.
