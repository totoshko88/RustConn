//! One jump-host route for sessions and script runs.
//!
//! Given the resolved hops of a connection (target-first) and what each needs
//! to authenticate, [`jump_route`] decides between OpenSSH's `-J` and an
//! explicit `ProxyCommand`, and builds the `ProxyCommand` when one is needed.
//! The GUI's session launch and `rustconn-cli run-script` both call it, so a
//! fix to the route reaches both: the two used to build it separately and
//! drifted apart.
//!
//! `-J` spawns child `ssh` processes that inherit neither `-o` nor `-i` from
//! the outer command, and that do inherit its askpass environment. So a
//! Flatpak `known_hosts`, a PKCS#11 token, the first hop's own key, or any
//! password in play forces the explicit `ProxyCommand`, which hands each of
//! them to the hop that needs it.

use std::path::Path;

use secrecy::SecretString;

use crate::config::NetworkSettings;
use crate::models::{Connection, ConnectionGroup};
use crate::ssh_askpass::{self, AskpassSlot};
use crate::ssh_tunnel::{self, HopHostKeys};

/// One jump host and the only password that may go to it.
///
/// The password travels with its hop, so no index can pair it with another
/// host, and [`jump_route`] gives it only that hop's own askpass slot.
#[derive(Debug, Clone)]
pub struct JumpHop {
    /// `[user@]host[:port]`.
    pub spec: String,
    /// This hop's own saved password, if it has one.
    pub password: Option<SecretString>,
}

/// What [`jump_route`] needs to know about a chain of jump hosts.
#[derive(Debug)]
pub struct JumpRouteRequest<'a> {
    /// The hops, the one closest to the target first.
    pub hops: &'a [JumpHop],
    /// The identity file of `hops[0]`, the hop next to the target, whose `ssh`
    /// (the outer `ProxyCommand`) runs on this computer.
    pub first_hop_identity: Option<&'a str>,
    /// The target's identity file. The first hop falls back to it, the common
    /// case of one key for the bastion and the target.
    pub target_identity: Option<&'a str>,
    /// The identity passed to the hops past the first in a nested
    /// `ProxyCommand`.
    pub deeper_hop_identity: Option<&'a str>,
    /// The PKCS#11 provider of the first hop, when it opts in.
    pub first_hop_pkcs11: Option<&'a str>,
    /// A writable `known_hosts` for every hop (Flatpak, where `~/.ssh` is
    /// read-only).
    pub known_hosts: Option<&'a Path>,
    /// What a `ProxyCommand` hop does with an unknown host key.
    pub hop_host_keys: HopHostKeys,
    /// Whether the target's password goes through the askpass helper. Its
    /// environment must then not reach a hop's `ssh`, which `-J` would allow.
    pub target_askpass_allowed: bool,
}

/// How to reach the target through its jump hosts.
#[derive(Debug)]
pub enum JumpRoute {
    /// Pass `-o ProxyCommand=<command>`.
    ProxyCommand {
        /// The command, without the `ProxyCommand=` prefix.
        command: String,
        /// The hop passwords whose askpass helpers the command refers to.
        passwords: Vec<(AskpassSlot, SecretString)>,
    },
    /// Pass `-J <value>`.
    ProxyJump {
        /// The hops in the client-first order `-J` expects.
        value: String,
        /// A key to load into the agent first, because `-J` passes no `-i`
        /// to its hops. Set only for a multi-hop chain that authenticates
        /// with that key alone.
        identity_for_agent: Option<String>,
    },
}

/// Builds the route to the target through `request.hops`.
///
/// Returns `None` when there are no hops. Creates the askpass helper scripts a
/// password hop needs; a hop whose helper cannot be created gets no password,
/// so no unused secret enters the process tree.
#[must_use]
pub fn jump_route(request: &JumpRouteRequest<'_>) -> Option<JumpRoute> {
    let first_hop = request.hops.first()?;
    let hops: Vec<String> = request.hops.iter().map(|hop| hop.spec.clone()).collect();

    // Each hop's own password (issue #191/#203) is delivered here: the nested
    // ProxyCommand ssh has no controlling TTY, so SSH_ASKPASS with
    // SSH_ASKPASS_REQUIRE=force — scoped to it via the shell env-assignment
    // prefix — authenticates the bastion with ITS password. If target askpass
    // is enabled, every hop clears `_RC_TGT_PW_FILE` before starting its own
    // ssh process; the outer helper remains target-only.
    let any_hop_has_password = request.hops.iter().any(|hop| hop.password.is_some());
    let askpass_script = if first_hop.password.is_some() {
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
    // child ssh processes either. So for this case route to `-J` and let the
    // caller load the key into the agent.
    let auth_shape = JumpAuthShape {
        hop_count: hops.len(),
        has_identity: request.first_hop_identity.is_some(),
        has_flatpak_known_hosts: request.known_hosts.is_some(),
        has_pkcs11: request.first_hop_pkcs11.is_some(),
        has_askpass: askpass_script.is_some(),
        any_hop_has_password,
        target_askpass_allowed: request.target_askpass_allowed,
        // A hop that is not plain, such as a free-text `b1,b2` chain or a
        // value from someone else's synced group, keeps `-J`, which OpenSSH
        // parses itself, rather than going unquoted into a shell command.
        known_only: request.hop_host_keys == HopHostKeys::KnownOnly && hops_are_plain(&hops),
    };

    if !auth_shape.needs_explicit_proxy_command() {
        // `hops` is target-first (RustConn's internal order); OpenSSH `-J`
        // visits hops client-first, so `proxy_jump_arg` reverses it.
        let identity_for_agent = auth_shape
            .is_multi_hop_identity_only()
            .then(|| request.first_hop_identity.or(request.target_identity))
            .flatten()
            .map(str::to_string);
        return Some(JumpRoute::ProxyJump {
            value: ssh_tunnel::proxy_jump_arg(&hops.join(",")),
            identity_for_agent,
        });
    }
    Some(proxy_command(
        request,
        &first_hop.spec,
        askpass_script.as_deref(),
        auth_shape.needs_proxy_for_auth(),
    ))
}

/// What [`tunnel_hop_args`] needs to know about the hops past a tunnel's own
/// jump host.
#[derive(Debug)]
pub struct TunnelHopsRequest<'a> {
    /// The hops past the tunnel's jump host as `[user@]host[:port]`, in the
    /// order of [`JumpRouteRequest::hops`]: the one next to that jump host
    /// first.
    pub hops: &'a [String],
    /// The identity file of `hops[0]`.
    pub first_hop_identity: Option<&'a str>,
    /// The identity passed to the hops past `hops[0]`.
    pub deeper_hop_identity: Option<&'a str>,
    /// The PKCS#11 provider of `hops[0]`, when it opts in.
    pub first_hop_pkcs11: Option<&'a str>,
    /// A writable `known_hosts` for every hop (Flatpak).
    pub known_hosts: Option<&'a Path>,
    /// What a hop does with an unknown host key.
    pub hop_host_keys: HopHostKeys,
}

/// The `ssh` arguments that take a tunnel past its jump host when the tunnel
/// answers that jump host's password through its askpass helper.
///
/// `-J` would start an `ssh` for each further hop that inherits the tunnel's
/// askpass environment, so a farther bastion could be sent the password of the
/// tunnel's jump host (the issue #191 class). The `ProxyCommand` built here
/// starts every hop behind an `env` boundary that clears that environment. The
/// hops get no password of their own. A GUI tunnel has no terminal to ask on,
/// so there they must log in with a key or the agent, and a hop that accepts
/// only a password fails. A tunnel that `rustconn-cli connect` or
/// `run-script` starts from a terminal may have a hop's `ssh` ask for its
/// password on that terminal.
///
/// Returns `None` when a hop, key path or provider is not plain, because it
/// would go into a command a shell runs. The caller then must not send the
/// password; [`crate::ssh_tunnel::create_tunnel`] refuses to as well.
#[must_use]
pub fn tunnel_hop_args(request: &TunnelHopsRequest<'_>) -> Option<Vec<String>> {
    if !hops_are_plain(request.hops) {
        return None;
    }
    // ponytail: farther tunnel hops get no saved password (`password: None`);
    // give a hop its own password here, as sessions do, if a password-only
    // bastion behind a tunnel jump host is reported.
    let hops: Vec<JumpHop> = request
        .hops
        .iter()
        .map(|spec| JumpHop {
            spec: spec.clone(),
            password: None,
        })
        .collect();
    let route = jump_route(&JumpRouteRequest {
        hops: &hops,
        first_hop_identity: request.first_hop_identity.and_then(plain_path),
        target_identity: None,
        deeper_hop_identity: request.deeper_hop_identity.and_then(plain_path),
        first_hop_pkcs11: request
            .first_hop_pkcs11
            .map(str::trim)
            .filter(|provider| !provider.is_empty())
            .and_then(plain_path),
        known_hosts: request.known_hosts,
        hop_host_keys: request.hop_host_keys,
        // The tunnel's own helper is the askpass environment to keep away
        // from the hops; this always gives a ProxyCommand.
        target_askpass_allowed: true,
    })?;
    match route {
        JumpRoute::ProxyCommand { command, .. } => {
            Some(vec!["-o".to_string(), format!("ProxyCommand={command}")])
        }
        JumpRoute::ProxyJump { .. } => None,
    }
}

/// The `ssh` arguments (`-J` or `-o ProxyCommand=…`) that take a tunnel (RDP,
/// VNC, SPICE, WinRM, Web) to `jump_conn` through the jump hosts past it, or
/// none when `jump_conn` has none.
///
/// Only the first hop of the chain is the tunnel's own jump host, so the
/// hops walked here are the ones behind it, each by its own `jump_host_id`: a
/// bastion's bastion belongs to that bastion, not to the target's group.
///
/// `tunnel_has_password` is true when the tunnel answers `jump_conn`'s password
/// through its askpass helper. Those hops then go through [`tunnel_hop_args`],
/// so none of them inherits that helper. When they cannot (an address a shell
/// could interpret), the usual route is returned and `create_tunnel` does not
/// send the password.
#[must_use]
pub fn tunnel_jump_args(
    jump_conn: &Connection,
    connections: &[Connection],
    groups: &[ConnectionGroup],
    network: &NetworkSettings,
    tunnel_has_password: bool,
    hop_host_keys: HopHostKeys,
) -> Vec<String> {
    // Check if the jump host itself has a jump host (recursive chain)
    let ssh_config = match &jump_conn.protocol_config {
        crate::ProtocolConfig::Ssh(cfg) => cfg,
        _ => return Vec::new(),
    };

    // Collect the chain of jump hosts above the immediate jump host
    let mut chain: Vec<String> = Vec::new();

    // First check for string-based proxy_jump on the jump host
    if let Some(proxy) =
        crate::connection::ssh_inheritance::resolve_ssh_proxy_jump(jump_conn, groups, network)
    {
        chain.push(proxy);
    }

    // Then resolve reference-based jump hosts recursively
    // Also resolve the identity file for the first hop (needed for ProxyCommand)
    let mut first_hop_identity: Option<String> = None;
    // PKCS#11 provider of the first hop (not inherited via -J — inject into ProxyCommand)
    let mut first_hop_pkcs11: Option<String> = None;
    if let Some(parent_jump_id) = ssh_config.jump_host_id {
        let mut current_id = Some(parent_jump_id);
        let mut visited = std::collections::HashSet::new();
        visited.insert(jump_conn.id); // Avoid self-reference
        let mut is_first = true;

        for _ in 0..10 {
            if let Some(jid) = current_id {
                if visited.contains(&jid) {
                    break;
                }
                visited.insert(jid);

                if let Some(parent_conn) = connections.iter().find(|c| c.id == jid) {
                    // Resolve identity file for the first hop
                    if is_first {
                        first_hop_identity =
                            crate::connection::ssh_inheritance::resolve_ssh_key_path(
                                parent_conn,
                                groups,
                            )
                            .and_then(|p| crate::resolve_key_path(&p))
                            .map(|p| p.to_string_lossy().to_string());
                        if let crate::ProtocolConfig::Ssh(parent_cfg) = &parent_conn.protocol_config
                        {
                            first_hop_pkcs11 = parent_cfg
                                .pkcs11_provider
                                .clone()
                                .filter(|p| !p.trim().is_empty());
                        }
                        is_first = false;
                    }

                    // Format: [user@]host[:port] for -J
                    let mut host_str = parent_conn.host.clone();
                    if let Some(user) = &parent_conn.username {
                        host_str = format!("{user}@{host_str}");
                    }
                    if parent_conn.port != 22 {
                        host_str = format!("{host_str}:{}", parent_conn.port);
                    }
                    chain.push(host_str);

                    // Continue up the chain
                    if let crate::ProtocolConfig::Ssh(parent_cfg) = &parent_conn.protocol_config {
                        if let Some(p) = &parent_cfg.proxy_jump {
                            chain.insert(chain.len() - 1, p.clone());
                        }
                        current_id = parent_cfg.jump_host_id;
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

    if chain.is_empty() {
        return Vec::new();
    }
    tunnel_route(
        &chain,
        first_hop_identity.as_deref(),
        first_hop_pkcs11.as_deref(),
        tunnel_has_password,
        hop_host_keys,
    )
}

/// The `ssh` arguments for the non-empty `chain` of hops behind a tunnel's jump
/// host: see [`tunnel_jump_args`].
fn tunnel_route(
    chain: &[String],
    first_hop_identity: Option<&str>,
    first_hop_pkcs11: Option<&str>,
    tunnel_has_password: bool,
    hop_host_keys: HopHostKeys,
) -> Vec<String> {
    // In Flatpak, use ProxyCommand so the nested SSH inherits known_hosts
    let flatpak_kh = crate::get_flatpak_known_hosts_path();
    let proxy_command_route = flatpak_kh.is_some() || first_hop_pkcs11.is_some();
    if tunnel_has_password {
        // The deeper hops get what the routes below give them: the first
        // hop's key only where those use a ProxyCommand.
        let request = TunnelHopsRequest {
            hops: chain,
            first_hop_identity,
            deeper_hop_identity: first_hop_identity.filter(|_| proxy_command_route),
            first_hop_pkcs11,
            known_hosts: flatpak_kh.as_deref(),
            hop_host_keys,
        };
        if let Some(args) = tunnel_hop_args(&request) {
            return args;
        }
        tracing::warn!(
            "Tunnel: a jump host past the first has an address RustConn does not pass to a \
             shell, so the first jump host's password is not sent"
        );
    }
    if proxy_command_route {
        // Build ProxyCommand for the first hop in the chain. `-J` does not pass
        // -o/-i to the nested SSH, so Flatpak known_hosts and PKCS#11 tokens must
        // be injected explicitly here.
        let mut proxy_parts = vec!["ssh".to_string(), "-W".to_string(), "%h:%p".to_string()];
        if let Some(option) = flatpak_kh.as_deref().and_then(proxy_known_hosts_option) {
            proxy_parts.push("-o".to_string());
            proxy_parts.push(option);
        }

        // Pass identity file for the first hop if available
        if let Some(key) = first_hop_identity {
            proxy_parts.push("-i".to_string());
            proxy_parts.push(ssh_tunnel::shell_word(key));
            proxy_parts.push("-o".to_string());
            proxy_parts.push("IdentitiesOnly=yes".to_string());
        }

        // Pass PKCS#11 provider for the first hop (token also auths the bastion)
        if let Some(option) = first_hop_pkcs11.and_then(proxy_pkcs11_option) {
            proxy_parts.push("-o".to_string());
            proxy_parts.push(option);
        }

        // Reuse the jump host's ControlMaster socket so parallel tunnel
        // establishments share the already-authenticated link. Only set
        // ControlPath — if the master exists, SSH multiplexes; if not, it
        // connects standalone (no auth issues from missing identity).
        {
            let (jh_host, jh_port) = parse_jump_host_for_control(&chain[0]);
            let jh_control = crate::ssh_control_path(&jh_host, jh_port);
            proxy_parts.push("-o".to_string());
            proxy_parts.push(format!("ControlPath={jh_control}"));
        }

        // ponytail: PKCS#11/identity reach only the first hop; deeper hops do
        // not get the bastion's own PKCS#11 token. Fine for the common
        // single-bastion case.
        //
        // Multi-hop: nest a ProxyCommand per remaining hop so EACH inherits the
        // identity file and Flatpak known_hosts. Plain `-J b,c` would drop them
        // and the deeper hops fail in Flatpak (issue #191 follow-up — double jump).
        if chain.len() > 1 {
            let inner_hops: Vec<&str> = chain[1..].iter().map(String::as_str).collect();
            let inner = ssh_tunnel::build_nested_proxy_command(
                &inner_hops,
                first_hop_identity,
                flatpak_kh.as_deref(),
                ssh_tunnel::HopHostKeys::UserConfig,
            );
            proxy_parts.push("-o".to_string());
            proxy_parts.push(format!(
                "ProxyCommand={}",
                ssh_tunnel::shell_single_quote(&inner)
            ));
        }

        // Add the first hop destination with proper -p port parsing
        ssh_tunnel::append_proxy_command_destination(&mut proxy_parts, &chain[0]);

        let proxy_cmd = proxy_parts.join(" ");
        tracing::debug!(
            proxy_command = %proxy_cmd,
            "Tunnel: using ProxyCommand for jump host chain (Flatpak known_hosts or PKCS#11)"
        );
        vec!["-o".to_string(), format!("ProxyCommand={proxy_cmd}")]
    } else {
        // Non-Flatpak: use standard -J. RustConn resolves chains target-first,
        // but OpenSSH `-J` visits hops client-first, so reverse them.
        let j_chain = ssh_tunnel::proxy_jump_arg(&chain.join(","));
        tracing::debug!(
            jump_chain = %j_chain,
            "Tunnel: using -J for jump host chain"
        );
        vec!["-J".to_string(), j_chain]
    }
}

/// A character a shell passes through unchanged inside a `ProxyCommand`.
fn is_plain(c: char) -> bool {
    c.is_ascii_alphanumeric() || ".-_:@[]".contains(c)
}

/// Whether every hop is a plain `[user@]host[:port]` that is safe to put into
/// a `ProxyCommand`, which ssh runs through a shell.
#[must_use]
pub fn hops_are_plain(hops: &[String]) -> bool {
    !hops.is_empty()
        && hops
            .iter()
            .all(|hop| !hop.is_empty() && !hop.starts_with('-') && hop.chars().all(is_plain))
}

/// `path` when a shell would pass it through a `ProxyCommand` unchanged.
#[must_use]
pub fn plain_path(path: &str) -> Option<&str> {
    path.chars()
        .all(|c| is_plain(c) || c == '/')
        .then_some(path)
}

/// The `PKCS11Provider=<path>` option for an `ssh` inside a `ProxyCommand`.
///
/// `None` when [`plain_path`] refuses the provider: a shell runs that command,
/// so a provider with a space or a metacharacter is left out, as the tunnel
/// route already does, rather than quoted twice.
fn proxy_pkcs11_option(provider: &str) -> Option<String> {
    let option = plain_path(provider.trim()).map(|plain| format!("PKCS11Provider={plain}"));
    if option.is_none() {
        tracing::warn!("PKCS#11 provider path is not passed to a shell; the jump host gets none");
    }
    option
}

/// The `UserKnownHostsFile=<path>` option for an `ssh` inside a `ProxyCommand`.
///
/// A shell runs that command, so this is `None` when [`plain_path`] refuses
/// `path`, such as a home folder with a space or a `;` in its name. Such a
/// path could be carried by quoting it twice, for the shell and then for ssh
/// (which reads `UserKnownHostsFile="<path>"` as one file, see
/// [`crate::ssh_tunnel::user_known_hosts_option`]); that is not done, because
/// a home path like that is rare. Without it the hop uses its default
/// `known_hosts`: a host already known there connects as before, and an
/// unknown one is left to the host key policy, so under `accept-new` it still
/// connects but its key cannot be recorded in the Flatpak's read-only `~/.ssh`.
#[must_use]
pub fn proxy_known_hosts_option(path: &Path) -> Option<String> {
    let option = path
        .to_str()
        .and_then(plain_path)
        .map(|plain| format!("UserKnownHostsFile={plain}"));
    if option.is_none() {
        tracing::debug!(
            "Flatpak known_hosts path is not passed to a shell; jump hosts use the default"
        );
    }
    option
}

/// Builds the explicit `ProxyCommand` route to `first_hop`, nesting the deeper
/// hops. `askpass_script` is the first hop's helper, when it has a password.
///
/// `offer_keys` is false when only [`HopHostKeys::KnownOnly`] asked for the
/// `ProxyCommand`. The hops then get no `-i`: under `-J` they offered
/// `~/.ssh/id_*` and the agent's keys, and `-i <target key> -o
/// IdentitiesOnly=yes` would take those away from a bastion that logs in
/// with one of them.
fn proxy_command(
    request: &JumpRouteRequest<'_>,
    first_hop: &str,
    askpass_script: Option<&Path>,
    offer_keys: bool,
) -> JumpRoute {
    let hop_count = request.hops.len();

    // Build a ProxyCommand for the first hop;
    // if there are multiple hops, nest them via nested ProxyCommand.
    let mut passwords: Vec<(AskpassSlot, SecretString)> = Vec::new();
    let mut proxy_parts: Vec<String> = Vec::new();

    // Every explicit proxy hop starts with an `env` boundary. A hop with its
    // own password gets that helper; all other hops explicitly disable
    // askpass and clear the target credential before `ssh`.
    if let Some(script) = askpass_script {
        proxy_parts.extend(ssh_tunnel::askpass_proxy_prefix(script, 0, hop_count));
        if let Some(pw) = request.hops.first().and_then(|hop| hop.password.as_ref()) {
            passwords.push((AskpassSlot::JumpHost(0), pw.clone()));
        }
    } else {
        proxy_parts.extend(ssh_tunnel::askpass_disabled_proxy_prefix(hop_count));
    }

    proxy_parts.push("ssh".to_string());
    proxy_parts.push("-W".to_string());
    proxy_parts.push("%h:%p".to_string());

    // Bound the bastion connect the same way the target ssh is bounded
    // (see SshConfig::build_command_args). This ProxyCommand ssh dials the
    // jump host directly, and its port check is always skipped, so without a
    // ConnectTimeout a dead bastion hangs the whole session on the OS TCP
    // timeout — the "connections through jump hang" report. Kept in step
    // with DEFAULT_CONNECT_TIMEOUT.
    proxy_parts.push("-o".to_string());
    proxy_parts.push("ConnectTimeout=15".to_string());

    // First hop reached via forced askpass (its own password, no TTY) must
    // not raise the "yes/no/[fingerprint]" prompt: it is routed to the
    // askpass helper, which answers with the PASSWORD and loops until the
    // bastion drops the connection ("Connection closed by UNKNOWN port
    // 65535", #203). A session accepts a first-seen key (`accept-new`); a
    // script run refuses it (`yes`).
    if let Some(option) =
        ssh_tunnel::proxy_hop_host_key_option(request.hop_host_keys, askpass_script.is_some())
    {
        proxy_parts.push("-o".to_string());
        proxy_parts.push(option.to_string());
    }

    // Pass identity file to jump host if we have one. Prefer the jump host's
    // OWN resolved identity; fall back to the target's key (common pattern:
    // same .pem for bastion + target).
    if offer_keys && let Some(key_path) = request.first_hop_identity.or(request.target_identity) {
        proxy_parts.push("-i".to_string());
        proxy_parts.push(ssh_tunnel::shell_word(key_path));
        proxy_parts.push("-o".to_string());
        proxy_parts.push("IdentitiesOnly=yes".to_string());
    }

    // Pass PKCS#11 provider to the first hop (token also auths the bastion)
    if let Some(option) = request.first_hop_pkcs11.and_then(proxy_pkcs11_option) {
        proxy_parts.push("-o".to_string());
        proxy_parts.push(option);
    }

    // Pass UserKnownHostsFile to jump host (Flatpak only)
    if let Some(option) = request.known_hosts.and_then(proxy_known_hosts_option) {
        proxy_parts.push("-o".to_string());
        proxy_parts.push(option);
    }

    // Reuse the jump host's ControlMaster socket (created by the direct
    // jump-host tab) so parallel ProxyCommand connections share the
    // already-authenticated link instead of each opening a new TCP. Only set
    // ControlPath (no ControlMaster=auto): if the socket exists, SSH uses it
    // as slave transparently; if not, SSH ignores the option and connects
    // standalone — no identity/auth issues.
    {
        let (jh_host, jh_port) = parse_jump_host_for_control(first_hop);
        let jh_control = crate::ssh_control_path(&jh_host, jh_port);
        proxy_parts.push("-o".to_string());
        proxy_parts.push(format!("ControlPath={jh_control}"));
    }

    // ponytail: PKCS#11/identity reach only the first hop; deeper hops still
    // don't get the bastion's own PKCS#11 token. Fine for the common
    // single-bastion case.
    //
    // Multi-hop (issue #203): nest a ProxyCommand per remaining hop so EACH
    // inherits the identity file, Flatpak known_hosts, AND its own
    // SSH_ASKPASS helper if the hop authenticates by password.
    if hop_count > 1 {
        let inner = nested_proxy_command(request, &mut passwords, offer_keys);
        proxy_parts.push("-o".to_string());
        proxy_parts.push(format!(
            "ProxyCommand={}",
            ssh_tunnel::shell_single_quote(&inner)
        ));
    }

    // Add the first hop destination (parse user@host:port into -p port user@host)
    ssh_tunnel::append_proxy_command_destination(&mut proxy_parts, first_hop);

    let command = proxy_parts.join(" ");
    tracing::debug!(
        protocol = "ssh",
        proxy_command = %command,
        hop_passwords_count = passwords.len(),
        "Using ProxyCommand instead of -J (identity/Flatpak/PKCS#11/password)"
    );
    JumpRoute::ProxyCommand { command, passwords }
}

/// The nested `ProxyCommand` that reaches `hops[1]` through the hops past it,
/// adding the passwords of those hops to `passwords`. `offer_keys` is the
/// [`proxy_command`] argument of that name.
fn nested_proxy_command(
    request: &JumpRouteRequest<'_>,
    passwords: &mut Vec<(AskpassSlot, SecretString)>,
    offer_keys: bool,
) -> String {
    let deeper = request.hops.get(1..).unwrap_or(&[]);
    let inner_hops: Vec<&str> = deeper.iter().map(|hop| hop.spec.as_str()).collect();

    // Build per-hop askpass scripts for deeper hops (issue #203).
    let inner_askpass: Vec<Option<std::path::PathBuf>> = deeper
        .iter()
        .enumerate()
        .map(|(rel_idx, hop)| {
            if hop.password.is_some() {
                ssh_askpass::helper_script(AskpassSlot::JumpHost(rel_idx + 1)).ok()
            } else {
                None
            }
        })
        .collect();
    // Expose a deeper hop's password only if its helper exists. Helper
    // creation failure is fail-closed: the hop remains manual/unavailable,
    // but no unused secret enters the process tree.
    for (rel_idx, (hop, script_opt)) in deeper.iter().zip(&inner_askpass).enumerate() {
        if let (Some(pw), Some(_script)) = (&hop.password, script_opt) {
            passwords.push((AskpassSlot::JumpHost(rel_idx + 1), pw.clone()));
        }
    }

    let askpass_refs: Vec<Option<&Path>> = inner_askpass.iter().map(|opt| opt.as_deref()).collect();

    // For a session the user's config decides the host-key posture of hops
    // without a password (the bastions are expected to already be in
    // known_hosts) and a password hop gets accept-new (#203); a script run
    // gives every hop `yes`.
    ssh_tunnel::build_nested_proxy_command_with_askpass(
        &inner_hops,
        request.deeper_hop_identity.filter(|_| offer_keys),
        request.known_hosts,
        request.hop_host_keys,
        &askpass_refs,
        1,
        request.hops.len(),
    )
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
    /// Every hop must already be in `known_hosts` ([`HopHostKeys::KnownOnly`])
    /// and every hop passes [`hops_are_plain`].
    known_only: bool,
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
    /// So does [`HopHostKeys::KnownOnly`]: only a `ProxyCommand` hop can be
    /// given `StrictHostKeyChecking=yes`, and a script run must never stop at a
    /// host-key question nobody is there to answer. Hops that are not plain
    /// keep `-J` (see [`Self::known_only`]).
    ///
    /// The one exception is [`Self::is_multi_hop_identity_only`], which `-J` +
    /// agent serves better than a nested `ProxyCommand` that cannot carry a
    /// local key past the first hop. Such a chain follows the user's ssh
    /// configuration even under `KnownOnly`.
    fn needs_explicit_proxy_command(&self) -> bool {
        !self.is_multi_hop_identity_only() && (self.needs_proxy_for_auth() || self.known_only)
    }

    /// Whether something the first hop authenticates or verifies with has to
    /// be handed to it, which `-J` cannot do. [`Self::known_only`] is left out.
    fn needs_proxy_for_auth(&self) -> bool {
        self.has_flatpak_known_hosts
            || self.has_pkcs11
            || self.has_identity
            || self.has_askpass
            || self.any_hop_has_password
            || self.target_askpass_allowed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            known_only: false,
        }
    }

    fn hops(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| (*n).to_string()).collect()
    }

    /// The hops of `names`, each bound to the password at its position.
    fn jump_hops(names: &[String], passwords: &[Option<SecretString>]) -> Vec<JumpHop> {
        assert_eq!(names.len(), passwords.len());
        names
            .iter()
            .zip(passwords)
            .map(|(spec, password)| JumpHop {
                spec: spec.clone(),
                password: password.clone(),
            })
            .collect()
    }

    fn request(hops: &[JumpHop], hop_host_keys: HopHostKeys) -> JumpRouteRequest<'_> {
        JumpRouteRequest {
            hops,
            first_hop_identity: None,
            target_identity: None,
            deeper_hop_identity: None,
            first_hop_pkcs11: None,
            known_hosts: None,
            hop_host_keys,
            target_askpass_allowed: false,
        }
    }

    fn secret() -> Option<SecretString> {
        Some(SecretString::from("hunter2".to_string()))
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
    fn the_control_path_host_is_parsed_from_every_hop_form() {
        assert_eq!(
            parse_jump_host_for_control("admin@[2001:db8::1]:2222"),
            ("2001:db8::1".to_string(), 2222)
        );
        assert_eq!(
            parse_jump_host_for_control("[2001:db8::1]"),
            ("2001:db8::1".to_string(), 22)
        );
        assert_eq!(
            parse_jump_host_for_control("bastion.example:2200"),
            ("bastion.example".to_string(), 2200)
        );
    }

    #[test]
    fn no_hops_give_no_route() {
        assert!(jump_route(&request(&[], HopHostKeys::UserConfig)).is_none());
    }

    #[test]
    fn a_password_hop_gets_its_helper_and_slot() {
        let chain = hops(&["admin@bastion.example"]);
        let chain = jump_hops(&chain, &[secret()]);
        let Some(JumpRoute::ProxyCommand { command, passwords }) =
            jump_route(&request(&chain, HopHostKeys::UserConfig))
        else {
            panic!("a password hop needs a ProxyCommand");
        };
        assert!(command.contains("SSH_ASKPASS_REQUIRE=force"), "{command}");
        assert!(command.contains("ConnectTimeout=15"), "{command}");
        assert!(command.contains("ControlPath="), "{command}");
        assert!(command.ends_with("admin@bastion.example"), "{command}");
        assert_eq!(passwords.len(), 1);
        assert_eq!(passwords[0].0, AskpassSlot::JumpHost(0));
        assert!(
            !command.contains("hunter2"),
            "the password never enters argv"
        );
    }

    #[test]
    fn known_only_puts_strict_checking_on_every_hop() {
        let chain = hops(&["b1.example", "b2.example"]);
        let chain = jump_hops(&chain, &[secret(), secret()]);
        let Some(JumpRoute::ProxyCommand { command, passwords }) =
            jump_route(&request(&chain, HopHostKeys::KnownOnly))
        else {
            panic!("password hops need a ProxyCommand");
        };
        assert_eq!(
            command.matches("StrictHostKeyChecking=yes").count(),
            2,
            "{command}"
        );
        assert!(!command.contains("accept-new"), "{command}");
        assert_eq!(passwords.len(), 2);
    }

    #[test]
    fn known_only_routes_a_key_only_hop_through_proxy_command() {
        let chain = hops(&["bastion.example"]);
        let chain = jump_hops(&chain, &[None]);
        let Some(JumpRoute::ProxyCommand { command, passwords }) =
            jump_route(&request(&chain, HopHostKeys::KnownOnly))
        else {
            panic!("a script-run hop needs a ProxyCommand to refuse unknown keys");
        };
        assert!(command.contains("StrictHostKeyChecking=yes"), "{command}");
        assert!(command.ends_with("bastion.example"), "{command}");
        assert!(passwords.is_empty());
    }

    #[test]
    fn known_only_alone_does_not_narrow_a_bastion_to_the_target_key() {
        // Under -J such a bastion offered ~/.ssh/id_* and the agent's keys;
        // `-i <target> -o IdentitiesOnly=yes` would leave it only the target's.
        for chain in [
            hops(&["bastion.example"]),
            hops(&["b1.example", "b2.example"]),
        ] {
            let chain = jump_hops(&chain, &vec![None; chain.len()]);
            let mut req = request(&chain, HopHostKeys::KnownOnly);
            req.target_identity = Some("/k/target");
            req.deeper_hop_identity = Some("/k/deeper");
            let Some(JumpRoute::ProxyCommand { command, .. }) = jump_route(&req) else {
                panic!("a script-run hop needs a ProxyCommand: {chain:?}");
            };
            for absent in [" -i ", "IdentitiesOnly", "/k/target", "/k/deeper"] {
                assert!(!command.contains(absent), "{absent}: {command}");
            }
            assert_eq!(
                command.matches("StrictHostKeyChecking=yes").count(),
                chain.len(),
                "{command}"
            );
        }
    }

    #[test]
    fn a_password_hop_under_known_only_still_falls_back_to_the_target_key() {
        let chain = hops(&["bastion.example"]);
        let chain = jump_hops(&chain, &[secret()]);
        let mut req = request(&chain, HopHostKeys::KnownOnly);
        req.target_identity = Some("/k/target");
        let Some(JumpRoute::ProxyCommand { command, .. }) = jump_route(&req) else {
            panic!("a password hop needs a ProxyCommand");
        };
        assert!(
            command.contains("-i /k/target -o IdentitiesOnly=yes"),
            "{command}"
        );
    }

    #[test]
    fn known_only_keeps_proxy_jump_for_a_hop_a_shell_could_interpret() {
        // `b1,b2` is a free-text chain that only `-J` understands; `;` would
        // reach /bin/sh through a ProxyCommand.
        for hop in ["b1.example,b2.example", "bastion;reboot"] {
            let chain = hops(&[hop]);
            let chain = jump_hops(&chain, &[None]);
            let mut req = request(&chain, HopHostKeys::KnownOnly);
            req.target_identity = Some("/k/target");
            let route = jump_route(&req);
            assert!(
                matches!(route, Some(JumpRoute::ProxyJump { .. })),
                "{hop}: {route:?}"
            );
        }
    }

    #[test]
    fn anything_a_shell_could_interpret_is_not_plain() {
        for hop in ["jump;reboot", "$(reboot)", "-oProxyCommand=x", "b1,b2", ""] {
            assert!(!hops_are_plain(&[hop.to_string()]), "{hop}");
        }
        assert!(!hops_are_plain(&[]), "no hops is not a route");
        assert!(hops_are_plain(&[
            "alice@jump:2222".to_string(),
            "[2001:db8::1]".to_string()
        ]));
        assert_eq!(
            plain_path("/home/a/.ssh/id_ed25519"),
            Some("/home/a/.ssh/id_ed25519")
        );
        assert_eq!(plain_path("/home/a/my key"), None);
    }

    #[test]
    fn user_config_keeps_proxy_jump_for_a_plain_hop() {
        let chain = hops(&["bastion.example"]);
        let chain = jump_hops(&chain, &[None]);
        let Some(JumpRoute::ProxyJump {
            value,
            identity_for_agent,
        }) = jump_route(&request(&chain, HopHostKeys::UserConfig))
        else {
            panic!("a session's plain hop keeps -J");
        };
        assert_eq!(value, "bastion.example");
        assert_eq!(identity_for_agent, None);
    }

    #[test]
    fn known_only_multi_hop_identity_only_stays_on_proxy_jump() {
        // A nested ProxyCommand cannot carry the local key past the first hop,
        // so this chain keeps -J with the key in the agent.
        let chain = hops(&["b1.example", "b2.example"]);
        let chain = jump_hops(&chain, &[None, None]);
        let mut req = request(&chain, HopHostKeys::KnownOnly);
        req.first_hop_identity = Some("/k/bastion");
        let Some(JumpRoute::ProxyJump {
            identity_for_agent, ..
        }) = jump_route(&req)
        else {
            panic!("a multi-hop key-only chain uses -J");
        };
        assert_eq!(identity_for_agent.as_deref(), Some("/k/bastion"));
    }

    #[test]
    fn user_config_keeps_accept_new_for_a_password_hop() {
        let chain = hops(&["bastion.example"]);
        let chain = jump_hops(&chain, &[secret()]);
        let Some(JumpRoute::ProxyCommand { command, .. }) =
            jump_route(&request(&chain, HopHostKeys::UserConfig))
        else {
            panic!("a password hop needs a ProxyCommand");
        };
        assert!(
            command.contains("StrictHostKeyChecking=accept-new"),
            "{command}"
        );
    }

    #[test]
    fn multi_hop_identity_only_goes_to_proxy_jump_with_the_agent_key() {
        let chain = hops(&["b1.example", "b2.example"]);
        let chain = jump_hops(&chain, &[None, None]);
        let mut req = request(&chain, HopHostKeys::UserConfig);
        req.first_hop_identity = Some("/k/bastion");
        req.target_identity = Some("/k/target");
        let Some(JumpRoute::ProxyJump {
            value,
            identity_for_agent,
        }) = jump_route(&req)
        else {
            panic!("a multi-hop key-only chain uses -J");
        };
        assert_eq!(value, "b2.example,b1.example");
        assert_eq!(identity_for_agent.as_deref(), Some("/k/bastion"));
    }

    #[test]
    fn no_identity_no_password_user_config_uses_proxy_jump() {
        let chain = hops(&["b1.example", "b2.example", "b3.example"]);
        let chain = jump_hops(&chain, &[None, None, None]);
        let Some(JumpRoute::ProxyJump {
            value,
            identity_for_agent,
        }) = jump_route(&request(&chain, HopHostKeys::UserConfig))
        else {
            panic!("nothing to hand to the hops, so -J");
        };
        assert_eq!(value, "b3.example,b2.example,b1.example");
        assert_eq!(identity_for_agent, None);
    }

    #[test]
    fn flatpak_known_hosts_and_pkcs11_reach_the_first_hop() {
        let chain = hops(&["bastion.example:2200"]);
        let chain = jump_hops(&chain, &[None]);
        let known_hosts = Path::new("/var/kh");
        let mut req = request(&chain, HopHostKeys::UserConfig);
        req.known_hosts = Some(known_hosts);
        req.first_hop_pkcs11 = Some(" /usr/lib/opensc-pkcs11.so ");
        req.target_identity = Some("/k/target");
        let Some(JumpRoute::ProxyCommand { command, passwords }) = jump_route(&req) else {
            panic!("Flatpak known_hosts needs a ProxyCommand");
        };
        assert!(command.contains("UserKnownHostsFile=/var/kh"), "{command}");
        assert!(
            command.contains("PKCS11Provider=/usr/lib/opensc-pkcs11.so"),
            "{command}"
        );
        assert!(
            command.contains("-i /k/target -o IdentitiesOnly=yes"),
            "{command}"
        );
        assert!(passwords.is_empty());
    }

    #[test]
    fn a_known_hosts_path_a_shell_would_split_stays_out_of_the_proxy_command() {
        assert_eq!(
            proxy_known_hosts_option(Path::new("/home/u/.var/app/x/.ssh/known_hosts")),
            Some("UserKnownHostsFile=/home/u/.var/app/x/.ssh/known_hosts".to_string())
        );
        let chain = hops(&["near.example", "far.example"]);
        let chain = jump_hops(&chain, &[None, None]);
        for unusual in ["/home/a b/kh", "/home/a;b/kh", "/home/a'b/kh"] {
            let known_hosts = Path::new(unusual);
            assert_eq!(proxy_known_hosts_option(known_hosts), None, "{unusual}");
            let mut req = request(&chain, HopHostKeys::UserConfig);
            req.known_hosts = Some(known_hosts);
            let Some(JumpRoute::ProxyCommand { command, .. }) = jump_route(&req) else {
                panic!("Flatpak known_hosts still takes the ProxyCommand route");
            };
            assert!(!command.contains("UserKnownHostsFile"), "{command}");
            assert!(!command.contains(unusual), "{command}");
        }
    }

    #[test]
    fn the_route_debug_does_not_show_a_password() {
        let chain = hops(&["bastion.example"]);
        let chain = jump_hops(&chain, &[secret()]);
        let req = request(&chain, HopHostKeys::UserConfig);
        assert!(!format!("{req:?}").contains("hunter2"));
        let route = jump_route(&req).expect("one hop gives a route");
        assert!(!format!("{route:?}").contains("hunter2"));
    }

    /// `tunnel_jump_args` walks the hops behind a tunnel's jump host by their own
    /// `jump_host_id`, whatever group the target is in.
    #[test]
    fn a_tunnel_walks_the_hops_behind_its_jump_host() {
        let mut far = Connection::new_ssh("far".into(), "far.example".into(), 22);
        far.username = Some("ops".into());
        let mut near = Connection::new_ssh("near".into(), "near.example".into(), 2200);
        let mut jump = Connection::new_ssh("jump".into(), "jump.example".into(), 22);
        let near_id = near.id;
        for (conn, next) in [(&mut near, far.id), (&mut jump, near_id)] {
            if let crate::models::ProtocolConfig::Ssh(cfg) = &mut conn.protocol_config {
                cfg.jump_host_id = Some(next);
            }
        }
        let all = [far.clone(), near.clone(), jump.clone()];
        let network = NetworkSettings::default();

        // No password: plain `-J`, client-first.
        let args = tunnel_jump_args(&jump, &all, &[], &network, false, HopHostKeys::UserConfig);
        assert_eq!(args, ["-J", "ops@far.example,near.example:2200"]);

        // With the tunnel's password: a ProxyCommand that starts every hop
        // behind the env boundary, none with a helper of its own.
        let args = tunnel_jump_args(&jump, &all, &[], &network, true, HopHostKeys::UserConfig);
        assert_eq!(args[0], "-o");
        assert_eq!(
            args[1].matches("env _RC_TGT_PW_FILE=").count(),
            2,
            "{args:?}"
        );
        assert!(!args[1].contains("SSH_ASKPASS=/"), "{args:?}");

        // A jump host with nothing behind it, and a cycle, give nothing / stop.
        let lone = Connection::new_ssh("lone".into(), "lone.example".into(), 22);
        assert!(
            tunnel_jump_args(&lone, &all, &[], &network, true, HopHostKeys::UserConfig).is_empty()
        );
        let mut looped = far;
        if let crate::models::ProtocolConfig::Ssh(cfg) = &mut looped.protocol_config {
            cfg.jump_host_id = Some(jump.id);
        }
        let all = [looped, near, jump.clone()];
        let args = tunnel_jump_args(&jump, &all, &[], &network, false, HopHostKeys::UserConfig);
        assert_eq!(args.len(), 2, "{args:?}");
    }
}
