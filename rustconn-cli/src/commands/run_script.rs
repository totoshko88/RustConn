//! Run a local bash script on SSH connections.

use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use rustconn_core::config::NetworkSettings;
use rustconn_core::connection::gateway_failover::{gateway_probes_by, probe_gateways, use_gateway};
use rustconn_core::connection::resolve_proxy_jump_value;
use rustconn_core::connection::ssh_inheritance::resolve_ssh_key_path;
use rustconn_core::models::{Connection, ConnectionGroup, ProtocolConfig};
use rustconn_core::remote_script::{
    self, OutputStream, RunOptions, ScriptEvent, ScriptEventSink, ScriptHost, ScriptOutcome,
};
use rustconn_core::ssh_key_setup::{DeployCredentials, SshTarget};

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
    let hosts: Vec<ScriptHost> = targets
        .hosts
        .iter()
        .filter_map(|c| script_host(c, &connections, &groups, &settings.network, timeout_secs))
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

/// Builds the launch of one host: its options, key and jump hosts.
///
/// ponytail: key and agent login only, so a password host fails with ssh's own
/// "Permission denied"; resolve secrets here as `rustconn-cli secret` does if
/// that is needed. The GUI handles passwords.
fn script_host(
    connection: &Connection,
    connections: &[Connection],
    groups: &[ConnectionGroup],
    network: &NetworkSettings,
    probe_timeout_secs: u32,
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
        session_args.push("-J".to_string());
        session_args.push(jump);
    }

    Some(ScriptHost {
        name: routed.name.clone(),
        target: SshTarget::new(
            routed.host.clone(),
            routed.port,
            routed.username.clone(),
            session_args,
        ),
        credentials: DeployCredentials::default(),
    })
}
