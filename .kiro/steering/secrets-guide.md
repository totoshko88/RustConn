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
  in memory for the session, and never store it in or fetch it from a vault
  (maintainer rule, 2026-10-23): SSH login must not depend on the vault.
  `Credentials` has no passphrase field for that reason; do not add one back.
- Releases before 0.24.6 stored a `key_passphrase` field. Leave it in users'
  vaults — no migration, no cleanup pass; a delete cannot be undone — and
  ignore it. No backend names the field at all, `delete` included: a rename
  or move is `retrieve` + `store` + `delete`, so a `delete` that took the
  field along would destroy the only copy. After a rename, a move or a
  credential delete the old item stays behind, orphaned. The encrypted-file
  payload (`StoredCredentials`) keeps the field as
  `#[serde(default, skip_serializing)]`, wiped on drop and never handed on,
  so an old entry decrypts and the next rewrite of that entry (save, rename,
  move, migration between the two file stores) drops the field.
  A user removes old ones with `secret-tool clear application rustconn key
  key_passphrase`, `pass rm rustconn/<id>/key_passphrase`, or
  `security delete-generic-password -s rustconn -a '<id>/key_passphrase'`
  (CHANGELOG 0.24.6).
- A key RustConn adds on its own (`SshAgentManager::add_own_key`,
  `sftp::ensure_identity_loaded`) goes in with `ssh-add -t` `OWN_KEY_LIFETIME`
  (12 h). `own_keys::add_and_record` asks the agent which identities it holds,
  adds, asks again, and records the key once the agent shows it — all on one
  socket (for SFTP: `AGENT_INFO`, then `SSH_AUTH_SOCK`). Record only a blob
  read from the private key file itself (`KeyFileBlobs::key_file`), never one
  from a `.pub`, which may be stale and is not read at all: a key without it
  (legacy PEM/PKCS#8) is added only into an empty agent and not recorded, and
  its lifetime is its only expiry. Its `<key>-cert.pub` blob
  (`KeyFileBlobs::cert_file`) is recorded too when it is new after the add.
  `evict_own_keys` removes exactly the recorded ones, one native
  `SSH_AGENTC_REMOVE_IDENTITY` each to that socket, on exit, screen lock and
  suspend (`lock_events`). The packages grant no extra permission for the
  listener: the Flatpak sees lock and suspend only after the user's opt-in
  `flatpak override` (`docs/INSTALL.md`), the Snap sees lock through `desktop`
  and never suspend. Exit and the 12 h lifetime are the only bounds that hold
  everywhere; do not add sandbox permissions for the listener without the
  maintainer.
- The automatic `ssh-add` (`add_own_key`) is bounded (`proc::wait_bounded`),
  has no stdin, no host `SSH_ASKPASS` and `SSH_ASKPASS_REQUIRE=never` (else
  OpenSSH falls back to its compiled-in askpass when `DISPLAY`/`WAYLAND_DISPLAY`
  is set): it runs on the GTK thread.
- The key-unlock dialog (`dialogs/key_unlock.rs`, gated by
  `window/key_unlock.rs` → `ssh_agent::check_key_unlock`) is the one place a
  connect asks for a key passphrase. It clears the entry when it reads it and
  moves the `SecretString` into the off-thread worker running
  `SshAgentManager::add_own_key_with_passphrase`, where it drops after the
  add. Only that function takes it, and only through the one-shot askpass
  secret file (0600, path in env, zeroized and unlinked before return); never
  argv, never an env value, never a log line, never a vault (no store, no
  fetch). Agents that refuse adds are remembered per socket path
  (`ssh_agent::agent_refused_add`) and get no dialog again; nothing secret is
  kept there. Never detect or answer a passphrase prompt by watching terminal
  output.
- Every other agent child (`ssh-agent -s`, the `ssh-add -l`/`-L`/`-d`
  fallbacks) goes through `ssh_agent::run_agent_command` with
  `AGENT_COMMAND_TIMEOUT` (5 s); never `.output()` or `wait_with_output()` on
  one. Prefer the native `wire` request (key removal, lookup by fingerprint)
  when the public blob is known. The socket connect is bounded too
  (`wire::connect_bounded`: a `socket2` socket with `SO_SNDTIMEO` and a
  blocking connect on the caller's thread, no `unsafe`, no worker thread;
  Linux waits a full listen queue out for the send timeout, macOS refuses
  it at once).
- Eviction keeps a key recorded only on a transient error (`TimedOut`,
  `WouldBlock`, `Interrupted`); a lasting one drops it with one `warn`. A
  `forget` during an eviction wins: the in-flight key is not put back.
  Each process (GUI, `rustconn-cli`) has its own record and no IPC: a key one
  process added and the other then found in the agent is removed by the first
  one's eviction. Documented, not fixed.
- Never touch the user's keys: no `ssh-add -D`, no `REMOVE_ALL_IDENTITIES`, no
  re-add of a key the agent already holds. When the agent query fails, do not
  add (`AddKeyFailed`); when its answer cannot settle it (no `key_file` blob
  and any key present), do not add either (`KeyPresenceUnknown`,
  `IdentityLoadError::Unverified`: the mc launch warns and goes on).
  A Settings → SSH Agent "Add" is the user's (`add_key`) and un-records the
  key, before and after the `ssh-add`; without the key file's own blob,
  every key recorded for that socket. A `REMOVE_IDENTITY` already sent can
  still land after the add; no shared lock, by decision (w6).

## New Backends

Implement `SecretBackend` trait → register in `secret/mod.rs`
