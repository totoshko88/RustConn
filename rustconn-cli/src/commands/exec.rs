//! Shared SSH selection/argv helper for the batch device commands.
//!
//! Kept separate from `report` so the target rules and the `ssh` argv contract
//! live in one place. This is the small part of the batch-execution work that
//! `report` needs; there is no `exec` subcommand in this branch.

use rustconn_core::models::{Connection, ConnectionGroup, ProtocolConfig, ProtocolType};
use rustconn_core::protocol::ProtocolRegistry;

use crate::error::CliError;
use crate::util::find_connection;

/// Resolves the target list, drops non-SSH connections, builds `ssh` argv.
///
/// # Errors
///
/// Returns [`CliError::Config`] when nothing was selected, no SSH connection
/// qualifies or `ssh` is missing; [`CliError::Group`] for an unknown group;
/// [`CliError::ConnectionNotFound`] for an unknown connection name; and
/// [`CliError::Protocol`] when argv cannot be built.
pub(super) fn select_ssh_jobs<'a>(
    connections: &'a [Connection],
    groups: &[ConnectionGroup],
    names: &[String],
    group: Option<&str>,
    tag: Option<&str>,
    command: &[String],
) -> Result<Vec<(&'a Connection, Vec<String>)>, CliError> {
    if names.is_empty() && group.is_none() && tag.is_none() {
        return Err(CliError::Config(
            "No targets selected — name connections or pick them with --group/--tag.".to_string(),
        ));
    }

    let selected = select_connections(connections, groups, names, group, tag)?;
    let (ssh_hosts, skipped): (Vec<&Connection>, Vec<&Connection>) = selected
        .into_iter()
        .partition(|c| c.protocol == ProtocolType::Ssh);
    if !skipped.is_empty() {
        let listed = skipped
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        eprintln!("Skipping non-SSH connection(s) {listed} — only ssh is used.");
    }
    if ssh_hosts.is_empty() {
        return Err(CliError::Config(
            "No SSH connection matches the selection.".to_string(),
        ));
    }

    let jobs = ssh_hosts
        .iter()
        .map(|conn| build_ssh_exec_args(conn, command).map(|args| (*conn, args)))
        .collect::<Result<Vec<_>, CliError>>()?;
    for (_, args) in &jobs {
        if !rustconn_core::which::is_available(&args[0]) {
            return Err(CliError::Config(format!(
                "Required program '{}' not found. Please install it to run SSH commands.",
                args[0]
            )));
        }
    }
    Ok(jobs)
}

/// Explicit names win; otherwise `--tag`/`--group` filter the connection list.
fn select_connections<'a>(
    all: &'a [Connection],
    groups: &[ConnectionGroup],
    names: &[String],
    group: Option<&str>,
    tag: Option<&str>,
) -> Result<Vec<&'a Connection>, CliError> {
    if !names.is_empty() {
        let mut selected: Vec<&Connection> = Vec::new();
        for name in names {
            let conn = find_connection(all, name)?;
            if !selected.iter().any(|c| c.id == conn.id) {
                selected.push(conn);
            }
        }
        return Ok(selected);
    }

    let group_id = group
        .map(|name| {
            let needle = name.to_lowercase();
            groups
                .iter()
                .find(|g| g.name.to_lowercase() == needle)
                .map(|g| g.id)
                .ok_or_else(|| CliError::Group(format!("Group not found: {name}")))
        })
        .transpose()?;

    Ok(all
        .iter()
        .filter(|c| {
            if let Some(gid) = group_id
                && c.group_id != Some(gid)
            {
                return false;
            }
            if let Some(tag_filter) = tag {
                let tag_lower = tag_filter.to_lowercase();
                if !c.tags.iter().any(|t| t.to_lowercase() == tag_lower) {
                    return false;
                }
            }
            true
        })
        .collect())
}

/// Builds `ssh -n -o BatchMode=yes -o ConnectTimeout=10 -- <dest> <command>` using the connection's own SSH options.
fn build_ssh_exec_args(
    connection: &Connection,
    command: &[String],
) -> Result<Vec<String>, CliError> {
    let registry = ProtocolRegistry::new();
    let Some(handler) = registry.get_by_type(ProtocolType::Ssh) else {
        return Err(CliError::Protocol(
            "ssh protocol handler missing".to_string(),
        ));
    };
    let Some(mut args) = handler.build_command(connection) else {
        return Err(CliError::Protocol(format!(
            "Cannot build the ssh command for '{}' — the connection is not SSH.",
            connection.name
        )));
    };

    if let ProtocolConfig::Ssh(config) = &connection.protocol_config
        && let Some(startup) = &config.startup_command
        && args
            .last()
            .is_some_and(|last| last.as_str() == startup.as_str())
    {
        args.pop();
    }
    let Some(destination) = args.pop() else {
        return Err(CliError::Protocol(format!(
            "Cannot build the ssh command for '{}': empty argv.",
            connection.name
        )));
    };

    // A destination that starts with `-` (an imported host such as
    // `-oProxyCommand=...`) would be parsed by ssh as an option.
    if destination.starts_with('-') {
        return Err(CliError::Protocol(format!(
            "Refusing to run ssh for '{}': the destination '{destination}' starts with '-'.",
            connection.name
        )));
    }
    // Never prompt: a password-only or unknown-host target fails fast instead
    // of blocking the whole batch on /dev/tty.
    for option in ["-n", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10"] {
        args.push(option.to_string());
    }
    // `--` ends option parsing, so the destination can never be read as one.
    args.push("--".to_string());
    args.push(destination);
    args.extend(command.iter().cloned());
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ssh(host: &str) -> Connection {
        Connection::new_ssh("sw".to_string(), host.to_string(), 22)
    }

    #[test]
    fn exec_args_are_non_interactive_with_the_destination_last_before_the_command() {
        let args =
            build_ssh_exec_args(&ssh("192.0.2.10"), &["show version".to_string()]).expect("args");
        let joined = args.join(" ");
        assert!(joined.contains("-o BatchMode=yes"), "{joined}");
        assert!(joined.contains("-o ConnectTimeout=10"), "{joined}");
        assert!(joined.ends_with("-- 192.0.2.10 show version"), "{joined}");
    }

    #[test]
    fn a_destination_starting_with_a_dash_is_refused() {
        let result = build_ssh_exec_args(&ssh("-oProxyCommand=x"), &["show version".to_string()]);
        assert!(matches!(result, Err(CliError::Protocol(_))));
    }
}
