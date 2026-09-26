# ADR-0026 — HUD chat history + UI passphrase unlock

- **Status:** Accepted
- **Date:** 2026-09-27

## Context

The Softwake HUD already keeps an in-memory ring of chat bubbles (cap 40) for
the operator to scroll. That ring disappeared on UI restart, and there was no
at-rest protection for retained turns. Operators want per-profile display
history and an optional passphrase gate — without feeding HUD history back into
the model, and without involving softwaked.

## Decision

1. **Store** each profile’s HUD turns in
   `$XDG_CONFIG_HOME/softwake/profiles/<id>/hud-chat.json` via
   `softwake_soul::profile_pack_dir`. Same tree as soul pack / `schedules.json`.
2. **Display only.** HUD history is never replayed into `softwake-session`.
   ADR-0021 compaction and sleep/hibernate session clears stay unchanged.
3. **Cap 40** turns (matches HUD `MAX_TURNS`). Atomic write `*.json.tmp` + rename,
   mode `0600` best-effort on Unix.
4. **Vault metadata** in `$XDG_CONFIG_HOME/softwake/ui-vault.json`:
   `mode` (`unset` | `plaintext` | `passphrase`), Argon2id salt, AEAD verifier,
   optional `keyring_wrap`. No raw key and no chat bytes in that file.
5. **Crypto (passphrase mode):** Argon2id (19 MiB, t=2, p=1) → 32-byte data key;
   ChaCha20-Poly1305 per write (random 12-byte nonce). Softwake-ui holds the key
   in process memory (`zeroize` on lock/drop). softwaked never sees passphrase
   or HUD history.
6. **First run:** Skip allowed → `plaintext` until the operator sets a
   passphrase. Setting a passphrase encrypts existing plaintext chat files for
   all profiles.
7. **Optional OS keyring wrap:** service `softwake` / user `ui-data-key` stores
   the data key (not chat bytes, not the provider secret bag). Best-effort;
   missing keyring → passphrase dialog only.
8. **UX:** HUD unlock overlay on start when locked; Settings → General “Chat
   lock” for status / set / change / lock-now / keyring checkbox.
9. **PROTOCOL** stays 1. No new `ClientMessage`.

## Consequences

- Softwake-ui gains `argon2`, `chacha20poly1305`, `rand`, `base64`, `zeroize`,
  and a direct `keyring` dependency for the UI wrap helper.
- Encrypted blobs in v1 are HUD chat history only (not `ui-prefs.json`,
  schedules, skills, or the secret bag).
- Operators who Skip see a plaintext warning until they set a passphrase.
