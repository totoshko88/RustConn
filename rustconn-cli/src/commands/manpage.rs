//! Man page generation via `clap_mangen`.

use clap::CommandFactory;

use crate::cli::Cli;
use crate::error::CliError;

/// Generate a man page for the CLI and write it to stdout.
pub fn cmd_manpage() -> Result<(), CliError> {
    let cmd = Cli::command();
    let man = clap_mangen::Man::new(cmd);
    man.render(&mut std::io::stdout()).map_err(CliError::Io)?;
    Ok(())
}
