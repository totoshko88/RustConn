//! `RustConn` CLI - Command-line interface for `RustConn` connection manager
//!
//! Provides commands for listing, adding, exporting, importing, testing
//! connections, managing snippets, groups, templates, clusters, variables,
//! and Wake-on-LAN functionality.

mod cli;
mod commands;
mod error;
mod format;
mod util;

use clap::Parser;
use cli::Cli;

fn main() {
    let cli = Cli::parse();
    let config_path = cli.config.as_deref();

    setup_logging(cli.verbose, cli.quiet);

    let result = commands::dispatch(config_path, cli.command);

    if let Err(e) = result {
        tracing::error!("{e}");
        if !cli.quiet {
            eprintln!("Error: {e}");
        }
        std::process::exit(e.exit_code());
    }
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
