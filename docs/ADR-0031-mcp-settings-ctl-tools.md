# ADR-0031 — MCP Settings, Softwake ctl tools, and `/refresh`

- **Status:** Accepted
- **Date:** 2026-09-27
- **Relates to:** [ADR-0025](ADR-0025-api-tool-calling.md), [ADR-0028](ADR-0028-slash-hud-self-sleep.md), [ADR-0029](ADR-0029-messengers-telegram.md), [ADR-0018](ADR-0018-tools-settings-shell.md)

## Context

Operators configure Messengers and Tools in Settings, and drive Softwake with
slash commands while awake ([ADR 0028](ADR-0028-slash-hud-self-sleep.md)). Agents
on Ask / Telegram see Hands tools via OpenAI advertise + the live permissions
appendix ([ADR 0025](ADR-0025-api-tool-calling.md)) but cannot:

1. Call Model Context Protocol (MCP) servers the operator trusts.
2. Mirror slash ctl (`/sleep`, `/profile`, `/model`, …) as function tools.
3. Force a full live-config reload (profile soul + chat) after Settings edits.

## Decision

### 1. Settings → MCP (Messengers-like pane)

Left-nav **MCP** with an expandable sublist of configured servers (chip +
`+ New`, same helpers as Messengers / Timers / Skills).

Non-secret config:

```text
~/.config/softwake/mcp.json
```

Shape (version 1):

```json
{
  "version": 1,
  "servers": [
    {
      "id": "filesystem",
      "label": "Filesystem",
      "enabled": true,
      "transport": "stdio",
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
      "env": {},
      "url": null,
      "auth_header_name": "Authorization",
      "permission": "ask"
    }
  ]
}
```

- **transport:** `stdio` (command + args + env) or `url` (HTTP MCP endpoint; v1
  stores the URL and optional auth; stdio is the supported invoke path).
- **enabled:** master switch. Disabled servers are not discovered or advertised.
- **permission:** server **group** default: `always_allow` | `ask` | `deny`.
  Tools Settings may override individual advertised MCP tool names; missing keys
  inherit the server group. Deny omits the tool from OpenAI advertise.
- Auth secrets (bearer / header value) live in the secret bag map
  `mcp_secrets: { "<server_id>": "<secret>" }` — never in `mcp.json`, never logged.

PROTOCOL stays **1**. Softwake-ui reads/writes bag + `mcp.json` in-process.

### 2. MCP tool bridge

- On daemon start, `/refresh`, and when Settings save enables a server, softwaked
  runs MCP `initialize` + `tools/list` for each enabled stdio server (spawn,
  Content-Length JSON-RPC, then shut down or keep a short-lived session per call
  in v1).
- Advertised names: `mcp_<serverId>_<toolName>` with non `[A-Za-z0-9_]` folded to
  `_`. OpenAI `tools` entries use the MCP tool description + inputSchema when
  present.
- Invoke: Hands permission gate (group/override), then MCP `tools/call`. Ask
  stages a pending card like other confirm tools. Registry floor is confirm
  (world I/O).
- Failed spawn / list is fail-open for that server (appendix notes error; other
  tools keep working).

### 3. Softwake ctl tools (slash mirrors)

Static Hands tools, OpenAI-advertised when not Deny:

| Tool | Default | Mirrors |
|---|---|---|
| `softwake_status` | Always allow | `/status` |
| `softwake_list_models` | Always allow | `/model` |
| `softwake_list_voices` | Always allow | `/voice` / list |
| `softwake_list_profiles` | Always allow | `/profile` |
| `softwake_set_model` | Ask | `/model ai\|voice <id>` |
| `softwake_set_voice` | Ask | `/voice <id>` |
| `softwake_set_profile` | Ask | `/profile <name>` |
| `softwake_sleep` | Ask | `/sleep` |
| `softwake_hibernate` | Ask | `/hibernate` |
| `softwake_resume` | Ask | `/resume` (only from hibernate; while awake the tool explains that) |
| `softwake_new_session` | Ask | `/new` |
| `softwake_refresh` | Ask | `/refresh` |

List tools default Always allow; state-changing tools default Ask.

### 4. `/refresh` and `softwake_refresh` (profile + chat)

**Must** reload profile and chat, not only tools/MCP:

1. Re-resolve the **active** profile pack directory and re-read the four-file
   soul pack (same as `/new` / reload-soul retarget when unlocked).
2. **Clear the awake model session** and open a fresh session with the reloaded
   pack instructions.
3. **Reseed** from plaintext `hud-chat.json` (same `hud_seed` path as wake /
   `/new`). Encrypted vaults stay UI `SeedChat` after unlock — refresh does not
   decrypt.
4. Rediscover MCP tool lists for enabled servers.
5. Tools permissions, providers/models/voices, messengers, and schedules stay
   disk-backed and are re-read on next use; refresh does not require a daemon
   restart.

**Does not** switch profile by itself. Pair with `/profile` /
`softwake_set_profile` when changing agents. While awake, `set_profile` already
applies a fresh session for the new profile; `refresh` afterward repeats the
full reload + clear + HUD reseed cycle for whatever profile is now active
(useful after editing that profile’s soul files or MCP/Tools Settings).

### 5. Slash surface

Add `/refresh` (and clear-typed `refresh` when unambiguous). Help text lists it.
Implementation shares `Runtime::full_refresh` with `softwake_refresh`.

### 6. Out of scope

- MCP resources/prompts/sampling, OAuth-for-MCP, Windows named-pipe MCP.
- Force-killing operator MCP child processes on Softwake exit beyond Drop.
- Changing PROTOCOL_VERSION.

## Consequences

- Agents can sleep, switch profile, pick models, and refresh after Settings
  edits without the operator typing slash commands.
- MCP servers are operator-mediated (Settings + Tools Always/Ask/Deny).
- `/refresh` is deliberately session-clearing so a profile/soul change cannot
  leave stale instructions in the model context.

## Demo

```bash
cargo test -p softwake-tools -p softwake-providers -p softwake-daemon -p softwake-ui
cargo check -p softwake-daemon --features live-http
cargo check -p softwake-ui --features live-http
```

Manual: Settings → MCP → add stdio server, enable, set group Ask; Tools page
shows Softwake ctl rows + MCP group; awake `/refresh` or ask the agent to call
`softwake_refresh`; confirm session cleared and HUD-seeded.
