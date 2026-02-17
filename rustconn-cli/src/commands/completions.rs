//! Shell completion generation.

use clap::CommandFactory;
use clap_complete::{generate, Shell};

use crate::cli::Cli;
use crate::error::CliError;

/// Generate shell completions and write to stdout.
pub fn cmd_completions(shell: Shell) -> Result<(), CliError> {
    let mut cmd = Cli::command();
    generate(shell, &mut cmd, "rustconn-cli", &mut std::io::stdout());
    Ok(())
}
