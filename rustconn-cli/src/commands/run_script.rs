//! Run a local script: bash on SSH connections, PowerShell on WinRM ones.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use rustconn_core::config::{AppSettings, NetworkSettings, SecretSettings};
use rustconn_core::connection::gateway_failover::{gateway_probes_by, probe_gateways, use_gateway};
use rustconn_core::connection::jump_chain::{JumpChain, resolve_jump_chain};
use rustconn_core::connection::ssh_inheritance::resolve_ssh_key_path;
use rustconn_core::jump_route::{
    JumpHop, JumpRoute, JumpRouteRequest, TunnelHopsRequest, hops_are_plain, jump_route,
    plain_path, tunnel_hop_args,
};
use rustconn_core::models::{Connection, ConnectionGroup, PasswordSource, ProtocolConfig};
use rustconn_core::remote_script::{
    self, LaunchError, OutputStream, RemoteHost, RunOptions, ScriptEvent, ScriptEventSink,
    ScriptKind, ScriptOutcome, ScriptTransport, WinRmTarget,
};
use rustconn_core::secret::{CredentialResolver, SecretManager};
use rustconn_core::ssh_key_setup::{DeployCredentials, SshTarget};
use rustconn_core::ssh_tunnel::{
    HopHostKeys, SshTunnelParams, proxy_jump_arg, ssh_config_declares_proxy,
    target_password_askpass_allowed,
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
    /// `--quiet`: leave out the progress lines (hosts left out, hosts that
    /// succeeded, the summary). The script's output, hosts that did not
    /// succeed, warnings and prompts still appear.
    pub quiet: bool,
    /// `--same-password`: one typed answer for every host set to ask.
    pub same_password: bool,
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
    if targets.skipped > 0 && !params.quiet {
        eprintln!(
            "Leaving out {} non-{protocol} connection(s).",
            targets.skipped
        );
    }

    // Resolved before the prompt, so it lists only the hosts that will run.
    let (runnable, left_out) = leave_out_ask_hosts(&targets.hosts, &settings.global_variables);

    if !params.assume_yes && !runnable.is_empty() {
        let names: Vec<&str> = runnable.iter().map(|c| c.name.as_str()).collect();
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
    let mut passwords = Passwords::new(&settings, &groups, params.same_password);
    let hosts: Vec<RemoteHost> = runnable
        .iter()
        .filter_map(|c| {
            script_host(
                c,
                &connections,
                &groups,
                &settings.network,
                timeout_secs,
                &mut passwords,
            )
        })
        .collect();
    // A host left out did not succeed, so it counts in the total and the
    // exit status.
    let total = hosts.len() + left_out;
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

    let succeeded = print_events(&receiver, &names, params.quiet);
    let _ = worker.join();
    run_result(succeeded, total, params.quiet)
}

/// Prints the closing count (not under `--quiet`) and turns it into the
/// command's result.
fn run_result(succeeded: usize, total: usize, quiet: bool) -> Result<(), CliError> {
    if !quiet {
        eprintln!("Succeeded on {succeeded} of {total} host(s).");
    }
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
///
/// A variable set to ask at connect (`@ask:`) cannot be answered here, so it
/// gives `Err` with that variable's name and the host is left out.
fn with_global_variables(
    connection: &Connection,
    globals: &[Variable],
) -> Result<Connection, String> {
    let mut connection = connection.clone();
    let has_reference = |text: &str| text.contains("${");
    if !has_reference(&connection.host)
        && !connection.username.as_deref().is_some_and(has_reference)
    {
        return Ok(connection);
    }
    let mut manager = VariableManager::new();
    for variable in globals {
        manager.set_global(variable.clone());
    }
    let texts = std::iter::once(connection.host.as_str()).chain(connection.username.as_deref());
    for text in texts {
        // A malformed reference is left to substitution, as before.
        if let Ok(requests) = manager.collect_ask_requests(text, VariableScope::Global)
            && let Some((name, _)) = requests.into_iter().next()
        {
            return Err(name);
        }
    }
    connection.host = substitute_global(&manager, &connection.host);
    connection.username = connection
        .username
        .as_deref()
        .map(|user| substitute_global(&manager, user));
    Ok(connection)
}

/// [`with_global_variables`], or `None` with a warning when the host has to be
/// left out. The warning is printed under `--quiet` too.
fn resolve_or_leave_out(connection: &Connection, globals: &[Variable]) -> Option<Connection> {
    with_global_variables(connection, globals)
        .map_err(|variable| {
            eprintln!(
                "[{}] left out: ${{{variable}}} asks for its value at connect, \
                 which run-script cannot do",
                connection.name
            );
        })
        .ok()
}

/// The hosts with global variables filled in, and how many were left out by
/// [`resolve_or_leave_out`].
fn leave_out_ask_hosts(hosts: &[&Connection], globals: &[Variable]) -> (Vec<Connection>, usize) {
    let runnable: Vec<Connection> = hosts
        .iter()
        .filter_map(|c| resolve_or_leave_out(c, globals))
        .collect();
    let left_out = hosts.len() - runnable.len();
    (runnable, left_out)
}

fn substitute_global(manager: &VariableManager, text: &str) -> String {
    manager
        .substitute_for_command(text, VariableScope::Global)
        .unwrap_or_else(|_| text.to_string())
}

/// Prints progress until every host has finished; returns how many succeeded.
/// With `quiet` a host that succeeded is not announced.
fn print_events(receiver: &Receiver<ScriptEvent>, names: &[String], quiet: bool) -> usize {
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
            ScriptEvent::NotStarted { host, reason } => {
                eprintln!("[{}] {reason}", names[host]);
                if reason == LaunchError::PyWinRmNotInstalled {
                    eprintln!("[{}] {}", names[host], pywinrm_hint());
                }
            }
            ScriptEvent::Finished { host, outcome } => {
                finished += 1;
                if outcome == ScriptOutcome::Succeeded {
                    succeeded += 1;
                }
                if announces_outcome(outcome, quiet) {
                    eprintln!("[{}] {}", names[host], outcome_text(outcome));
                }
            }
        }
    }
    succeeded
}

/// Where to get `pywinrm` for this package.
fn pywinrm_hint() -> &'static str {
    if rustconn_core::is_sandboxed() {
        "install pywinrm from Components in the RustConn window"
    } else {
        "install python3-winrm, or run: pip install pywinrm"
    }
}

/// Whether a host's outcome is printed: always, except a success under
/// `--quiet`, which is progress rather than an error.
fn announces_outcome(outcome: ScriptOutcome, quiet: bool) -> bool {
    !quiet || outcome != ScriptOutcome::Succeeded
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
    runtime: Option<tokio::runtime::Runtime>,
    /// One resolver per KeePass database (issue #411): `None` is the global
    /// one from Settings → Secrets, `Some` a connection's or group's own file.
    resolvers: HashMap<Option<PathBuf>, CredentialResolver>,
    known: HashMap<Uuid, Option<SecretString>>,
    /// `--same-password`: the first answer is used for every host set to ask.
    same_password: bool,
    /// Whether that answer has been given; `shared` is `None` when it was empty.
    shared_asked: bool,
    shared: Option<SecretString>,
}

impl<'a> Passwords<'a> {
    fn new(settings: &'a AppSettings, groups: &'a [ConnectionGroup], same_password: bool) -> Self {
        Self {
            settings,
            groups,
            runtime: None,
            resolvers: HashMap::new(),
            known: HashMap::new(),
            same_password,
            shared_asked: false,
            shared: None,
        }
    }

    /// The password of a jump host.
    ///
    /// An answer belongs to the connection, so a host that is both a target and
    /// another target's jump host uses its first answer in both roles, which may
    /// be the `--same-password` one. A jump host that is not itself a target is
    /// asked on its own.
    fn of(&mut self, connection: &Connection) -> Option<SecretString> {
        self.lookup(connection, false)
    }

    /// The password of a host the script runs on.
    fn of_target(&mut self, connection: &Connection) -> Option<SecretString> {
        self.lookup(connection, true)
    }

    fn lookup(&mut self, connection: &Connection, target: bool) -> Option<SecretString> {
        if let Some(known) = self.known.get(&connection.id) {
            return known.clone();
        }
        let password = match connection.password_source {
            PasswordSource::None => None,
            PasswordSource::Prompt => self.typed(&connection.name, target),
            _ => self.saved(connection),
        };
        self.known.insert(connection.id, password.clone());
        password
    }

    /// Asks on the terminal. With none to ask on, the connection uses its keys.
    fn typed(&mut self, name: &str, target: bool) -> Option<SecretString> {
        use std::io::IsTerminal;

        if !std::io::stdin().is_terminal() {
            return None;
        }
        self.typed_with(name, target, |prompt| {
            eprint!("{prompt}");
            rpassword::read_password().ok()
        })
    }

    /// Asks through `ask`, which shows the prompt and returns the answer. An
    /// empty answer gives no password: the host then uses its keys. Only with
    /// `--same-password` is an answer reused, and then for every `target`;
    /// a jump host is always asked on its own.
    fn typed_with(
        &mut self,
        name: &str,
        target: bool,
        ask: impl FnOnce(&str) -> Option<String>,
    ) -> Option<SecretString> {
        let shares = self.same_password && target;
        if shares && self.shared_asked {
            return self.shared.clone();
        }
        let prompt = if shares {
            "Password for every host set to ask at connect: ".to_string()
        } else {
            format!("Password for {name}: ")
        };
        let password = ask(&prompt)
            .filter(|password| !password.is_empty())
            .map(SecretString::from);
        if shares {
            self.shared_asked = true;
            self.shared.clone_from(&password);
        }
        password
    }

    /// A password that cannot be read is reported and left out, so that host
    /// tries its keys and fails on its own instead of stopping the whole run.
    fn saved(&mut self, connection: &Connection) -> Option<SecretString> {
        if self.runtime.is_none() {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .inspect_err(|error| eprintln!("Cannot read saved passwords: {error}"))
                .ok()?;
            self.runtime = Some(runtime);
        }
        let database = database_override(connection, self.groups);
        let settings = self.settings;
        let resolver = self
            .resolvers
            .entry(database)
            .or_insert_with_key(|database| {
                let secrets = secrets_for_database(&settings.secrets, database.as_deref());
                let manager = Arc::new(SecretManager::build_from_settings(&secrets));
                CredentialResolver::new(manager, secrets)
            });
        let runtime = self.runtime.as_ref()?;
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

/// The KeePass database a connection's saved password is read from, when it is
/// not the global one (issue #411).
///
/// A Vault password lives in the connection's own `.kdbx`, else its nearest
/// group's, the order the GUI reads and saves in. An inherited password is the
/// group's own credential, so it lives in the group's database even when the
/// connection names a file of its own. `None` means the global database.
fn database_override(connection: &Connection, groups: &[ConnectionGroup]) -> Option<PathBuf> {
    match connection.password_source {
        PasswordSource::Inherit => {
            rustconn_core::secret::resolve_group_kdbx_path_override(connection.group_id, groups)
        }
        _ => rustconn_core::secret::resolve_kdbx_path_override(connection, groups),
    }
}

/// The secret settings with `database` as the KeePass file, as the GUI opens
/// a per-connection database: same password, key file and backend, other file.
fn secrets_for_database(secrets: &SecretSettings, database: Option<&Path>) -> SecretSettings {
    let mut secrets = secrets.clone();
    if let Some(database) = database {
        secrets.kdbx_path = Some(database.to_path_buf());
    }
    secrets
}

/// Runs `lookup` on `runtime` for at most `limit`; `None` when it took longer.
fn within<T>(
    runtime: &tokio::runtime::Runtime,
    limit: Duration,
    lookup: impl Future<Output = T>,
) -> Option<T> {
    runtime.block_on(async { tokio::time::timeout(limit, lookup).await.ok() })
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
///
/// `hop_host_keys` applies to the jump hosts past the nearest one when they go
/// through a `ProxyCommand`, which they do when the tunnel sends a password.
fn winrm_tunnel(
    routed: &Connection,
    connections: &[Connection],
    groups: &[ConnectionGroup],
    network: &NetworkSettings,
    passwords: &mut Passwords<'_>,
    hop_host_keys: HopHostKeys,
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
    let mut password = hop.and_then(|hop| passwords.of(hop));
    let farther = &chain.hops[1..];
    let mut extra_args = Vec::new();
    if !farther.is_empty() {
        // Through `-J` the farther hops would inherit the askpass helper that
        // answers with the nearest hop's password, so with a password they
        // get a ProxyCommand that keeps it away from them.
        let known_hosts = rustconn_core::get_flatpak_known_hosts_path();
        let routed_past = password.as_ref().and_then(|_| {
            let next = chain
                .hop_ids
                .get(1)
                .copied()
                .flatten()
                .and_then(|id| connections.iter().find(|c| c.id == id));
            let identity = next
                .and_then(|hop| resolve_ssh_key_path(hop, groups))
                .and_then(|path| rustconn_core::resolve_key_path(&path))
                .map(|path| path.to_string_lossy().into_owned());
            let pkcs11 = next.and_then(|hop| match &hop.protocol_config {
                ProtocolConfig::Ssh(c) => c.pkcs11_provider.clone(),
                _ => None,
            });
            tunnel_hop_args(&TunnelHopsRequest {
                hops: farther,
                first_hop_identity: identity.as_deref(),
                deeper_hop_identity: None,
                first_hop_pkcs11: pkcs11.as_deref(),
                known_hosts: known_hosts.as_deref(),
                hop_host_keys,
            })
        });
        if let Some(args) = routed_past {
            extra_args = args;
        } else {
            if password.take().is_some() {
                eprintln!(
                    "[{}] not sending the jump host password: unexpected jump host address",
                    routed.name
                );
            }
            extra_args = vec!["-J".to_string(), proxy_jump_arg(&farther.join(","))];
        }
    }
    Some(SshTunnelParams {
        jump_host,
        jump_port,
        remote_host: routed.host.clone(),
        remote_port: routed.port,
        identity_file: hop
            .and_then(|hop| resolve_ssh_key_path(hop, groups))
            .and_then(|path| rustconn_core::resolve_key_path(&path))
            .map(|path| path.to_string_lossy().into_owned()),
        password,
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
    client: &rustconn_core::protocol::WinRmClient,
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
    let mut passwords = Passwords::new(settings, groups, false);
    // Someone is at the terminal: the user's ssh configuration decides about
    // a farther jump host's unknown key, as it does under `-J`.
    let Some(params) = winrm_tunnel(
        &routed,
        connections,
        groups,
        &settings.network,
        &mut passwords,
        HopHostKeys::UserConfig,
    ) else {
        return Ok(false);
    };
    let tunnel_error = |e: ssh_tunnel::SshTunnelError| CliError::Connection(e.to_string());
    let mut tunnel = ssh_tunnel::create_tunnel(&params).map_err(tunnel_error)?;
    ssh_tunnel::wait_for_tunnel_ready(&mut tunnel, 50, Duration::from_millis(200))
        .map_err(tunnel_error)?;

    let mut local = routed;
    local.host = "127.0.0.1".to_string();
    local.port = tunnel.local_port();
    let command = WinRmProtocol::new()
        .build_command(&local)
        .map(|built| client.command(built))
        .ok_or_else(|| CliError::Connection("Could not build the WinRM command".to_string()))?;
    // A host client reaches the tunnel's 127.0.0.1 port because the Flatpak
    // shares the host's network namespace (`--share=network`).
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
) -> Option<RemoteHost> {
    let routed = route(connection, connections, groups, network, probe_timeout_secs);
    if matches!(routed.protocol_config, ProtocolConfig::WinRm(_)) {
        let tunnel = winrm_tunnel(
            &routed,
            connections,
            groups,
            network,
            passwords,
            HopHostKeys::KnownOnly,
        );
        let password = passwords.of_target(connection);
        return Some(RemoteHost {
            name: routed.name.clone(),
            transport: ScriptTransport::WinRm(WinRmTarget::for_connection(
                &routed, password, tunnel,
            )?),
        });
    }
    ssh_script_host(
        &routed,
        connections,
        groups,
        network,
        passwords,
        ssh_config_declares_proxy,
    )
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

/// The route through `chain`, whose hops are plain, built the way the GUI
/// builds it. Nobody watches a run, so the route is a `ProxyCommand` whose hops
/// refuse a host key the user has not accepted; only a chain of several hops
/// that log in with a key alone goes through `-J`. A key path or PKCS#11
/// provider a shell could interpret is left out of the `ProxyCommand`.
fn plain_jump_route(
    chain: &JumpChain,
    hop_passwords: &[Option<SecretString>],
    connections: &[Connection],
    groups: &[ConnectionGroup],
    session_args: &[String],
    target_askpass_allowed: bool,
) -> Option<JumpRoute> {
    let first_hop = chain
        .hop_ids
        .first()
        .copied()
        .flatten()
        .and_then(|id| connections.iter().find(|c| c.id == id));
    let first_hop_identity = first_hop
        .and_then(|hop| resolve_ssh_key_path(hop, groups))
        .and_then(|path| rustconn_core::resolve_key_path(&path))
        .map(|path| path.to_string_lossy().into_owned());
    let first_hop_pkcs11 = first_hop.and_then(|hop| match &hop.protocol_config {
        ProtocolConfig::Ssh(c) => c.pkcs11_provider.clone(),
        _ => None,
    });
    let target_identity = session_args
        .iter()
        .position(|arg| arg == "-i")
        .and_then(|position| session_args.get(position + 1));
    // `chain.hops` has the length; a short `hop_passwords` leaves the rest without one.
    let hops: Vec<JumpHop> = chain
        .hops
        .iter()
        .zip(hop_passwords.iter().cloned().chain(std::iter::repeat(None)))
        .map(|(spec, password)| JumpHop {
            spec: spec.clone(),
            password,
        })
        .collect();
    jump_route(&JumpRouteRequest {
        hops: &hops,
        first_hop_identity: first_hop_identity.as_deref().and_then(plain_path),
        target_identity: target_identity.map(String::as_str).and_then(plain_path),
        deeper_hop_identity: None,
        first_hop_pkcs11: first_hop_pkcs11
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .and_then(plain_path),
        known_hosts: None,
        hop_host_keys: HopHostKeys::KnownOnly,
        target_askpass_allowed,
    })
}

/// Builds the launch of an SSH host: its options, key, jump hosts and passwords.
/// `declares_proxy` is passed to [`withhold_from_config_proxy`].
fn ssh_script_host(
    routed: &Connection,
    connections: &[Connection],
    groups: &[ConnectionGroup],
    network: &NetworkSettings,
    passwords: &mut Passwords<'_>,
    declares_proxy: impl FnOnce(&str, u16, Option<&str>) -> bool,
) -> Option<RemoteHost> {
    let ProtocolConfig::Ssh(config) = &routed.protocol_config else {
        return None;
    };

    let mut password = passwords
        .of_target(routed)
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
    password = withhold_from_config_proxy(routed, password, chain.hops.is_empty(), declares_proxy);
    let mut jump_host_passwords = Vec::new();
    if !chain.hops.is_empty() {
        let find = |id: Option<Uuid>| id.and_then(|id| connections.iter().find(|c| c.id == id));
        let hop_passwords: Vec<Option<SecretString>> = chain
            .hop_ids
            .iter()
            .map(|id| find(*id).and_then(|hop| passwords.of(hop)))
            .collect();
        if hops_are_plain(&chain.hops) {
            let route = plain_jump_route(
                &chain,
                &hop_passwords,
                connections,
                groups,
                &session_args,
                password.is_some(),
            );
            match route {
                Some(JumpRoute::ProxyCommand {
                    command,
                    passwords: hop_secrets,
                }) => {
                    session_args.push("-o".to_string());
                    session_args.push(format!("ProxyCommand={command}"));
                    jump_host_passwords = hop_secrets;
                }
                // The CLI loads no key into the agent, so `identity_for_agent`
                // is left to the user's own agent.
                Some(JumpRoute::ProxyJump { value, .. }) => {
                    session_args.push("-J".to_string());
                    session_args.push(value);
                }
                None => {}
            }
        } else {
            if password.is_some() || hop_passwords.iter().any(Option::is_some) {
                eprintln!(
                    "[{}] not sending passwords: unexpected jump host address",
                    routed.name
                );
                password = None;
            }
            session_args.push("-J".to_string());
            session_args.extend(chain.proxy_jump_value());
        }
    }

    Some(RemoteHost {
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

    use rustconn_core::connection::jump_chain::JumpChain;
    use rustconn_core::jump_route::JumpRoute;
    use rustconn_core::models::{Connection, ProtocolConfig, SshKeySource};
    use secrecy::{ExposeSecret, SecretString};

    use super::{plain_jump_route, withhold_from_config_proxy, within};

    fn target() -> Connection {
        let mut connection = Connection::new_ssh("web".to_string(), "web.lan".to_string(), 2222);
        connection.username = Some("alice".to_string());
        connection
    }

    fn secret() -> Option<SecretString> {
        Some(SecretString::from("hunter2".to_string()))
    }

    mod database {
        use std::path::{Path, PathBuf};

        use rustconn_core::config::SecretSettings;
        use rustconn_core::models::{Connection, ConnectionGroup, PasswordSource};

        use super::super::{database_override, secrets_for_database};

        /// `Prod` with its own database, `Web` under it, and a connection in `Web`.
        fn fixture() -> (Vec<ConnectionGroup>, Connection) {
            let mut prod = ConnectionGroup::new("Prod".to_string());
            prod.kdbx_path_override = Some(PathBuf::from("/vaults/prod.kdbx"));
            let web = ConnectionGroup::with_parent("Web".to_string(), prod.id);
            let mut conn = Connection::new_ssh("web1".to_string(), "web1".to_string(), 22);
            conn.group_id = Some(web.id);
            conn.password_source = PasswordSource::Vault;
            (vec![prod, web], conn)
        }

        #[test]
        fn a_vault_password_reads_the_inherited_group_database() {
            let (groups, conn) = fixture();
            assert_eq!(
                database_override(&conn, &groups),
                Some(PathBuf::from("/vaults/prod.kdbx"))
            );
        }

        #[test]
        fn the_connections_own_database_wins_for_vault() {
            let (groups, mut conn) = fixture();
            conn.kdbx_path_override = Some(PathBuf::from("/vaults/web1.kdbx"));
            assert_eq!(
                database_override(&conn, &groups),
                Some(PathBuf::from("/vaults/web1.kdbx"))
            );
        }

        #[test]
        fn an_inherited_password_reads_the_groups_database() {
            let (groups, mut conn) = fixture();
            conn.kdbx_path_override = Some(PathBuf::from("/vaults/web1.kdbx"));
            conn.password_source = PasswordSource::Inherit;
            assert_eq!(
                database_override(&conn, &groups),
                Some(PathBuf::from("/vaults/prod.kdbx"))
            );
        }

        #[test]
        fn nothing_set_means_the_global_database() {
            let conn = Connection::new_ssh("solo".to_string(), "solo".to_string(), 22);
            assert_eq!(database_override(&conn, &[]), None);
        }

        #[test]
        fn only_the_file_changes() {
            let global = SecretSettings {
                kdbx_enabled: true,
                kdbx_path: Some(PathBuf::from("/vaults/global.kdbx")),
                ..SecretSettings::default()
            };
            let own = secrets_for_database(&global, Some(Path::new("/vaults/prod.kdbx")));
            assert_eq!(own.kdbx_path, Some(PathBuf::from("/vaults/prod.kdbx")));
            assert!(own.kdbx_enabled);
            assert_eq!(own.preferred_backend, global.preferred_backend);
            assert_eq!(
                secrets_for_database(&global, None).kdbx_path,
                global.kdbx_path
            );
        }
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
    fn ssh_script_host_drops_the_password_a_config_proxy_would_receive() {
        use rustconn_core::config::{AppSettings, NetworkSettings};
        use rustconn_core::remote_script::ScriptTransport;

        let settings = AppSettings::default();
        let network = NetworkSettings::default();
        let target = target();
        let mut passwords = super::Passwords::new(&settings, &[], false);
        passwords.known.insert(target.id, secret());

        let mut sent = |declares_proxy: bool| {
            let host =
                super::ssh_script_host(&target, &[], &[], &network, &mut passwords, |_, _, _| {
                    declares_proxy
                })
                .expect("an SSH connection gives a host");
            let ScriptTransport::Ssh { credentials, .. } = host.transport else {
                panic!("an SSH host runs over SSH");
            };
            credentials.password.map(|p| p.expose_secret().to_string())
        };
        assert_eq!(sent(true), None);
        assert_eq!(sent(false), Some("hunter2".to_string()));
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
        let resolved = super::with_global_variables(&connection, &globals).unwrap();
        assert_eq!(resolved.host, "web01.example.com");
        assert_eq!(resolved.username.as_deref(), Some("bob"));
        assert_eq!(resolved.id, connection.id, "passwords are found by id");
    }

    #[test]
    fn plain_hosts_and_unsafe_values_are_left_alone() {
        use rustconn_core::variables::Variable;

        let plain = super::with_global_variables(&target(), &[Variable::new("site", "x")]).unwrap();
        assert_eq!(plain.host, "web.lan");
        assert_eq!(plain.username.as_deref(), Some("alice"));

        let mut connection = target();
        connection.host = "${site}".to_string();
        let resolved =
            super::with_global_variables(&connection, &[Variable::new("site", "a;reboot")])
                .unwrap();
        assert_eq!(resolved.host, "${site}");
    }

    #[test]
    fn a_secret_variable_without_a_value_counts_as_undefined() {
        use rustconn_core::variables::Variable;

        let mut connection = target();
        connection.host = "${site}.lan".to_string();
        let resolved =
            super::with_global_variables(&connection, &[Variable::new_secret("site", "")]).unwrap();
        assert_eq!(resolved.host, ".lan");
    }

    #[test]
    fn an_undefined_reference_is_cleared() {
        use rustconn_core::variables::Variable;

        let mut connection = target();
        connection.host = "${site}.lan".to_string();
        connection.username = Some("${login}".to_string());
        let resolved =
            super::with_global_variables(&connection, &[Variable::new("other", "x")]).unwrap();
        assert_eq!(resolved.host, ".lan");
        assert_eq!(resolved.username.as_deref(), Some(""));
    }

    #[test]
    fn an_ask_variable_leaves_the_host_out() {
        use rustconn_core::variables::Variable;

        let mut connection = target();
        connection.host = "${region}.lan".to_string();
        let globals = [Variable::new("region", "@ask:Region|eu|us")];
        assert_eq!(
            super::with_global_variables(&connection, &globals).map(|c| c.host),
            Err("region".to_string())
        );
    }

    #[test]
    fn a_host_left_out_counts_as_not_succeeded() {
        use rustconn_core::variables::Variable;

        let mut asking = target();
        asking.name = "asking".to_string();
        asking.host = "${region}.lan".to_string();
        let plain = target();
        let globals = [Variable::new("region", "@ask:Region|eu|us")];
        let (runnable, left_out) = super::leave_out_ask_hosts(&[&asking, &plain], &globals);
        assert_eq!(
            runnable.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            ["web"]
        );
        assert_eq!(left_out, 1);
        // The one host that ran succeeded; the run still fails.
        assert!(super::run_result(runnable.len(), runnable.len() + left_out, true).is_err());
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
    fn an_empty_answer_is_not_the_previous_password() {
        let settings = rustconn_core::config::AppSettings::default();
        let mut passwords = super::Passwords::new(&settings, &[], false);
        let first = passwords.typed_with("a", true, |prompt| {
            assert_eq!(prompt, "Password for a: ");
            Some("hunter2".to_string())
        });
        assert_eq!(first.unwrap().expose_secret(), "hunter2");
        let second = passwords.typed_with("b", true, |prompt| {
            assert_eq!(prompt, "Password for b: ");
            Some(String::new())
        });
        assert!(second.is_none(), "b must not get a's password");
    }

    #[test]
    fn same_password_asks_once_for_targets_only() {
        let settings = rustconn_core::config::AppSettings::default();
        let mut passwords = super::Passwords::new(&settings, &[], true);
        let asked = Cell::new(0);
        let ask = |answer: &'static str| {
            let asked = &asked;
            move |_: &str| {
                asked.set(asked.get() + 1);
                Some(answer.to_string())
            }
        };
        let a = passwords.typed_with("a", true, ask("shared"));
        let b = passwords.typed_with("b", true, ask("unused"));
        assert_eq!(a.unwrap().expose_secret(), "shared");
        assert_eq!(b.unwrap().expose_secret(), "shared");
        assert_eq!(asked.get(), 1);
        let jump = passwords.typed_with("jump", false, |prompt| {
            assert_eq!(prompt, "Password for jump: ");
            Some("bastion".to_string())
        });
        assert_eq!(jump.unwrap().expose_secret(), "bastion");
    }

    #[test]
    fn quiet_hides_only_successes() {
        use rustconn_core::remote_script::ScriptOutcome;

        assert!(super::announces_outcome(ScriptOutcome::Succeeded, false));
        assert!(!super::announces_outcome(ScriptOutcome::Succeeded, true));
        for outcome in [
            ScriptOutcome::ScriptFailed { code: Some(1) },
            ScriptOutcome::ConnectionFailed,
            ScriptOutcome::TimedOut,
            ScriptOutcome::NotStarted,
        ] {
            assert!(super::announces_outcome(outcome, true), "{outcome:?}");
        }
    }

    fn bastion(host: &str, pkcs11: Option<&str>, key: Option<&std::path::Path>) -> Connection {
        let mut hop = Connection::new_ssh(host.to_string(), host.to_string(), 22);
        if let ProtocolConfig::Ssh(config) = &mut hop.protocol_config {
            config.pkcs11_provider = pkcs11.map(str::to_string);
            if let Some(key) = key {
                config.key_source = SshKeySource::File {
                    path: key.to_path_buf(),
                };
            }
        }
        hop
    }

    /// The chain through `hops`, the one closest to the target first.
    fn chain_through(hops: &[&Connection]) -> JumpChain {
        JumpChain {
            hops: hops.iter().map(|hop| hop.host.clone()).collect(),
            hop_ids: hops.iter().map(|hop| Some(hop.id)).collect(),
        }
    }

    /// A WinRM host behind `near`, which is behind `far`, with `near`'s password
    /// known. Returns the tunnel `winrm_tunnel` builds.
    fn winrm_tunnel_through(far_host: &str) -> rustconn_core::ssh_tunnel::SshTunnelParams {
        use rustconn_core::config::{AppSettings, NetworkSettings};
        use rustconn_core::models::WinRmConfig;

        let far = bastion(far_host, None, None);
        let mut near = bastion("near.lan", None, None);
        near.set_jump_host_id(far.id);
        let mut win = Connection::new(
            "win".to_string(),
            "win.lan".to_string(),
            5985,
            ProtocolConfig::WinRm(WinRmConfig::default()),
        );
        win.set_jump_host_id(near.id);
        let settings = AppSettings::default();
        let mut passwords = super::Passwords::new(&settings, &[], false);
        passwords.known.insert(near.id, secret());
        let connections = [near, far];
        super::winrm_tunnel(
            &win,
            &connections,
            &[],
            &NetworkSettings::default(),
            &mut passwords,
            rustconn_core::ssh_tunnel::HopHostKeys::KnownOnly,
        )
        .expect("a jump host gives a tunnel")
    }

    #[test]
    fn a_winrm_tunnel_keeps_the_near_password_from_farther_hops() {
        let tunnel = winrm_tunnel_through("far.lan");
        assert_eq!(tunnel.jump_host, "near.lan");
        assert_eq!(
            tunnel.password.as_ref().map(ExposeSecret::expose_secret),
            Some("hunter2")
        );
        assert!(!tunnel.extra_args.iter().any(|arg| arg.starts_with("-J")));
        let [option, route] = tunnel.extra_args.as_slice() else {
            panic!("one ProxyCommand: {:?}", tunnel.extra_args);
        };
        assert_eq!(option, "-o");
        let command = route.strip_prefix("ProxyCommand=env ").expect(route);
        assert!(command.contains(" SSH_ASKPASS= "), "{command}");
        assert!(command.contains("StrictHostKeyChecking=yes"), "{command}");
        assert!(command.ends_with(" far.lan"), "{command}");
    }

    #[test]
    fn a_winrm_tunnel_past_an_unexpected_address_sends_no_password() {
        let tunnel = winrm_tunnel_through("far;reboot");
        assert!(tunnel.password.is_none());
        assert_eq!(tunnel.extra_args, ["-J", "far;reboot"]);
    }

    #[test]
    fn every_proxy_command_hop_refuses_an_unknown_host_key() {
        let near = bastion("near.lan", Some("/usr/lib/opensc-pkcs11.so"), None);
        let far = bastion("far.lan", None, None);
        let chain = chain_through(&[&near, &far]);
        let connections = [near, far];

        let route = plain_jump_route(&chain, &[None, None], &connections, &[], &[], false);
        let Some(JumpRoute::ProxyCommand { command, passwords }) = route else {
            panic!("a PKCS#11 bastion needs a ProxyCommand: {route:?}");
        };
        assert!(passwords.is_empty());
        assert_eq!(
            command.matches("StrictHostKeyChecking=yes").count(),
            2,
            "{command}"
        );
        assert!(!command.contains("accept-new"), "{command}");
        assert!(
            command.contains("PKCS11Provider=/usr/lib/opensc-pkcs11.so"),
            "{command}"
        );
    }

    #[test]
    fn a_single_bastion_gets_its_own_key() {
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("id_bastion");
        std::fs::write(&key, "").unwrap();
        let hop = bastion("jump.lan", None, Some(&key));
        let chain = chain_through(&[&hop]);
        let session_args = ["-i".to_string(), "/k/target".to_string()];

        let route = plain_jump_route(
            &chain,
            &[None],
            std::slice::from_ref(&hop),
            &[],
            &session_args,
            false,
        );
        let Some(JumpRoute::ProxyCommand { command, .. }) = route else {
            panic!("a bastion key needs a ProxyCommand: {route:?}");
        };
        assert!(
            command.contains(&format!("-i {}", key.display())),
            "{command}"
        );
        assert!(!command.contains("/k/target"), "{command}");
        assert!(command.contains("StrictHostKeyChecking=yes"), "{command}");
    }

    #[test]
    fn paths_a_shell_could_interpret_stay_out_of_the_proxy_command() {
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("my key");
        std::fs::write(&key, "").unwrap();
        let hop = bastion("jump.lan", Some("/usr/lib/my pkcs11.so"), Some(&key));
        let chain = chain_through(&[&hop]);
        let session_args = ["-i".to_string(), "/k/target key".to_string()];
        let connections = std::slice::from_ref(&hop);

        // With nothing usable left, the hop still gets a ProxyCommand, so it
        // refuses an unknown host key; no refused path goes into it.
        let route = plain_jump_route(&chain, &[None], connections, &[], &session_args, false);
        let Some(JumpRoute::ProxyCommand { command, .. }) = route else {
            panic!("a script-run hop needs a ProxyCommand: {route:?}");
        };
        for refused in ["my key", "target key", "pkcs11", " -i "] {
            assert!(!command.contains(refused), "{refused}: {command}");
        }
        assert!(command.contains("StrictHostKeyChecking=yes"), "{command}");

        // A password still needs a ProxyCommand, without the refused paths.
        let route = plain_jump_route(&chain, &[secret()], connections, &[], &session_args, false);
        let Some(JumpRoute::ProxyCommand { command, .. }) = route else {
            panic!("a bastion password needs a ProxyCommand: {route:?}");
        };
        for refused in ["my key", "target key", "pkcs11", " -i "] {
            assert!(!command.contains(refused), "{refused}: {command}");
        }
        assert!(command.contains("StrictHostKeyChecking=yes"), "{command}");
    }
}
