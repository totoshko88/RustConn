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

    if cli.verbose > 0 && !cli.quiet {
        let level = match cli.verbose {
            1 => "info",
            2 => "debug",
            _ => "trace",
        };
        eprintln!("[verbosity: {level}]");
    }

    let result = commands::dispatch(config_path, cli.command);

    if let Err(e) = result {
        if !cli.quiet {
            eprintln!("Error: {e}");
        }
        std::process::exit(e.exit_code());
    }
}
