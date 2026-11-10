//! Run a local script: bash on SSH connections, PowerShell on WinRM ones.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use rustconn_core::config::{AppSettings, NetworkSettings};
use rustconn_core::connection::gateway_failover::{gateway_probes_by, probe_gateways, use_gateway};
use rustconn_core::connection::jump_chain::{JumpChain, resolve_jump_chain};
use rustconn_core::connection::ssh_inheritance::resolve_ssh_key_path;
use rustconn_core::models::{Connection, ConnectionGroup, PasswordSource, ProtocolConfig};
use rustconn_core::remote_script::{
    self, OutputStream, RunOptions, ScriptEvent, ScriptEventSink, ScriptHost, ScriptKind,
    ScriptOutcome, ScriptTransport, WinRmTarget,
};
use rustconn_core::secret::{CredentialResolver, SecretManager};
use rustconn_core::ssh_askpass::{self, AskpassSlot};
use rustconn_core::ssh_key_setup::{DeployCredentials, SshTarget};
use rustconn_core::ssh_tunnel::{
    HopHostKeys, SshTunnelParams, append_proxy_command_destination, askpass_disabled_proxy_prefix,
    askpass_proxy_prefix, build_nested_proxy_command_with_askpass, proxy_hop_host_key_option,
    proxy_jump_arg, shell_single_quote, ssh_config_declares_proxy, target_password_askpass_allowed,
};
use rustconn_core::variables::{Variable, VariableManager, VariableScope};
use secrecy::{ExposeSecret, SecretString};
use uuid::Uuid;

use super::group::find_group;
use crate::error::CliError;
use crate::util::{Confirmation, confirm_on_terminal, create_config_manager, find_connection};

/// What to run and where.
pub(super) struct RunScriptParams<'a> {
    pub script: &'a Path,
    pub connections: &'a [String],
    pub groups: &'a [String],
    pub parallel: usize,
    pub timeout_secs: u64,
    pub assume_yes: bool,
}

/// Runs a local script on the named connections and groups.
///
/// # Errors
///
/// Returns [`CliError::Config`] when the configuration or script cannot be read
/// or nothing is selected, [`CliError::ConnectionNotFound`] or [`CliError::Group`]
/// for an unknown target, and [`CliError::Connection`] when any host did not
/// succeed.
pub(super) fn cmd_run_script(
    config_path: Option<&Path>,
    params: &RunScriptParams<'_>,
) -> Result<(), CliError> {
    if params.connections.is_empty() && params.groups.is_empty() {
        return Err(CliError::Config(
            "Name at least one target with --on or --group".to_string(),
        ));
    }
    let script = remote_script::load_script(params.script)
        .map_err(|e| CliError::Config(format!("{}: {e}", params.script.display())))?;

    let config_manager = create_config_manager(config_path)?;
    let connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;
    let groups = config_manager
        .load_groups()
        .map_err(|e| CliError::Config(format!("Failed to load groups: {e}")))?;
    let settings = config_manager.load_settings().unwrap_or_default();

    let mut selected = Vec::new();
    for reference in params.connections {
        selected.push(find_connection(&connections, reference)?.id);
    }
    for reference in params.groups {
        selected.push(find_group(&groups, reference)?.id);
    }
    let kind = ScriptKind::of_path(params.script);
    let protocol = match kind {
        ScriptKind::Bash => "SSH",
        ScriptKind::PowerShell => "WinRM",
    };
    let targets = remote_script::script_targets(&selected, &connections, &groups, kind);
    if targets.hosts.is_empty() {
        return Err(CliError::Config(format!(
            "The selection holds no {protocol} connections"
        )));
    }
    if targets.skipped > 0 {
        eprintln!(
            "Leaving out {} non-{protocol} connection(s).",
            targets.skipped
        );
    }

    if !params.assume_yes {
        let names: Vec<&str> = targets.hosts.iter().map(|c| c.name.as_str()).collect();
        eprintln!("Hosts: {}", names.join(", "));
        let prompt = format!(
            "Run '{}' on {} host(s)?",
            params.script.display(),
            names.len()
        );
        match confirm_on_terminal(&prompt) {
            Confirmation::Confirmed => {}
            Confirmation::Declined => return Ok(()),
            Confirmation::NotInteractive => {
                return Err(CliError::Config(
                    "Refusing to run without confirmation; pass --yes".to_string(),
                ));
            }
        }
    }

    let timeout_secs = settings.connection.port_check_timeout_secs;
    let mut passwords = Passwords::new(&settings, &groups);
    let hosts: Vec<ScriptHost> = targets
        .hosts
        .iter()
        .filter_map(|c| {
            script_host(
                &with_global_variables(c, &settings.global_variables),
                &connections,
                &groups,
                &settings.network,
                timeout_secs,
                &mut passwords,
            )
        })
        .collect();
    let total = hosts.len();
    let names: Vec<String> = hosts.iter().map(|h| h.name.clone()).collect();

    let (sender, receiver) = std::sync::mpsc::channel();
    let sink: ScriptEventSink = Arc::new(move |event| {
        let _ = sender.send(event);
    });
    let options = RunOptions {
        parallel: params.parallel,
        timeout: Duration::from_secs(params.timeout_secs),
        ..RunOptions::default()
    };
    let script = Arc::new(script);
    let worker = std::thread::spawn(move || {
        remote_script::run_script(&script, &hosts, &options, &sink);
    });

    let succeeded = print_events(&receiver, &names);
    let _ = worker.join();

    eprintln!("Succeeded on {succeeded} of {total} host(s).");
    if succeeded == total {
        Ok(())
    } else {
        Err(CliError::Connection(format!(
            "The script did not succeed on {} of {total} host(s)",
            total - succeeded
        )))
    }
}

/// The connection with global variables substituted into its host and user
/// name, as the GUI runner does.
///
/// The semantics are the GUI's `substitute_for_command`: a reference nothing
/// defines becomes empty, and a value holding a shell metacharacter leaves the
/// text unchanged. A secret variable is not read from the vault here, so one
/// whose value is not in the configuration file counts as undefined.
fn with_global_variables(connection: &Connection, globals: &[Variable]) -> Connection {
    let mut connection = connection.clone();
    let has_reference = |text: &str| text.contains("${");
    if !has_reference(&connection.host)
        && !connection.username.as_deref().is_some_and(has_reference)
    {
        return connection;
    }
    let mut manager = VariableManager::new();
    for variable in globals {
        manager.set_global(variable.clone());
    }
    connection.host = substitute_global(&manager, &connection.host);
    connection.username = connection
        .username
        .as_deref()
        .map(|user| substitute_global(&manager, user));
    connection
}

fn substitute_global(manager: &VariableManager, text: &str) -> String {
    manager
        .substitute_for_command(text, VariableScope::Global)
        .unwrap_or_else(|_| text.to_string())
}

/// Prints progress until every host has finished; returns how many succeeded.
fn print_events(receiver: &Receiver<ScriptEvent>, names: &[String]) -> usize {
    let total = names.len();
    let mut succeeded = 0;
    let mut finished = 0;
    while finished < total {
        let Ok(event) = receiver.recv() else {
            break;
        };
        match event {
            ScriptEvent::Started { .. } => {}
            ScriptEvent::Output { host, stream, line } => match stream {
                OutputStream::Stdout => println!("[{}] {line}", names[host]),
                OutputStream::Stderr => eprintln!("[{}] {line}", names[host]),
            },
            ScriptEvent::OutputTruncated { host } => {
                eprintln!("[{}] further output dropped", names[host]);
            }
            ScriptEvent::Finished { host, outcome } => {
                finished += 1;
                if outcome == ScriptOutcome::Succeeded {
                    succeeded += 1;
                }
                eprintln!("[{}] {}", names[host], outcome_text(outcome));
            }
        }
    }
    succeeded
}

fn outcome_text(outcome: ScriptOutcome) -> String {
    match outcome {
        ScriptOutcome::Succeeded => "succeeded".to_string(),
        ScriptOutcome::ScriptFailed { code: Some(code) } => format!("failed with exit code {code}"),
        ScriptOutcome::ScriptFailed { code: None } => "stopped by a signal".to_string(),
        ScriptOutcome::ConnectionFailed => "connection failed".to_string(),
        ScriptOutcome::TimedOut => "timed out".to_string(),
        ScriptOutcome::Cancelled => "cancelled".to_string(),
        ScriptOutcome::NotStarted => "could not start".to_string(),
    }
}

/// Finds the passwords of a run: saved ones from the configured secret backend,
/// and typed ones for connections set to ask at connect. Each is looked up once.
struct Passwords<'a> {
    settings: &'a AppSettings,
    groups: &'a [ConnectionGroup],
    /// Built on first use: making it reaches for the backends and warns about
    /// missing ones, which a run on keys alone has no reason to hear about.
    backend: Option<(tokio::runtime::Runtime, CredentialResolver)>,
    known: HashMap<Uuid, Option<SecretString>>,
    last_typed: Option<SecretString>,
}

impl<'a> Passwords<'a> {
    fn new(settings: &'a AppSettings, groups: &'a [ConnectionGroup]) -> Self {
        Self {
            settings,
            groups,
            backend: None,
            known: HashMap::new(),
            last_typed: None,
        }
    }

    fn of(&mut self, connection: &Connection) -> Option<SecretString> {
        if let Some(known) = self.known.get(&connection.id) {
            return known.clone();
        }
        let password = match connection.password_source {
            PasswordSource::None => None,
            PasswordSource::Prompt => self.typed(&connection.name),
            _ => self.saved(connection),
        };
        self.known.insert(connection.id, password.clone());
        password
    }

    /// Asks on the terminal. With none to ask on, the connection uses its keys.
    fn typed(&mut self, name: &str) -> Option<SecretString> {
        use std::io::IsTerminal;

        if !std::io::stdin().is_terminal() {
            return None;
        }
        if self.last_typed.is_some() {
            eprint!("Password for {name} (empty reuses the previous one): ");
        } else {
            eprint!("Password for {name}: ");
        }
        if let Some(password) = rpassword::read_password()
            .ok()
            .filter(|password| !password.is_empty())
        {
            self.last_typed = Some(SecretString::from(password));
        }
        self.last_typed.clone()
    }

    /// A password that cannot be read is reported and left out, so that host
    /// tries its keys and fails on its own instead of stopping the whole run.
    fn saved(&mut self, connection: &Connection) -> Option<SecretString> {
        if self.backend.is_none() {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .inspect_err(|error| eprintln!("Cannot read saved passwords: {error}"))
                .ok()?;
            let manager = Arc::new(SecretManager::build_from_settings(&self.settings.secrets));
            let resolver = CredentialResolver::new(manager, self.settings.secrets.clone());
            self.backend = Some((runtime, resolver));
        }
        let (runtime, resolver) = self.backend.as_ref()?;
        let lookup = resolver.resolve_with_hierarchy(connection, self.groups);
        match within(runtime, CREDENTIAL_LOOKUP_TIMEOUT, lookup) {
            Some(Ok(credentials)) => credentials
                .and_then(|c| c.password)
                .filter(|p| !p.expose_secret().is_empty()),
            Some(Err(error)) => {
                eprintln!(
                    "[{}] could not read the saved password: {error}",
                    connection.name
                );
                None
            }
            None => {
                eprintln!(
                    "[{}] could not read the saved password: timed out",
                    connection.name
                );
                None
            }
        }
    }
}

/// How long one saved-password lookup may take, as in the GUI.
const CREDENTIAL_LOOKUP_TIMEOUT: Duration = Duration::from_secs(30);

/// Runs `lookup` on `runtime` for at most `limit`; `None` when it took longer.
fn within<T>(
    runtime: &tokio::runtime::Runtime,
    limit: Duration,
    lookup: impl Future<Output = T>,
) -> Option<T> {
    runtime.block_on(async { tokio::time::timeout(limit, lookup).await.ok() })
}

/// A `ProxyCommand` that reaches `hops[0]` through the rest of `hops`, each with
/// its own askpass helper or with askpass switched off.
///
/// Under `-J` each bastion's `ssh` inherits the askpass environment and would be
/// handed the target's password, so a run with any password goes through this
/// instead. Every hop gets `StrictHostKeyChecking=yes`. A run with no password
/// keeps `-J`, whose hops take no `-o` and follow the user's ssh configuration,
/// as in `rustconn_core::sftp`. `None` when a hop holds anything a shell could
/// interpret.
fn bastion_proxy_command(
    hops: &[&str],
    helpers: &[Option<&Path>],
    first_hop_key: Option<&str>,
) -> Option<String> {
    let is_safe = |c: char| c.is_ascii_alphanumeric() || ".-_:@[]".contains(c);
    if hops.is_empty()
        || hops
            .iter()
            .any(|hop| hop.is_empty() || hop.starts_with('-') || !hop.chars().all(is_safe))
    {
        return None;
    }
    let mut parts = match helpers.first().copied().flatten() {
        Some(helper) => askpass_proxy_prefix(helper, 0, hops.len()),
        None => askpass_disabled_proxy_prefix(hops.len()),
    };
    parts.extend(["ssh", "-W", "%h:%p", "-o", "ConnectTimeout=15"].map(String::from));
    // Nobody watches a run, so no hop may accept a key the user has not:
    // an unknown one fails before any password is sent, without the
    // host-key question a helper would answer with the password (#203).
    if let Some(option) = proxy_hop_host_key_option(HopHostKeys::KnownOnly, true) {
        parts.extend(["-o".to_string(), option.to_string()]);
    }
    if let Some(key) = first_hop_key.filter(|key| key.chars().all(|c| is_safe(c) || c == '/')) {
        parts.extend(["-i".to_string(), key.to_string()]);
    }
    if hops.len() > 1 {
        let inner = build_nested_proxy_command_with_askpass(
            &hops[1..],
            None,
            None,
            HopHostKeys::KnownOnly,
            &helpers[1..],
            1,
            hops.len(),
        );
        parts.push("-o".to_string());
        parts.push(format!("ProxyCommand={}", shell_single_quote(&inner)));
    }
    append_proxy_command_destination(&mut parts, hops[0]);
    Some(parts.join(" "))
}

/// Picks the reachable jump host for `connection`, on a copy of it.
fn route(
    connection: &Connection,
    connections: &[Connection],
    groups: &[ConnectionGroup],
    network: &NetworkSettings,
    probe_timeout_secs: u32,
) -> Connection {
    let probes = gateway_probes_by(
        connection,
        |id| connections.iter().find(|c| c.id == id),
        groups,
        network,
    );
    let mut routed = connection.clone();
    if let Some(selection) = probe_gateways(&probes, probe_timeout_secs) {
        use_gateway(&mut routed, selection.gateway_id);
    }
    routed
}

/// The SSH tunnel that reaches a WinRM host through its jump hosts, if it has any.
fn winrm_tunnel(
    routed: &Connection,
    connections: &[Connection],
    groups: &[ConnectionGroup],
    network: &NetworkSettings,
    passwords: &mut Passwords<'_>,
) -> Option<SshTunnelParams> {
    let chain = resolve_jump_chain(routed, connections, groups, network);
    let closest = chain.hops.first()?;
    let hop = chain
        .hop_ids
        .first()
        .copied()
        .flatten()
        .and_then(|id| connections.iter().find(|c| c.id == id));
    let (jump_host, jump_port) = hop.map_or_else(
        || match closest.rsplit_once(':') {
            Some((host, port)) if port.parse::<u16>().is_ok() => {
                (host.to_string(), port.parse().unwrap_or(22))
            }
            _ => (closest.clone(), 22),
        },
        |hop| {
            let host = hop
                .username
                .as_ref()
                .map_or_else(|| hop.host.clone(), |user| format!("{user}@{}", hop.host));
            (host, hop.port)
        },
    );
    let extra_args = if chain.hops.len() > 1 {
        vec!["-J".to_string(), proxy_jump_arg(&chain.hops[1..].join(","))]
    } else {
        Vec::new()
    };
    Some(SshTunnelParams {
        jump_host,
        jump_port,
        remote_host: routed.host.clone(),
        remote_port: routed.port,
        identity_file: hop
            .and_then(|hop| resolve_ssh_key_path(hop, groups))
            .and_then(|path| rustconn_core::resolve_key_path(&path))
            .map(|path| path.to_string_lossy().into_owned()),
        password: hop.and_then(|hop| passwords.of(hop)),
        extra_args,
        launcher: rustconn_core::ssh_tunnel::TunnelLauncher::OpenSsh,
    })
}

/// Opens `evil-winrm` through an SSH tunnel and waits for it. Returns `Ok(false)`
/// when `connection` has no jump host, so the caller launches it directly.
///
/// # Errors
///
/// Returns [`CliError::Connection`] when the tunnel or the client cannot start.
#[cfg(feature = "client-launch")]
pub(super) fn connect_winrm_through_tunnel(
    connection: &Connection,
    connections: &[Connection],
    groups: &[ConnectionGroup],
    settings: &AppSettings,
) -> Result<bool, CliError> {
    use rustconn_core::protocol::{Protocol, WINRM_CLIENT, WinRmProtocol};
    use rustconn_core::ssh_tunnel;

    let routed = route(
        connection,
        connections,
        groups,
        &settings.network,
        settings.connection.port_check_timeout_secs,
    );
    let mut passwords = Passwords::new(settings, groups);
    let Some(params) = winrm_tunnel(
        &routed,
        connections,
        groups,
        &settings.network,
        &mut passwords,
    ) else {
        return Ok(false);
    };
    if !rustconn_core::which::is_available(WINRM_CLIENT) {
        return Err(CliError::Config(format!(
            "Required program '{WINRM_CLIENT}' not found. Install it with: gem install evil-winrm"
        )));
    }
    let tunnel_error = |e: ssh_tunnel::SshTunnelError| CliError::Connection(e.to_string());
    let mut tunnel = ssh_tunnel::create_tunnel(&params).map_err(tunnel_error)?;
    ssh_tunnel::wait_for_tunnel_ready(&mut tunnel, 50, Duration::from_millis(200))
        .map_err(tunnel_error)?;

    let mut local = routed;
    local.host = "127.0.0.1".to_string();
    local.port = tunnel.local_port();
    let command = WinRmProtocol::new()
        .build_command(&local)
        .ok_or_else(|| CliError::Connection("Could not build the WinRM command".to_string()))?;
    let status = std::process::Command::new(&command[0])
        .args(&command[1..])
        .status()
        .map_err(|e| CliError::Connection(format!("Failed to run {WINRM_CLIENT}: {e}")))?;
    if status.success() {
        Ok(true)
    } else {
        Err(CliError::Connection(format!(
            "{WINRM_CLIENT} exited with {status}"
        )))
    }
}

/// Builds the launch of one host, by its protocol.
fn script_host(
    connection: &Connection,
    connections: &[Connection],
    groups: &[ConnectionGroup],
    network: &NetworkSettings,
    probe_timeout_secs: u32,
    passwords: &mut Passwords<'_>,
) -> Option<ScriptHost> {
    let routed = route(connection, connections, groups, network, probe_timeout_secs);
    if matches!(routed.protocol_config, ProtocolConfig::WinRm(_)) {
        let tunnel = winrm_tunnel(&routed, connections, groups, network, passwords);
        let password = passwords.of(connection);
        return Some(ScriptHost {
            name: routed.name.clone(),
            transport: ScriptTransport::WinRm(WinRmTarget::for_connection(
                &routed, password, tunnel,
            )?),
        });
    }
    ssh_script_host(&routed, connections, groups, network, passwords)
}

/// Askpass helpers and passwords for the hops of `chain`; slots stay aligned.
fn hop_askpass(
    chain: &JumpChain,
    connections: &[Connection],
    passwords: &mut Passwords<'_>,
) -> (
    Vec<Option<std::path::PathBuf>>,
    Vec<(AskpassSlot, SecretString)>,
) {
    let mut helpers = Vec::with_capacity(chain.hops.len());
    let mut secrets = Vec::new();
    for (index, id) in chain.hop_ids.iter().enumerate() {
        let password = id
            .and_then(|id| connections.iter().find(|c| c.id == id))
            .and_then(|hop| passwords.of(hop));
        let slot = AskpassSlot::JumpHost(index);
        // No helper, no exposure: the hop then has to use a key or the agent.
        let helper = password
            .is_some()
            .then(|| ssh_askpass::helper_script(slot).ok())
            .flatten();
        if let (Some(password), Some(_)) = (password, &helper) {
            secrets.push((slot, password));
        }
        helpers.push(helper);
    }
    (helpers, secrets)
}

/// Drops the target's password when OpenSSH's own config routes the host
/// through a proxy, as the GUI runner does.
///
/// With no resolved jump chain nothing on the command line overrides a
/// `ProxyJump`/`ProxyCommand` from `~/.ssh/config`. The bastion's `ssh` then
/// inherits `SSH_ASKPASS`, and the helper answers its `'s password:` prompt
/// with the target's password (issue #191). A resolved chain is safe: its
/// `-J`/`ProxyCommand` outranks the config. `declares_proxy` is
/// [`ssh_config_declares_proxy`] outside tests, and runs only when there is a
/// password to protect.
fn withhold_from_config_proxy(
    routed: &Connection,
    password: Option<SecretString>,
    chain_is_empty: bool,
    declares_proxy: impl FnOnce(&str, u16, Option<&str>) -> bool,
) -> Option<SecretString> {
    let password = password?;
    if chain_is_empty && declares_proxy(&routed.host, routed.port, routed.username.as_deref()) {
        eprintln!(
            "[{}] not sending the password: ~/.ssh/config routes this host through a proxy",
            routed.name
        );
        return None;
    }
    Some(password)
}

/// Builds the launch of an SSH host: its options, key, jump hosts and passwords.
fn ssh_script_host(
    routed: &Connection,
    connections: &[Connection],
    groups: &[ConnectionGroup],
    network: &NetworkSettings,
    passwords: &mut Passwords<'_>,
) -> Option<ScriptHost> {
    let ProtocolConfig::Ssh(config) = &routed.protocol_config else {
        return None;
    };

    let mut password = passwords
        .of(routed)
        .filter(|_| target_password_askpass_allowed(config, false));
    let mut session_args = config.build_command_args();
    if !session_args.iter().any(|arg| arg == "-i")
        && let Some(key) = resolve_ssh_key_path(routed, groups)
            .and_then(|path| rustconn_core::resolve_key_path(&path))
    {
        session_args.push("-i".to_string());
        session_args.push(key.to_string_lossy().into_owned());
    }
    // The resolved chain replaces any `-J` the config emitted on its own: only
    // the resolver knows about saved jump hosts and inherited ones.
    if let Some(position) = session_args.iter().position(|arg| arg == "-J") {
        session_args.drain(position..(position + 2).min(session_args.len()));
    }

    let chain = resolve_jump_chain(routed, connections, groups, network);
    password = withhold_from_config_proxy(
        routed,
        password,
        chain.hops.is_empty(),
        ssh_config_declares_proxy,
    );
    let (helpers, mut jump_host_passwords) = hop_askpass(&chain, connections, passwords);
    if !chain.hops.is_empty() {
        let plain_jump = chain.proxy_jump_value();
        if password.is_some() || !jump_host_passwords.is_empty() {
            let hops: Vec<&str> = chain.hops.iter().map(String::as_str).collect();
            let helper_refs: Vec<Option<&Path>> = helpers.iter().map(|h| h.as_deref()).collect();
            let first_hop_key = chain
                .hop_ids
                .first()
                .copied()
                .flatten()
                .and_then(|id| connections.iter().find(|c| c.id == id))
                .and_then(|hop| resolve_ssh_key_path(hop, groups))
                .and_then(|path| rustconn_core::resolve_key_path(&path))
                .map(|path| path.to_string_lossy().into_owned());
            if let Some(command) =
                bastion_proxy_command(&hops, &helper_refs, first_hop_key.as_deref())
            {
                session_args.push("-o".to_string());
                session_args.push(format!("ProxyCommand={command}"));
            } else {
                eprintln!(
                    "[{}] not sending passwords: unexpected jump host address",
                    routed.name
                );
                password = None;
                jump_host_passwords.clear();
                session_args.push("-J".to_string());
                session_args.extend(plain_jump);
            }
        } else {
            session_args.push("-J".to_string());
            session_args.extend(plain_jump);
        }
    }

    Some(ScriptHost {
        name: routed.name.clone(),
        transport: ScriptTransport::Ssh {
            target: SshTarget::new(
                routed.host.clone(),
                routed.port,
                routed.username.clone(),
                session_args,
            ),
            credentials: DeployCredentials {
                password,
                jump_host_passwords,
            },
        },
    })
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::path::Path;

    use rustconn_core::models::Connection;
    use secrecy::{ExposeSecret, SecretString};

    use super::{bastion_proxy_command, withhold_from_config_proxy, within};

    fn target() -> Connection {
        let mut connection = Connection::new_ssh("web".to_string(), "web.lan".to_string(), 2222);
        connection.username = Some("alice".to_string());
        connection
    }

    fn secret() -> Option<SecretString> {
        Some(SecretString::from("hunter2".to_string()))
    }

    #[test]
    fn a_config_proxy_with_no_chain_withholds_the_password() {
        let asked = Cell::new(None);
        let kept = withhold_from_config_proxy(&target(), secret(), true, |host, port, user| {
            asked.set(Some((host.to_string(), port, user.map(str::to_string))));
            true
        });
        assert!(kept.is_none());
        assert_eq!(
            asked.take(),
            Some(("web.lan".to_string(), 2222, Some("alice".to_string())))
        );
    }

    #[test]
    fn no_config_proxy_keeps_the_password() {
        let kept = withhold_from_config_proxy(&target(), secret(), true, |_, _, _| false);
        assert_eq!(
            kept.map(|p| p.expose_secret().to_string()),
            Some("hunter2".to_string())
        );
    }

    #[test]
    fn a_resolved_chain_or_no_password_skips_the_config_check() {
        let asked = Cell::new(false);
        let probe = |_: &str, _: u16, _: Option<&str>| {
            asked.set(true);
            true
        };
        assert!(withhold_from_config_proxy(&target(), secret(), false, probe).is_some());
        assert!(withhold_from_config_proxy(&target(), None, true, probe).is_none());
        assert!(!asked.get(), "ssh -G must only run to protect a password");
    }

    #[test]
    fn global_variables_are_substituted_in_host_and_user() {
        use rustconn_core::variables::Variable;

        let mut connection = target();
        connection.host = "${site}.example.com".to_string();
        connection.username = Some("${login}".to_string());
        let globals = [
            Variable::new("site", "web01"),
            Variable::new("login", "bob"),
        ];
        let resolved = super::with_global_variables(&connection, &globals);
        assert_eq!(resolved.host, "web01.example.com");
        assert_eq!(resolved.username.as_deref(), Some("bob"));
        assert_eq!(resolved.id, connection.id, "passwords are found by id");
    }

    #[test]
    fn plain_hosts_and_unsafe_values_are_left_alone() {
        use rustconn_core::variables::Variable;

        let plain = super::with_global_variables(&target(), &[Variable::new("site", "x")]);
        assert_eq!(plain.host, "web.lan");
        assert_eq!(plain.username.as_deref(), Some("alice"));

        let mut connection = target();
        connection.host = "${site}".to_string();
        let resolved =
            super::with_global_variables(&connection, &[Variable::new("site", "a;reboot")]);
        assert_eq!(resolved.host, "${site}");
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn a_lookup_that_never_answers_is_given_up() {
        let started = std::time::Instant::now();
        let result = within(
            &runtime(),
            std::time::Duration::from_millis(50),
            std::future::pending::<()>(),
        );
        assert!(result.is_none());
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }

    #[test]
    fn a_lookup_that_answers_in_time_is_kept() {
        let result = within(
            &runtime(),
            std::time::Duration::from_secs(5),
            std::future::ready(7),
        );
        assert_eq!(result, Some(7));
    }

    #[test]
    fn a_single_hop_goes_straight_to_the_bastion() {
        let command = bastion_proxy_command(&["alice@jump:2222"], &[None], None).unwrap();
        assert!(command.starts_with("env "), "{command}");
        assert!(command.contains("SSH_ASKPASS= "), "{command}");
        assert!(command.contains("-W %h:%p"), "{command}");
        assert!(command.contains("alice@jump"), "{command}");
        assert!(command.contains("2222"), "{command}");
    }

    #[test]
    fn a_hop_with_a_password_gets_its_helper() {
        let helper = Path::new("/run/helper.sh");
        let command = bastion_proxy_command(&["jump"], &[Some(helper)], None).unwrap();
        assert!(command.contains("SSH_ASKPASS=/run/helper.sh"), "{command}");
        assert!(command.contains("StrictHostKeyChecking=yes"), "{command}");
        assert!(!command.contains("accept-new"), "{command}");
    }

    #[test]
    fn deeper_hops_are_nested() {
        let command = bastion_proxy_command(&["near", "far"], &[None, None], None).unwrap();
        assert!(command.contains("ProxyCommand="), "{command}");
        assert!(command.contains("far"), "{command}");
        assert!(command.trim_end().ends_with("near"), "{command}");
    }

    #[test]
    fn every_hop_needs_a_known_host_key() {
        let helper = Path::new("/run/helper-1.sh");
        let command = bastion_proxy_command(&["near", "far"], &[None, Some(helper)], None).unwrap();
        assert_eq!(
            command.matches("StrictHostKeyChecking=yes").count(),
            2,
            "{command}"
        );
        assert!(!command.contains("accept-new"), "{command}");
    }

    #[test]
    fn anything_a_shell_could_interpret_is_refused() {
        for hop in ["jump;reboot", "$(reboot)", "-oProxyCommand=x", ""] {
            assert!(
                bastion_proxy_command(&[hop], &[None], None).is_none(),
                "{hop}"
            );
        }
    }
}
