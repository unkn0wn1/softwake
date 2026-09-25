# ADR 0009 — Long-term memory

- **Status:** Accepted
- **Date:** 2026-09-25
- **Amended:** 2026-09-25 (opt-in `FileMemory` writes `memory.json`)
- **Amended:** 2026-09-25 (budgeted recall on awake ask/chat)

## Decision

Long-term memory lives in `softwake-memory`, separate from `softwake-soul`.

[`Memory`](../crates/softwake-memory/src/memory.rs) is the trait: `remember`, `recall`, and `forget`. [`MockMemory`](../crates/softwake-memory/src/mock.rs) is the default backend. Snippets live on that value. The mock does not open a socket, read credentials, or create a directory.

A new `MockMemory` is disabled. `remember`, `recall`, and `forget` return "memory is disabled" until that value is opted in. `MockMemory::enabled` constructs an opted-in empty value whose first successful `remember` returns id 1. `enable` opts an existing value back in. It does not restore snippets and does not reset the id counter. `disable` drops whatever that value held and leaves the counter where it is, so the next id keeps counting upward.

`MemoryId` is a `u64` newtype. Ids start at 1 and are not reused after `forget` or `disable`. `MemoryId::from_raw` rebuilds an id from that number. It does not check that this value issued it.

`Snippet` is an id plus text. There are no embeddings, tags, timestamps, or peer ids.

`remember` stores text unchanged, including surrounding spaces and a duplicate of text already stored. A duplicate gets a new id. While the value is enabled, checks run in this order: empty or whitespace-only text is rejected, then text longer than 8 KiB (8192 UTF-8 bytes, [`MAX_TEXT_BYTES`](../crates/softwake-memory/src/memory.rs)) is rejected. While the value is disabled, those checks do not run. The call returns disabled, so an off value does not describe the text it refused.

`recall` is a case-sensitive substring over stored text. Hits come back oldest first. A query that matches nothing returns an empty list. An empty query also returns an empty list, including when the store holds snippets. There is no regular expression and no case fold.

`forget` of a live id removes that snippet and leaves the others in order. `forget` of an id this value never issued, or already removed, returns missing. While disabled, `forget` returns disabled instead, including for an unknown id.

`MockMemory::records` returns the stored snippets, oldest first. It is empty while the value is disabled.

The trait's error is associated, matching [`EmailConnector`](../crates/softwake-connectors/src/email.rs). `MockMemory` uses `MemoryError`: disabled, empty, too long, and missing. [`FileMemory`](../crates/softwake-memory/src/file.rs) uses `FileMemoryError`. Those four sentences match `MemoryError`. A missing state directory, an empty path, a filesystem failure, a file that is not JSON, a `version` other than 1, a document that breaks a store rule, a file over 1 MiB, and an id counter that cannot advance are their own variants. Opt-in methods sit on the backend value, not on the trait.

[`FileMemory`](../crates/softwake-memory/src/file.rs) is the durable backend. `FileMemory::new` is disabled and does no I/O. `open_enabled` reads `memory.json` when the file exists and does not create it. A missing file is an empty store whose first successful `remember` returns id 1. `enable` loads the file. Calling `enable` on a value that is already on does not reread. `disable` drops that handle's cache and does not open, truncate, or remove the file. A later `enable` or `open_enabled` loads the snippets again. `forget` is what removes a snippet, and that removal is what persists. The id counter on disk does not go backwards.

The file is `memory.json` under `$XDG_STATE_HOME/softwake` when `XDG_STATE_HOME` is set and non-blank, and under `~/.local/state/softwake` otherwise. A blank or whitespace-only variable is unset. When both are unset, resolving the path is an error. The helper does not expand `~` and does not create the directory. The directory is created on the first successful write, mode `0700` for directories that write creates. The file is mode `0600`. An existing directory is not chmodded.

The document is pretty-printed JSON:

- `version` is `1`. Any other version is refused and left on disk.
- `next_id` is the last id successfully stored, or `0` when no id has been issued.
- `snippets` is an array of `{id, text}` in recall order.

Unknown fields, a snippet id of `0`, a duplicate id, a `next_id` below a snippet id, and text that would be rejected by `remember` are refused and left on disk. An empty snippet list with `next_id` greater than `0` is valid: every snippet was forgotten. The next remember returns `next_id + 1`.

A write builds the next document, writes a sibling temp file, `sync_all`s it, and renames it over `memory.json`. The cache updates only after that rename. A failed write does not consume an id and leaves the previous file in place. A file longer than 1 MiB ([`MAX_FILE_BYTES`](../crates/softwake-memory/src/file.rs)) is refused before it is parsed, and a replacement that would exceed that cap is refused before the rename.

One handle does not share its cache with another. A second handle sees a write only after it is opened again. Two processes can overwrite each other. This backend does not take a file lock.

`softwake-soul` renders instructions only. Closing a session must not be how memory is forgotten. `softwake-soul` does not depend on `softwake-memory`. [`TextStubSession`](../crates/softwake-session/src/lib.rs) does not depend on this crate either: it accepts a pre-rendered memory appendix string and appends it after the pack when non-empty ([`assemble_system`](../crates/softwake-session/src/lib.rs)).

`softwake-daemon` depends on this crate for budgeted recall on awake `ask` / `chat` ([ADR 0013](ADR-0013-session-provider.md)). On each ask it resolves `memory.json`, and **only when that file already exists** it opens `FileMemory::open_enabled`. Missing file, resolve failure, open failure, disabled memory, and recall errors are an empty appendix (fail-open). The ask does not fail. Caps: [`MAX_RECALL_SNIPPETS`](../crates/softwake-memory/src/recall.rs) (4) and [`MAX_RECALL_BYTES`](../crates/softwake-memory/src/recall.rs) (2048 UTF-8 bytes of snippet text). Query is the user line. Empty query yields no hits. Assemble order: rendered pack → budgeted memory section → user turn. The memory lead-in states that snippets do not override rules. The runtime policy stub stays last among pack sections.

The default build has no Honcho client, no HTTP client, and no memory feature flag. CI does not set a memory key and does not start a database or Redis. `MockMemory` stays the backend a caller gets when it does not construct `FileMemory`. Tests inject `MockMemory` or a temp-dir `FileMemory`; they do not open the operator home file.

IPC protocol generation stays `1`. Memory is not a status field and not a socket command.

## Context

Phase 3 can name a memory boundary now that connectors exist. [The soul pack](03-soul-pack.md) already says memory is a separate module and soul parsing stays instruction-only. [ADR 0006](ADR-0006-on-device-wake.md), [ADR 0007](ADR-0007-awake-stt-tts.md), and [ADR 0008](ADR-0008-connector-boundary.md) already pick a mock default so CI has no device, no weights, and no cloud keys. Memory follows that rule.

Snippets are operator data. The default value stores nothing until that value is enabled. `MockMemory` never leaves the process. `FileMemory` leaves the process only after a caller opens it enabled and a remember or forget writes.

Honcho is a later optional backend at most, behind this same trait, in its own ADR, and off in CI. The official server is AGPL-3.0. The official clients are Python and TypeScript. There is no first-party Rust client. A deployment is the managed API, which needs a key, or Postgres, Redis, a deriver, and model keys. The third-party `honcho-ai` crate is not a dependency: it still needs a Honcho server, tokio, and Rust 1.88, above this workspace's `rust-version` of 1.85. Any of those would make default CI keyed or networked.

The 8 KiB cap is the snippet-sized form of the soul pack's limit on a huge paste. A soul file may be 1 MiB. A snippet is text a session would attach beside those instructions. The file cap is that same 1 MiB, so opening `memory.json` cannot decode an unbounded paste.

## Alternatives

- Honcho as the default, or as a crate dependency, or as a feature CI compiles. Rejected. The runtime weight, the keys, the network, the MSRV, and the AGPL server do not fit the workspace. A later optional backend can implement `Memory` behind a feature that CI leaves off.
- Depend on `honcho-ai`. Rejected. It is a third-party client of the Honcho server, not a first-party Rust SDK, and it still needs the server.
- Cloud-only retrieval as the memory implementation. Rejected. The default build has no network and no key.
- Stuff recall into `softwake-soul` parsing. Rejected. The soul pack is instructions. Memory is retrieved text the session layer attaches.
- Durable sqlite or a file under the XDG state directory in the original decision. Rejected for that change. That was the productized store, and the mock kept the trait honest before a format existed. The 2026-09-25 amendment is that store: one JSON file, not sqlite, off until `open_enabled`.
- sqlite for the durable store. Rejected. Substring recall is a scan of a short list. A bundled database compiles C into CI for transactions this file does not need.
- A JSONL log. Rejected. `forget` and the monotonic id still need a rewrite or tombstones plus a header.
- Delete `memory.json` on `disable`. Rejected. That would make turning a handle off destroy operator data. `forget` removes a snippet.
- Wire `MockMemory` or `FileMemory` into `TextStubSession` as a trait dependency. Rejected. Session close drops one awake period. Memory must outlive that period. The session takes an appendix string; the daemon opens `FileMemory` when `memory.json` exists. No IPC change. Protocol generation stays 1.
- An async trait. Rejected. Library crates stay runtime-agnostic. The daemon is where the runtime is chosen.
- Embeddings, peer ids, or a dialectic query. Rejected. The record is an id and text until a backend needs more.

## How to demo

```bash
cargo test -p softwake-memory
cargo test -p softwake-daemon --lib memory
```

Enable the file store (creates `memory.json` on the first successful remember). `resolve_memory_file` does not create the directory; the first successful `remember` does:

```rust
let path = softwake_memory::resolve_memory_file()?;
let mut memory = softwake_memory::FileMemory::open_enabled(&path)?;
memory.remember("the garage code is on the hook")?;
```

`FileMemory::new` stays off and writes nothing. There is no CLI flag and no config key. Once `memory.json` exists, awake typed `ask` / `chat` attach budgeted substring hits for the user line.

## Consequences

- Callers can keep snippets across processes by holding an enabled `FileMemory`. Callers who use `MockMemory` still lose those snippets when the value is dropped or disabled.
- Callers still cannot reach Honcho from this build.
- `disable` on the file backend does not forget. `forget` does, and the next id keeps counting.
- The daemon loads `FileMemory` for ask/chat only when `memory.json` already exists. Session close still does not forget it.
- Clients that speak protocol generation 1 see no new message kinds.
- The decision line, the productized durable store line, and budgeted ask/chat recall are closed. The parent long-term memory checkbox can close when the remaining phase-3/4 memory product items are done.
- Honcho, if it ever appears, is another backend behind this trait. It is not a second memory API and not a soul-pack feature.
