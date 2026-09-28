# Opt-in OAuth mirror — approve-ready plan (ADR-0045)

**Status:** Plan only. Do not treat this file as shipped behavior.
**Branch:** `feat/oauth-mirror` @ `ba99e154085640162c00153385b1b88eaaec4bac` (merge of #99, ADR-0044 installer).
**Worktree:** `/tmp/softwake-oauth-mirror`. Product tree is clean aside from untracked `.grok-briefs/`.
**This phase:** write this plan only. Do not commit, push, open a PR, force-push, deploy, or restart `softwaked` / the UI.
**PROTOCOL_VERSION:** stays `1` (`crates/softwake-ipc/src/types.rs`). No new socket message.

## Decisions (locked)

1. **Node live HTTP: feature-gate `live-http` on `softwake-node`, default ON.** Do not extract a new crate and do not move laptop `cloud_tools` / `email_send` / `calendar_write`. The node grows a slim runner that calls the existing network-free connector URL builders and `softwake-providers` refresh (`ensure_fresh_account` + `Transport`). `LiveTransport` is compiled only with the feature. `MockTransport` keeps CI offline.
2. **Clear on disable.** When the resolved enabled companion has `oauth_mirror: false`, the ~45s mirror loop `PUT`s an empty document and the node deletes `vault/oauth.json`. A failed or partial JSON body does not delete the file.

## Verified baseline

Checked in this worktree. Do not invent a flat `src/` layout or a new crate.

| Claim | Result |
|---|---|
| `RemoteAgentConfig` has no `oauth_mirror` | True. Fields: `id`, `name`, `tailscale_hostname`, `ssh_user`, `roles`, `conflict_policy`, `enabled`, `created_ms`, `updated_ms`. `DOCUMENT_VERSION` is `1`. File: `crates/softwake-tools/src/remote_agents.rs`. |
| Settings has `#remote-agent-enabled` and no OAuth checkbox | True. `crates/softwake-ui/ui/index.html`, `crates/softwake-ui/ui/app.js`. Commands: `crates/softwake-ui/src/remote_agent.rs` (`remote_agent_snapshot` / `save` / `add` / `test` / `install`). |
| Laptop mirror comment | Exact text is `OAuth tokens are never mirrored (ADR-0043)` on `mirror_soul_skills_tools` in `crates/softwake-daemon/src/remote_agent.rs`. The ~45s loop (`MIRROR_SECS = 45`) calls `mirror_schedules`, `mirror_telegram_vault`, `mirror_soul_skills_tools`. HTTP client is raw TCP, not `ureq`. |
| Node routes | `GET /health` is open. Everything else is Bearer / `X-Softwake-Remote-Token` (`crates/softwake-node/src/auth.rs`). Present: presence, leases, outbox, schedules, `PUT /v1/vault/telegram`, `PUT /v1/vault/llm`, messengers, tools, skills, soul, telegram ownership. **No `/v1/vault/oauth`.** |
| Companion OAuth tools | `invoke_companion` in `crates/softwake-node/src/agent.rs` refuses `email_*`, `calendar_*`, `drive_*` with `tool \`{name}\` needs OAuth tokens that stay laptop-local (not mirrored to companion)`. |
| Node deps | `crates/softwake-node/Cargo.toml`: `softwake-tools`, `softwake-soul`, `softwake-skills`, `ureq` only. No connectors, providers, or `live-http`. `ureq` is already unconditional (xAI + Telegram). |
| Token bag | `SecretBag` in `crates/softwake-providers/src/secrets.rs`: `google_connections`, `microsoft_connections`, `active_google_connection_id`, `active_microsoft_connection_id`. `AccountConnection` redacts tokens in `Debug`. Routing is `SecretBag::resolve_account` (ADR-0033). Refresh is `ensure_fresh_account` / `refresh_token_body` / `publisher_google_client_id` / `publisher_google_client_secret` / `publisher_microsoft_client_id`. Microsoft refresh sends **no** client secret. |
| Connectors | **Correction.** `softwake-connectors` does not open sockets. It exposes URL builders, parsers, and formatters (`inbox_api`, `calendar_api`, `calendar_write_api`, `drive_api`, `email_send_api`). Live HTTPS for those tools lives in the **daemon** (`cloud_tools.rs`, `email_send.rs`, `calendar_write.rs`) behind daemon `live-http`, and it reads the **laptop secret bag** through `with_account`. Those functions are `pub(crate)`. The node must not depend on `softwake-daemon`. |
| Vault files today | Single-line secrets under `$SOFTWAKE_NODE_DATA/vault/` via `write_mode_0600` (`telegram_bot_token`, `xai_api_key`). Data dir: `SOFTWAKE_NODE_DATA`, else `$XDG_DATA_HOME/softwake-node`, else `$HOME/.local/share/softwake-node`. |
| Next ADR | **0045**. Highest file is `docs/ADR-0044-remote-agent-ssh-installer.md`. |
| Pairing honesty | `docs/remote-agent-pairing.md` row is `OAuth mirror \| **Out** (opt-in later)`. Mirror table says OAuth tokens are not mirrored. Base URL is `http://{host}:8790` (`node_base_url`). Tailscale is the transport encryption. There is no second crypto layer on telegram/llm vault. |
| Installer | `resolve_softwake_node_bin` runs `cargo build --release -p softwake-node --locked` with **no** features. `render_node_env` rewrites `/etc/softwake-node.env` in full. Unit runs `User=softwake`, `EnvironmentFile=/etc/softwake-node.env`, `ReadWritePaths=/var/lib/softwake-node`, home `/var/lib/softwake-node`. |
| Telegram away | `companion_ask` is an xAI **oneshot** whose system line says `No tools.` It does not call `run_agent_turn`. Timer fan-out (`maybe_fanout_timer`) only sends text. Sticky ownership is unchanged. |
| ctl | `softwaked ctl remote-agent` is only `test` and `install`. There is no dump subcommand. |
| Mirror target | `first_enabled_agent` only. Other mirrors already behave this way. |

Laptop cloud tools keep using the secret bag. This PR does not change `with_account`, dispatch, or Email Connect.

## Product locks

| Lock | Choice |
|---|---|
| Default | `oauth_mirror: false` per agent. Serde missing field loads as false. Document version stays 1. |
| Network | Tailscale only. No public bind, no new transport. Installer still refuses non-TS hosts. |
| Auth | Existing pairing-secret Bearer. `GET /health` stays open. OAuth route does not. |
| What is mirrored | All `google_connections` and `microsoft_connections` plus both active ids. Not the rest of the bag (no xAI, Telegram, SMTP, webhook, MCP, pairing secrets, xAI OAuth). |
| Multi-account | Same `account=` / active-or-first / Google-preferred rules as ADR-0033, by calling `SecretBag::resolve_account` on a bag that contains **only** those four fields. |
| Settings | Checkbox + danger copy. Tokens land on the CT. |
| Refresh | Node refreshes when access is near expiry, writes the merged row back to the node vault, never logs tokens. Publisher client id/secret come from env or `oauth-clients.env` (already implemented in providers). |
| Revoke | Disable the flag while that agent is still the enabled target (so the clear PUT can land), rotate the pairing secret, delete the vault file if the daemon cannot reach the node. |
| Laptop path | Flag off → no token bytes on the wire except the empty clear document. Laptop connectors unchanged. |
| Ask | Companion still never silent-Always-allows. Ask tools return the existing pending sentence. |
| Scopes | Do not change `GOOGLE_EMAIL_SCOPES` or `MICROSOFT_EMAIL_SCOPES`. Refresh does not enlarge a grant. |

## 1. Flag

In `crates/softwake-tools/src/remote_agents.rs`, add to `RemoteAgentConfig`:

```rust
/// When true, laptop mirrors Email OAuth connections to this companion (ADR-0045).
#[serde(default)]
pub oauth_mirror: bool,
```

No `Default` impl is required. `DOCUMENT_VERSION` stays `1`. `normalize` does not need to touch the flag.

Update every struct literal (compiler will list them if one is missed):

- `crates/softwake-tools/src/remote_agents.rs` (tests and any helpers)
- `crates/softwake-tools/src/remote_agent_install.rs` `sample`
- `crates/softwake-ui/src/remote_agent.rs` `remote_agent_add` and `remote_agent_save`

`remote_agent_add` sets `oauth_mirror: false`. Save persists `args.oauth_mirror`.

Test: JSON without the key loads `false`. JSON with `true` round-trips. Old files keep working.

There is no ctl dump. Do not add one. The flag is non-secret and visible in `remote-agents.json`. Operators can read `oauth_mirror` there. Never print token fields from the secret bag in ctl output.

## 2. Settings checkbox

`crates/softwake-ui/ui/index.html`, directly under the Enabled checkbox:

```html
<label class="row">
  <input type="checkbox" id="remote-agent-oauth-mirror" />
  <span>Mirror OAuth tokens to companion (email/calendar/Drive while away)</span>
</label>
<p id="remote-agent-oauth-mirror-danger" class="meta">OAuth tokens will be stored on the companion CT. Disable mirror and rotate the pairing secret to revoke.</p>
```

`crates/softwake-ui/ui/app.js`:

- Query `#remote-agent-oauth-mirror`.
- `applyRemoteAgentSnapshot` sets `checked` from `snap.oauthMirror` (missing → false).
- `saveRemoteAgent` sends `oauthMirror: !!(remoteAgentOauthMirror && remoteAgentOauthMirror.checked)`.

`crates/softwake-ui/src/remote_agent.rs`:

- `RemoteAgentSnapshot.oauth_mirror: bool` with the existing `camelCase` rename (`oauthMirror`).
- `RemoteAgentSaveArgs.oauth_mirror: bool`.
- `snapshot_for` copies `row.oauth_mirror` (no row → false).

Do not put tokens, the pairing secret, or client secrets in the snapshot. Do not enable the disabled Telegram-owner checkbox. Do not change Test / Install button wiring except the installer notes in section 8.

No browser harness is required for the gate: this window is Tauri, and this job must not launch or restart `softwake-ui` / `softwaked`. The offline proof is the serde round-trip plus the command struct. A reviewer can read the three call sites (HTML id, JS property, Rust field).

## 3. Laptop mirror

Add `mirror_oauth_vault()` in `crates/softwake-daemon/src/remote_agent.rs` and call it from the existing 45s thread, next to `mirror_telegram_vault`. Do not also call it from `schedule_tick.rs` (telegram vault is not called there either).

`resolve_target()` gains `oauth_mirror: bool` copied from `first_enabled_agent`.

Behavior:

- No enabled agent, or no pairing secret → return. Do not open a socket. **Cannot clear** in this case (see residual risks).
- `oauth_mirror == true` → read the secret bag once. Build a document with only the four OAuth fields (section 4). `PUT /v1/vault/oauth` with `Content-Type: application/json` and the existing Bearer header. Same `http_json` helper. Ignore the response the same way telegram mirror does (`let _ = ...`). Do not `eprintln` the body, the URL with a secret, or `Debug` of the bag.
- `oauth_mirror == false` → `PUT` `OauthMirrorDocument::default()` (empty arrays, null active ids). That is the clear.

Full replace while the flag is on, including when the bag has zero connections (empty document, node deletes the file). The laptop bag stays the source of truth on each successful PUT. Do not GET tokens back. That would be bidirectional sync.

Pure helper, unit-tested without a node:

```rust
fn oauth_mirror_document(bag: &SecretBag) -> OauthMirrorDocument
```

Put the serde type in `softwake-providers` (next to `AccountConnection`) so the node and the daemon share it:

```rust
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct OauthMirrorDocument {
    #[serde(default, deserialize_with = "null_as_empty_vec")]
    pub google_connections: Vec<AccountConnection>,
    #[serde(default, deserialize_with = "null_as_empty_vec")]
    pub microsoft_connections: Vec<AccountConnection>,
    #[serde(default)]
    pub active_google_connection_id: Option<String>,
    #[serde(default)]
    pub active_microsoft_connection_id: Option<String>,
}
```

`null_as_empty_vec` accepts a JSON array or `null` (brief’s clear shape). Derived `Debug` is safe because `AccountConnection` already redacts both tokens. Add `fn is_empty(&self) -> bool` (both vecs empty).

Export the type from `crates/softwake-providers/src/lib.rs`.

Daemon test bag fills **distinct** sentinels for `xai_api_key`, `telegram_bot_token`, `email_smtp_password`, one MCP secret, one pairing secret, and one Google `AccountConnection` (`access_token` / `refresh_token` sentinels, email `ada@example.com`). Assert the OAuth JSON contains the email and the access sentinel, and does **not** contain the other sentinels. Assert `Debug` of the document does not contain the access sentinel. Assert the clear document serializes with empty arrays and no access sentinel.

Optional daemon test: one-shot `TcpListener` on `127.0.0.1:0`, call the payload builder + `http_json` shape (or a tiny test double of the PUT). Do not require a live companion. If that test is awkward, the node integration test covers the wire and the daemon test covers the JSON. Do not log the request body on assertion failure beyond what `assert!` already prints; keep sentinels obvious (`sentinel-access`, not a real token).

Replace the comment on `mirror_soul_skills_tools` with: soul/skills/tools mirror does not include OAuth; OAuth is `mirror_oauth_vault` (ADR-0045), default off.

## 4. Node vault

Path: `{data_dir}/vault/oauth.json`. With the installer unit that is `/var/lib/softwake-node/vault/oauth.json`. Code uses `data_dir`, not a hard-coded absolute path.

Add on `NodeState` (`crates/softwake-node/src/state.rs`):

- `oauth: Mutex<()>` created in `open`. Every read-modify-write of the file holds it. Do not cache the document in memory; the laptop replaces the file.
- `put_oauth_document(&self, doc: &OauthMirrorDocument) -> Result<(), String>`
  - If `doc.is_empty()`, delete `oauth.json` and `oauth.json.tmp` if present. Return Ok.
  - Otherwise write `oauth.json.tmp` in the vault dir, mode `0600`, then rename over `oauth.json`. Reuse `write_mode_0600`. On Unix, set the `vault/` directory to `0700` after `create_dir_all`.
  - Pretty JSON + trailing newline, same style as messengers.
- `load_oauth_document(&self) -> OauthMirrorDocument` — missing or unreadable file → empty. Corrupt JSON → empty for tool use, but **PUT** of corrupt JSON must not call this and then delete (see route).
- `oauth_public_status(&self) -> Value` — counts, active ids, and `{id, email, provider}` only. No `access_token`, `refresh_token`, `scope` is optional; **omit scope** too so the GET stays dull. Emails are not tokens; they may appear.

Route in `crates/softwake-node/src/main.rs`, after the llm vault arm, still behind `authorized`:

| Method | Path | Behavior |
|---|---|---|
| `PUT` | `/v1/vault/oauth` | If `req.body.len() > 256 * 1024`, `400` and do not touch the file. Parse `OauthMirrorDocument`. Parse error → `400` and **do not** delete an existing file. Success → `put_oauth_document`. Response `{"ok":true,"google":N,"microsoft":M}` only. |
| `GET` | `/v1/vault/oauth` | `200` with `oauth_public_status`. No tokens. |

No token-bearing GET. `401` when the bearer is missing or wrong, same as the other vault routes. Do not log `req.body`.

`GET /health` stays unauthenticated and must not learn about the vault.

## 5. Companion tools

### When they run

`invoke_companion` currently returns the laptop-local sentence before the `match`. Replace that branch:

- Load the node document.
- If it has no usable row (`access_token` or `refresh_token` non-empty, same idea as `connection_has_secret`) → return the enable hint (exact string below). Do not open a cloud socket.
- If the `live-http` feature is off → return `live-http is required for inbox/calendar/Drive tools in this build` (same words as daemon `LIVE_REQUIRED`).
- Else run the tool.

Exact empty-vault string:

```text
tool `{name}` needs OAuth tokens mirrored to the companion. Enable “Mirror OAuth tokens to companion” in Settings → Remote Agent (default off).
```

Keep the word `OAuth` so the existing assertion style still applies. Update `oauth_tools_refuse_laptop_local` to expect this sentence when the vault is empty, and add a second test with a vault fixture.

Ask / Deny handling **stays above** the OAuth call, unchanged. AlwaysAllow is the only permission that reaches the runner. Default Ask tools (`email_search`, `email_get`, `email_send`, calendar write, drive search/get, and any operator Ask) still return `Pending confirmation on laptop Softwake: ...` and do not call Google or Microsoft. Do not change default `tools.json` permissions.

### Runner (`crates/softwake-node/src/oauth_tools.rs`)

Public to the crate:

```rust
pub fn run_oauth_tool(
    state: &NodeState,
    name: &str,
    args: &[String],
    transport: &dyn Transport,
) -> String
```

Production `invoke_companion` passes `LiveTransport::bounded(Duration::from_secs(30))` only inside `#[cfg(feature = "live-http")]`. Tests pass `MockTransport`.

Steps inside the runner:

1. Parse args with the existing softwake-tools parsers (`parse_email_list_args`, `parse_email_search_args`, `parse_email_get_args`, `parse_email_send_args`, `parse_calendar_*`, `parse_drive_*`). A parse error returns `err.to_string()` and does not refresh.
2. Build `SecretBag::empty()`, copy the four OAuth fields from the node document, call `resolve_account(hint)` where `hint` is the parsed `account`.
3. `ensure_fresh_account(provider, transport, &connection, now_ms)`.
   - Not expired (60s skew already in `account_needs_refresh`) → no token HTTP, client id not required.
   - Expired or `expires_at_ms == 0` → refresh. Missing publisher client id returns the existing `OAUTH_CLIENT_MISSING` (`This build has no OAuth client configured`). No panic.
   - Refresh HTTP failure stays the existing `account token refresh failed` string. Do not append the token or the form body.
4. If `access_token` or `expires_at_ms` or `refresh_token` changed, replace that row in the node document (`upsert` by id, keep the other rows and active ids) and `put_oauth_document`. Mode `0600` again.
5. Call the matching connector URL builder and `Transport`, then the existing formatter (`format_inbox_list`, `format_calendar_*`, `format_drive_*`, gmail send id, and so on).

Tool set, same names the refuse arm matches today:

| Tool | Laptop source to mirror in behavior | HTTP |
|---|---|---|
| `email_list`, `email_search`, `email_get` | `cloud_tools.rs` | GET. Microsoft search sends header `ConsistencyLevel: eventual`. |
| `calendar_list`, `calendar_get` | `cloud_tools.rs` | GET. Microsoft list sends `Prefer: outlook.timezone="UTC"`. Time window: now through `days` (clamped), RFC3339 UTC. |
| `calendar_create`, `calendar_update`, `calendar_delete` | `calendar_write.rs` | POST / PATCH / DELETE on the primary calendar. |
| `drive_list`, `drive_search`, `drive_get` | `cloud_tools.rs` | GET. `drive_get` + `read_text` does the second text/export GET when `is_cheap_text_mime`. |
| `email_send` | `email_send.rs` | POST Gmail `users.messages.send` or Graph `sendMail`. Scope check: empty scope still allows the POST; a 403 whose stored scope lacks `gmail.send` / `Mail.Send` appends the reconnect sentence (Disconnect and Connect in Settings → Email). Duplicate the small predicate in the node file. Do not `pub` use daemon `pub(crate)` helpers. |

Calendar windows: use workspace `chrono` (`chrono = { workspace = true }` on the node) to format UTC RFC3339. Do not copy `civil_from_days` out of the daemon.

### Transport gaps (small, default methods)

`Transport` already has `post_form`, `get_bearer`, and `post_json_bearer`. It does not have PATCH, DELETE, or extra headers. Calendar update/delete and two Microsoft GETs need them.

Add **default** trait methods on `Transport` so the four existing test impls (`ScriptedTransport`, `RecordingTransport`, `QueueTransport` in providers and daemon) keep compiling:

- `get_bearer_with_headers(&self, url, bearer, headers) -> Result<HttpResponse, TransportError>` defaults to `get_bearer` and ignores headers.
- `patch_json_bearer(...)` defaults to `TransportError::Failed { message: "patch is not implemented" }`.
- `delete_bearer(...)` defaults the same way with `delete is not implemented`.

Override all three on `MockTransport` (URL-keyed, same as `with_get` / `with_post_json`; add `with_patch_json` and `with_delete`) and on `LiveTransport` (real `ureq`, feature `live-http`). `LiveTransport` error text must stay secret-free (the trait already says `Failed.message` must not include secrets). Do not log the bearer.

Node cloud errors: `cloud API HTTP {status}` plus JSON `error.message` or `message`, truncated to 240 chars. If that detail contains the access token, replace it with `<redacted>`. Do not attach the `Authorization` header or the raw vault.

Do not add a Connect / PKCE / browser flow on the node. Refresh only.

### System prompt

In `run_agent_turn`, stop passing `EmailOauthStatus::default()` always.

- Vault empty → keep a companion note that OAuth is unavailable until mirror is enabled, and keep the appendix disconnected.
- Vault non-empty → build `EmailOauthStatus` from **emails and ids only** (same shape as daemon `email_oauth_status_for_appendix`: `effective_account`, `google_accounts` / `microsoft_accounts`). Companion note: mirrored accounts are available; Ask still cannot be approved on the CT; pass `account` to pick a mailbox; be concise.

Do not put access or refresh tokens in the system prompt.

### Telegram away turns

`handle_inbound` → `companion_ask` stays the oneshot (`No tools.`) when `load_oauth_document().is_empty()` **or** there is no usable row. That preserves ADR-0042 for the default.

When the vault has a usable connection, `companion_ask(state, profile_id, text)` calls `run_agent_turn` instead of `xai_oneshot`, so the away chat can use email/calendar/drive under the same permission rules. No key → the existing honest no-key string from `run_agent_turn`. Do not change who long-polls, the grace period, TTS-off, or `maybe_fanout_timer`.

This is the smallest change that makes “Telegram away turns use the tokens” true. Do not give the oneshot path tools.

## 6. `live-http` on the node

`crates/softwake-node/Cargo.toml`:

```toml
[features]
default = ["live-http"]
live-http = ["softwake-providers/live-http"]

[dependencies]
softwake-connectors.workspace = true
softwake-providers.workspace = true
chrono = { workspace = true }
# ureq stays a direct dependency (xAI and Telegram already use it)
```

Why default on: ADR-0044’s installer builds `cargo build --release -p softwake-node --locked` and would otherwise ship a node that can store the vault and then refuse every cloud call. The node binary already links `ureq`.

Also change `resolve_softwake_node_bin` to pass `--features live-http` so a later default flip cannot silently ship a store-only binary. `--no-default-features` remains the vault-only build: PUT/GET work, tools with a non-empty vault return `LIVE_REQUIRED`, empty vault returns the enable hint.

Do not enable daemon `live-http` as a side effect. Laptop `cargo test` stays offline. Node tests that need HTTP inject `MockTransport` and never construct `LiveTransport`.

Providers on the node pulls the existing `keyring` crate. **Do not call** `open_store`, `update_bag`, or any keyring API from the node. The CT has no laptop bag. Use `SecretBag::empty()` plus the four fields only.

Update the package `description` string to mention opt-in OAuth vault (one line).

## 7. Refresh client on the CT

No new env-var parser. `publisher_google_client_id`, `publisher_google_client_secret`, and `publisher_microsoft_client_id` already read process env, then `$XDG_CONFIG_HOME/softwake/oauth-clients.env`, else `$HOME/.config/softwake/oauth-clients.env`.

The systemd unit does not set `XDG_CONFIG_HOME`. `User=softwake` has `HOME=/var/lib/softwake-node` (installer `useradd --home`). So the file path on a normal install is:

`/var/lib/softwake-node/.config/softwake/oauth-clients.env`

mode `0600`, owner `softwake`. That path is inside `ReadWritePaths` and **survives reinstall** (the installer does not delete `/var/lib/softwake-node`).

Alternate: put the same keys in `/etc/softwake-node.env` (mode `0640`, `root:softwake`). Process env wins over the file. Keys:

| Key | Required for refresh |
|---|---|
| `SOFTWAKE_GOOGLE_CLIENT_ID` | Google, when the access token is near expiry |
| `SOFTWAKE_GOOGLE_CLIENT_SECRET` | Only if the desktop client has a secret (same as laptop) |
| `SOFTWAKE_MICROSOFT_CLIENT_ID` | Microsoft refresh. No Microsoft client secret. |
| `MEETREC_*` aliases | Already accepted. Do not document a real value. |

Missing id while the access token is still valid: the call proceeds. Missing id when refresh is required: tool returns `This build has no OAuth client configured`.

`render_node_env` gains **comments only** (no values):

```text
# Optional, ADR-0045: publisher client for refresh while the laptop is away.
# SOFTWAKE_GOOGLE_CLIENT_ID=
# SOFTWAKE_GOOGLE_CLIENT_SECRET=
# SOFTWAKE_MICROSOFT_CLIENT_ID=
# Or oauth-clients.env under the softwake user's home
# (/var/lib/softwake-node/.config/softwake/oauth-clients.env).
```

### Reinstall must not wipe those keys

Today `render_node_env` replaces the whole env file, which would drop a hand-edited client id and would also drop `SOFTWAKE_NODE_XAI_API_KEY` / `SOFTWAKE_NODE_MODEL`.

Add `merge_preserved_node_env(previous: &str, rendered: &str) -> String` in `remote_agent_install.rs`. The **remote shell** runs it (or an equivalent `grep`) **on the CT** before overwrite. The laptop process must not `cat` the env file back over SSH (that would copy client secrets to the laptop).

Allowlist, exact keys only:

- `SOFTWAKE_NODE_XAI_API_KEY`
- `SOFTWAKE_NODE_MODEL`
- `SOFTWAKE_GOOGLE_CLIENT_ID`
- `SOFTWAKE_GOOGLE_CLIENT_SECRET`
- `SOFTWAKE_MICROSOFT_CLIENT_ID`
- `MEETREC_GOOGLE_CLIENT_ID`
- `MEETREC_GOOGLE_CLIENT_SECRET`
- `MEETREC_MICROSOFT_CLIENT_ID`

Rules: keep a previous line when the new render does not already set that key. Do not keep arbitrary lines. Do not echo the file. Unit-test the merge with fake strings (sentinels must not appear in `render_node_env()` itself).

Do not scp the laptop `oauth-clients.env`. The operator copies the publisher ids on purpose.

Comment in the generated unit stays as it is. No `ProtectHome` change.

## 8. Multi-account

Mirror every row, not only the active one. Node routing is `resolve_account`:

- `account` hint: connection id, else full email, else email substring. Zero matches or two matches → the existing error strings (`no connected account matches '…'`). Do not fall through to another account.
- No hint and one usable row → that row.
- No hint and several usable rows, any of them Google → active Google id, else the first usable Google row.
- Otherwise active Microsoft, else the first usable Microsoft row.
- No usable row → `no Google or Microsoft account connected (Settings → Email → Connect)`.

Do not add per-profile account bind.

After refresh, `merge_account_refresh` already keeps the old refresh token, id, email, and scope when the token endpoint omits them. Persist that merged row only. Do not replace the whole document with one row.

## 9. Laptop-local safety

- Flag false: the PUT body is the empty document. Test asserts it has no access sentinel.
- Flag true: PUT is the four fields only. Laptop `with_account` is not on that path.
- Enabling the checkbox does not change daemon dispatch, Email settings, or which account the laptop tools use.
- Node must not write the laptop secrets file or keyring.
- `cargo test -p softwake-daemon` without `live-http` still passes. Do not require daemon `live-http` for the new mirror tests.

## 10. Docs

New `docs/ADR-0045-oauth-mirror.md` (Accepted, date 2026-09-28). Depends on ADR-0033, ADR-0039, ADR-0040, ADR-0043, ADR-0044. Body is this plan’s locks: default off, Tailscale + Bearer, no second cipher, clear-on-disable, refresh, Ask stays Ask, PROTOCOL 1, residual risks. Link `remote-agent-pairing.md` and `oauth-clients.md`.

Amend (short “Amendment — OAuth mirror (ADR-0045)” at the bottom, do not rewrite the original decision as if it never said laptop-local):

| Doc | Change |
|---|---|
| ADR-0033 | Node mirror uses the same `account=` / active rules on the copied rows. Still no per-profile bind. |
| ADR-0039 | OAuth row: default laptop-local; opt-in mirror is ADR-0045. Slice matrix cell “OAuth mirror” → Live, default off (ADR-0045). |
| ADR-0040 | Replace the absolute “no mirror” sentence with: default no mirror; opt-in vault is ADR-0045. Drop “OAuth token mirror” from the still-open out-of-scope list. |
| ADR-0043 | OAuth tools run when the node vault has a usable connection and the tool is AlwaysAllow; otherwise the enable hint. Ask unchanged. Mirror is opt-in, default off. |
| ADR-0042 | One paragraph: inbound Telegram stays a no-tools oneshot unless the OAuth vault has a usable connection, in which case it uses `run_agent_turn`. Ownership unchanged. The old “OAuth mirror remain later” line points at ADR-0045. |
| ADR-0044 | Out-of-scope line points at ADR-0045. Installer note: `--features live-http`, preserve the allowlisted env keys on the CT, do not ship client secrets inside the binary. |
| ADR-0036 | The sentence “OAuth stays laptop-local” gains “unless the operator enabled OAuth mirror (ADR-0045)”. |

`docs/remote-agent-pairing.md`:

- Intro: OAuth stays laptop-local unless that companion’s **Mirror OAuth tokens** checkbox is on.
- Honesty row: `OAuth mirror | **Live, default off** (ADR-0045)`.
- Security note: Tailscale encrypts transit (`http://` on the tailnet IP is the same pattern as the telegram and llm vaults). Pairing secret authenticates. Files on the CT are mode `0600`. Do not bind a public interface.
- Mirror table: `PUT /v1/vault/oauth` → `vault/oauth.json`, only when the flag is on; flag off clears the file.
- Enable, revoke, reinstall, and publisher-client sections as in the operator section below. No real hostnames, home directories, or tokens. Keep `100.x.y.z` as the placeholder already used in this doc.

`docs/oauth-clients.md`: add a “Companion refresh” section. Same three variables. CT file path under the softwake home. Refresh does not add scopes. No secrets in the doc.

`CHANGELOG.md`: new bullet at the top of the first `### Added` under `## Unreleased`. Do not rewrite the older unreleased bullets (they record what those slices shipped).

`README.md` Remote Agent paragraph (~line 380): default off, checkbox, ADR-0045, link the pairing doc. One ADR index row if that table is extended; it currently ends before ADR-0039, so a single row for ADR-0045 next to the Remote Agent section is enough. Do not boil the ocean on the index.

`crates/softwake-node/README.md`: document `PUT`/`GET /v1/vault/oauth`, the `0600` file, the feature, the publisher env, and that empty vault refuses with the enable hint. Health stays open.

Public docs only. No shop paths, no absolute `/home/...`, no real tokens, no private MagicDNS names.

## 11. Tests (offline)

No live Google, Microsoft, xAI, or Telegram.

| Test | Where | Assert |
|---|---|---|
| Missing `oauth_mirror` deserializes false; `true` round-trips | `softwake-tools` | version stays 1 |
| Mirror JSON omits non-OAuth secrets | `softwake-daemon` | distinct sentinels |
| `Debug` redacts access and refresh | providers or daemon | sentinel absent |
| Clear document has empty arrays | daemon | |
| PUT without bearer → 401, file absent | `softwake-node` `main` tests, temp dir | |
| PUT with bearer writes `0600` and the sentinel is in the file | node | unix mode bits |
| GET body has counts and email, not `access_token` / sentinel | node | |
| PUT `{}` or empty arrays deletes the file | node | |
| PUT corrupt JSON → 400, previous file kept | node | |
| PUT larger than 256 KiB → 400, file kept | node | |
| Empty vault → enable hint, and the string contains `OAuth` | `agent.rs` | |
| Vault + `MockTransport` gmail list fixture → formatted inbox, result does not contain the access sentinel | `oauth_tools.rs` | |
| Expired access + scripted token POST → file gains the new access token, old refresh kept if the response omits refresh | node | `MockTransport::with_form` on `GOOGLE_TOKEN_URL` |
| `account=` picks the second Google row, not the active one | node | two fixtures |
| Ambiguous substring → error, no HTTP | node | |
| Ask permission still returns pending and does not call the runner | agent test, existing pattern | |
| Telegram: empty vault does not require tools; this can be a pure branch test on `companion_ask` if you pass a state with an empty dir | telegram | do not hit xAI |
| `merge_preserved_node_env` keeps an allowlisted line and drops a random `FOO=` | tools installer tests | render itself has no sentinel |
| `--no-default-features`: non-empty vault returns `live-http is required...` | node | separate cargo invocation |

`cargo test -p softwake-node --locked` uses default features (live-http on) but must not open a cloud socket. Do not `#[ignore]` a live OAuth test. Do not add one.

## 12. Out of scope (reject)

- Wider Drive or calendar scopes, Drive upload, or a new consent screen.
- Public ingress, `0.0.0.0`, non-Tailscale SSH, HTTPS terminator, or a second encryption layer.
- Mirroring the rest of `SecretBag`, or copying node-refreshed tokens back to the laptop.
- Per-profile OAuth bind.
- Changing `MAX_TOOL_ROUNDS`, presence grace, lease TTL, sticky-ownership rules, or Ask → silent allow.
- PROTOCOL_VERSION bump.
- New ctl subcommand.
- Force-push, production deploy, restart of `softwaked` or the UI, secret exfil, copying a real `oauth-clients.env` into git or into the PR.
- Rewriting laptop `cloud_tools.rs` onto `Transport` (follow-up, not this PR).
- Calling keyring on the node.

## 13. Quality gates

Run from the worktree. Locked. Do not start the user’s daemon or UI.

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test -p softwake-tools -p softwake-providers -p softwake-daemon -p softwake-node -p softwake-ui --locked
cargo test -p softwake-node --locked --no-default-features
cargo check -p softwake-daemon --locked --features live-http
```

`cargo clippy --workspace` uses default features, so the node’s live-http code is linted. Daemon live-http stays a separate `cargo check` (already the pattern). If clippy is newly noisy only inside `#[cfg(feature = "live-http")]` daemon code, do not drive-by fix it.

PR merge, when execution is authorized later: push the branch (no force), `gh pr create`, wait until CI is green, `gh pr merge --merge` (merge commit, not squash). If `gh` auth fails, use the repo’s MCP/SSH fallback. Do not hunt for tokens.

## 14. PR text (for the execute phase)

**Title:** `feat: opt-in OAuth mirror to the companion (ADR-0045)`

**Body:**

```markdown
## Summary

- Settings → Remote Agent gains **Mirror OAuth tokens to companion**, default off, with danger copy. The flag is `oauth_mirror` on `RemoteAgentConfig` (`remote-agents.json` version stays 1).
- While the flag is on, softwaked `PUT`s Google and Microsoft `AccountConnection` rows plus active ids to `PUT /v1/vault/oauth` on the existing ~45s Tailscale mirror (pairing-secret Bearer). Flag off sends an empty document and the node deletes `vault/oauth.json` (mode 0600).
- Companion `agent_task` runs `email_*` / `calendar_*` / `drive_*` from that vault when the tool is AlwaysAllow. Ask still returns pending text. Empty vault keeps refusing, and the message tells the operator to enable the checkbox. Telegram away replies stay a no-tools oneshot unless the vault has a usable connection, then they use the same agent turn.
- The node refreshes access tokens with the publisher client id (env or `oauth-clients.env` on the CT) and writes the refreshed row back. `account=` routing matches ADR-0033. Laptop tools are unchanged when the flag is off. PROTOCOL stays 1.

## Test plan

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --locked -- -D warnings`
- [ ] `cargo test -p softwake-tools -p softwake-providers -p softwake-daemon -p softwake-node -p softwake-ui --locked`
- [ ] `cargo test -p softwake-node --locked --no-default-features`
- [ ] `cargo check -p softwake-daemon --locked --features live-http`
- [ ] Confirmed no live Google/Microsoft call in CI
- [ ] Docs: ADR-0045, pairing honesty table, CHANGELOG, README, node README

## Security

Tokens sit on the companion disk (`0600`) and cross the tailnet as HTTP inside Tailscale, same as the Telegram and xAI vaults. Default remains off. Revoke: disable the checkbox (softwaked running), rotate the pairing secret, delete `vault/oauth.json` if a clear PUT cannot land.
```

## 15. Operator: enable

1. Settings → Email already has the Google and/or Microsoft accounts you want. This PR does not add a Connect button on the CT.
2. Confirm inbox/calendar/Drive scopes on those rows. Refresh will not add scopes. If a scope is missing, Remove and Connect on the laptop first.
3. Settings → Remote Agent: pairing secret set, agent **Enabled**, checkbox **Mirror OAuth tokens to companion** on, Save.
4. `softwaked` must be running. Within about 45 seconds it `PUT`s the rows. Only `first_enabled_agent` is the target. The flag on a disabled second agent does nothing.
5. On the CT, publisher client ids for refresh while the laptop is asleep (section 7). An unexpired access token works before that file exists; the first refresh will fail closed with `This build has no OAuth client configured` until it exists.
6. Reinstall / install the companion so the binary is built with `live-http` (default, and the installer passes the feature).
7. Tools that are Ask on the laptop stay Ask on the node (pending text, no send). To let an away turn actually list or send, set that tool to Always allow in Settings → Tools and wait for the tools mirror. Defaults: list tools are already Always allow; search, get, send, and calendar write are Ask.
8. Check without dumping tokens: `GET /v1/vault/oauth` with the pairing bearer. You should see counts and emails, not access tokens. Do not paste `oauth.json` into chat or tickets.

## 16. Operator: revoke

1. Leave the companion **Enabled** with the pairing secret still valid. Turn the checkbox **off**. Save.
2. Leave `softwaked` running until a mirror tick lands (about 45s). That PUT deletes `vault/oauth.json`.
3. Rotate the pairing secret (Settings → clear secret, set a new one, Save, reinstall so `/etc/softwake-node.env` matches). Old bearer then 401s.
4. If softwaked cannot reach the node (agent disabled, secret already rotated, laptop suspended), SSH to the CT and delete `/var/lib/softwake-node/vault/oauth.json` (and `oauth.json.tmp` if present). Root can read the file; deleting it is the real revoke.
5. Rotating the Google/Microsoft grant (Settings → Email → Remove, or the provider’s security page) invalidates refresh tokens the CT might still have.

Disabling the whole agent, or deleting it, stops the laptop from sending the clear PUT. The file remains until step 4.

## 17. Operator: reinstall

Same **Install companion** button or `softwaked ctl remote-agent install [id]`.

- Overwrites `/usr/local/bin/softwake-node` and restarts the unit (ADR-0044).
- Rewrites listen address, pairing secret, and `SOFTWAKE_NODE_DATA`.
- Preserves the allowlisted keys already in `/etc/softwake-node.env` (xAI key, model, publisher client ids) without printing them.
- Does not delete `/var/lib/softwake-node`, so `vault/oauth.json` and `oauth-clients.env` under that home survive. Reinstall is not a revoke.
- After reinstall, if the checkbox is still on, the next mirror tick replaces the vault from the laptop bag.

Build the binary from this repo (`--features live-http`). A `SOFTWAKE_NODE_BIN` or `PATH` binary built without the feature will store tokens and then refuse cloud calls.

## 18. Residual risks

- **CT compromise is token compromise.** Root on the CT can read mode `0600`. Danger copy exists because this is intentional.
- **Tailnet HTTP.** Anyone on the tailnet who has the pairing secret can `PUT` or `GET` metadata. They cannot `GET` the raw tokens. They can read the file if they are also root on the CT. Same model as the Telegram bot token vault.
- **Clear PUT needs a live enabled target.** Agent disabled, secret empty, or softwaked not running → stale `oauth.json` until manual delete.
- **Up to ~45s** after unchecking before the clear runs.
- **Refresh-token rotation.** The node writes a new refresh token if the provider returns one. The next laptop PUT replaces the whole file from the laptop bag and can put the old refresh token back. While the laptop is suspended the mirror thread does not run, so the node’s row sticks until wake, then gets overwritten. If the provider invalidated the old refresh token, both sides fail until Settings → Email → Remove → Connect. This PR does not merge node tokens back into the laptop bag (that is bidirectional sync, rejected).
- **Microsoft refresh** needs `SOFTWAKE_MICROSOFT_CLIENT_ID` on the CT. Google needs its client id, and the secret only when the desktop client has one.
- **Ask tools do not run away** until the operator sets Always allow. A scheduled “check my mail” works for `email_list` under defaults; “search” and “send” wait for a laptop approval that will not happen while away.
- **First enabled agent only.** A second enabled companion is ignored, same as schedules and the telegram vault.
- **HTTP body cap** is checked on the OAuth route after the existing reader has already buffered the body. A hostile client that already has the pairing secret can still make the process allocate. Not a new unauthenticated hole. Do not rewrite the HTTP parser in this PR.
- **No second cipher.** Disk is plaintext `0600`, same as `telegram_bot_token`.
- **Installer preserve** only keeps the allowlist. A typo’d key name is dropped on reinstall.
- **Default-on `live-http`** means the node binary can open HTTPS to Google, Microsoft, xAI, and Telegram. It already could open xAI and Telegram. Cloud tool calls still require a vault.

## 19. Implementation order

1. `OauthMirrorDocument` + null-as-empty + providers export + debug/serde tests.
2. `RemoteAgentConfig.oauth_mirror` and every struct literal. Tools tests.
3. UI snapshot/save/HTML/JS. No window launch.
4. Node vault methods, routes, 0600, GET redaction, auth tests.
5. `Transport` default methods + `MockTransport` / `LiveTransport` overrides. Providers tests still pass.
6. `oauth_tools.rs` + `invoke_companion` + system-prompt status + Telegram branch. Mock tests.
7. Node `Cargo.toml` feature, default on. `--no-default-features` test.
8. Daemon `mirror_oauth_vault` + 45s call + comment. Payload tests.
9. Installer: `--features live-http`, env comments, `merge_preserved_node_env`, remote script does not print the old file. Unit test the merge.
10. ADR-0045 and the amendments, pairing doc, oauth-clients, CHANGELOG, README, node README.
11. Format, clippy, the test commands in section 13. Do not restart processes.

## 20. Files to touch

| Path | Change |
|---|---|
| `crates/softwake-providers/src/account_oauth.rs` or `secrets.rs` | `OauthMirrorDocument` |
| `crates/softwake-providers/src/lib.rs` | export |
| `crates/softwake-providers/src/transport.rs` | default methods + mock |
| `crates/softwake-providers/src/live.rs` | live patch/delete/headers |
| `crates/softwake-tools/src/remote_agents.rs` | flag |
| `crates/softwake-tools/src/remote_agent_install.rs` | literals, cargo features, env merge |
| `crates/softwake-ui/src/remote_agent.rs` | snapshot/save |
| `crates/softwake-ui/ui/index.html` | checkbox + danger copy |
| `crates/softwake-ui/ui/app.js` | load/save |
| `crates/softwake-daemon/src/remote_agent.rs` | `mirror_oauth_vault` |
| `crates/softwake-node/Cargo.toml` | deps + feature |
| `crates/softwake-node/src/state.rs` | vault file + mutex |
| `crates/softwake-node/src/main.rs` | routes |
| `crates/softwake-node/src/oauth_tools.rs` | new |
| `crates/softwake-node/src/agent.rs` | invoke + prompt |
| `crates/softwake-node/src/telegram.rs` | branch into `run_agent_turn` only when vault is usable |
| `crates/softwake-node/src/lib` is not a lib; `main.rs` declares `mod oauth_tools;` | |
| `docs/ADR-0045-oauth-mirror.md` | new |
| `docs/ADR-0033-multi-account-oauth.md` | amendment |
| `docs/ADR-0036-agent-task-cron.md` | one sentence |
| `docs/ADR-0039-remote-agent.md` | amendment |
| `docs/ADR-0040-remote-agent-presence-outbox.md` | amendment |
| `docs/ADR-0042-telegram-sticky-ownership.md` | amendment |
| `docs/ADR-0043-companion-agent-task-llm.md` | amendment |
| `docs/ADR-0044-remote-agent-ssh-installer.md` | amendment |
| `docs/remote-agent-pairing.md` | honesty, enable, revoke, reinstall |
| `docs/oauth-clients.md` | companion refresh |
| `CHANGELOG.md` | unreleased bullet |
| `README.md` | Remote Agent paragraph |
| `crates/softwake-node/README.md` | endpoint + env |

Do not edit laptop `cloud_tools.rs`, `email_send.rs`, `calendar_write.rs`, or `dispatch.rs` except a compile fix if a `Transport` default method is somehow not enough (it should be enough).

---

## Ready to execute

- [ ] Baseline re-checked at `ba99e15` on `feat/oauth-mirror` (this plan’s table).
- [ ] Pick confirmed: node `live-http` default on + slim `Transport` runner; not a new crate; laptop `with_account` untouched.
- [ ] Pick confirmed: clear-on-disable via empty `PUT` on the 45s loop; corrupt PUT does not delete.
- [ ] `oauth_mirror` default false, document version stays 1, checkbox + danger copy wired through save/load.
- [ ] `PUT /v1/vault/oauth` Bearer-only, 0600 file, redacted GET, 256 KiB cap, no token logs.
- [ ] Empty vault refuses with the enable hint. Non-empty + AlwaysAllow runs email/calendar/drive. Ask still pending.
- [ ] Refresh persists the merged row. Publisher client documented. Missing client id is an error, not a panic.
- [ ] `account=` uses `SecretBag::resolve_account` on the four fields only.
- [ ] Telegram oneshot unchanged when the vault is empty; `run_agent_turn` only when a usable connection is stored.
- [ ] Installer passes `--features live-http` and preserves allowlisted env keys on the CT without copying them to the laptop.
- [ ] ADR-0045 + amendments + pairing honesty + CHANGELOG + README. PROTOCOL stays 1.
- [ ] Offline tests listed in section 11. No live Google/Microsoft.
- [ ] Scope rejects in section 12 honored (no force-push, no softwaked restart, no public ingress, no scope widen).
- [ ] PR title and body from section 14. Merge with `gh pr merge --merge` only after CI is green, and only when execution is authorized.
- [ ] Execute report later: `.grok-briefs/oauth-mirror-EXECUTE-REPORT.md` with PR number, merge SHA, enable, revoke, reinstall, residual risks.
