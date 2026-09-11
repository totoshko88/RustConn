//! SSH connection launch and reconnect logic.
//!
//! Extracted from `window/protocols.rs` to reduce module complexity.

use std::rc::Rc;

use gtk4::prelude::*;
use rustconn_core::connection::{check_port, ssh_inheritance};
use secrecy::SecretString;
use uuid::Uuid;

use super::MainWindow;
use super::protocols::{
    SharedNotebook, SharedSidebar, append_proxy_command_destination, contains_ssh_failure,
    resolve_automation_for_connection, substitute_variables,
};
use crate::state::SharedAppState;
use crate::utils::spawn_blocking_with_callback;

/// Environment variable carrying the target account password to the outer
/// OpenSSH process. Nested proxy hops explicitly clear it before starting SSH.
const TARGET_PASSWORD_ENV: &str = "_RC_TGT_PW_FILE";

/// Environment variable carrying the jump host (bastion) password to the
/// `SSH_ASKPASS` helper. Intentionally obscure to reduce exposure in
/// `/proc/<pid>/environ`, matching the SSH tunnel askpass convention.
const JUMP_HOST_PW_ENV: &str = "_RC_JH_PW_FILE";

/// Returns the path to the prompt-aware helper for the target account password.
fn target_password_askpass_script() -> Option<std::path::PathBuf> {
    askpass_script_for_env(TARGET_PASSWORD_ENV)
}

/// Returns the path to a reusable `SSH_ASKPASS` helper that reads the jump
/// host secret-file path from the given env var name.
///
/// The script holds no secret — only the env var name — so it is safe to keep
/// for the process lifetime and share across sessions. The password is written
/// directly from `SecretString` to a randomized mode-0600 runtime file; the env
/// carries only its path, and the helper opens then unlinks it before output.
/// The script itself is placed in `$XDG_RUNTIME_DIR` (mode 0700) to avoid `/tmp`
/// symlink races on a fixed filename, falling back to a randomized temp path.
/// Created once (mode 0700) and cached per env var name; returns `None` if
/// creation fails.
///
/// For the legacy single-hop case (env var `_RC_JH_PW_FILE`), the script is the
/// same singleton as before. For multi-hop (issue #203), each deeper hop gets
/// its own script reading `_RC_JH_PW_FILE_1`, `_RC_JH_PW_FILE_2`, etc.
fn jump_host_askpass_script() -> Option<std::path::PathBuf> {
    askpass_script_for_env(JUMP_HOST_PW_ENV)
}

/// Points the session's Backspace and Delete keys at what this host expects.
///
/// The bytes are a property of the VTE widget rather than of the `ssh` command,
/// so this runs against the live session instead of adding arguments. The pair
/// itself comes from [`rustconn_core::ProtocolConfig::erase_modes`], the single
/// mapping shared with Telnet, MOSH and the Preferences re-apply pass, so a
/// connection that is not a terminal protocol — or never changed the setting —
/// gets the same defaults as before (issue
/// [#271](https://github.com/totoshko88/RustConn/issues/271)).
fn apply_ssh_erase_mode(
    notebook: &SharedNotebook,
    session_id: Uuid,
    conn: &rustconn_core::Connection,
) {
    let (backspace_sends, delete_sends) = conn.protocol_config.erase_modes();
    notebook.set_erase_mode(session_id, backspace_sends, delete_sends);
}

/// Inserts an opaque proxy hop while preserving host-to-credential alignment.
fn insert_opaque_jump_host(
    jump_hosts: &mut Vec<String>,
    hop_ids: &mut Vec<Option<Uuid>>,
    index: usize,
    host: String,
) {
    debug_assert_eq!(jump_hosts.len(), hop_ids.len());
    jump_hosts.insert(index, host);
    hop_ids.insert(index, None);
}

/// Parses a jump-host string (`[user@]host[:port]`) and returns `(host, port)`
/// for `ssh_control_path()`. Handles IPv6 in brackets.
fn parse_jump_host_for_control(jump_host: &str) -> (String, u16) {
    // Strip user@ prefix
    let host_port = if let Some(at_pos) = jump_host.rfind('@') {
        &jump_host[at_pos + 1..]
    } else {
        jump_host
    };

    let (host, port_str) = if host_port.starts_with('[') {
        // IPv6: [addr]:port
        if let Some(bracket_end) = host_port.find(']') {
            let after = &host_port[bracket_end + 1..];
            if let Some(p) = after.strip_prefix(':') {
                (&host_port[1..bracket_end], Some(p))
            } else {
                (&host_port[1..bracket_end], None)
            }
        } else {
            (host_port, None)
        }
    } else if let Some(colon_pos) = host_port.rfind(':') {
        let maybe_port = &host_port[colon_pos + 1..];
        if !maybe_port.is_empty() && maybe_port.bytes().all(|b| b.is_ascii_digit()) {
            (&host_port[..colon_pos], Some(maybe_port))
        } else {
            (host_port, None)
        }
    } else {
        (host_port, None)
    };

    let port = port_str.and_then(|p| p.parse().ok()).unwrap_or(22);
    (host.to_string(), port)
}

/// Builds a helper that releases a credential only for OpenSSH's account-password prompt.
///
/// Host-key confirmation, private-key passphrases, keyboard-interactive challenges, OTP/token
/// prompts, and password-change prompts all exit non-zero without printing the credential.
fn askpass_script_contents(env_var_name: &str) -> String {
    format!(
        "#!/bin/sh\ncase \"${{1-}}\" in\n  *\"'s password:\"*)\n    secret_file=\"${{{env_var_name}}}\"\n    [ -n \"$secret_file\" ] || exit 1\n    exec 3<\"$secret_file\" || exit 1\n    rm -f \"$secret_file\"\n    cat <&3\n    ;;\n  *) exit 1 ;;\nesac\n"
    )
}

/// Creates (or returns cached) an askpass script that prints the value of
/// `env_var_name`. Each unique env var gets its own on-disk script so that
/// nested ProxyCommand hops read their own password (issue #203).
fn askpass_script_for_env(env_var_name: &str) -> Option<std::path::PathBuf> {
    use std::sync::Mutex;
    static SCRIPTS: std::sync::OnceLock<
        Mutex<std::collections::HashMap<String, std::path::PathBuf>>,
    > = std::sync::OnceLock::new();
    let cache = SCRIPTS.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    let mut map = cache.lock().ok()?;

    if let Some(path) = map.get(env_var_name) {
        return Some(path.clone());
    }

    // Shared resolver: `$XDG_RUNTIME_DIR` on Linux, `$TMPDIR` on macOS. When a
    // user-private directory exists the script gets a stable per-env name so a
    // reconnect reuses it; only when the resolver declines (a Linux box with no
    // runtime dir) do we fall back to a randomized `temp_dir()` path to avoid a
    // fixed name in world-writable `/tmp`.
    let path = match rustconn_core::secret_file_dir() {
        Some(dir) => {
            if env_var_name == TARGET_PASSWORD_ENV {
                dir.join("rustconn-target-askpass.sh")
            } else if env_var_name == JUMP_HOST_PW_ENV {
                dir.join("rustconn-jh-askpass.sh")
            } else {
                // Unique file per hop index: rustconn-jh-askpass-1.sh, etc.
                let suffix = env_var_name
                    .strip_prefix("_RC_JH_PW_FILE_")
                    .unwrap_or(env_var_name);
                dir.join(format!("rustconn-jh-askpass-{suffix}.sh"))
            }
        }
        None => std::env::temp_dir().join(format!("rc-askpass-{}.sh", Uuid::new_v4())),
    };

    let script = askpass_script_contents(env_var_name);
    if let Err(e) = std::fs::write(&path, script.as_bytes()) {
        tracing::error!(error = %e, "Failed to create SSH askpass script");
        return None;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)) {
            tracing::error!(error = %e, "Failed to chmod SSH askpass script");
            return None;
        }
    }

    map.insert(env_var_name.to_string(), path.clone());
    Some(path)
}

fn create_askpass_secret_file(password: &SecretString) -> std::io::Result<std::path::PathBuf> {
    use secrecy::ExposeSecret;
    use std::io::Write;

    // Shared resolver: `$XDG_RUNTIME_DIR` on Linux, `$TMPDIR` on macOS (which has
    // no runtime dir). Only when it declines — a Linux box with no runtime dir,
    // unusual — do we accept the weaker `temp_dir()` so a password can still be
    // delivered rather than dropped.
    let directory = rustconn_core::secret_file_dir().unwrap_or_else(std::env::temp_dir);
    let path = directory.join(format!("rustconn-askpass-secret-{}", Uuid::new_v4()));
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let mut file = options.open(&path)?;
    if let Err(error) = file.write_all(password.expose_secret().as_bytes()) {
        drop(file);
        let _ = std::fs::remove_file(&path);
        return Err(error);
    }
    drop(file);
    Ok(path)
}

/// Builds the per-child askpass environment for one SSH launch.
///
/// Concurrency invariant (why this cannot race between simultaneous sessions):
/// the environment is returned as a plain `Vec` and applied only to the one
/// child process spawned for this connection (through `build_child_env` →
/// the VTE spawn's env vector). Nothing here touches the *process-global*
/// environment — there is deliberately no `std::env::set_var`, which Rust 2024
/// forbids anyway. Although the variable *names* are fixed (`_RC_TGT_PW_FILE`,
/// `_RC_JH_PW_FILE`), their *values* are freshly randomised secret-file paths
/// from `create_askpass_secret_file`, so two connections opened at the same time
/// each carry their own file in their own child environment and cannot read each
/// other's password. Keep it that way: never promote these to a global set.
fn ssh_askpass_env(
    jump_host_passwords: &[(String, SecretString)],
    target_password: Option<(&SecretString, &std::path::Path)>,
) -> (Vec<zeroize::Zeroizing<String>>, Vec<std::path::PathBuf>) {
    let mut env = Vec::with_capacity(jump_host_passwords.len() + 3);
    let mut cleanup_paths = Vec::with_capacity(jump_host_passwords.len() + 1);
    for (env_name, password) in jump_host_passwords {
        match create_askpass_secret_file(password) {
            Ok(path) => {
                env.push(zeroize::Zeroizing::new(format!(
                    "{env_name}={}",
                    path.display()
                )));
                cleanup_paths.push(path);
            }
            Err(error) => {
                tracing::error!(%error, "Failed to create jump host askpass secret file");
            }
        }
    }
    if let Some((password, script)) = target_password {
        match create_askpass_secret_file(password) {
            Ok(path) => {
                env.push(zeroize::Zeroizing::new(format!(
                    "{TARGET_PASSWORD_ENV}={}",
                    path.display()
                )));
                cleanup_paths.push(path);
                env.push(zeroize::Zeroizing::new(format!(
                    "SSH_ASKPASS={}",
                    script.display()
                )));
                env.push(zeroize::Zeroizing::new(
                    "SSH_ASKPASS_REQUIRE=force".to_string(),
                ));
            }
            Err(error) => {
                tracing::error!(%error, "Failed to create target askpass secret file");
            }
        }
    }
    (env, cleanup_paths)
}

/// Resolves the identity file for a connection that authenticates with a key
/// picked from the SSH agent.
///
/// The dialog stores the choice as `SshKeySource::Agent { fingerprint, .. }`
/// (mirrored in the legacy `agent_key_fingerprint` field), but until now nothing
/// consumed it: `ssh` was launched with no `-i` at all, so the agent offered
/// every key it held and the picker had no effect on the connection.
///
/// Passing the *private* key path is not an option — it makes ssh try file-based
/// auth first and produces a second agent confirmation prompt (issue #125).
/// Instead the selected key's public half is written to a file and used as the
/// identity; ssh then signs through the agent with exactly that key.
///
/// Returns `None` when the connection does not use an agent key, when no agent
/// is reachable, or when the agent no longer holds the selected key — in that
/// case the caller falls back to the previous behaviour (all agent keys offered)
/// rather than failing the connection.
fn agent_identity_file(ssh_config: &rustconn_core::SshConfig) -> Option<String> {
    use rustconn_core::SshKeySource;
    use rustconn_core::ssh_agent::SshAgentManager;

    let fingerprint = match &ssh_config.key_source {
        SshKeySource::Agent { fingerprint, .. } if !fingerprint.trim().is_empty() => {
            fingerprint.trim()
        }
        // Legacy connections stored the choice only in `agent_key_fingerprint`.
        _ => ssh_config
            .agent_key_fingerprint
            .as_deref()
            .map(str::trim)
            .filter(|f| !f.is_empty())?,
    };

    match SshAgentManager::from_env().materialize_agent_identity(fingerprint) {
        Ok(path) => Some(path.to_string_lossy().to_string()),
        Err(e) => {
            // Fingerprints are public key material — safe to log.
            tracing::warn!(
                fingerprint,
                error = %e,
                "cannot restrict connection to the selected agent key; offering all agent keys"
            );
            None
        }
    }
}

/// Builds the SSH command pieces shared by initial connect and in-place
/// reconnect: the resolved identity file, the extra CLI args (including the
/// jump-host `ProxyCommand`/`-J` wiring and Flatpak known_hosts), whether
/// waypipe is used, and the resolved jump-host chain string (for monitoring).
///
/// Returns `(identity_file, extra_args, use_waypipe, jump_host_chain,
/// jump_host_passwords, target_askpass_allowed)`. Bastion passwords are set
/// only when per-hop helpers were wired into `ProxyCommand`; the last flag is
/// set only for an explicit password-only target with a non-empty cached
/// credential and no competing identity/auth override.
///
/// Callers expose bastion passwords via `_RC_JH_PW_FILE[_N]`. When the target flag
/// is set and the helper can be created, they expose the target password via
/// `_RC_TGT_PW_FILE` together with `SSH_ASKPASS` and `SSH_ASKPASS_REQUIRE=force`.
/// For non-SSH protocols this returns empty defaults.
///
/// Extracted from `start_ssh_connection` and `reconnect_ssh_in_place`, which
/// previously carried ~150 near-identical lines each — a fix to one path could
/// silently miss the other.
fn build_ssh_command_args(
    conn: &rustconn_core::Connection,
    connection_id: Uuid,
    state: &SharedAppState,
    groups: &[rustconn_core::ConnectionGroup],
    has_cached_target_password: bool,
) -> (
    Option<String>,
    Vec<String>,
    bool,
    Option<String>,
    Vec<(String, SecretString)>,
    bool,
) {
    let rustconn_core::ProtocolConfig::Ssh(ssh_config) = &conn.protocol_config else {
        return (None, Vec::new(), false, None, Vec::new(), false);
    };

    // Resolve key path via inheritance (connection → group → parent group → root)
    let key = ssh_inheritance::resolve_ssh_key_path(conn, groups)
        .and_then(|p| {
            // Resolve stale portal paths: if the stored path doesn't exist,
            // check the Flatpak SSH dir for a file with the same name.
            rustconn_core::resolve_key_path(&p)
        })
        .map(|p| p.to_string_lossy().to_string());

    // When the user picked a specific key from the SSH agent, honour it.
    // `resolve_ssh_key_path` returns None for agent sources by design — it deals
    // in private key paths — so the selected key is materialized as a PUBLIC key
    // file instead and `-i` points at that. See `agent_identity_file`.
    let (key, restrict_to_agent_key) = match key {
        Some(path) => (Some(path), false),
        None => match agent_identity_file(ssh_config) {
            Some(path) => (Some(path), true),
            None => (None, false),
        },
    };

    // A dynamic SOCKS forward stored with local_port == 0 means "pick a free
    // port now". Assign it before building the args so `-D <port>` carries a
    // real, non-colliding port; the chosen value is logged so the user can point
    // a browser at it. Resolving on a clone keeps the stored config untouched
    // (the sentinel must survive to the next connect). When no random forward is
    // configured this clones nothing extra of note and assigns nothing.
    let resolved_socks = if ssh_config.has_random_socks_forward() {
        let mut resolved = ssh_config.clone();
        match resolved.assign_random_socks_ports(rustconn_core::ssh_tunnel::find_free_port) {
            Ok(port) => {
                if let Some(port) = port {
                    tracing::info!(
                        socks_port = port,
                        "Assigned a random local SOCKS proxy port for the dynamic forward"
                    );
                }
                Some((resolved, port))
            }
            Err(e) => {
                // Could not find a free port — fall back to the config as stored
                // (a `-D 0` that OpenSSH itself rejects loudly), rather than
                // failing the whole connection here.
                tracing::error!(error = %e, "Could not pick a free SOCKS port; leaving the forward as configured");
                None
            }
        }
    } else {
        None
    };
    let effective_ssh_config = resolved_socks.as_ref().map_or(ssh_config, |(cfg, _)| cfg);

    // Use build_command_args() for all SSH-specific flags:
    // identity, IdentitiesOnly, proxy_jump, ControlMaster/Persist,
    // agent forwarding, X11, compression, custom options, port forwards
    let mut args = effective_ssh_config.build_command_args();

    // Unlike the old VTE watcher, forced askpass is scoped to OpenSSH's target
    // authentication phase: the helper answers only the prompt OpenSSH generates
    // for the `password` method and exits for anything else.
    //
    // The `ssh -G` term is last because it spawns a process: it only runs once every cheap
    // condition already holds. It answers whether OpenSSH's *own* config routes this host through
    // a proxy RustConn did not build — a nested ssh there would inherit the helper and be asked
    // for a password with the very shape the helper answers, so the target credential would land
    // on the bastion. Declining delivery leaves the connection to prompt, which is the documented
    // fallback; the alternative shipped in 0.21.2 was to append `ProxyJump=none`, which protected
    // the credential by discarding the user's routing.
    let target_askpass_allowed = has_cached_target_password
        && rustconn_core::ssh_tunnel::target_password_askpass_allowed(ssh_config, key.is_some())
        && !rustconn_core::ssh_tunnel::ssh_config_declares_proxy(
            &conn.host,
            conn.port,
            conn.username.as_deref(),
        );
    if target_askpass_allowed {
        // Two options, both about the *shape* of the launch rather than about
        // which method authenticates.
        //
        // `NumberOfPasswordPrompts=1` keeps the one-shot posture that stops a
        // wrong stored password from walking an account into a lockout.
        // `StrictHostKeyChecking=accept-new` keeps the host-key question away
        // from a forced helper, which would otherwise be handed it; a *changed*
        // key is still refused. The user's own value wins if they set one.
        //
        // Deliberately NOT pinned here — `PreferredAuthentications=password`,
        // `PubkeyAuthentication=no`, `KbdInteractiveAuthentication=no` and
        // `PasswordAuthentication=yes` were, until the 0.21.2 review, and each
        // one broke a working setup for no security gain. What keeps the
        // credential away from the wrong prompt is the helper's prompt matching,
        // not the method list: a method the helper refuses simply fails and
        // OpenSSH moves to the next one. Pinning instead meant a server that
        // offers keyboard-interactive could not authenticate at all, a key
        // inherited from a group was never offered, and a user's
        // `PasswordAuthentication no` was overridden from under them. With the
        // list left alone, OpenSSH tries keyboard-interactive first (its default
        // order), the helper declines that prompt without consuming the secret
        // file, and the `password` attempt that follows is the one it answers.
        args.push("-o".to_string());
        args.push("NumberOfPasswordPrompts=1".to_string());
        if !ssh_config
            .custom_options
            .keys()
            .any(|key| key.eq_ignore_ascii_case("StrictHostKeyChecking"))
        {
            args.push("-o".to_string());
            args.push("StrictHostKeyChecking=accept-new".to_string());
        }
    }

    // The agent identity is only meaningful together with IdentitiesOnly: without
    // it ssh keeps offering every other key the agent holds, which is exactly the
    // restriction the picker promises. `build_command_args` cannot add this — for
    // an agent source with no identity file, IdentitiesOnly would hide all agent
    // keys and break authentication outright.
    if restrict_to_agent_key
        && !args
            .windows(2)
            .any(|w| w[0] == "-o" && w[1] == "IdentitiesOnly=yes")
    {
        args.push("-o".to_string());
        args.push("IdentitiesOnly=yes".to_string());
    }

    // Remove -i <path> from args because the identity file is already
    // resolved separately via resolve_ssh_key_path() and passed as
    // `identity_file` to spawn_ssh(). Keeping both causes the key to
    // appear twice in the final command line.
    if key.is_some()
        && let Some(pos) = args.iter().position(|a| a == "-i")
    {
        args.remove(pos); // remove "-i"
        if pos < args.len() {
            args.remove(pos); // remove the path value
        }
    }

    // Resolve jump host chain from connection references (needs state access)
    let mut jump_hosts = Vec::new();
    // PKCS#11 provider of the immediate (first) jump hop, if it opts in.
    // `-o PKCS11Provider` is NOT inherited by ProxyJump child connections,
    // so it must be injected into the first hop's ProxyCommand explicitly.
    let mut first_hop_pkcs11: Option<String> = None;
    // Identity file of the immediate jump hop. `-J` does NOT inherit `-i` from
    // the outer ssh command, so when the bastion authenticates by key we must
    // switch to ProxyCommand and pass its own identity explicitly.
    let mut first_hop_identity: Option<String> = None;
    // Password of the immediate jump hop, resolved from its OWN cached
    // credentials. Without this the target connection's password is fed to the
    // bastion prompt (issue #191). Delivered to the bastion via SSH_ASKPASS on
    // the nested ProxyCommand ssh, NOT via the VTE prompt.
    let mut first_hop_password: Option<SecretString> = None;

    // Handle string-based proxy jump: the connection's own, or one inherited
    // from its group chain or from the global network settings.
    let network = state
        .try_borrow()
        .ok()
        .map(|s| s.settings().network.clone())
        .unwrap_or_default();
    // A dedicated/custom route is opaque and may win before a generated
    // ProxyCommand under OpenSSH's first-value semantics. Never combine it
    // with RustConn-managed hops or their credential environment.
    let has_unmanaged_proxy_route =
        rustconn_core::ssh_tunnel::has_unmanaged_proxy_route(ssh_config);
    if !has_unmanaged_proxy_route
        && let Some(proxy) = ssh_inheritance::resolve_ssh_proxy_jump(conn, groups, &network)
    {
        jump_hosts.push(proxy);
    }

    // Parallel vector: for each element in jump_hosts, the connection UUID
    // (if the hop is reference-based) so we can resolve its password later.
    let mut hop_ids: Vec<Option<Uuid>> = vec![None; jump_hosts.len()];

    // Handle reference-based jump host (recursive resolution).
    //
    // The first hop goes through `resolve_first_hop_id`, not `ssh_config
    // .jump_host_id`: a Jump Host picked on a group or in Preferences → Network
    // has to be honoured here, and reading the field directly is what made #301
    // still reproducible after 0.20.9 claimed to have fixed it. Hops further out,
    // below, keep reading their own field — see that function's docs for why that
    // asymmetry is deliberate.
    if !has_unmanaged_proxy_route
        && let Ok(state_ref) = state.try_borrow()
        && let Some(jump_id) = super::protocols::resolve_first_hop_id(&state_ref, conn)
    {
        let mut current_id = Some(jump_id);
        let mut visited = std::collections::HashSet::new();
        visited.insert(connection_id); // Avoid self-reference loop

        // Track whether the first REFERENCE hop (the jump_host_id chain) has
        // already had its own credentials resolved. We must NOT key this off
        // `jump_hosts.is_empty()`: a string `proxy_jump` may already occupy
        // `jump_hosts[0]`, which would make the heuristic think the first
        // reference hop is not the first hop and skip its password/PKCS#11
        // resolution entirely (issue #191, string+ref combo, Req 2.3).
        let mut first_ref_hop_resolved = false;

        // Limit recursion depth to avoid infinite loops
        for _ in 0..10 {
            if let Some(jid) = current_id {
                if visited.contains(&jid) {
                    break;
                }
                visited.insert(jid);

                if let Some(jump_conn) = state_ref.get_connection(jid) {
                    // The immediate hop is the one we ProxyCommand into.
                    // First reference hop = the first iteration of this chain,
                    // regardless of a pre-pushed string proxy_jump (Req 2.3).
                    let is_first_hop = !first_ref_hop_resolved;
                    // Format: [user@]host[:port]
                    let mut host_str = jump_conn.host.clone();
                    if let Some(user) = &jump_conn.username {
                        host_str = format!("{user}@{host_str}");
                    }
                    if jump_conn.port != 22 {
                        host_str = format!("{}:{}", host_str, jump_conn.port);
                    }
                    jump_hosts.push(host_str);
                    hop_ids.push(Some(jid));

                    // Check if this jump host has its own jumper
                    if let rustconn_core::ProtocolConfig::Ssh(jump_config) =
                        &jump_conn.protocol_config
                    {
                        // Opt-in PKCS#11 for the first hop (token to reach the bastion)
                        if is_first_hop {
                            // Mark the first reference hop as handled so later
                            // iterations of this chain are treated as deeper
                            // hops, independent of any string proxy in
                            // jump_hosts[0] (Req 2.3).
                            first_ref_hop_resolved = true;
                            first_hop_pkcs11 = jump_config
                                .pkcs11_provider
                                .clone()
                                .filter(|p| !p.trim().is_empty());
                            // Resolve the jump host's OWN identity file so it
                            // can be passed to ProxyCommand (issue #241: -J does
                            // NOT inherit -i from the outer ssh).
                            first_hop_identity =
                                rustconn_core::connection::ssh_inheritance::resolve_ssh_key_path(
                                    jump_conn, &groups,
                                )
                                .and_then(|p| rustconn_core::resolve_key_path(&p))
                                .map(|p| p.to_string_lossy().to_string());
                            // Resolve the bastion's OWN password (issue #191).
                            // First try the in-memory cache (fast path).
                            first_hop_password = state_ref
                                .get_cached_credentials(jid)
                                .filter(|c| {
                                    use secrecy::ExposeSecret;
                                    !c.password.expose_secret().is_empty()
                                })
                                .map(|c| c.password.clone());
                            // Fallback: resolve from vault/variable if not
                            // cached (issue #191). By this point the vault is
                            // already unlocked (target credentials were resolved
                            // first), so this is fast (~100ms). Honor the
                            // bastion's PasswordSource (Variable/Vault) via the
                            // shared resolver so a Variable-source bastion
                            // authenticates with ITS OWN password (Req 2.1).
                            if first_hop_password.is_none() {
                                let jump_conn_owned = jump_conn.clone();
                                let next_jump_id = jump_config.jump_host_id;
                                let manual_proxy = jump_config.proxy_jump.clone();
                                // Must drop state borrow before the blocking
                                // vault call, then re-borrow briefly to resolve.
                                drop(state_ref);
                                let resolved_pw = state.try_borrow().ok().and_then(|s| {
                                    s.resolve_connection_password_blocking(&jump_conn_owned)
                                });
                                if let Some(pw_secret) = resolved_pw {
                                    // Cache for future fast-path use.
                                    if let Ok(mut state_mut) = state.try_borrow_mut() {
                                        use secrecy::ExposeSecret;
                                        state_mut.cache_credentials(
                                            jid,
                                            jump_conn_owned.username.as_deref().unwrap_or(""),
                                            pw_secret.expose_secret(),
                                            "",
                                        );
                                    }
                                    first_hop_password = Some(pw_secret);
                                }
                                // Prepend manual proxy from first hop (saved before drop).
                                // Keep the parallel ID vector aligned so this opaque hop can
                                // never receive the following reference hop's credential.
                                if let Some(p) = manual_proxy {
                                    let insert_at = jump_hosts.len() - 1;
                                    insert_opaque_jump_host(
                                        &mut jump_hosts,
                                        &mut hop_ids,
                                        insert_at,
                                        p,
                                    );
                                }
                                // Continue collecting the rest of the chain if multi-hop.
                                // Re-borrow and resume from next_jump_id.
                                if let Some(nid) = next_jump_id
                                    && let Ok(state_ref2) = state.try_borrow()
                                {
                                    let mut cid = Some(nid);
                                    for _ in 0..9 {
                                        if let Some(id) = cid {
                                            if visited.contains(&id) {
                                                break;
                                            }
                                            visited.insert(id);
                                            if let Some(jc) = state_ref2.get_connection(id) {
                                                let mut hs = jc.host.clone();
                                                if let Some(u) = &jc.username {
                                                    hs = format!("{u}@{hs}");
                                                }
                                                if jc.port != 22 {
                                                    hs = format!("{}:{}", hs, jc.port);
                                                }
                                                jump_hosts.push(hs);
                                                hop_ids.push(Some(id));
                                                cid = match &jc.protocol_config {
                                                    rustconn_core::ProtocolConfig::Ssh(c) => {
                                                        c.jump_host_id
                                                    }
                                                    _ => None,
                                                };
                                            } else {
                                                break;
                                            }
                                        } else {
                                            break;
                                        }
                                    }
                                }
                                break;
                            }
                        }
                        // Prepend manual proxy if it exists on the jump host.
                        // Insert a matching empty ID to preserve host/credential alignment.
                        if let Some(p) = &jump_config.proxy_jump {
                            let insert_at = jump_hosts.len() - 1;
                            insert_opaque_jump_host(
                                &mut jump_hosts,
                                &mut hop_ids,
                                insert_at,
                                p.clone(),
                            );
                        }
                        current_id = jump_config.jump_host_id;
                    } else {
                        current_id = None;
                    }
                } else {
                    current_id = None;
                }
            } else {
                break;
            }
        }
    }

    // Resolve passwords for deeper hops (issue #203: multi-bastion password
    // chains). `first_hop_password` already covers `hop_ids[first_ref_index]`;
    // deeper hops need their own password resolved from cache or vault.
    // Build a parallel vec `hop_passwords` aligned with `jump_hosts`.
    let mut hop_passwords: Vec<Option<SecretString>> = vec![None; jump_hosts.len()];
    // Place first_hop_password at its correct index (first reference hop).
    // The first reference hop is the first entry with a Some(id) in hop_ids.
    if let Some(first_ref_idx) = hop_ids.iter().position(Option::is_some) {
        hop_passwords[first_ref_idx] = first_hop_password;
    }
    // Resolve passwords for remaining reference hops.
    for (idx, hop_id) in hop_ids.iter().enumerate() {
        if hop_passwords[idx].is_some() {
            continue; // already resolved (first hop)
        }
        if let Some(hid) = hop_id {
            // Try cache first (fast path).
            let cached = state
                .try_borrow()
                .ok()
                .and_then(|s| s.get_cached_credentials(*hid).cloned())
                .and_then(|c| {
                    use secrecy::ExposeSecret;
                    if c.password.expose_secret().is_empty() {
                        None
                    } else {
                        Some(c.password.clone())
                    }
                });
            if cached.is_some() {
                hop_passwords[idx] = cached;
                continue;
            }
            // Fallback: resolve from vault/variable.
            let conn_for_hop = state
                .try_borrow()
                .ok()
                .and_then(|s| s.get_connection(*hid).cloned());
            if let Some(hop_conn) = conn_for_hop {
                let resolved_pw = state
                    .try_borrow()
                    .ok()
                    .and_then(|s| s.resolve_connection_password_blocking(&hop_conn));
                if let Some(pw_secret) = resolved_pw {
                    if let Ok(mut state_mut) = state.try_borrow_mut() {
                        use secrecy::ExposeSecret;
                        state_mut.cache_credentials(
                            *hid,
                            hop_conn.username.as_deref().unwrap_or(""),
                            pw_secret.expose_secret(),
                            "",
                        );
                    }
                    hop_passwords[idx] = Some(pw_secret);
                }
            }
        }
    }

    // In Flatpak, ~/.ssh is read-only — point known_hosts to a writable path.
    // Must be set BEFORE jump host resolution because ProxyCommand needs it too.
    let flatpak_known_hosts = {
        let user_set = ssh_config
            .custom_options
            .keys()
            .any(|k| k.eq_ignore_ascii_case("UserKnownHostsFile"));
        if user_set {
            None
        } else {
            rustconn_core::get_flatpak_known_hosts_path()
        }
    };
    if let Some(ref kh_path) = flatpak_known_hosts {
        tracing::debug!(
            protocol = "ssh",
            path = %kh_path.display(),
            "Using Flatpak-writable known_hosts"
        );
        args.push("-o".to_string());
        args.push(format!("UserKnownHostsFile={}", kh_path.display()));
    }

    // Override proxy_jump with resolved jump host chain if we have
    // reference-based jump hosts (build_command_args already added -J
    // for the string-based proxy_jump, so only add if we have more)
    //
    // In Flatpak, -J (ProxyJump) spawns a nested SSH process that does NOT
    // inherit -o or -i flags from the outer command. This means the jump host
    // SSH tries to write to ~/.ssh/known_hosts (read-only) and cannot find
    // identity files. Fix: replace -J with -o ProxyCommand that passes
    // UserKnownHostsFile and identity to the jump host SSH process.
    // Passwords to deliver to each hop via per-hop SSH_ASKPASS (issue #191/#203).
    // Each entry: (env_var_name, password). Set only when askpass helpers are
    // successfully wired into ProxyCommand.
    let mut jump_host_passwords: Vec<(String, SecretString)> = Vec::new();

    let jump_host_str = if jump_hosts.is_empty() {
        None
    } else {
        // Remove the -J added by build_command_args (if proxy_jump was set)
        if ssh_config.proxy_jump.is_some()
            && let Some(pos) = args.iter().position(|a| a == "-J")
        {
            args.remove(pos); // remove "-J"
            if pos < args.len() {
                args.remove(pos); // remove the value
            }
        }
        let chain = jump_hosts.join(",");

        // `-J` spawns a nested SSH process that does NOT inherit -o/-i
        // from the outer command. When the jump host needs Flatpak
        // known_hosts/identity OR a PKCS#11 token, switch to an explicit
        // ProxyCommand that passes those to the first hop.
        //
        // Each hop's own password (issue #191/#203) is delivered here:
        // the nested ProxyCommand ssh has no controlling TTY, so SSH_ASKPASS
        // with SSH_ASKPASS_REQUIRE=force — scoped to it via the shell
        // env-assignment prefix — authenticates the bastion with ITS password.
        // If target askpass is enabled, every hop clears `_RC_TGT_PW_FILE` before
        // starting its own ssh process; the outer helper remains target-only.
        let any_hop_has_password = hop_passwords.iter().any(Option::is_some);
        let askpass_script = if hop_passwords.first().is_some_and(Option::is_some) {
            jump_host_askpass_script()
        } else {
            None
        };

        if flatpak_known_hosts.is_some()
            || first_hop_pkcs11.is_some()
            || first_hop_identity.is_some()
            || askpass_script.is_some()
            || any_hop_has_password
            || target_askpass_allowed
        {
            // Build a ProxyCommand for the first hop;
            // if there are multiple hops, nest them via nested ProxyCommand.
            let mut proxy_parts: Vec<String> = Vec::new();

            // Every explicit proxy hop starts with an `env` boundary. A hop
            // with its own password gets that helper; all other hops explicitly
            // disable askpass and clear the target credential before `ssh`.
            if let Some(ref script) = askpass_script {
                proxy_parts.extend(rustconn_core::ssh_tunnel::askpass_proxy_prefix(
                    script,
                    0,
                    jump_hosts.len(),
                ));
                if let Some(Some(pw)) = hop_passwords.first() {
                    let env_name = rustconn_core::ssh_tunnel::jump_host_pw_env_name(0);
                    jump_host_passwords.push((env_name, pw.clone()));
                }
            } else {
                proxy_parts.extend(rustconn_core::ssh_tunnel::askpass_disabled_proxy_prefix(
                    jump_hosts.len(),
                ));
            }

            proxy_parts.push("ssh".to_string());
            proxy_parts.push("-W".to_string());
            proxy_parts.push("%h:%p".to_string());

            // First hop reached via forced askpass (its own password, no TTY)
            // must accept a first-seen host key non-interactively: otherwise the
            // "yes/no/[fingerprint]" prompt is routed to the askpass helper,
            // which answers with the PASSWORD and loops until the bastion drops
            // the connection ("Connection closed by UNKNOWN port 65535", #203).
            if askpass_script.is_some() {
                proxy_parts.push("-o".to_string());
                proxy_parts.push("StrictHostKeyChecking=accept-new".to_string());
            }

            // Pass identity file to jump host if we have one.
            // Prefer the jump host's OWN resolved identity; fall back to the
            // target's key (common pattern: same .pem for bastion + target).
            let jump_identity = first_hop_identity.as_ref().or(key.as_ref());
            if let Some(key_path) = jump_identity {
                proxy_parts.push("-i".to_string());
                proxy_parts.push(key_path.clone());
                proxy_parts.push("-o".to_string());
                proxy_parts.push("IdentitiesOnly=yes".to_string());
            }

            // Pass PKCS#11 provider to the first hop (token also auths the bastion)
            if let Some(ref provider) = first_hop_pkcs11 {
                proxy_parts.push("-o".to_string());
                proxy_parts.push(format!("PKCS11Provider={}", provider.trim()));
            }

            // Pass UserKnownHostsFile to jump host (Flatpak only)
            if let Some(ref kh_path) = flatpak_known_hosts {
                proxy_parts.push("-o".to_string());
                proxy_parts.push(format!("UserKnownHostsFile={}", kh_path.display()));
            }

            // Reuse the jump host's ControlMaster socket (created by the direct
            // jump-host tab) so parallel ProxyCommand connections share the
            // already-authenticated link instead of each opening a new TCP.
            // Only set ControlPath (no ControlMaster=auto): if the socket
            // exists, SSH uses it as slave transparently; if not, SSH ignores
            // the option and connects standalone — no identity/auth issues.
            {
                let jump_host_str = &jump_hosts[0];
                let (jh_host, jh_port) = parse_jump_host_for_control(jump_host_str);
                let jh_control = rustconn_core::ssh_control_path(&jh_host, jh_port);
                proxy_parts.push("-o".to_string());
                proxy_parts.push(format!("ControlPath={jh_control}"));
            }

            // ponytail: PKCS#11/identity reach only the first hop; deeper
            // hops still don't get the bastion's own PKCS#11 token. Fine for
            // the common single-bastion case.
            //
            // Multi-hop (issue #203): nest a ProxyCommand per remaining hop so
            // EACH inherits the identity file, Flatpak known_hosts, AND its own
            // SSH_ASKPASS helper if the hop authenticates by password.
            if jump_hosts.len() > 1 {
                let identity_key = args
                    .iter()
                    .position(|a| a == "-i")
                    .and_then(|pos| args.get(pos + 1))
                    .map(String::as_str);
                let inner_hops: Vec<&str> = jump_hosts[1..].iter().map(String::as_str).collect();

                // Build per-hop askpass scripts for deeper hops (issue #203).
                let inner_askpass: Vec<Option<std::path::PathBuf>> = hop_passwords[1..]
                    .iter()
                    .enumerate()
                    .map(|(rel_idx, pw_opt)| {
                        if pw_opt.is_some() {
                            let env_name =
                                rustconn_core::ssh_tunnel::jump_host_pw_env_name(rel_idx + 1);
                            askpass_script_for_env(&env_name)
                        } else {
                            None
                        }
                    })
                    .collect();
                // Expose a deeper hop's password only if its helper exists.
                // Helper creation failure is fail-closed: the hop remains
                // manual/unavailable, but no unused secret enters the process tree.
                for (rel_idx, (pw_opt, script_opt)) in
                    hop_passwords[1..].iter().zip(&inner_askpass).enumerate()
                {
                    if let (Some(pw), Some(_script)) = (pw_opt, script_opt) {
                        let env_name =
                            rustconn_core::ssh_tunnel::jump_host_pw_env_name(rel_idx + 1);
                        jump_host_passwords.push((env_name, pw.clone()));
                    }
                }

                let askpass_refs: Vec<Option<&std::path::Path>> =
                    inner_askpass.iter().map(|opt| opt.as_deref()).collect();

                // accept_new = false: keep the existing host-key posture (the
                // bastions are expected to already be in known_hosts).
                let inner = rustconn_core::ssh_tunnel::build_nested_proxy_command_with_askpass(
                    &inner_hops,
                    identity_key,
                    flatpak_known_hosts.as_deref(),
                    false,
                    &askpass_refs,
                    1,
                    jump_hosts.len(),
                );
                proxy_parts.push("-o".to_string());
                proxy_parts.push(format!(
                    "ProxyCommand={}",
                    rustconn_core::ssh_tunnel::shell_single_quote(&inner)
                ));
            }

            // Add the first hop destination (parse user@host:port into -p port user@host)
            append_proxy_command_destination(&mut proxy_parts, &jump_hosts[0]);

            let proxy_cmd = proxy_parts.join(" ");
            tracing::debug!(
                protocol = "ssh",
                proxy_command = %proxy_cmd,
                hop_passwords_count = jump_host_passwords.len(),
                "Using ProxyCommand instead of -J (identity/Flatpak/PKCS#11/password)"
            );
            args.push("-o".to_string());
            args.push(format!("ProxyCommand={proxy_cmd}"));
        } else {
            // Non-Flatpak, no passwords: use standard -J. `chain` is target-first
            // (RustConn's internal order); OpenSSH `-J` visits hops client-first, so reverse.
            args.push("-J".to_string());
            args.push(rustconn_core::ssh_tunnel::proxy_jump_arg(&chain));
        }

        Some(chain)
    };

    // Check waypipe: enabled in config + binary available on PATH
    let waypipe = ssh_config.waypipe && rustconn_core::protocol::detect_waypipe().installed;
    if ssh_config.waypipe && !waypipe {
        tracing::warn!(
            protocol = "ssh",
            host = %conn.host,
            "Waypipe enabled but not found on PATH, falling back to direct SSH"
        );
    }
    if waypipe {
        tracing::info!(
            protocol = "ssh",
            host = %conn.host,
            "Using waypipe for Wayland application forwarding"
        );
    }

    (
        key,
        args,
        waypipe,
        jump_host_str,
        jump_host_passwords,
        target_askpass_allowed,
    )
}

/// Returns `true` if the first reference jump hop may show an interactive
/// password prompt in the VTE (any [`PasswordSource`] other than `None`).
///
/// Guards the terminal auto-fill fallback, which is what answers a password
/// prompt for every SSH connection the askpass gate declines — a token, a
/// security key, a user-supplied authentication option, or an opaque proxy
/// route. Without the guard, the first VTE prompt on a bastioned connection may
/// be the *bastion's*, and typing the target's password into it hands one host's
/// credential to another (issue #191).
///
/// A key/agent-only bastion (`PasswordSource::None`) authenticates
/// non-interactively inside the `ProxyCommand`, so it never prompts in the
/// terminal — the first VTE prompt is then the target's and target-password
/// auto-fill is safe even with a jump host present. This narrows the issue #191
/// suppression so the common key-auth-bastion + password-target case still
/// auto-fills instead of being suppressed alongside the leak-prone case.
///
/// A connection whose only routing is its own `proxy_command` is inspected by
/// [`proxy_command_can_prompt_for_password`]: a plain TCP relay (`nc`, `socat`,
/// …) opens no prompt, so the first VTE prompt is the target's and auto-fill is
/// safe (issue #322); a nested `ssh` hop or an unrecognised command stays
/// conservative. Otherwise returns `true` when the first hop cannot be
/// inspected — a string `proxy_jump` with no backing connection — so a bastion
/// that might prompt keeps auto-fill suppressed. Returns `false` for non-SSH
/// protocols (the caller's own `has_jump_host` term already covers them).
///
/// [`proxy_command_can_prompt_for_password`]: rustconn_core::ssh_tunnel::proxy_command_can_prompt_for_password
///
/// [`PasswordSource`]: rustconn_core::models::PasswordSource
fn bastion_may_prompt_for_password(
    conn: &rustconn_core::Connection,
    state: &SharedAppState,
) -> bool {
    use rustconn_core::models::PasswordSource;
    if !matches!(&conn.protocol_config, rustconn_core::ProtocolConfig::Ssh(_)) {
        return false;
    }
    // Through the resolver, not `ssh.jump_host_id`: an inherited bastion prompts
    // exactly like one set on the connection, and reading the raw field here
    // would have this answer `false` for the very hop the launcher is about to
    // dial — suppressing nothing and feeding the target's password to the
    // bastion prompt, which is the defect #191 fixed.
    let inherited_hop = state
        .try_borrow()
        .ok()
        .and_then(|s| super::protocols::resolve_first_hop_id(&s, conn));
    if let Some(jid) = inherited_hop {
        // Reference hop: inspect its own password source.
        return state
            .try_borrow()
            .ok()
            .and_then(|s| {
                s.get_connection(jid)
                    .map(|c| c.password_source != PasswordSource::None)
            })
            .unwrap_or(true);
    }

    // No reference hop. When the only routing is this connection's own
    // ProxyCommand — and there is no string ProxyJump bastion — inspect the
    // command: a plain TCP relay (`nc`, `socat`, …) opens no prompt of its own,
    // so the first VTE prompt is the target's and auto-fill is safe (issue
    // #322). A nested `ssh` hop or an unrecognised command stays conservative.
    if let rustconn_core::ProtocolConfig::Ssh(ssh) = &conn.protocol_config
        && ssh.proxy_jump.is_none()
        && let Some(proxy_command) = ssh.proxy_command.as_deref()
    {
        return rustconn_core::ssh_tunnel::proxy_command_can_prompt_for_password(proxy_command);
    }

    // String proxy_jump / inherited proxy / anything else: not inspectable.
    true
}

/// Installs the terminal auto-fill fallback for a connection askpass does not cover.
///
/// The askpass path is the better mechanism where it applies: OpenSSH asks for the credential at
/// exactly the right moment, so there is no deadline to miss, and the helper can tell an account
/// password prompt from a passphrase or an OTP challenge. But it does not apply to every
/// connection — see [`target_password_askpass_allowed`] — and until the 0.21.2 review a connection
/// outside that set got nothing at all, where 0.21.1 had typed its password. This puts the
/// watcher back for exactly those, which also restores the per-connection and per-group *expected
/// password prompt* override from issue #254 for them.
///
/// Does nothing when there is no password to type. The issue #191 guard applies: the target
/// password is typed only when there is no jump host, or the bastion was already authenticated
/// out of band through its own `SSH_ASKPASS`, or the bastion uses key/agent auth and so never
/// prompts in the terminal.
///
/// [`target_password_askpass_allowed`]: rustconn_core::ssh_tunnel::target_password_askpass_allowed
#[expect(
    clippy::too_many_arguments,
    reason = "the fallback needs the shared UI owners plus the connection, credential, automation config and both bastion facts"
)]
fn install_ssh_autofill_fallback(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    conn: &rustconn_core::Connection,
    session_id: Uuid,
    cached_password: Option<SecretString>,
    automation: &rustconn_core::models::AutomationConfig,
    has_jump_host: bool,
    bastion_handled_out_of_band: bool,
) {
    let Some(password) = cached_password else {
        return;
    };
    if has_jump_host && !bastion_handled_out_of_band && bastion_may_prompt_for_password(conn, state)
    {
        tracing::info!(
            protocol = "ssh",
            "Jump host not handled out-of-band; target password auto-fill suppressed to avoid leaking to bastion"
        );
        return;
    }
    crate::window::prompt_autofill::install_login_autofill(
        notebook,
        session_id,
        crate::window::prompt_autofill::LoginAutofill {
            // SSH never types the account name: it travels in the command line.
            username: None,
            password: Some(password),
            matcher: rustconn_core::LoginPromptMatcher::new(
                None,
                automation.password_prompt.as_deref(),
            ),
            protocol: "ssh",
            deadline_secs: automation.login_timeout_secs,
        },
    );
}

/// Starts SSH and observes a session created after asynchronous setup.
#[expect(
    clippy::too_many_arguments,
    reason = "SSH startup requires the shared UI owners, monitoring, connection data, logging policy, and observer"
)]
pub fn start_ssh_connection_observed(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    sidebar: &SharedSidebar,
    monitoring: &super::types::SharedMonitoring,
    connection_id: Uuid,
    conn: &rustconn_core::Connection,
    logging_enabled: bool,
    observer: Option<super::types::SessionStartObserver>,
) -> Option<Uuid> {
    // Check if port check is needed
    let settings = state.borrow().settings().clone();
    // Collect groups for SSH inheritance resolution (proxy_jump can be inherited from group)
    let groups: Vec<rustconn_core::ConnectionGroup> = state
        .try_borrow()
        .ok()
        .map(|s| s.list_groups_owned())
        .unwrap_or_default();
    let has_inherited_proxy =
        ssh_inheritance::resolve_ssh_proxy_jump(conn, &groups, &settings.network).is_some();
    // Use centralized probe-bypass logic + inherited proxy jump from groups
    let should_check = conn.should_pre_connect_check(&settings.connection) && !has_inherited_proxy;

    if conn.bypasses_direct_probe() || has_inherited_proxy {
        tracing::debug!(
            protocol = "ssh",
            host = %conn.host,
            port = conn.port,
            "Skipping port check — connection bypasses direct probe"
        );
    }

    if should_check {
        let host = conn.host.clone();
        let port = conn.port;
        let timeout = settings.connection.port_check_timeout_secs;
        let state_clone = state.clone();
        let notebook_clone = notebook.clone();
        let sidebar_clone = sidebar.clone();
        let monitoring_clone = Rc::clone(monitoring);
        let conn_clone = conn.clone();

        // Run port check in background thread
        spawn_blocking_with_callback(
            move || check_port(&host, port, timeout),
            move |result| {
                match result {
                    Ok(_) => {
                        // Port is open, proceed with connection
                        start_ssh_connection_internal(
                            &state_clone,
                            &notebook_clone,
                            &sidebar_clone,
                            &monitoring_clone,
                            connection_id,
                            &conn_clone,
                            logging_enabled,
                            observer,
                        );
                    }
                    Err(e) => {
                        // Port check failed, show error with retry
                        tracing::warn!(
                            protocol = "ssh",
                            host = %conn_clone.host,
                            port = conn_clone.port,
                            error = %e,
                            "Port check failed for SSH connection"
                        );
                        sidebar_clone
                            .update_connection_status(&connection_id.to_string(), "failed");
                        // Record the failed attempt in history (the session is
                        // never created on a port-check failure, so do it here).
                        if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                            state_mut.record_connection_attempt_failed(
                                &conn_clone,
                                conn_clone.username.as_deref(),
                                &e.to_string(),
                            );
                        }
                        if let Some(root) = notebook_clone.widget().root()
                            && let Some(window) = root.downcast_ref::<gtk4::Window>()
                        {
                            crate::toast::show_retry_toast_on_window(
                                window,
                                &e.to_string(),
                                &connection_id.to_string(),
                            );
                        }
                    }
                }
            },
        );
        // Return None since the actual session will be created asynchronously
        None
    } else {
        // Port check disabled, proceed directly
        start_ssh_connection_internal(
            state,
            notebook,
            sidebar,
            monitoring,
            connection_id,
            conn,
            logging_enabled,
            observer,
        )
    }
}

/// Internal function to start SSH connection (after port check).
///
/// Creates a terminal tab and spawns the SSH process with the given configuration.
#[expect(
    clippy::too_many_arguments,
    reason = "SSH startup requires the shared UI owners, monitoring, connection data, logging policy, and observer"
)]
fn start_ssh_connection_internal(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    sidebar: &SharedSidebar,
    monitoring: &super::types::SharedMonitoring,
    connection_id: Uuid,
    conn: &rustconn_core::Connection,
    logging_enabled: bool,
    observer: Option<super::types::SessionStartObserver>,
) -> Option<Uuid> {
    use rustconn_core::protocol::{format_command_message, format_connection_message};

    let conn_name = conn.name.clone();

    // Get terminal settings from state
    let terminal_settings = state
        .try_borrow()
        .ok()
        .map(|s| s.settings().terminal.clone())
        .unwrap_or_default();

    // Get global variables for substitution (secret values resolved from vault),
    // overlaid with this connection's interactive `@ask:` answers.
    let global_variables = super::protocols::resolve_globals_with_ask(state, connection_id);

    // Resolve automation config with group inheritance
    let resolved_automation = resolve_automation_for_connection(state, conn);

    // Create terminal tab for SSH with user settings
    let session_id = notebook.create_terminal_tab_with_settings(
        connection_id,
        &conn.name,
        "ssh",
        Some(&resolved_automation),
        &terminal_settings,
        conn.theme_override.as_ref(),
        &super::protocols::automation_variables(
            state,
            connection_id,
            conn,
            &resolved_automation,
            &global_variables,
        ),
    );
    if let Some(observer) = observer {
        observer.complete(session_id);
    }

    // Apply highlight rules (built-in defaults + global + per-connection)
    {
        let global_rules = state
            .try_borrow()
            .ok()
            .map(|s| s.settings().highlight_rules.clone())
            .unwrap_or_default();
        notebook.set_highlight_rules(session_id, &global_rules, &conn.highlight_rules);
    }

    // Record connection start in history
    let history_entry_id = if let Ok(mut state_mut) = state.try_borrow_mut() {
        Some(state_mut.record_connection_start(conn, conn.username.as_deref()))
    } else {
        None
    };

    // Store history entry ID in session for later use
    if let Some(entry_id) = history_entry_id {
        notebook.set_history_entry_id(session_id, entry_id);
    }

    // Build and spawn SSH command
    let port = conn.port;

    // Collect groups for SSH inheritance resolution
    let groups: Vec<rustconn_core::ConnectionGroup> = state
        .try_borrow()
        .ok()
        .map(|s| s.list_groups_owned())
        .unwrap_or_default();
    // The outermost bastion tier, below the group chain.
    let network = state
        .try_borrow()
        .ok()
        .map(|s| s.settings().network.clone())
        .unwrap_or_default();

    // Detect jump host / proxy for status detection and monitoring.
    //
    // Both forms go through inheritance. This used to inherit the free-text
    // ProxyJump and read the picker's `jump_host_id` raw, so a connection reached
    // through a group-level or global Jump Host reported `has_jump_host = false`
    // while the launcher dialled that very bastion — the status and monitoring
    // paths then treated a bastioned host as directly reachable.
    let has_jump_host = matches!(
        &conn.protocol_config,
        rustconn_core::ProtocolConfig::Ssh(ssh) if ssh.proxy_command.is_some()
    ) || state
        .try_borrow()
        .ok()
        .and_then(|s| super::protocols::resolve_first_hop_id(&s, conn))
        .is_some()
        || ssh_inheritance::resolve_ssh_proxy_jump(conn, &groups, &network).is_some();

    // Apply variable substitution to host and username (e.g., ${VAR_NAME} -> actual value)
    let host = substitute_variables(&conn.host, &global_variables);
    let username = conn
        .username
        .as_ref()
        .map(|u| substitute_variables(u, &global_variables));

    // Retrieve the cached target credential resolved from the vault earlier.
    // It is only exposed to OpenSSH when the strict password-only gate below passes.
    let cached_password: Option<SecretString> = state
        .try_borrow()
        .ok()
        .and_then(|s| s.get_cached_credentials(connection_id).cloned())
        .and_then(|c| {
            use secrecy::ExposeSecret;
            if c.password.expose_secret().is_empty() {
                None
            } else {
                Some(c.password.clone())
            }
        });

    // Get SSH-specific options
    let (
        identity_file,
        extra_args,
        use_waypipe,
        jump_host_chain,
        jump_host_passwords,
        target_askpass_allowed,
    ) = build_ssh_command_args(
        conn,
        connection_id,
        state,
        &groups,
        cached_password.is_some(),
    );

    // Extract MPTCP flag from SSH config
    let use_mptcp =
        matches!(&conn.protocol_config, rustconn_core::ProtocolConfig::Ssh(cfg) if cfg.mptcp);

    // Update last_connected timestamp
    if let Ok(mut state_mut) = state.try_borrow_mut()
        && let Err(e) = state_mut.update_last_connected(connection_id)
    {
        tracing::warn!(?e, "Failed to update last_connected");
    }

    // Set up session logging if enabled
    if logging_enabled {
        MainWindow::setup_session_logging(state, notebook, session_id, connection_id, &conn_name);
    }

    // Wire up child exited callback for session cleanup
    MainWindow::setup_child_exited_handler(state, notebook, sidebar, session_id, connection_id);

    // Build SSH command string for display
    let mut ssh_cmd_parts = if use_waypipe {
        vec!["waypipe".to_string(), "ssh".to_string()]
    } else {
        vec!["ssh".to_string()]
    };
    if port != 22 {
        ssh_cmd_parts.push("-p".to_string());
        ssh_cmd_parts.push(port.to_string());
    }
    if let Some(ref key) = identity_file {
        ssh_cmd_parts.push("-i".to_string());
        ssh_cmd_parts.push(key.clone());
    }
    ssh_cmd_parts.extend(extra_args.clone());
    let destination = if let Some(ref user) = username {
        format!("{user}@{host}")
    } else {
        host.clone()
    };
    ssh_cmd_parts.push(destination);
    // Quote arguments with spaces (e.g. a ProxyCommand value) so the echoed
    // line matches the single argv element that is actually spawned and is
    // copy-paste safe — a plain join(" ") split it and read as if the option
    // were dropped (issue #322).
    let ssh_command = rustconn_core::ssh_tunnel::format_argv_for_display(&ssh_cmd_parts);

    // Display CLI output feedback before executing command
    let conn_msg = format_connection_message("SSH", &host);
    let cmd_msg = format_command_message(&ssh_command);
    let feedback = format!("{conn_msg}\r\n{cmd_msg}\r\n\r\n");
    notebook.display_output(session_id, &feedback);

    // Spawn SSH. Password-only targets use prompt-aware OpenSSH askpass;
    // mixed or custom authentication remains an interactive terminal prompt.
    {
        let extra_refs: Vec<&str> = extra_args.iter().map(std::string::String::as_str).collect();
        let agent_socket = ssh_inheritance::resolve_ssh_agent_socket(conn, &groups);
        let startup_cmd = match &conn.protocol_config {
            rustconn_core::ProtocolConfig::Ssh(cfg) => cfg.startup_command.as_deref(),
            _ => None,
        };
        // Backspace/Delete bytes this host expects (issue #271). Applied to the
        // widget, not the command line, and after the tab's terminal settings —
        // see TerminalNotebook::set_erase_mode.
        apply_ssh_erase_mode(notebook, session_id, conn);
        // Bastion credentials use per-hop helpers. The target credential is
        // added only for the strict password-only launch plan and is consumed
        // by the prompt-aware outer OpenSSH helper, never by VTE output.
        let target_askpass_script = if target_askpass_allowed {
            target_password_askpass_script()
        } else {
            None
        };
        let (askpass_env, askpass_cleanup_paths) = ssh_askpass_env(
            &jump_host_passwords,
            cached_password
                .as_ref()
                .zip(target_askpass_script.as_deref()),
        );
        let extra_env_refs: Vec<&str> = askpass_env.iter().map(|entry| entry.as_str()).collect();
        notebook.spawn_ssh_with_cleanup(
            session_id,
            &host,
            port,
            username.as_deref(),
            identity_file.as_deref(),
            &extra_refs,
            use_waypipe,
            agent_socket.as_deref(),
            startup_cmd,
            if extra_env_refs.is_empty() {
                None
            } else {
                Some(extra_env_refs.as_slice())
            },
            use_mptcp,
            askpass_cleanup_paths,
        );
    }

    // Askpass covers what it can prove; everything else keeps the terminal
    // watcher, including the issue #254 prompt override. Skipped when askpass is
    // active so the password is never delivered twice.
    if !target_askpass_allowed {
        install_ssh_autofill_fallback(
            state,
            notebook,
            conn,
            session_id,
            cached_password.clone(),
            &resolved_automation,
            has_jump_host,
            !jump_host_passwords.is_empty(),
        );
    }

    // --- SSH status detection: mark sidebar "connected" once terminal output appears ---
    // For jump host connections, also check terminal text for SSH failure patterns
    // to avoid false positives (jump host connects but destination times out).
    {
        let sidebar_clone = sidebar.clone();
        let notebook_clone = notebook.clone();
        let connection_id_str = connection_id.to_string();
        let session_connected = std::rc::Rc::new(std::cell::Cell::new(false));
        let session_connected_clone = session_connected.clone();
        let protocol_str = String::from("ssh");
        let uses_jump_host = has_jump_host;

        notebook.connect_contents_changed(session_id, move || {
            if session_connected_clone.get() {
                return;
            }
            if let Some(row) = notebook_clone.get_terminal_cursor_row(session_id) {
                tracing::debug!(
                    protocol = "ssh",
                    cursor_row = row,
                    threshold = 2,
                    "SSH status detection: checking cursor row"
                );
                if row > 2 {
                    // When using a jump host, the cursor may advance past row 2
                    // due to jump host banners or SSH error output even if the
                    // final destination is unreachable. Check terminal text for
                    // known SSH failure patterns before marking as connected.
                    if uses_jump_host
                        && let Some(text) = notebook_clone.get_terminal_text(session_id)
                        && contains_ssh_failure(&text)
                    {
                        tracing::debug!(
                            protocol = "ssh",
                            cursor_row = row,
                            "Jump host connection: SSH failure detected in terminal"
                        );
                        return;
                    }
                    sidebar_clone.increment_session_count(&connection_id_str);
                    session_connected_clone.set(true);
                    tracing::info!(
                        protocol = %protocol_str,
                        cursor_row = row,
                        "Terminal connection detected as established"
                    );
                }
            }
        });
    }

    // --- Auto-recording: start recording once SSH connection is established ---
    if conn.session_recording_enabled {
        let notebook_clone = notebook.clone();
        let recording_conn_name = conn_name.clone();
        let recording_started = std::rc::Rc::new(std::cell::Cell::new(false));
        let recording_started_clone = recording_started.clone();
        let recording_ssh_params = Some(crate::terminal::SshRecordingParams {
            host: host.clone(),
            port,
            username: username.clone(),
            identity_file: identity_file.clone(),
        });

        notebook.connect_contents_changed(session_id, move || {
            if recording_started_clone.get() {
                return;
            }
            // Wait for connection to be established (cursor row > 2)
            if let Some(row) = notebook_clone.get_terminal_cursor_row(session_id)
                && row > 2
            {
                recording_started_clone.set(true);
                notebook_clone.start_recording(
                    session_id,
                    &recording_conn_name,
                    recording_ssh_params.clone(),
                );
                tracing::info!(
                    %session_id,
                    "Auto-recording started after SSH connection established"
                );
            }
        });
    }

    // --- Deferred monitoring start: wait for SSH to connect before opening monitor ---
    if let Ok(state_ref) = state.try_borrow() {
        let settings = state_ref.settings().monitoring.clone();
        let mon_enabled = conn
            .monitoring_config
            .as_ref()
            .map_or(settings.enabled, |mc| mc.is_enabled(&settings));
        if mon_enabled {
            let effective = rustconn_core::MonitoringSettings {
                enabled: true,
                interval_secs: conn.monitoring_config.as_ref().map_or_else(
                    || settings.effective_interval_secs(),
                    |mc| mc.effective_interval(&settings),
                ),
                ..settings
            };
            let identity_file_mon = ssh_inheritance::resolve_ssh_key_path(conn, &groups)
                .and_then(|p| rustconn_core::resolve_key_path(&p))
                .map(|p| p.to_string_lossy().to_string());
            let cached_pw = state_ref
                .get_cached_credentials(connection_id)
                .and_then(|c| {
                    use secrecy::ExposeSecret;
                    let pw = c.password.expose_secret();
                    if pw.is_empty() {
                        None
                    } else {
                        Some(c.password.clone())
                    }
                });

            let monitoring_clone = Rc::clone(monitoring);
            let notebook_clone = notebook.clone();
            let mon_host = conn.host.clone();
            let mon_port = conn.port;
            let mon_username = conn.username.clone();
            let mon_jump_host = jump_host_chain.clone();
            let monitoring_started = std::rc::Rc::new(std::cell::Cell::new(false));
            let monitoring_started_clone = monitoring_started.clone();

            notebook.connect_contents_changed(session_id, move || {
                if monitoring_started_clone.get() {
                    return;
                }
                let Some(row) = notebook_clone.get_terminal_cursor_row(session_id) else {
                    return;
                };
                if row <= 2 {
                    return;
                }
                monitoring_started_clone.set(true);
                if let Some(container) = notebook_clone.get_session_container(session_id) {
                    monitoring_clone.start_monitoring(
                        session_id,
                        &container,
                        &effective,
                        &mon_host,
                        mon_port,
                        mon_username.as_deref(),
                        identity_file_mon.as_deref(),
                        cached_pw.clone(),
                        mon_jump_host.as_deref(),
                    );
                }
            });
        }
    }

    Some(session_id)
}

/// Returns `true` if reconnect was initiated, `false` if the tab no longer exists.
pub fn reconnect_ssh_in_place(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    sidebar: &SharedSidebar,
    monitoring: &super::types::SharedMonitoring,
    session_id: Uuid,
    connection_id: Uuid,
) -> bool {
    use rustconn_core::protocol::{format_command_message, format_connection_message};

    // Prepare the existing tab for reconnect
    if !notebook.prepare_for_reconnect(session_id) {
        tracing::warn!(%session_id, "Tab no longer exists, cannot reconnect in-place");
        return false;
    }

    // Show "connecting" status in sidebar immediately
    sidebar.update_connection_status(&connection_id.to_string(), "connecting");

    // Get connection data
    let conn = {
        let Ok(state_ref) = state.try_borrow() else {
            return false;
        };
        match state_ref.get_connection(connection_id) {
            Some(c) => c.clone(),
            None => return false,
        }
    };

    // Re-apply highlight rules
    {
        let global_rules = state
            .try_borrow()
            .ok()
            .map(|s| s.settings().highlight_rules.clone())
            .unwrap_or_default();
        notebook.set_highlight_rules(session_id, &global_rules, &conn.highlight_rules);
    }

    // Record connection start in history
    let history_entry_id = if let Ok(mut state_mut) = state.try_borrow_mut() {
        Some(state_mut.record_connection_start(&conn, conn.username.as_deref()))
    } else {
        None
    };
    if let Some(entry_id) = history_entry_id {
        notebook.set_history_entry_id(session_id, entry_id);
    }

    // Get global variables for substitution, overlaid with this connection's
    // interactive `@ask:` answers (kept from the initial connect so a reconnect
    // does not re-prompt).
    let global_variables = super::protocols::resolve_globals_with_ask(state, connection_id);

    let host = substitute_variables(&conn.host, &global_variables);
    let username = conn
        .username
        .as_ref()
        .map(|u| substitute_variables(u, &global_variables));

    // Collect groups for SSH inheritance resolution
    let groups: Vec<rustconn_core::ConnectionGroup> = state
        .try_borrow()
        .ok()
        .map(|s| s.list_groups_owned())
        .unwrap_or_default();
    // The outermost bastion tier, below the group chain.
    let network = state
        .try_borrow()
        .ok()
        .map(|s| s.settings().network.clone())
        .unwrap_or_default();

    // Both forms inherited — see the note on the identical guard in
    // `start_ssh_connection`.
    let has_jump_host = matches!(
        &conn.protocol_config,
        rustconn_core::ProtocolConfig::Ssh(ssh) if ssh.proxy_command.is_some()
    ) || state
        .try_borrow()
        .ok()
        .and_then(|s| super::protocols::resolve_first_hop_id(&s, &conn))
        .is_some()
        || ssh_inheritance::resolve_ssh_proxy_jump(&conn, &groups, &network).is_some();

    // Retrieve the cached target credential before building the launch plan so
    // proxy routing and auth flags are scoped consistently with initial connect.
    let cached_password: Option<SecretString> = state
        .try_borrow()
        .ok()
        .and_then(|s| s.get_cached_credentials(connection_id).cloned())
        .and_then(|c| {
            use secrecy::ExposeSecret;
            if c.password.expose_secret().is_empty() {
                None
            } else {
                Some(c.password.clone())
            }
        });

    // Build SSH args (shared with start_ssh_connection).
    let (
        identity_file,
        extra_args,
        use_waypipe,
        jump_host_chain,
        jump_host_passwords,
        target_askpass_allowed,
    ) = build_ssh_command_args(
        &conn,
        connection_id,
        state,
        &groups,
        cached_password.is_some(),
    );

    // Extract MPTCP flag from SSH config
    let use_mptcp =
        matches!(&conn.protocol_config, rustconn_core::ProtocolConfig::Ssh(cfg) if cfg.mptcp);

    // Re-wire child-exited handler for the new process
    MainWindow::setup_child_exited_handler(state, notebook, sidebar, session_id, connection_id);

    // Build SSH command string for display
    let port = conn.port;
    let mut ssh_cmd_parts = if use_waypipe {
        vec!["waypipe".to_string(), "ssh".to_string()]
    } else {
        vec!["ssh".to_string()]
    };
    if port != 22 {
        ssh_cmd_parts.push("-p".to_string());
        ssh_cmd_parts.push(port.to_string());
    }
    if let Some(ref key) = identity_file {
        ssh_cmd_parts.push("-i".to_string());
        ssh_cmd_parts.push(key.clone());
    }
    ssh_cmd_parts.extend(extra_args.clone());
    let destination = if let Some(ref user) = username {
        format!("{user}@{host}")
    } else {
        host.clone()
    };
    ssh_cmd_parts.push(destination);
    // Quote arguments with spaces (e.g. a ProxyCommand value) so the echoed
    // line matches the single argv element that is actually spawned and is
    // copy-paste safe — a plain join(" ") split it and read as if the option
    // were dropped (issue #322).
    let ssh_command = rustconn_core::ssh_tunnel::format_argv_for_display(&ssh_cmd_parts);

    // Display CLI output feedback
    let conn_msg = format_connection_message("SSH", &host);
    let cmd_msg = format_command_message(&ssh_command);
    let feedback = format!("{conn_msg}\r\n{cmd_msg}\r\n\r\n");
    notebook.display_output(session_id, &feedback);

    // Spawn SSH in the existing terminal
    {
        let extra_refs: Vec<&str> = extra_args.iter().map(std::string::String::as_str).collect();
        let agent_socket = ssh_inheritance::resolve_ssh_agent_socket(&conn, &groups);
        let startup_cmd = match &conn.protocol_config {
            rustconn_core::ProtocolConfig::Ssh(cfg) => cfg.startup_command.as_deref(),
            _ => None,
        };
        // Re-assert the erase mode (issue #271): a reconnect spawns into the
        // same terminal, and nothing else restores it after a VTE reset.
        apply_ssh_erase_mode(notebook, session_id, &conn);
        // Same auth-scoped environment as the initial launch.
        let target_askpass_script = if target_askpass_allowed {
            target_password_askpass_script()
        } else {
            None
        };
        let (askpass_env, askpass_cleanup_paths) = ssh_askpass_env(
            &jump_host_passwords,
            cached_password
                .as_ref()
                .zip(target_askpass_script.as_deref()),
        );
        let extra_env_refs: Vec<&str> = askpass_env.iter().map(|entry| entry.as_str()).collect();
        notebook.spawn_ssh_with_cleanup(
            session_id,
            &host,
            port,
            username.as_deref(),
            identity_file.as_deref(),
            &extra_refs,
            use_waypipe,
            agent_socket.as_deref(),
            startup_cmd,
            if extra_env_refs.is_empty() {
                None
            } else {
                Some(extra_env_refs.as_slice())
            },
            use_mptcp,
            askpass_cleanup_paths,
        );
    }

    // Same fallback as the initial launch — a reconnect must not silently lose
    // the password delivery the first connect had.
    if !target_askpass_allowed {
        let resolved_automation = resolve_automation_for_connection(state, &conn);
        install_ssh_autofill_fallback(
            state,
            notebook,
            &conn,
            session_id,
            cached_password.clone(),
            &resolved_automation,
            has_jump_host,
            !jump_host_passwords.is_empty(),
        );
    }

    // SSH status detection
    {
        let sidebar_clone = sidebar.clone();
        let notebook_clone = notebook.clone();
        let connection_id_str = connection_id.to_string();
        let session_connected = std::rc::Rc::new(std::cell::Cell::new(false));
        let session_connected_clone = session_connected.clone();
        let uses_jump_host = has_jump_host;

        notebook.connect_contents_changed(session_id, move || {
            if session_connected_clone.get() {
                return;
            }
            if let Some(row) = notebook_clone.get_terminal_cursor_row(session_id)
                && row > 2
            {
                if uses_jump_host
                    && let Some(text) = notebook_clone.get_terminal_text(session_id)
                    && contains_ssh_failure(&text)
                {
                    return;
                }
                sidebar_clone.increment_session_count(&connection_id_str);
                session_connected_clone.set(true);
            }
        });
    }

    // Deferred monitoring start
    if let Ok(state_ref) = state.try_borrow() {
        let settings = state_ref.settings().monitoring.clone();
        let mon_enabled = conn
            .monitoring_config
            .as_ref()
            .map_or(settings.enabled, |mc| mc.is_enabled(&settings));
        if mon_enabled {
            let effective = rustconn_core::MonitoringSettings {
                enabled: true,
                interval_secs: conn.monitoring_config.as_ref().map_or_else(
                    || settings.effective_interval_secs(),
                    |mc| mc.effective_interval(&settings),
                ),
                ..settings
            };
            let identity_file_mon = ssh_inheritance::resolve_ssh_key_path(&conn, &groups)
                .and_then(|p| rustconn_core::resolve_key_path(&p))
                .map(|p| p.to_string_lossy().to_string());
            let cached_pw = state_ref
                .get_cached_credentials(connection_id)
                .and_then(|c| {
                    use secrecy::ExposeSecret;
                    let pw = c.password.expose_secret();
                    if pw.is_empty() {
                        None
                    } else {
                        Some(c.password.clone())
                    }
                });

            let monitoring_clone = Rc::clone(monitoring);
            let notebook_clone = notebook.clone();
            let mon_host = conn.host.clone();
            let mon_port = conn.port;
            let mon_username = conn.username.clone();
            let mon_jump_host = jump_host_chain;
            let monitoring_started = std::rc::Rc::new(std::cell::Cell::new(false));
            let monitoring_started_clone = monitoring_started.clone();

            notebook.connect_contents_changed(session_id, move || {
                if monitoring_started_clone.get() {
                    return;
                }
                let Some(row) = notebook_clone.get_terminal_cursor_row(session_id) else {
                    return;
                };
                if row <= 2 {
                    return;
                }
                monitoring_started_clone.set(true);
                if let Some(container) = notebook_clone.get_session_container(session_id) {
                    monitoring_clone.start_monitoring(
                        session_id,
                        &container,
                        &effective,
                        &mon_host,
                        mon_port,
                        mon_username.as_deref(),
                        identity_file_mon.as_deref(),
                        cached_pw.clone(),
                        mon_jump_host.as_deref(),
                    );
                }
            });
        }
    }

    // Update last_connected timestamp
    if let Ok(mut state_mut) = state.try_borrow_mut()
        && let Err(e) = state_mut.update_last_connected(connection_id)
    {
        tracing::warn!(?e, "Failed to update last_connected");
    }

    true
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn run_askpass_script(prompt: &str) -> (std::process::Output, std::path::PathBuf) {
        let secret_path =
            std::env::temp_dir().join(format!("rustconn-askpass-test-secret-{}", Uuid::new_v4()));
        std::fs::write(&secret_path, b"test-account-password")
            .expect("test secret file must be writable");
        let output = std::process::Command::new("sh")
            .args([
                "-c",
                &askpass_script_contents("_RC_TEST_PW_FILE"),
                "rustconn-askpass-test",
                prompt,
            ])
            .env("_RC_TEST_PW_FILE", &secret_path)
            .output()
            .expect("test askpass script must run");
        (output, secret_path)
    }

    #[test]
    fn opaque_jump_host_keeps_credentials_aligned_with_reference_hops() {
        let reference_id = Uuid::new_v4();
        let mut hosts = vec!["reference.example.com".to_string()];
        let mut ids = vec![Some(reference_id)];

        insert_opaque_jump_host(&mut hosts, &mut ids, 0, "opaque.example.com".to_string());

        assert_eq!(
            hosts,
            vec![
                "opaque.example.com".to_string(),
                "reference.example.com".to_string()
            ]
        );
        assert_eq!(ids, vec![None, Some(reference_id)]);
    }

    #[test]
    fn askpass_secret_file_is_owner_only_and_not_an_environment_value() {
        use std::os::unix::fs::PermissionsExt;

        let secret = SecretString::new("file-only-secret".to_string().into());
        let path = create_askpass_secret_file(&secret).expect("secret file must be created");
        let metadata = std::fs::metadata(&path).expect("secret file metadata must exist");
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        assert_eq!(
            std::fs::read(&path).expect("secret file must be readable"),
            b"file-only-secret"
        );
        std::fs::remove_file(path).expect("test secret file must be removed");
    }

    #[test]
    fn askpass_helper_releases_password_only_for_openssh_account_prompt() {
        let (accepted, accepted_secret_path) = run_askpass_script("alice@example.com's password: ");
        assert!(accepted.status.success());
        assert_eq!(accepted.stdout, b"test-account-password");
        assert!(!accepted_secret_path.exists());

        for rejected_prompt in [
            "Enter passphrase for key '/home/alice/.ssh/id_ed25519': ",
            "The authenticity of host cannot be established. Continue connecting (yes/no)? ",
            "Verification code: ",
            "Password expired. Enter new password: ",
            "[sudo] password for alice: ",
            "Password: ",
        ] {
            let (rejected, rejected_secret_path) = run_askpass_script(rejected_prompt);
            assert!(!rejected.status.success(), "accepted: {rejected_prompt}");
            assert!(rejected.stdout.is_empty(), "leaked for: {rejected_prompt}");
            std::fs::remove_file(rejected_secret_path)
                .expect("rejected prompt must leave the test file for owner cleanup");
        }
    }
}
