//! Shared utility functions used across command modules.

use std::path::Path;

use rustconn_core::config::ConfigManager;
use rustconn_core::models::Connection;

use crate::error::CliError;

/// What came back from asking the user to confirm something.
///
/// Three outcomes rather than a `bool`, because the difference between "the user
/// said no" and "there was nobody to ask" is the whole reason three near-identical
/// prompts had grown in this crate. `connection delete` treats a non-interactive
/// stdin as a silent abort, `history clear` and `snippet run --execute` both fail
/// and name `--force`; a `bool` cannot carry that, so each call site rolled its
/// own prompt to keep its own answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confirmation {
    /// The user typed `y`.
    Confirmed,
    /// The user typed anything else, or closed stdin.
    Declined,
    /// stdin is not a terminal, so no question was asked. Never treat this as
    /// consent: it is how a destructive command ends up running unattended.
    NotInteractive,
}

impl Confirmation {
    /// Whether the action may proceed.
    #[must_use]
    pub const fn is_confirmed(self) -> bool {
        matches!(self, Self::Confirmed)
    }
}

/// Asks a yes/no question on the terminal, defaulting to no.
///
/// `prompt` is the question without the `[y/N]` suffix, which is appended here so
/// every prompt in the CLI looks the same.
///
/// The question goes to stderr, not stdout. A prompt is not output — it must not
/// land in a file or a pipe that is collecting the command's results (clig.dev).
/// The `history clear` copy printed to stdout and consequently needed an explicit
/// flush, because stdout is line-buffered and a prompt has no trailing newline;
/// Rust's stderr is unbuffered, so writing there removes both problems.
///
/// Only a bare `y`/`Y` confirms. Not `yes`, deliberately: every existing copy
/// behaved this way, and widening it here would quietly lower the bar on
/// `connection delete`.
pub fn confirm_on_terminal(prompt: &str) -> Confirmation {
    use std::io::IsTerminal;

    if !std::io::stdin().is_terminal() {
        return Confirmation::NotInteractive;
    }

    eprint!("{prompt} [y/N] ");

    let mut input = String::new();
    // A read error and a typed "n" are the same decision: do not proceed.
    if std::io::stdin().read_line(&mut input).is_err() {
        return Confirmation::Declined;
    }

    if input.trim().eq_ignore_ascii_case("y") {
        Confirmation::Confirmed
    } else {
        Confirmation::Declined
    }
}

/// Creates a `ConfigManager` using the optional custom config directory
/// from CLI args, falling back to the `RUSTCONN_CONFIG_DIR` environment
/// variable when no explicit path is provided.
pub fn create_config_manager(config_path: Option<&Path>) -> Result<ConfigManager, CliError> {
    if let Some(path) = config_path {
        return Ok(ConfigManager::with_config_dir(path.to_path_buf()));
    }

    if let Ok(env_dir) = std::env::var("RUSTCONN_CONFIG_DIR")
        && !env_dir.is_empty()
    {
        return Ok(ConfigManager::with_config_dir(std::path::PathBuf::from(
            env_dir,
        )));
    }

    ConfigManager::new().map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))
}
/// Creates a `TemplateManager` using the optional custom config directory.
///
/// Convenience wrapper that builds a `ConfigManager` and initializes
/// a `TemplateManager` from it.
pub fn create_template_manager(
    config_path: Option<&Path>,
) -> Result<rustconn_core::TemplateManager, CliError> {
    let config_manager = create_config_manager(config_path)?;
    rustconn_core::TemplateManager::new(config_manager)
        .map_err(|e| CliError::Template(format!("Failed to initialize template manager: {e}")))
}

/// Parse a baud rate integer into the corresponding `SerialBaudRate` enum.
///
/// Returns an error for unsupported values instead of silently falling back.
pub fn parse_baud_rate(
    baud: u32,
) -> Result<rustconn_core::models::SerialBaudRate, crate::error::CliError> {
    use rustconn_core::models::SerialBaudRate;
    match baud {
        9600 => Ok(SerialBaudRate::B9600),
        19_200 => Ok(SerialBaudRate::B19200),
        38_400 => Ok(SerialBaudRate::B38400),
        57_600 => Ok(SerialBaudRate::B57600),
        115_200 => Ok(SerialBaudRate::B115200),
        230_400 => Ok(SerialBaudRate::B230400),
        460_800 => Ok(SerialBaudRate::B460800),
        921_600 => Ok(SerialBaudRate::B921600),
        _ => Err(crate::error::CliError::Config(format!(
            "Unsupported baud rate: {baud}. \
             Supported: 9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600"
        ))),
    }
}

/// A `--kdbx-path` value made absolute against the current directory, so the
/// stored override names the same file when the GUI reads it from elsewhere.
///
/// # Errors
///
/// Returns [`crate::error::CliError::Io`] when the current directory cannot be
/// read.
pub fn absolute_kdbx_path(path: &Path) -> Result<std::path::PathBuf, crate::error::CliError> {
    Ok(std::path::absolute(path)?)
}

/// A clearable `--kdbx-path` value: an empty or blank string is `None`
/// (inherit again), anything else is made absolute.
///
/// # Errors
///
/// As [`absolute_kdbx_path`].
pub fn kdbx_path_arg(value: &str) -> Result<Option<std::path::PathBuf>, crate::error::CliError> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    absolute_kdbx_path(Path::new(value)).map(Some)
}

/// Parse a key=value pair for variable substitution
pub fn parse_key_val(s: &str) -> Result<(String, String), String> {
    let pos = s
        .find('=')
        .ok_or_else(|| format!("invalid KEY=value: no `=` found in `{s}`"))?;
    Ok((s[..pos].to_string(), s[pos + 1..].to_string()))
}

/// Find a connection by name or UUID
pub fn find_connection<'a>(
    connections: &'a [Connection],
    name_or_id: &str,
) -> Result<&'a Connection, CliError> {
    // First try to find by exact name match
    if let Some(conn) = connections.iter().find(|c| c.name == name_or_id) {
        return Ok(conn);
    }

    // Try to find by UUID
    if let Ok(uuid) = uuid::Uuid::parse_str(name_or_id)
        && let Some(conn) = connections.iter().find(|c| c.id == uuid)
    {
        return Ok(conn);
    }

    // Try case-insensitive name match
    if let Some(conn) = connections
        .iter()
        .find(|c| c.name.eq_ignore_ascii_case(name_or_id))
    {
        return Ok(conn);
    }

    // Try partial name match (prefix)
    let matches: Vec<_> = connections
        .iter()
        .filter(|c| {
            c.name
                .to_lowercase()
                .starts_with(&name_or_id.to_lowercase())
        })
        .collect();

    match matches.len() {
        0 => {
            // Fuzzy substring suggestions (CLI-08)
            let needle = name_or_id.to_lowercase();
            let suggestions: Vec<&str> = connections
                .iter()
                .filter(|c| c.name.to_lowercase().contains(&needle))
                .take(5)
                .map(|c| c.name.as_str())
                .collect();

            if suggestions.is_empty() {
                Err(CliError::ConnectionNotFound(name_or_id.to_string()))
            } else {
                Err(CliError::ConnectionNotFound(format!(
                    "'{}'. Did you mean: {}?",
                    name_or_id,
                    suggestions.join(", ")
                )))
            }
        }
        1 => Ok(matches[0]),
        _ => {
            let names: Vec<_> = matches.iter().map(|c| c.name.as_str()).collect();
            Err(CliError::Config(format!(
                "Ambiguous connection name '{}'. Matches: {}",
                name_or_id,
                names.join(", ")
            )))
        }
    }
}

/// Resolves a group name to its UUID, creating the group if it does not exist.
///
/// Tries case-sensitive match, then case-insensitive. If no match is found,
/// appends a new `ConnectionGroup` to the slice's owner via the provided callback.
///
/// # Errors
///
/// Returns `CliError::Config` if the name is empty.
pub fn find_or_create_group_id(
    groups: &mut Vec<rustconn_core::models::ConnectionGroup>,
    name: &str,
) -> Result<uuid::Uuid, CliError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(CliError::Config("Group name cannot be empty".to_string()));
    }

    if let Some(g) = groups.iter().find(|g| g.name == trimmed) {
        return Ok(g.id);
    }

    if let Some(g) = groups.iter().find(|g| g.name.eq_ignore_ascii_case(trimmed)) {
        return Ok(g.id);
    }

    // Group doesn't exist — create it
    let new_group = rustconn_core::models::ConnectionGroup::new(trimmed.to_string());
    let id = new_group.id;
    groups.push(new_group);
    Ok(id)
}

/// Outputs content through a pager (`less`) if stdout is a terminal and
/// the content exceeds 40 lines. Falls back to direct printing otherwise.
///
/// # Errors
///
/// Returns `CliError` if writing to the pager fails.
pub fn output_with_pager(content: &str) -> Result<(), CliError> {
    use std::io::{IsTerminal, Write};

    if !std::io::stdout().is_terminal() || content.lines().count() < 40 {
        print!("{content}");
        return Ok(());
    }

    let pager = std::process::Command::new("less")
        .args(["-FIRX"])
        .stdin(std::process::Stdio::piped())
        .spawn();

    if let Ok(mut child) = pager {
        if let Some(ref mut stdin) = child.stdin {
            let _ = stdin.write_all(content.as_bytes());
        }
        let _ = child.wait();
        Ok(())
    } else {
        // Fallback: print directly if less is not available
        print!("{content}");
        Ok(())
    }
}

/// Parse a protocol string into `ProtocolType`
pub fn parse_protocol_type(s: &str) -> Result<rustconn_core::models::ProtocolType, CliError> {
    use rustconn_core::models::ProtocolType;
    match s.to_lowercase().as_str() {
        "ssh" => Ok(ProtocolType::Ssh),
        "rdp" => Ok(ProtocolType::Rdp),
        "vnc" => Ok(ProtocolType::Vnc),
        "spice" => Ok(ProtocolType::Spice),
        "telnet" => Ok(ProtocolType::Telnet),
        "winrm" => Ok(ProtocolType::WinRm),
        "serial" => Ok(ProtocolType::Serial),
        "sftp" => Ok(ProtocolType::Sftp),
        "kubernetes" | "k8s" => Ok(ProtocolType::Kubernetes),
        "zerotrust" | "zero-trust" | "zt" => Ok(ProtocolType::ZeroTrust),
        "mosh" => Ok(ProtocolType::Mosh),
        "web" | "http" | "https" => Ok(ProtocolType::Web),
        _ => Err(CliError::Config(format!("Unknown protocol: {s}"))),
    }
}

/// Get the default port for a protocol
pub fn default_port_for_protocol(proto: rustconn_core::models::ProtocolType) -> u16 {
    use rustconn_core::models::ProtocolType;
    match proto {
        ProtocolType::Ssh | ProtocolType::Sftp | ProtocolType::Mosh => 22,
        ProtocolType::Rdp => 3389,
        ProtocolType::Vnc => 5900,
        ProtocolType::Spice => 5930,
        ProtocolType::Telnet => 23,
        ProtocolType::WinRm => 5985,
        ProtocolType::Serial => 0,
        ProtocolType::Kubernetes => 0,
        ProtocolType::ZeroTrust => 22,
        ProtocolType::Web => 443,
    }
}

#[cfg(test)]
mod confirmation_tests {
    use super::Confirmation;

    /// The rule the three-way enum exists to protect: only an explicit yes lets a
    /// destructive command proceed. `NotInteractive` in particular must never read
    /// as consent — that is how `snippet run --execute` or `history clear` would
    /// end up running unattended in a pipeline.
    #[test]
    fn only_an_explicit_confirmation_proceeds() {
        assert!(Confirmation::Confirmed.is_confirmed());
        assert!(!Confirmation::Declined.is_confirmed());
        assert!(!Confirmation::NotInteractive.is_confirmed());
    }

    /// `Declined` and `NotInteractive` both refuse, but they are not the same
    /// answer and callers act differently on them: `connection delete` aborts
    /// silently on a non-interactive stdin, while `history clear` and
    /// `snippet run --execute` fail and name `--force`. Collapsing the two back
    /// into a `bool` is what produced three copies of the prompt.
    #[test]
    fn declining_and_having_nobody_to_ask_stay_distinguishable() {
        assert_ne!(Confirmation::Declined, Confirmation::NotInteractive);
    }
}

#[cfg(test)]
mod protocol_parse_tests {
    use super::parse_protocol_type;
    use rustconn_core::models::ProtocolType;

    /// `add --help` advertises `web`, and the Web construction path
    /// (`ProtocolType::Web`) exists, but the parser used to reject it — so a
    /// documented protocol could not be created at all. Guard the three
    /// accepted spellings.
    #[test]
    fn web_aliases_parse() {
        for s in ["web", "http", "https", "WEB", "HTTPS"] {
            assert_eq!(parse_protocol_type(s).unwrap(), ProtocolType::Web);
        }
    }

    /// A few of the other protocols, to prove the added branch did not shadow
    /// them.
    #[test]
    fn known_protocols_still_parse() {
        assert_eq!(parse_protocol_type("ssh").unwrap(), ProtocolType::Ssh);
        assert_eq!(
            parse_protocol_type("k8s").unwrap(),
            ProtocolType::Kubernetes
        );
        assert_eq!(parse_protocol_type("zt").unwrap(), ProtocolType::ZeroTrust);
        assert!(parse_protocol_type("nonsense").is_err());
    }
}
