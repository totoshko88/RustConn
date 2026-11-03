//! SSH connection launch and reconnect logic.
//!
//! Extracted from `window/protocols.rs` to reduce module complexity.

use std::rc::Rc;

use gtk4::prelude::*;
use rustconn_core::connection::{check_port, ssh_inheritance};
use rustconn_core::ssh_askpass::{self, AskpassSlot};
use secrecy::SecretString;
use uuid::Uuid;

use super::MainWindow;
use super::protocols::{
    SharedNotebook, SharedSidebar, append_proxy_command_destination,
    resolve_automation_for_connection, substitute_variables,
};
use crate::state::SharedAppState;
use crate::utils::spawn_blocking_with_callback;

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

/// Builds the per-hop password vector aligned with the jump-host chain (P2a).
///
/// One place for the multi-bastion password logic that was inline in
/// [`build_ssh_command_args`]: the first reference hop's password is already
/// resolved (through the chain loop, with its #191/#241/PKCS#11 handling) and
/// is placed at the first `Some` entry of `hop_ids`; every other reference hop
/// has its own password resolved through `resolve_hop` (issue #203). A `None`
/// entry in `hop_ids` is a string proxy with no credential and is left `None`.
///
/// `resolve_hop` is the only side-effecting part — it reads the cache, falls
/// back to the vault and warms the cache — so it is injected, which keeps this
/// function pure and testable with a fake resolver. The result is the same
/// length as `hop_ids`, so it stays index-aligned with `jump_hosts`
/// (SHALL CONTINUE TO: host↔password alignment).
///
/// `len` is the jump-host count; `hop_ids` must be exactly that long.
fn assemble_hop_passwords(
    len: usize,
    hop_ids: &[Option<Uuid>],
    first_hop_password: Option<SecretString>,
    mut resolve_hop: impl FnMut(Uuid) -> Option<SecretString>,
) -> Vec<Option<SecretString>> {
    debug_assert_eq!(hop_ids.len(), len);
    let mut hop_passwords: Vec<Option<SecretString>> = vec![None; len];

    // The first reference hop (first Some in hop_ids) is already resolved.
    if let Some(first_ref_idx) = hop_ids.iter().position(Option::is_some) {
        hop_passwords[first_ref_idx] = first_hop_password;
    }

    for (idx, hop_id) in hop_ids.iter().enumerate() {
        if hop_passwords[idx].is_some() {
            continue; // already resolved (first hop, or a prior pass)
        }
        if let Some(hid) = hop_id {
            hop_passwords[idx] = resolve_hop(*hid);
        }
    }

    hop_passwords
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

/// The authentication/known-hosts shape of a jump-host chain, used to decide
/// how the first hop is wired (`-J` vs an explicit `ProxyCommand`).
///
/// Grouped into a struct rather than passed as a row of booleans so the two
/// decision helpers read by field name and stay under clippy's
/// `fn_params_excessive_bools` ceiling.
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is an independent auth/known-hosts condition the routing decision depends on; \
              collapsing them into an enum would lose the per-condition meaning the two predicates read"
)]
struct JumpAuthShape {
    hop_count: usize,
    has_identity: bool,
    has_flatpak_known_hosts: bool,
    has_pkcs11: bool,
    has_askpass: bool,
    any_hop_has_password: bool,
    target_askpass_allowed: bool,
}

impl JumpAuthShape {
    /// Multi-hop with only an identity key — no password, Flatpak known-hosts,
    /// or PKCS#11 token in play.
    ///
    /// This is the one case the nested `ProxyCommand` cannot serve (a deep `-W`
    /// hop runs ssh on the previous hop, where the client's local `-i <path>`
    /// does not exist), so it is routed to `-J` with the key loaded into the
    /// agent instead.
    fn is_multi_hop_identity_only(&self) -> bool {
        self.hop_count > 1
            && self.has_identity
            && !self.has_flatpak_known_hosts
            && !self.has_pkcs11
            && !self.has_askpass
            && !self.any_hop_has_password
            && !self.target_askpass_allowed
    }

    /// Whether the first hop needs an explicit `ProxyCommand` rather than `-J`.
    ///
    /// `-J` spawns child ssh processes that do not inherit `-o`/`-i`, so Flatpak
    /// known-hosts, a PKCS#11 token, a first-hop identity, or any password hop
    /// all force the explicit `ProxyCommand` that passes them to the first hop.
    /// The one exception is [`Self::is_multi_hop_identity_only`], which `-J` +
    /// agent serves better than a nested `ProxyCommand` that cannot carry a
    /// local key past the first hop.
    fn needs_explicit_proxy_command(&self) -> bool {
        !self.is_multi_hop_identity_only()
            && (self.has_flatpak_known_hosts
                || self.has_pkcs11
                || self.has_identity
                || self.has_askpass
                || self.any_hop_has_password
                || self.target_askpass_allowed)
    }
}

/// Ensures a private-key identity is loaded into the running ssh-agent.
///
/// The multi-hop `-J` path needs this: OpenSSH's `ProxyJump` does not pass the
/// outer command's `-i <path>` to the ssh processes it spawns for each hop, so
/// the only identity every hop can see is one the agent already holds. Adding
/// it here (idempotently — a key the agent already holds is not added again) makes a
/// key-authenticated multi-hop chain work without the user pre-running
/// `ssh-add`. Best-effort: a passphrase-protected key whose passphrase nothing
/// can supply cannot be added non-interactively, so a failure is logged and the
/// connection still proceeds (the agent may already hold it, or the user can add
/// it).
fn ensure_identity_in_agent(key_path: &std::path::Path) {
    use rustconn_core::ssh_agent::SshAgentManager;

    // Times the ssh-agent identity phase and counts the agent operations it
    // runs — the A1 target is driving both the time and the spawn/process count
    // down. Off unless `RUST_LOG=rustconn::perf=info`.
    let mut perf = rustconn_core::tracing::perf::PerfSpan::new("ssh.connect.agent", "ssh-agent");

    if !key_path.exists() {
        return;
    }
    let manager = SshAgentManager::from_env();
    if manager.socket_path().is_none() {
        tracing::debug!("no ssh-agent socket; multi-hop -J relies on an existing agent identity");
        return;
    }
    // One add-key round trip against the agent — the call this phase exists for.
    perf.add_calls(1);
    // No passphrase: this covers unencrypted keys and keys the agent/askpass
    // can resolve; an encrypted key with no resolvable passphrase fails here
    // and is left to the agent/user. A key added here gets the 12 h lifetime
    // and is removed again on exit, screen lock and suspend; a key the agent
    // already holds is left alone.
    if let Err(e) = manager.add_own_key(key_path) {
        tracing::debug!(error = %e, "could not add identity to ssh-agent for multi-hop -J");
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
    // `ssh -G`'s answer when it was run off the main thread (P3). `Some(v)` is
    // used as-is; `None` means it was not precomputed (the reconnect path, or
    // port check disabled) and this falls back to spawning `ssh -G` here.
    precomputed_declares_proxy: Option<bool>,
) -> (
    Option<String>,
    Vec<String>,
    bool,
    Option<String>,
    Vec<(AskpassSlot, SecretString)>,
    bool,
) {
    let rustconn_core::ProtocolConfig::Ssh(ssh_config) = &conn.protocol_config else {
        return (None, Vec::new(), false, None, Vec::new(), false);
    };

    // Times the connect-args phase: key inheritance, jump-host/ProxyCommand
    // wiring, bastion and target askpass helpers. Placed after the SSH guard so
    // a non-SSH call emits no line. Off unless `RUST_LOG=rustconn::perf=info`.
    let _perf = rustconn_core::tracing::perf::PerfSpan::new("ssh.connect.args", "ssh");

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
    //
    // P3: when the caller ran `ssh -G` off the main thread (the initial connect,
    // in the same background phase as the port check) the answer is passed in
    // and no process is spawned here. `None` — the reconnect path, or port check
    // disabled — falls back to spawning it synchronously, as before. The `&&`
    // short-circuit means this is only evaluated once the two cheap conditions
    // hold, so a `None` fallback spawn still runs only when it mattered before.
    let declares_proxy = || -> bool {
        precomputed_declares_proxy.unwrap_or_else(|| {
            rustconn_core::ssh_tunnel::ssh_config_declares_proxy(
                &conn.host,
                conn.port,
                conn.username.as_deref(),
            )
        })
    };
    let target_askpass_allowed = has_cached_target_password
        && rustconn_core::ssh_tunnel::target_password_askpass_allowed(ssh_config, key.is_some())
        && !declares_proxy();
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
    // chains). `first_hop_password` already covers the first reference hop; the
    // alignment and per-hop resolution live in `assemble_hop_passwords`, which
    // is given a resolver that reads the cache, falls back to the vault and
    // warms the cache. The vault read stays synchronous here (P2a scope: the
    // logic is extracted and deduplicated; moving it off the main thread is a
    // separate change).
    let hop_passwords: Vec<Option<SecretString>> =
        assemble_hop_passwords(jump_hosts.len(), &hop_ids, first_hop_password, |hid| {
            // Cache first (fast path).
            let cached = state
                .try_borrow()
                .ok()
                .and_then(|s| s.get_cached_credentials(hid).cloned())
                .and_then(|c| {
                    use secrecy::ExposeSecret;
                    if c.password.expose_secret().is_empty() {
                        None
                    } else {
                        Some(c.password.clone())
                    }
                });
            if cached.is_some() {
                return cached;
            }
            // Fallback: resolve from vault/variable, then warm the cache.
            let conn_for_hop = state
                .try_borrow()
                .ok()
                .and_then(|s| s.get_connection(hid).cloned());
            let hop_conn = conn_for_hop?;
            let resolved_pw = state
                .try_borrow()
                .ok()
                .and_then(|s| s.resolve_connection_password_blocking(&hop_conn))?;
            if let Ok(mut state_mut) = state.try_borrow_mut() {
                use secrecy::ExposeSecret;
                state_mut.cache_credentials(
                    hid,
                    hop_conn.username.as_deref().unwrap_or(""),
                    resolved_pw.expose_secret(),
                    "",
                );
            }
            Some(resolved_pw)
        });

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
    // Each entry: (slot, password). Set only when askpass helpers are
    // successfully wired into ProxyCommand.
    let mut jump_host_passwords: Vec<(AskpassSlot, SecretString)> = Vec::new();

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
            ssh_askpass::helper_script(AskpassSlot::JumpHost(0)).ok()
        } else {
            None
        };

        // Multi-hop with only an identity key (no password/Flatpak/PKCS#11)
        // cannot use the nested ProxyCommand: each `-W` hop past the first runs
        // ssh ON the previous hop, where the client's local `-i <path>` file
        // does not exist, so deep hops fail with "Permission denied" (proven
        // against a genuine laptop→bastion1→bastion2→target chain). OpenSSH's
        // `-J` is the mechanism that carries one client identity across every
        // hop — but only via the agent, since `-J` does not pass `-i` to its
        // child ssh processes either. So for this case route to `-J` and load
        // the key into the agent below (`ensure_identity_in_agent`).
        let auth_shape = JumpAuthShape {
            hop_count: jump_hosts.len(),
            has_identity: first_hop_identity.is_some(),
            has_flatpak_known_hosts: flatpak_known_hosts.is_some(),
            has_pkcs11: first_hop_pkcs11.is_some(),
            has_askpass: askpass_script.is_some(),
            any_hop_has_password,
            target_askpass_allowed,
        };
        let multi_hop_identity_only = auth_shape.is_multi_hop_identity_only();

        if auth_shape.needs_explicit_proxy_command() {
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
                    jump_host_passwords.push((AskpassSlot::JumpHost(0), pw.clone()));
                }
            } else {
                proxy_parts.extend(rustconn_core::ssh_tunnel::askpass_disabled_proxy_prefix(
                    jump_hosts.len(),
                ));
            }

            proxy_parts.push("ssh".to_string());
            proxy_parts.push("-W".to_string());
            proxy_parts.push("%h:%p".to_string());

            // Bound the bastion connect the same way the target ssh is bounded
            // (see SshConfig::build_command_args). This ProxyCommand ssh dials
            // the jump host directly, and its port check is always skipped, so
            // without a ConnectTimeout a dead bastion hangs the whole session on
            // the OS TCP timeout — the "connections through jump hang" report.
            // Kept in step with DEFAULT_CONNECT_TIMEOUT in rustconn-core.
            proxy_parts.push("-o".to_string());
            proxy_parts.push("ConnectTimeout=15".to_string());

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
                            ssh_askpass::helper_script(AskpassSlot::JumpHost(rel_idx + 1)).ok()
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
                        jump_host_passwords.push((AskpassSlot::JumpHost(rel_idx + 1), pw.clone()));
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
            //
            // For the multi-hop identity-only case (routed here deliberately),
            // `-J` needs the identity in the agent because it does not pass `-i`
            // to the ssh processes it spawns for each hop. Load it now; the key
            // is added idempotently and only when a real path is known.
            if multi_hop_identity_only
                && let Some(ref key_path) = first_hop_identity.as_ref().or(key.as_ref())
            {
                ensure_identity_in_agent(std::path::Path::new(key_path.as_str()));
            }
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

/// What "Set Up Key Authentication" needs to reach a server the way a live
/// session does: the target with the session's `ssh` arguments, and the
/// credentials the same gates would let a session hand to OpenSSH.
pub(super) struct KeySetupLaunch {
    /// Host, port, user and the arguments of a live session.
    pub(super) target: rustconn_core::ssh_key_setup::SshTarget,
    /// The account password and bastion passwords the gates allow. `Debug`
    /// redacts both.
    pub(super) credentials: rustconn_core::ssh_key_setup::DeployCredentials,
    /// An account password goes to OpenSSH through askpass.
    pub(super) password_sent: bool,
    /// A password was available but the connection's options or proxy route
    /// stopped it from being passed (`target_password_askpass_allowed`,
    /// `ssh_config_declares_proxy`), exactly as for a normal connect.
    pub(super) password_gated_off: bool,
}

/// The session arguments for key setup: the identity file first, then the
/// arguments [`build_ssh_command_args`] produced.
///
/// A live session passes the identity to `spawn_ssh` separately from the extra
/// arguments, so rebuilding the same command line means putting `-i` back. The
/// verify step drops any identity option it is given and uses the new key.
fn key_setup_session_args(identity: Option<&str>, extra_args: Vec<String>) -> Vec<String> {
    match identity {
        Some(path) => {
            let mut args = Vec::with_capacity(extra_args.len() + 2);
            args.push("-i".to_string());
            args.push(path.to_string());
            args.extend(extra_args);
            args
        }
        None => extra_args,
    }
}

/// Builds the key setup target and credentials from a connection, through the
/// same [`build_ssh_command_args`] a live session uses, so the jump hosts, the
/// bastion passwords, the Flatpak `known_hosts` and the askpass gate are not
/// re-derived here.
///
/// `password` is the account password from the vault or typed in the dialog. It
/// reaches the result only if the connect gate lets it; otherwise
/// `password_gated_off` says why a stored password was left out.
/// `declares_proxy` is `ssh -G`'s answer, run off the main thread by the caller.
///
// ponytail: the bastion vault fallback inside `build_ssh_command_args` is
// synchronous, as it is for a connect; and the `ssh.connect.args` perf span
// fires here too. Both are the connect path's behaviour, unchanged.
pub(super) fn build_key_setup_launch(
    conn: &rustconn_core::Connection,
    state: &SharedAppState,
    groups: &[rustconn_core::ConnectionGroup],
    password: Option<SecretString>,
    declares_proxy: Option<bool>,
) -> KeySetupLaunch {
    let global_variables = super::protocols::resolve_globals_with_ask(state, conn.id);
    let host = substitute_variables(&conn.host, &global_variables);
    let username = conn
        .username
        .as_ref()
        .map(|u| substitute_variables(u, &global_variables));

    let (identity, extra_args, _use_waypipe, _jump_host_chain, jump_host_passwords, allowed) =
        build_ssh_command_args(
            conn,
            conn.id,
            state,
            groups,
            password.is_some(),
            declares_proxy,
        );

    assemble_key_setup_launch(
        KeySetupParts {
            host,
            port: conn.port,
            username,
            identity,
            extra_args,
            jump_host_passwords,
        },
        password,
        allowed,
    )
}

/// What [`build_ssh_command_args`] and the variable substitution produced.
struct KeySetupParts {
    host: String,
    port: u16,
    username: Option<String>,
    identity: Option<String>,
    extra_args: Vec<String>,
    jump_host_passwords: Vec<(AskpassSlot, SecretString)>,
}

/// Applies the askpass gate to the account password and builds the launch.
///
/// `allowed` is the connect path's verdict (`target_password_askpass_allowed`
/// and no `ssh_config_declares_proxy`). A closed gate drops the password, so
/// it never reaches `DeployCredentials`, and records why. Split from
/// [`build_key_setup_launch`] so this decision is testable without an
/// application state.
fn assemble_key_setup_launch(
    parts: KeySetupParts,
    password: Option<SecretString>,
    allowed: bool,
) -> KeySetupLaunch {
    let password_gated_off = password.is_some() && !allowed;
    let password = if allowed { password } else { None };
    let password_sent = password.is_some();
    KeySetupLaunch {
        target: rustconn_core::ssh_key_setup::SshTarget::new(
            parts.host,
            parts.port,
            parts.username,
            key_setup_session_args(parts.identity.as_deref(), parts.extra_args),
        ),
        credentials: rustconn_core::ssh_key_setup::DeployCredentials {
            password,
            jump_host_passwords: parts.jump_host_passwords,
        },
        password_sent,
        password_gated_off,
    }
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
    // A live ControlMaster socket means the TCP connection to this host is
    // already up and authenticated, so the pre-connect port check is redundant
    // (P4). Checked here, once, and reused below to also skip writing a
    // target-password askpass file — the master needs no password.
    let has_control_socket = rustconn_core::control_socket_present(&conn.host, conn.port);
    // Use centralized probe-bypass logic + inherited proxy jump from groups
    let should_check = conn.should_pre_connect_check(&settings.connection)
        && !has_inherited_proxy
        && !has_control_socket;

    if conn.bypasses_direct_probe() || has_inherited_proxy || has_control_socket {
        tracing::debug!(
            protocol = "ssh",
            host = %conn.host,
            port = conn.port,
            control_socket = has_control_socket,
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

        // `ssh -G` inputs, cloned for the background closure. Computing whether
        // OpenSSH's own config routes this host through a proxy is a process
        // spawn (~4 ms); it used to run on the GTK main thread inside
        // `build_ssh_command_args`. Here it runs in the same background phase as
        // the port check, so neither blocks the UI, and the answer is handed to
        // the internal start below (P3). It lives only for this connect cycle.
        let gss_host = conn.host.clone();
        let gss_port = conn.port;
        let gss_user = conn.username.clone();

        // Run port check and `ssh -G` in the background thread, together.
        spawn_blocking_with_callback(
            move || {
                let port_ok = check_port(&host, port, timeout);
                let declares_proxy = rustconn_core::ssh_tunnel::ssh_config_declares_proxy(
                    &gss_host,
                    gss_port,
                    gss_user.as_deref(),
                );
                (port_ok, declares_proxy)
            },
            move |(result, declares_proxy)| {
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
                            Some(declares_proxy),
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
            // Port check disabled, so there is no background phase to run
            // `ssh -G` in; fall back to a synchronous spawn in build.
            None,
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
    // `ssh -G`'s answer, computed off the main thread alongside the port check
    // (P3). `None` when it was not precomputed, and `build_ssh_command_args`
    // then falls back to a synchronous spawn.
    precomputed_declares_proxy: Option<bool>,
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

    // Pipe this session's output through the configured output filter, when one
    // is set and installed (`PostpendCommand`). Registered before the spawn,
    // which is what reads it.
    notebook.set_output_filter(session_id, conn.postpend.as_ref());

    // Apply highlight rules (built-in defaults + global + per-connection)
    super::protocols::apply_highlight_rules(state, notebook, session_id, &conn.highlight_rules);

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

    // Retrieve the target credential resolved from the vault earlier. The
    // initial connect caches it just before this, so this is a cache hit; the
    // vault fallback shared with the reconnect path (issue #330) only matters if
    // the entry is somehow absent. It is only exposed to OpenSSH when the strict
    // password-only gate below passes. No other AppState borrow is held across
    // the call.
    let cached_password: Option<SecretString> = state
        .try_borrow_mut()
        .ok()
        .and_then(|mut s| s.ensure_connection_password(connection_id));

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
        precomputed_declares_proxy,
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
            ssh_askpass::helper_script(AskpassSlot::Target).ok()
        } else {
            None
        };
        let (askpass_env, askpass_cleanup_paths) = ssh_askpass::child_env(
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
            let Some(row) = notebook_clone.get_terminal_cursor_row(session_id) else {
                return;
            };
            tracing::debug!(
                protocol = "ssh",
                cursor_row = row,
                uses_jump_host,
                "SSH status detection: checking cursor row"
            );
            // Single shared verdict — see `ssh_connection_established`. For a
            // jump host it requires more output and a failure-free buffer, so a
            // bastion banner alone no longer latches the session green over a
            // target that never came up.
            let text = uses_jump_host
                .then(|| notebook_clone.get_terminal_text(session_id))
                .flatten();
            if super::protocols::ssh_connection_established(row, uses_jump_host, text.as_deref()) {
                sidebar_clone.increment_session_count(&connection_id_str);
                session_connected_clone.set(true);
                tracing::info!(
                    protocol = %protocol_str,
                    cursor_row = row,
                    "Terminal connection detected as established"
                );
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
        // The connection's own on/off value first, then the global switch — the
        // same resolver the in-place reconnect below uses.
        let session_settings = rustconn_core::monitoring::effective_monitoring(
            conn.monitoring_config.as_ref(),
            &state_ref.settings().monitoring,
        );
        if let Some(effective) = session_settings {
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
            let mon_uses_jump_host = has_jump_host;
            let monitoring_started = std::rc::Rc::new(std::cell::Cell::new(false));
            let monitoring_started_clone = monitoring_started.clone();

            notebook.connect_contents_changed(session_id, move || {
                if monitoring_started_clone.get() {
                    return;
                }
                let Some(row) = notebook_clone.get_terminal_cursor_row(session_id) else {
                    return;
                };
                // Exactly the sidebar "connected" verdict — see
                // `ssh_connection_established`. A jump chain can push the cursor
                // past the direct threshold with a bastion banner or an SSH error
                // even when the FINAL host never came up. Starting monitoring then
                // opens the bar for a session that failed — and because the
                // monitoring probe uses accept-new + its own ControlMaster, it can
                // even reach the target and show live data for a session the user
                // saw fail. The closure runs again on the next output, so a later
                // successful prompt still starts it.
                let text = mon_uses_jump_host
                    .then(|| notebook_clone.get_terminal_text(session_id))
                    .flatten();
                if !super::protocols::ssh_connection_established(
                    row,
                    mon_uses_jump_host,
                    text.as_deref(),
                ) {
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

    // Discard the previous connection's monitoring bar and collector so a
    // stale, dimmed bar (or a race with a collector that has not yet
    // noticed the drop) does not survive into the new connection (#398).
    // Done only now, after the two `return false` cases above, so a failed
    // reconnect attempt does not tear down a still-valid bar for nothing.
    // `had_monitoring_bar` records whether a bar actually existed — used
    // below to decide whether to restart monitoring through
    // `session_content_box` (tab or detached window) rather than
    // `get_session_container` (tab only), without ever placing a bar into a
    // split pane where monitoring is suspended by design.
    let had_monitoring_bar = monitoring.reset_for_reconnect(session_id);

    // Re-apply highlight rules
    super::protocols::apply_highlight_rules(state, notebook, session_id, &conn.highlight_rules);

    // Re-register the output filter from the connection as it stands now: it is
    // kept across a reconnect, so without this an edit made in between would be
    // ignored until the tab was closed.
    notebook.set_output_filter(session_id, conn.postpend.as_ref());

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

    // Retrieve the target credential before building the launch plan so proxy
    // routing and auth flags are scoped consistently with initial connect.
    // Resolve from the vault on a cache miss — the initial connect always
    // re-resolves, but a reconnect after the cache TTL expired used to launch
    // with no password (issue #330). No other AppState borrow is held across
    // this blocking call.
    let cached_password: Option<SecretString> = state
        .try_borrow_mut()
        .ok()
        .and_then(|mut s| s.ensure_connection_password(connection_id));

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
        // Reconnect runs on the main thread with no prior background phase, so
        // `ssh -G` is spawned synchronously here (P3 only backgrounds the
        // initial connect, which has the port-check phase to ride along).
        None,
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
            ssh_askpass::helper_script(AskpassSlot::Target).ok()
        } else {
            None
        };
        let (askpass_env, askpass_cleanup_paths) = ssh_askpass::child_env(
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
            let Some(row) = notebook_clone.get_terminal_cursor_row(session_id) else {
                return;
            };
            let text = uses_jump_host
                .then(|| notebook_clone.get_terminal_text(session_id))
                .flatten();
            if super::protocols::ssh_connection_established(row, uses_jump_host, text.as_deref()) {
                sidebar_clone.increment_session_count(&connection_id_str);
                session_connected_clone.set(true);
            }
        });
    }

    // Deferred monitoring start — resolved exactly as on the initial connect.
    if let Ok(state_ref) = state.try_borrow() {
        let session_settings = rustconn_core::monitoring::effective_monitoring(
            conn.monitoring_config.as_ref(),
            &state_ref.settings().monitoring,
        );
        if let Some(effective) = session_settings {
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
            let mon_uses_jump_host = has_jump_host;
            let monitoring_started = std::rc::Rc::new(std::cell::Cell::new(false));
            let monitoring_started_clone = monitoring_started.clone();

            notebook.connect_contents_changed(session_id, move || {
                if monitoring_started_clone.get() {
                    return;
                }
                let Some(row) = notebook_clone.get_terminal_cursor_row(session_id) else {
                    return;
                };
                // See the initial-connect path and `ssh_connection_established`:
                // don't open the monitor for a jump-chain session whose terminal
                // has not really established — the accept-new probe could
                // otherwise show live data for a session that never came up.
                let text = mon_uses_jump_host
                    .then(|| notebook_clone.get_terminal_text(session_id))
                    .flatten();
                if !super::protocols::ssh_connection_established(
                    row,
                    mon_uses_jump_host,
                    text.as_deref(),
                ) {
                    return;
                }
                monitoring_started_clone.set(true);
                // Resolve through `get_session_container` first (tab-only).
                // Only fall back to `session_content_box` when a bar existed
                // before the reconnect reset it (#398) AND the session is
                // confirmed detached right now — not merely "not in a tab".
                // Between `reset_for_reconnect` and this closure running, the
                // session may have been moved into a split as a guest (no
                // `TabPage`, so `get_session_container` also returns `None`
                // for it); `session_content_box` resolves that case through
                // the split-pane box provider, which would place the bar in
                // the pane while monitoring stays suspended there by design.
                // Checking `is_detached` here rules that branch out, so this
                // reconnect path never places a bar into a split pane.
                let container = notebook_clone
                    .get_session_container(session_id)
                    .or_else(|| {
                        (had_monitoring_bar && notebook_clone.is_detached(session_id))
                            .then(|| notebook_clone.session_content_box(session_id))
                            .flatten()
                    });
                if let Some(container) = container {
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

    #[test]
    fn key_setup_session_args_puts_the_identity_first() {
        let args = key_setup_session_args(
            Some("/home/u/.ssh/id_work"),
            vec!["-o".to_string(), "IdentitiesOnly=yes".to_string()],
        );
        assert_eq!(
            args,
            ["-i", "/home/u/.ssh/id_work", "-o", "IdentitiesOnly=yes"]
        );
    }

    #[test]
    fn key_setup_session_args_without_identity_is_unchanged() {
        let extra = vec!["-J".to_string(), "bastion".to_string()];
        assert_eq!(key_setup_session_args(None, extra.clone()), extra);
        assert!(key_setup_session_args(None, Vec::new()).is_empty());
    }

    fn key_setup_parts() -> KeySetupParts {
        KeySetupParts {
            host: "web.example".to_string(),
            port: 2222,
            username: Some("anton".to_string()),
            identity: None,
            extra_args: vec!["-o".to_string(), "IdentitiesOnly=yes".to_string()],
            jump_host_passwords: vec![(
                AskpassSlot::JumpHost(0),
                SecretString::from("bastion-secret".to_string()),
            )],
        }
    }

    #[test]
    fn key_setup_launch_passes_the_password_when_the_gate_is_open() {
        let launch = assemble_key_setup_launch(
            key_setup_parts(),
            Some(SecretString::from("account-secret".to_string())),
            true,
        );
        assert!(launch.password_sent);
        assert!(!launch.password_gated_off);
        assert!(launch.credentials.password.is_some());
        assert_eq!(launch.target.host, "web.example");
        assert_eq!(launch.target.port, 2222);
        assert_eq!(launch.target.username.as_deref(), Some("anton"));
        assert_eq!(launch.target.session_args, ["-o", "IdentitiesOnly=yes"]);
    }

    #[test]
    fn key_setup_launch_drops_the_password_when_the_gate_is_closed() {
        let launch = assemble_key_setup_launch(
            key_setup_parts(),
            Some(SecretString::from("account-secret".to_string())),
            false,
        );
        // The password never reaches the credentials, and the dialog is told why.
        assert!(launch.credentials.password.is_none());
        assert!(!launch.password_sent);
        assert!(launch.password_gated_off);
        // Bastion passwords are not the account password; the connect path
        // decides them and they are carried through.
        assert_eq!(launch.credentials.jump_host_passwords.len(), 1);
    }

    #[test]
    fn key_setup_launch_without_a_password_is_neither_sent_nor_gated_off() {
        for allowed in [true, false] {
            let launch = assemble_key_setup_launch(key_setup_parts(), None, allowed);
            assert!(launch.credentials.password.is_none());
            assert!(!launch.password_sent);
            assert!(!launch.password_gated_off);
        }
    }

    #[test]
    fn key_setup_launch_puts_the_identity_before_the_session_arguments() {
        let mut parts = key_setup_parts();
        parts.identity = Some("/k/id".to_string());
        let launch = assemble_key_setup_launch(parts, None, true);
        assert_eq!(
            launch.target.session_args,
            ["-i", "/k/id", "-o", "IdentitiesOnly=yes"]
        );
    }

    fn jump_shape(hop_count: usize) -> JumpAuthShape {
        // A neutral baseline: identity-only, no password/Flatpak/PKCS#11.
        JumpAuthShape {
            hop_count,
            has_identity: true,
            has_flatpak_known_hosts: false,
            has_pkcs11: false,
            has_askpass: false,
            any_hop_has_password: false,
            target_askpass_allowed: false,
        }
    }

    #[test]
    fn multi_hop_identity_only_routes_to_proxy_jump() {
        // Two hops, identity only, nothing else: this is the case that must use
        // -J + agent, not a nested ProxyCommand.
        let s = jump_shape(2);
        assert!(s.is_multi_hop_identity_only());
        assert!(!s.needs_explicit_proxy_command());
    }

    #[test]
    fn single_hop_identity_still_uses_proxy_command() {
        // One hop with an identity works fine as a local ProxyCommand, so it is
        // NOT flagged multi-hop-identity-only and DOES take the explicit path.
        let s = jump_shape(1);
        assert!(!s.is_multi_hop_identity_only());
        assert!(s.needs_explicit_proxy_command());
    }

    #[test]
    fn multi_hop_with_password_or_flatpak_stays_on_proxy_command() {
        // A password hop or Flatpak known-hosts must keep the explicit
        // ProxyCommand even across multiple hops.
        let mut pw = jump_shape(3);
        pw.any_hop_has_password = true;
        assert!(!pw.is_multi_hop_identity_only());
        assert!(pw.needs_explicit_proxy_command());

        let mut flatpak = jump_shape(3);
        flatpak.has_flatpak_known_hosts = true;
        assert!(!flatpak.is_multi_hop_identity_only());
        assert!(flatpak.needs_explicit_proxy_command());
    }

    #[test]
    fn no_bastion_needs_neither_path() {
        // No hop and no special requirement: not multi-hop-identity, and with
        // no identity/password/etc. the explicit path is not needed.
        let mut s = jump_shape(0);
        s.has_identity = false;
        assert!(!s.is_multi_hop_identity_only());
        assert!(!s.needs_explicit_proxy_command());
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

    // ── P2a: assemble_hop_passwords (pure, fake resolver) ─────────────────

    #[test]
    fn hop_passwords_stay_aligned_with_a_string_proxy_gap() {
        // jump_hosts = [string-proxy, ref-hop-A, ref-hop-B]; the string proxy
        // has no id, so its slot must stay None and the two reference hops keep
        // their own index (SHALL CONTINUE TO: host↔password alignment).
        use secrecy::ExposeSecret;
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let hop_ids = vec![None, Some(a), Some(b)];

        let out = assemble_hop_passwords(3, &hop_ids, None, |hid| {
            Some(SecretString::from(format!("pw-{hid}")))
        });

        assert_eq!(out.len(), 3);
        assert!(out[0].is_none(), "string proxy slot has no password");
        assert_eq!(out[1].as_ref().unwrap().expose_secret(), format!("pw-{a}"));
        assert_eq!(out[2].as_ref().unwrap().expose_secret(), format!("pw-{b}"));
    }

    #[test]
    fn first_hop_password_lands_at_the_first_reference_hop() {
        // first_hop_password covers the first Some entry; the resolver must not
        // be called for it, only for the deeper hop.
        use secrecy::ExposeSecret;
        let first = Uuid::new_v4();
        let deeper = Uuid::new_v4();
        let hop_ids = vec![Some(first), Some(deeper)];
        let mut resolved_ids = Vec::new();

        let out =
            assemble_hop_passwords(2, &hop_ids, Some(SecretString::from("first-pw")), |hid| {
                resolved_ids.push(hid);
                Some(SecretString::from("deeper-pw"))
            });

        assert_eq!(out[0].as_ref().unwrap().expose_secret(), "first-pw");
        assert_eq!(out[1].as_ref().unwrap().expose_secret(), "deeper-pw");
        assert_eq!(
            resolved_ids,
            vec![deeper],
            "the resolver runs only for the deeper hop, never the already-resolved first"
        );
    }

    #[test]
    fn an_unresolvable_hop_stays_none() {
        // A hop whose resolver returns None (no vault entry) leaves a None slot
        // rather than shifting the others (SHALL CONTINUE TO: alignment).
        use secrecy::ExposeSecret;
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let hop_ids = vec![Some(a), Some(b)];

        let out = assemble_hop_passwords(2, &hop_ids, None, |hid| {
            if hid == a {
                None
            } else {
                Some(SecretString::from("b-pw"))
            }
        });

        assert!(out[0].is_none());
        assert_eq!(out[1].as_ref().unwrap().expose_secret(), "b-pw");
    }

    #[test]
    fn no_hops_yields_an_empty_aligned_vec() {
        let out = assemble_hop_passwords(0, &[], None, |_| {
            panic!("resolver must not run when there are no hops")
        });
        assert!(out.is_empty());
    }
}
