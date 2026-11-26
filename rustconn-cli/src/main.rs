//! `RustConn` CLI - Command-line interface for `RustConn` connection manager
//!
//! Provides commands for listing, adding, exporting, importing, testing
//! connections, managing snippets, groups, templates, clusters, variables,
//! and Wake-on-LAN functionality.

// Binary crate: pub items are internal helpers shared between modules, not a public API.
#![allow(
    unreachable_pub,
    reason = "binary crate — pub is for inter-module visibility only"
)]
// CLI output IS println/eprintln — this is the intended interface, not a debug leftover.
#![allow(clippy::print_stdout, reason = "CLI binary communicates via stdout")]
#![allow(
    clippy::print_stderr,
    reason = "CLI binary uses stderr for warnings/errors"
)]

mod cli;
mod color;
mod commands;
mod error;
mod format;
mod util;

use clap::{CommandFactory, Parser};
use cli::Cli;

fn main() {
    let cli = Cli::try_parse().unwrap_or_else(|error| with_rename_hint(error).exit());
    let config_path = cli.config.as_deref();

    color::init(cli.no_color || std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()));
    setup_logging(cli.verbose, cli.quiet);

    let result = commands::dispatch(config_path, cli.command, cli.quiet);

    if let Err(e) = result {
        tracing::error!("{e}");
        if !cli.quiet {
            eprintln!("Error: {e}");
        }
        std::process::exit(e.exit_code());
    }
}

/// Adds a tip to clap's "unexpected argument '--name'" from a command that
/// renames with `--new-name` (`update`, the `edit` subcommands). `--name` is
/// not an alias there: in `add` it is the connection's own name, and in
/// `update` it would read as "the connection called". The error, its exit
/// status and clap's own tips stay as they are.
fn with_rename_hint(mut error: clap::Error) -> clap::Error {
    use clap::error::{ContextKind, ContextValue, ErrorKind};
    let asked_for_name = error.kind() == ErrorKind::UnknownArgument
        && matches!(
            error.get(ContextKind::InvalidArg),
            Some(ContextValue::String(arg)) if arg.split('=').next() == Some("--name")
        );
    let usage = error.get(ContextKind::Usage).map(ToString::to_string);
    if !asked_for_name || !usage.as_deref().is_some_and(renames_with_new_name) {
        return error;
    }
    let mut tips = match error.get(ContextKind::Suggested) {
        Some(ContextValue::StyledStrs(tips)) => tips.clone(),
        _ => Vec::new(),
    };
    tips.insert(0, "to rename, use '--new-name'".into());
    error.insert(ContextKind::Suggested, ContextValue::StyledStrs(tips));
    error
}

/// Whether the command in a clap usage line (`Usage: rustconn-cli update
/// [OPTIONS] <NAME>`) takes `--new-name`.
// ponytail: reads clap's rendered usage line to find the subcommand; if clap
// changes its layout the tip is just not added (the error is unchanged).
fn renames_with_new_name(usage: &str) -> bool {
    // Past "Usage:" and the program name, up to the first `[OPTIONS]`, `<ARG>`
    // or flag (a usage built around clap's own suggestion: `update --namespace
    // <NAME> <NAME>`).
    let path = usage
        .split_whitespace()
        .skip_while(|word| *word != "Usage:")
        .skip(2)
        .take_while(|word| !word.starts_with(['[', '<', '-']));
    let mut command = Cli::command();
    for name in path {
        let Some(sub) = command.find_subcommand(name).cloned() else {
            return false;
        };
        command = sub;
    }
    command
        .get_arguments()
        .any(|arg| arg.get_long() == Some("new-name"))
}

/// Initializes `tracing-subscriber` with a level derived from `--verbose` / `--quiet`.
fn setup_logging(verbose: u8, quiet: bool) {
    let filter = match (quiet, verbose) {
        (true, _) => "error",
        (_, 0) => "warn",
        (_, 1) => "info",
        (_, 2) => "debug",
        _ => "trace",
    };
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(filter)),
        )
        .with_writer(std::io::stderr)
        .without_time()
        .try_init();
}
