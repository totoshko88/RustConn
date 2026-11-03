---
inclusion: fileMatch
fileMatchPattern: "rustconn-core/src/secret/**/*.rs,rustconn-core/src/ssh_agent/**/*.rs,rustconn/src/window/rdp_vnc.rs,rustconn/src/vault_ops.rs"
---

# Secrets — Security Rules

You are editing a file that handles credentials.

## Mandatory

1. Passwords → `secrecy::SecretString`, never plain `String`
2. Intermediate password Strings → wrap in `zeroize::Zeroizing::new()` or call `.zeroize()` after use
3. External CLIs → password via stdin pipe (`Command::new().stdin(Stdio::piped())`), NEVER via `.arg(password)`
4. Error messages → MUST NOT include secret values
5. Logging → MUST NOT log password/secret/token variables via `tracing`/`println!`/`dbg!`
6. GUI layer: `expose_secret().to_string()` → always wrap in `Zeroizing::new()`

## Timeouts

- Vault operations (store/retrieve/delete) → 10s timeout for local backends
  (keyring, KDBX, encrypted files); 45s for the CLI-backed ones (Bitwarden,
  1Password, Passbolt, pass) — `vault_op_timeout` in `vault_ops.rs`, measured in
  issue #312
- Credential resolution → 30s timeout
- Bitwarden auto_unlock → 30s timeout
- Backend availability probes (Settings → Secrets detection, startup banner
  via `detect_secret_backends`, off the GTK main thread) → 5s timeout. Never
  `block_on` an availability probe on the GTK main thread

## SSH key passphrases and the agent

- A key passphrase is used for one `ssh-add` and then dropped. Never cache it
  in memory for the session, and never store it in or fetch it from a vault.
- A key RustConn adds on its own (`SshAgentManager::add_own_key`,
  `sftp::ensure_identity_loaded`) goes in with `ssh-add -t` `OWN_KEY_LIFETIME`
  (12 h) and is recorded in `ssh_agent::own_keys`. `evict_own_keys` removes
  exactly those, one native `SSH_AGENTC_REMOVE_IDENTITY` each to the socket the
  add used, on exit, screen lock and suspend (`lock_events`).
- Never touch the user's keys: no `ssh-add -D`, no `REMOVE_ALL_IDENTITIES`, no
  re-add of a key the agent already holds. A Settings → SSH Agent "Add" is the
  user's (`add_key`) and un-records the key.

## New Backends

Implement `SecretBackend` trait → register in `secret/mod.rs`
