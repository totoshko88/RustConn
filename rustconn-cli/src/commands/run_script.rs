//! Run a local bash script on SSH connections.

use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use rustconn_core::config::AppSettings;
use rustconn_core::config::NetworkSettings;
use rustconn_core::connection::gateway_failover::{gateway_probes_by, probe_gateways, use_gateway};
use rustconn_core::connection::resolve_proxy_jump_value;
use rustconn_core::connection::ssh_inheritance::resolve_ssh_key_path;
use rustconn_core::models::{Connection, ConnectionGroup, PasswordSource, ProtocolConfig};
use rustconn_core::remote_script::{
    self, OutputStream, RunOptions, ScriptEvent, ScriptEventSink, ScriptHost, ScriptOutcome,
};
use rustconn_core::secret::{CredentialResolver, SecretManager};
use rustconn_core::ssh_key_setup::{DeployCredentials, SshTarget};
use rustconn_core::ssh_tunnel::{askpass_disabled_proxy_prefix, target_password_askpass_allowed};
use secrecy::{ExposeSecret, SecretString};

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
    let targets = remote_script::script_targets(&selected, &connections, &groups);
    if targets.hosts.is_empty() {
        return Err(CliError::Config(
            "The selection holds no SSH connections".to_string(),
        ));
    }
    if targets.skipped > 0 {
        eprintln!("Leaving out {} non-SSH connection(s).", targets.skipped);
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
    let passwords = host_passwords(&targets.hosts, &groups, &settings)?;
    let hosts: Vec<ScriptHost> = targets
        .hosts
        .iter()
        .zip(passwords)
        .filter_map(|(c, password)| {
            script_host(
                c,
                &connections,
                &groups,
                &settings.network,
                timeout_secs,
                password,
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

/// The password of each host: saved ones from the configured secret backend,
/// and one typed answer shared by the hosts set to ask at connect.
///
/// A password that cannot be read is reported and left out, so that host tries
/// its keys and fails on its own instead of stopping the whole run.
fn host_passwords(
    hosts: &[&Connection],
    groups: &[ConnectionGroup],
    settings: &AppSettings,
) -> Result<Vec<Option<SecretString>>, CliError> {
    let typed = typed_password(hosts);
    // Building the secret manager reaches for the backends and warns about the
    // missing ones, which a run on keys alone has no reason to hear about.
    let needs_backend = hosts.iter().any(|c| {
        !matches!(
            c.password_source,
            PasswordSource::None | PasswordSource::Prompt
        )
    });
    if !needs_backend {
        return Ok(hosts
            .iter()
            .map(|c| (c.password_source == PasswordSource::Prompt).then(|| typed.clone())?)
            .collect());
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| CliError::Secret(format!("Runtime error: {e}")))?;
    let resolver = CredentialResolver::new(
        Arc::new(SecretManager::build_from_settings(&settings.secrets)),
        settings.secrets.clone(),
    );
    Ok(hosts
        .iter()
        .map(|connection| match connection.password_source {
            PasswordSource::None => None,
            PasswordSource::Prompt => typed.clone(),
            _ => match runtime.block_on(resolver.resolve_with_hierarchy(connection, groups)) {
                Ok(credentials) => credentials
                    .and_then(|c| c.password)
                    .filter(|p| !p.expose_secret().is_empty()),
                Err(error) => {
                    eprintln!(
                        "[{}] could not read the saved password: {error}",
                        connection.name
                    );
                    None
                }
            },
        })
        .collect())
}

/// Asks once for the hosts set to ask at connect. With no terminal to ask on,
/// they fall back to their keys.
fn typed_password(hosts: &[&Connection]) -> Option<SecretString> {
    use std::io::IsTerminal;

    let names: Vec<&str> = hosts
        .iter()
        .filter(|c| c.password_source == PasswordSource::Prompt)
        .map(|c| c.name.as_str())
        .collect();
    if names.is_empty() || !std::io::stdin().is_terminal() {
        return None;
    }
    eprint!("Password for {}: ", names.join(", "));
    rpassword::read_password()
        .ok()
        .filter(|password| !password.is_empty())
        .map(SecretString::from)
}

/// A `ProxyCommand` reaching the last hop of `chain` with askpass switched off.
///
/// Under `-J` each bastion's `ssh` inherits the askpass environment and would be
/// handed the target's password, so a password host goes through this instead.
/// `None` when the chain holds anything a shell could interpret.
fn bastion_proxy_command(chain: &str) -> Option<String> {
    let is_safe = |c: char| c.is_ascii_alphanumeric() || ".-_:@,[]".contains(c);
    if !chain.chars().all(is_safe) {
        return None;
    }
    let hops: Vec<&str> = chain.split(',').collect();
    let (last, earlier) = hops.split_last()?;
    if last.is_empty() || hops.iter().any(|hop| hop.starts_with('-')) {
        return None;
    }
    let mut parts = askpass_disabled_proxy_prefix(hops.len());
    parts.extend(["ssh", "-o", "ConnectTimeout=15"].map(String::from));
    if !earlier.is_empty() {
        parts.push("-J".to_string());
        parts.push(earlier.join(","));
    }
    parts.extend(["-W".to_string(), "%h:%p".to_string()]);
    // A plain destination cannot carry a port; the URI form can.
    parts.push(format!("ssh://{last}"));
    Some(parts.join(" "))
}

/// Builds the launch of one host: its options, key, jump hosts and password.
///
/// ponytail: a bastion's own password is not supplied, so bastions use keys or
/// the agent here; the GUI's per-hop askpass wiring lives in `rustconn` and would
/// have to move to core to be shared.
fn script_host(
    connection: &Connection,
    connections: &[Connection],
    groups: &[ConnectionGroup],
    network: &NetworkSettings,
    probe_timeout_secs: u32,
    password: Option<SecretString>,
) -> Option<ScriptHost> {
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
    let ProtocolConfig::Ssh(config) = &routed.protocol_config else {
        return None;
    };

    let mut password = password.filter(|_| target_password_askpass_allowed(config, false));
    let mut session_args = config.build_command_args();
    if !session_args.iter().any(|arg| arg == "-i")
        && let Some(key) = resolve_ssh_key_path(&routed, groups)
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
    if let Some(jump) = resolve_proxy_jump_value(&routed, connections, groups, network) {
        match password.is_some().then(|| bastion_proxy_command(&jump)) {
            Some(Some(command)) => {
                session_args.push("-o".to_string());
                session_args.push(format!("ProxyCommand={command}"));
            }
            unusable => {
                if unusable.is_some() {
                    eprintln!(
                        "[{}] not sending the password: unexpected jump host address",
                        routed.name
                    );
                    password = None;
                }
                session_args.push("-J".to_string());
                session_args.push(jump);
            }
        }
    }

    Some(ScriptHost {
        name: routed.name.clone(),
        target: SshTarget::new(
            routed.host.clone(),
            routed.port,
            routed.username.clone(),
            session_args,
        ),
        credentials: DeployCredentials {
            password,
            jump_host_passwords: Vec::new(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::bastion_proxy_command;

    #[test]
    fn a_single_hop_goes_straight_to_the_bastion() {
        let command = bastion_proxy_command("alice@jump:2222").unwrap();
        assert!(command.starts_with("env "), "{command}");
        assert!(command.contains("SSH_ASKPASS= "), "{command}");
        assert!(
            command.ends_with("ssh -o ConnectTimeout=15 -W %h:%p ssh://alice@jump:2222"),
            "{command}"
        );
    }

    #[test]
    fn earlier_hops_are_chained_with_jump() {
        let command = bastion_proxy_command("a,b,c").unwrap();
        assert!(command.ends_with("-J a,b -W %h:%p ssh://c"), "{command}");
    }

    #[test]
    fn anything_a_shell_could_interpret_is_refused() {
        assert!(bastion_proxy_command("jump;reboot").is_none());
        assert!(bastion_proxy_command("$(reboot)").is_none());
        assert!(bastion_proxy_command("-oProxyCommand=x").is_none());
        assert!(bastion_proxy_command("").is_none());
    }
}
