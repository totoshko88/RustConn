//! Shell completion generation.

use clap::CommandFactory;
use clap_complete::{Shell, generate};

use crate::cli::Cli;
use crate::error::CliError;

/// Generate shell completions and write to stdout.
///
/// # Errors
///
/// Always returns `Ok(())` today; the signature keeps `Result` for symmetry
/// with the rest of the `cmd_*` API.
pub(super) fn cmd_completions(shell: Shell) -> Result<(), CliError> {
    let mut cmd = Cli::command();
    generate(shell, &mut cmd, "rustconn-cli", &mut std::io::stdout());
    Ok(())
}
