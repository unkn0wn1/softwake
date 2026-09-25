# ADR 0010 — Policy engine

- **Status:** Accepted
- **Date:** 2026-09-25
- **Amended:** 2026-09-25 (`drive` / `list` and `calendar` / `list` are confirm); 2026-09-25 (the durable memory store is the opt-in file in ADR 0009)

## Decision

Classification of a tool name and of a connector pair lives in `softwake-policy`. The allowlists stay the static tables in [`ToolRegistry`](../crates/softwake-tools/src/lib.rs) and [`ConnectorRegistry`](../crates/softwake-connectors/src/registry.rs). This crate does not copy those rows and has no method that inserts one.

[`PolicyEngine::evaluate`](../crates/softwake-policy/src/lib.rs) is the evaluation path. It takes a [`Subject`](../crates/softwake-policy/src/lib.rs): a tool name, or a connector and an action. It returns a [`PolicyDecision`](../crates/softwake-policy/src/lib.rs): `safe`, `confirm`, or `deny`.

| Subject | Decision |
|---------|----------|
| tool `echo` | safe |
| tool `notify` | confirm |
| tool `email_send` | confirm |
| tool `shell` | deny |
| connector `email` / `send` | confirm |
| connector `email` / `delete` | deny |
| connector `drive` / `list` | confirm |
| connector `drive` / `delete` | deny |
| connector `calendar` / `list` | confirm |
| connector `calendar` / `delete` | deny |
| any other tool name | deny |
| any other connector pair | deny |

`safe` runs while awake. `confirm` does not run until `confirm_tool`. `deny` never runs. Those meanings match [ADR 0005](ADR-0005-tool-confirmation.md). A connector `confirm` means the caller may perform the action only after that confirmation, matching [ADR 0008](ADR-0008-connector-boundary.md). The engine does not send, does not append, and does not spawn a process.

[`ToolRisk`](../crates/softwake-tools/src/lib.rs) stays on the tool row. [`ConnectorRisk`](../crates/softwake-connectors/src/registry.rs) stays on the connector row and still has no `safe` variant. `evaluate` on a connector subject never returns `safe`. A missing registry row becomes `deny` inside the engine. The daemon still reports an unregistered tool name as unknown, and `shell` as denied. Those operator strings stay distinct.

[`tighten`](../crates/softwake-policy/src/tighten.rs) takes a floor and a requested decision and returns the more restrictive of the two. The order is `safe`, then `confirm`, then `deny`. A weaker request is ignored. [`PolicyOverrides`](../crates/softwake-policy/src/lib.rs) maps a known tool name, or a known connector pair, to a requested decision. Duplicate rows keep the more restrictive request. An unknown subject does not read the map, so an override cannot add an allowlist entry.

A future `tools.md` or policy file may only raise the risk of a known row, or be refused. It must not lower `deny` to `confirm` or `confirm` to `safe`. This change does not parse a file. The daemon builds [`PolicyEngine::builtin`](../crates/softwake-policy/src/lib.rs), whose override map is empty, so each registered row keeps the decision in the table above.

While awake, daemon `Hands::request` branches on `evaluate` for the tool name. Voice state still runs first: sleep and hibernate refuse every name and do not consult the engine. Descriptions, `invoke`, and `invoke_confirmed` stay on `ToolRegistry`. Before `commit_email_send`, `email` / `send` must evaluate to `confirm`. `authorize_confirmed` and the in-memory send stay in `softwake-connectors`. [`permits_confirmed_connector`](../crates/softwake-policy/src/lib.rs) is true only for `confirm`, so a `safe` or `deny` answer does not send.

`softwake-tools` does not depend on `softwake-connectors`. Neither crate depends on `softwake-policy`. `softwake-policy` reads both registries. `softwake-daemon` depends on `softwake-policy`.

IPC protocol generation stays `1`. Policy is not a status field and not a socket command.

The default build has no OAuth types, no live Gmail, Drive, or Calendar client, and no policy feature flag. The in-memory Drive and calendar mocks are the [ADR 0008](ADR-0008-connector-boundary.md) amendment. This crate still does not list, send, or open a socket. CI does not set a credential and does not enable a network backend.

## Context

Phase 3 already classifies session tools in [ADR 0005](ADR-0005-tool-confirmation.md) and world actions in [ADR 0008](ADR-0008-connector-boundary.md). Those are two registries and two risk enums. The stricter-policy milestone needs one evaluation path, with room for a later soul or policy file to tighten a row. The file format can wait. The function that refuses to loosen a row cannot.

World I/O stays confirm or deny. [ADR 0008](ADR-0008-connector-boundary.md) kept `ConnectorRisk` separate so a safe send could not be written down. This ADR keeps that split on the rows and still offers one answer type from the engine.

## Alternatives

- One shared row enum for tools and connectors. Rejected. A safe variant would remain representable on a send. Same rejection as [ADR 0008](ADR-0008-connector-boundary.md).
- Copy the tool rows and the connector rows into this crate. Rejected. Two tables drift.
- Parse `tools.md` or policy TOML in this change. Rejected. The soul pack still renders a fixed instruction stub. A later parser must refuse a file that would loosen a row.
- Collapse unknown and deny into one daemon error. Rejected. `volume` stays unknown. `shell` stays denied.
- Load a non-empty override map into `Hands` in this change. Rejected. The builtin map is empty. Confirming a tool whose registry row is still safe needs `invoke`, because `invoke_confirmed` rejects that row. That wiring waits until a file is loaded.
- A live mailbox, Drive client, or Calendar client. Rejected. CI would need a token or a network.
- Bump the protocol generation. Rejected. The socket surface is unchanged.

## How to demo

```bash
cargo test -p softwake-policy
```

The typed demo is unchanged. `echo` runs while awake. `notify` and `email_send` wait for confirm. `shell` is denied.

## Consequences

- Callers can ask one engine whether a tool or a connector action may run. They cannot add a name or lower a row through this crate.
- `shell`, `email` / `delete`, `drive` / `delete`, and `calendar` / `delete` stay deny until a later ADR changes the registry row on purpose. `drive` / `list` and `calendar` / `list` are confirm. A connector evaluation is still never safe. The daemon still does not list files or events.
- A future policy file may tighten a known row or be refused. `tighten` keeps the stricter decision even when a caller passes a weaker request.
- Wiring a non-empty map into the daemon is a follow-up. It must keep an unknown tool distinct from a denied tool, and it must run a confirmed tool whose registry row is still safe through `invoke`.
- Clients that speak protocol generation 1 see no new message kinds.
- The stricter-policy milestone line is closed. Live connectors stay open. The durable memory store is the opt-in file in [ADR 0009](ADR-0009-long-term-memory.md).
