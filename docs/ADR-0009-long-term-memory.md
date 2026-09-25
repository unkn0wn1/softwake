# ADR 0009 — Long-term memory

- **Status:** Accepted
- **Date:** 2026-09-25

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

The trait's error is associated, matching [`EmailConnector`](../crates/softwake-connectors/src/email.rs). `MockMemory` uses `MemoryError`: disabled, empty, too long, and missing. A later backend maps failures through its own error type. Opt-in methods sit on the mock, not on the trait.

`softwake-soul` renders instructions only. A later session change may attach `recall` snippets beside those instructions. [`TextStubSession`](../crates/softwake-session/src/lib.rs) does not call this crate. Closing a session must not be how memory is forgotten. `softwake-daemon`, `softwake-session`, and `softwake-soul` do not depend on `softwake-memory`.

The default build has no Honcho client, no HTTP client, and no memory feature flag. CI does not set a memory key and does not start a database.

IPC protocol generation stays `1`. Memory is not a status field and not a socket command.

A future durable backend writes under `$XDG_STATE_HOME/softwake` when that variable is set and non-blank, and under `~/.local/state/softwake` otherwise. This change does not create that directory and does not name a file it writes.

## Context

Phase 3 can name a memory boundary now that connectors exist. [The soul pack](03-soul-pack.md) already says memory is a separate module and soul parsing stays instruction-only. [ADR 0006](ADR-0006-on-device-wake.md), [ADR 0007](ADR-0007-awake-stt-tts.md), and [ADR 0008](ADR-0008-connector-boundary.md) already pick a mock default so CI has no device, no weights, and no cloud keys. Memory follows that rule.

Snippets are operator data. The default value stores nothing until that value is enabled. Nothing in this build leaves the process.

Honcho is a later optional backend at most, behind this same trait, in its own ADR, and off in CI. The official server is AGPL-3.0. The official clients are Python and TypeScript. There is no first-party Rust client. A deployment is the managed API, which needs a key, or Postgres, Redis, a deriver, and model keys. The third-party `honcho-ai` crate is not a dependency: it still needs a Honcho server, tokio, and Rust 1.88, above this workspace's `rust-version` of 1.85. Any of those would make default CI keyed or networked.

The 8 KiB cap is the snippet-sized form of the soul pack's limit on a huge paste. A soul file may be 1 MiB. A snippet is text a session would attach beside those instructions.

## Alternatives

- Honcho as the default, or as a crate dependency, or as a feature CI compiles. Rejected. The runtime weight, the keys, the network, the MSRV, and the AGPL server do not fit the workspace. A later optional backend can implement `Memory` behind a feature that CI leaves off.
- Depend on `honcho-ai`. Rejected. It is a third-party client of the Honcho server, not a first-party Rust SDK, and it still needs the server.
- Cloud-only retrieval as the memory implementation. Rejected. The default build has no network and no key.
- Stuff recall into `softwake-soul` parsing. Rejected. The soul pack is instructions. Memory is retrieved text the session layer attaches.
- Durable sqlite or a file under the XDG state directory in this change. Rejected. That is the productized store. The mock keeps the trait honest without a format to migrate.
- Wire `MockMemory` into `TextStubSession` or the daemon. Rejected. Session close drops one awake period. Memory must outlive that period. No IPC change is required to keep protocol generation 1.
- An async trait. Rejected. Library crates stay runtime-agnostic. The daemon is where the runtime is chosen.
- Embeddings, peer ids, or a dialectic query. Rejected. The record is an id and text until a backend needs more.

## How to demo

```bash
cargo test -p softwake-memory
```

The daemon does not call the crate. There is no typed-demo step.

## Consequences

- Callers can enable a value, store a snippet, search it, and drop it. They cannot reach Honcho or a disk store from this build.
- Records die with the value. Process exit clears them.
- A durable backend is a follow-up ADR. It stays off unless the operator opts in, writes only under the state directory above, and implements `Memory`.
- Honcho, if it ever appears, is that kind of backend. It is not a second memory API and not a soul-pack feature.
- Clients that speak protocol generation 1 see no new message kinds.
- The long-term memory milestone's decision line is closed. The productized durable store stays open.
