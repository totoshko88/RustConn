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
- The automatic `ssh-add` is bounded (`proc::wait_bounded`), has no stdin, no
  host `SSH_ASKPASS` and `SSH_ASKPASS_REQUIRE=never` (else OpenSSH falls back
  to its compiled-in askpass when `DISPLAY`/`WAYLAND_DISPLAY` is set): it runs
  on the GTK thread.
- Every other agent child (`ssh-agent -s`, the `ssh-add -l`/`-L`/`-d`
  fallbacks) goes through `ssh_agent::run_agent_command` with
  `AGENT_COMMAND_TIMEOUT` (5 s); never `.output()` or `wait_with_output()` on
  one. Prefer the native `wire` request (key removal, lookup by fingerprint)
  when the public blob is known. The socket connect is bounded too
  (`wire::connect_bounded`, a worker thread, since `UnixStream` has no
  connect timeout and the bounded FFI would be `unsafe`).
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
  key; without the key file's own blob, every key recorded for that socket.

## New Backends

Implement `SecretBackend` trait → register in `secret/mod.rs`
