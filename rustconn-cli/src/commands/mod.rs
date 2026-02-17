//! Command handler modules for the CLI.

mod add;
mod cluster;
mod completions;
mod connect;
mod delete;
mod duplicate;
mod export_import;
mod group;
mod list;
mod secret;
mod sftp;
mod show;
mod snippet;
mod stats;
mod template;
mod test;
mod update;
mod variable;
mod wol;

use std::path::Path;

use crate::cli::Commands;
use crate::error::CliError;

/// Dispatch a CLI command to the appropriate handler.
pub fn dispatch(config_path: Option<&Path>, command: Commands) -> Result<(), CliError> {
    match command {
        Commands::List {
            format,
            protocol,
            group,
            tag,
        } => list::cmd_list(
            config_path,
            format,
            protocol.as_deref(),
            group.as_deref(),
            tag.as_deref(),
        ),
        Commands::Connect { name, dry_run } => connect::cmd_connect(config_path, &name, dry_run),
        Commands::Add {
            name,
            host,
            port,
            protocol,
            user,
            key,
            auth_method,
            device,
            baud_rate,
        } => add::cmd_add(
            config_path,
            add::AddParams {
                name: &name,
                host: &host,
                port,
                protocol: &protocol,
                user: user.as_deref(),
                key: key.as_deref(),
                auth_method: auth_method.as_deref(),
                device: device.as_deref(),
                baud_rate,
            },
        ),
        Commands::Export { format, output } => {
            export_import::cmd_export(config_path, format, &output)
        }
        Commands::Import { format, file } => export_import::cmd_import(config_path, format, &file),
        Commands::Test { name, timeout } => test::cmd_test(config_path, &name, timeout),
        Commands::Delete { name } => delete::cmd_delete(config_path, &name),
        Commands::Show { name } => show::cmd_show(config_path, &name),
        Commands::Update {
            name,
            new_name,
            host,
            port,
            user,
            key,
            auth_method,
            device,
            baud_rate,
        } => update::cmd_update(
            config_path,
            update::UpdateParams {
                name: &name,
                new_name: new_name.as_deref(),
                host: host.as_deref(),
                port,
                user: user.as_deref(),
                key: key.as_deref(),
                auth_method: auth_method.as_deref(),
                device: device.as_deref(),
                baud_rate,
            },
        ),
        Commands::Wol {
            target,
            broadcast,
            port,
        } => wol::cmd_wol(config_path, &target, &broadcast, port),
        Commands::Snippet(subcmd) => snippet::cmd_snippet(config_path, subcmd),
        Commands::Group(subcmd) => group::cmd_group(config_path, subcmd),
        Commands::Template(subcmd) => template::cmd_template(config_path, subcmd),
        Commands::Cluster(subcmd) => cluster::cmd_cluster(config_path, subcmd),
        Commands::Var(subcmd) => variable::cmd_var(config_path, subcmd),
        Commands::Secret(subcmd) => secret::cmd_secret(config_path, subcmd),
        Commands::Duplicate { name, new_name } => {
            duplicate::cmd_duplicate(config_path, &name, new_name.as_deref())
        }
        Commands::Sftp { name, cli, mc } => sftp::cmd_sftp(config_path, &name, cli, mc),
        Commands::Stats => stats::cmd_stats(config_path),
        Commands::Completions { shell } => completions::cmd_completions(shell),
    }
}
