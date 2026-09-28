# Softwake job: Remote Agent — opt-in OAuth mirror

**Branch:** `feat/oauth-mirror`  
**Worktree:** `/tmp/softwake-oauth-mirror` (from `origin/main` @ `ba99e15` — includes #99 RA installer / ADR-0044)  
**Repo:** unkn0wn1/softwake  
**Model:** grok-4.7, `--effort xhigh`  
**Daemon features when building:** `live-http,sherpa-kws,pipewire-capture` (local verify only; do **not** restart softwaked/UI)

Also copy this brief to `.grok-briefs/oauth-mirror-brief.md` in the worktree.

## Goal (this PR ONLY — ship real behavior)

Ship **opt-in OAuth token mirror** from laptop Softwake → softwake-node so companion `agent_task` / Telegram away turns can run **email / calendar / Drive** tools while the laptop is away.

Spencer chose **yes** for this feature. Default remains **OFF**. When off, OAuth stays laptop-local (today’s refuse path). When on, mirror only what companion cloud tools need, over Tailscale, authenticated with the existing pairing secret.

## Product locks

| Lock | Choice |
|---|---|
| Default | **OFF** — OAuth stays laptop-local unless operator enables mirror |
| Network | **Tailscale-only**; no public ingress / non-TS transport |
| Auth | Existing pairing-secret Bearer (same as vault/telegram/llm). TS encrypts transit |
| Scope of tokens | Mirror Google + Microsoft `AccountConnection` rows (+ active ids) needed for inbox/calendar/drive/email_send |
| Multi-account (#88 / ADR-0033) | Prefer mirroring **connections the laptop already has** (all rows in bag); node uses same `account=` / active-or-first routing |
| Settings | Clear **opt-in checkbox** + **danger copy** (tokens land on the CT) |
| Refresh | Companion must **refresh tokens safely** on node (update node vault; never log tokens) |
| Revoke | Document: **disable mirror** + **rotate pairing secret** (+ clear node OAuth vault) |
| Laptop-local | **Do NOT break** laptop path when mirror is off |
| PROTOCOL | Stay **1** |

## IN (must ship)

1. **Opt-in flag** per remote-agent (preferred) on `RemoteAgentConfig` — e.g. `oauth_mirror: bool` default `false` (serde default). Persist in `remote-agents.json`. Document version bump only if required; prefer additive optional field (keep document version if other fields did).
2. **Settings UI**: checkbox on Remote Agent pane + danger copy (“OAuth tokens will be stored on the companion CT. Disable mirror and rotate the pairing secret to revoke.”). Wire save/load.
3. **Laptop mirror** (`remote_agent.rs`): when enabled agent has `oauth_mirror`, on existing ~45s cadence (+ with other vault mirrors) `PUT /v1/vault/oauth` with google_connections + microsoft_connections + active_* ids from the secret bag. When disabled / flag off: `PUT` null/empty to clear node vault (or skip clear on transient disable — **plan must pick**: prefer clear-on-disable for revoke honesty). Never log tokens.
4. **Node vault**: durable store under `$SOFTWAKE_NODE_DATA` (mode 0600), Bearer-guarded PUT/GET optional. Clear endpoint or null body clears.
5. **Node agent tools**: when OAuth vault has usable connections, **run** email_*/calendar_*/drive_* (reuse softwake-connectors / providers refresh + live APIs with `live-http` feature on node **or** a slim shared path — **plan picks**; prefer feature-gate `live-http` on softwake-node matching daemon). When vault empty / mirror off: keep today’s refuse string (update copy if needed: “enable OAuth mirror in Settings”).
6. **Token refresh on node**: if access expired, refresh with publisher client id/secret (env / `oauth-clients.env` on CT — **document**). Persist refreshed row back into node vault. Do not require laptop awake for refresh while away.
7. **Multi-account**: mirror all bag connections; honor `account=` and active-id routing on node (same rules as ADR-0033).
8. **ADR-0045** + amend ADR-0039/0040/0043/0033 + `docs/remote-agent-pairing.md` honesty table (OAuth mirror: Live, default off). CHANGELOG + README / node README honesty.
9. **Offline tests**: flag default false; mirror payload shape (redacted asserts); node refuse when empty; node accept when vault present (mock); UI/settings round-trip if cheap; no live Google/MS in CI.
10. Document enable steps, revoke, reinstall, residual risks.

## OUT (reject scope creep)

- Expanding Drive/calendar OAuth scopes further
- Non-Tailscale transport / public ingress
- Bidirectional full secret-bag sync (xAI/telegram already separate; do not dump whole bag)
- Changing agent_task LLM bounds / sticky ownership / installer unless required for OAuth path
- Force-push, production deploy, restart softwaked/UI, secret exfil, PROTOCOL_VERSION bump, mega-PR
- Per-profile OAuth bind (still global bag; tools already take `account=`)

## Verified context (`origin/main` @ ba99e15)

- Next free ADR: **0045**
- `RemoteAgentConfig` — id, name, tailscale_hostname, ssh_user, roles, conflict_policy, enabled, created/updated_ms — **no oauth_mirror yet** (`crates/softwake-tools/src/remote_agents.rs`)
- UI pane: `#remote-agent-enabled` etc. (`ui/index.html` / `app.js`); no OAuth mirror checkbox
- Laptop mirror: `crates/softwake-daemon/src/remote_agent.rs` — `mirror_telegram_vault`, `mirror_schedules`, `mirror_soul_skills_tools` on ~45s tick; comment “OAuth tokens are never mirrored (ADR-0043)”
- Node routes: presence, leases, outbox, schedules, vault/telegram, vault/llm, messengers, tools, skills, soul — **no `/v1/vault/oauth`**
- Node `agent.rs` `invoke_companion`: email/calendar/drive **refuse** “OAuth tokens that stay laptop-local”
- Tokens live in provider secret bag: `google_connections` / `microsoft_connections` / `active_*_connection_id` (`softwake-providers` `AccountConnection`)
- softwake-node Cargo.toml: softwake-tools/soul/skills/ureq only — **no** connectors/providers/`live-http` yet
- Pairing doc honesty: “OAuth mirror | **Out** (opt-in later)”

## Locked design sketch (plan may refine; do not invent crates)

### Flag + Settings

- `RemoteAgentConfig.oauth_mirror: bool` default false (`#[serde(default)]`).
- Checkbox label e.g. “Mirror OAuth tokens to companion (email/calendar/Drive while away)” + danger `<p class="meta">` copy.
- Save via existing remote-agent save path; ctl dump shows flag if present.

### Sync surface

| Item | Endpoint | Node storage |
|---|---|---|
| OAuth connections | `PUT /v1/vault/oauth` | `$SOFTWAKE_NODE_DATA/vault/oauth.json` (0600) |
| Body | `{ "google_connections": [...], "microsoft_connections": [...], "active_google_connection_id": ?, "active_microsoft_connection_id": ? }` or `{...: null}` to clear | — |
| Auth | Bearer pairing secret | same as other vault routes |

Laptop source of truth while awake: periodic full replace of node vault from bag when flag on. On flag off / agent disabled: clear node vault.

### Companion use

1. `invoke_companion` for OAuth tool family: load node OAuth vault; if empty → refuse with enable hint; else call shared live helpers (prefer softwake-connectors + providers refresh under node `live-http` feature).
2. Refresh: update matching `AccountConnection` in node vault file after successful refresh (0600 write). Client id/secret from env / oauth-clients.env on CT (document in pairing + node README). Missing client id → clear tool error, no panic.
3. Do not mirror laptop plaintext keyring path; only the connection rows over TS+Bearer.
4. Encryption: Tailscale provides transit encryption; pairing secret authenticates. Do **not** invent a second crypto layer unless trivial AES with pairing-secret-derived key is already patterned — prefer document “TS + Bearer” as the security model (matches telegram/llm vault).

### Laptop-local safety

- When `oauth_mirror` false: zero PUT of tokens; node vault empty/cleared; laptop connectors unchanged.
- Enabling mirror must not alter laptop tool routing.

## Quality gates

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `cargo test -p softwake-node --locked` (+ providers/tools/daemon/ui targeted tests)
- Prefer offline unit tests (tempdirs); no live OAuth in CI
- Do not restart softwaked/UI. No force-push. Merge with `gh pr merge --merge` after CI green.
- Public docs only: no private hostnames, shop paths, absolute homes, real tokens.

## Deliverables

1. Plan → `.grok-briefs/oauth-mirror-APPROVED-PLAN.md` (+ copy `/tmp/…`)
2. Implement → commit on `feat/oauth-mirror` → push → PR → CI green → merge
3. Execute report → `.grok-briefs/oauth-mirror-EXECUTE-REPORT.md` with PR#, merge SHA, how to enable, security notes, revoke, reinstall, residual risks

## PR flow

- Worktree only under `/tmp/softwake-oauth-mirror` from `origin/main` @ `ba99e15`
- Branch `feat/oauth-mirror`
- Push SSH; `gh pr create`; wait CI; `gh pr merge --merge` (merge commit, not squash)
- If `gh` invalid → MCP/SSH fallback; do **not** hunt secrets
- If grok stalls at 0 edits → implement against the approved plan yourself
