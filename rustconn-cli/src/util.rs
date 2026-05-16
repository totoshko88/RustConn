//! Shared utility functions used across command modules.

use std::path::Path;

use rustconn_core::config::ConfigManager;
use rustconn_core::models::Connection;

use crate::error::CliError;

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
        "serial" => Ok(ProtocolType::Serial),
        "sftp" => Ok(ProtocolType::Sftp),
        "kubernetes" | "k8s" => Ok(ProtocolType::Kubernetes),
        "zerotrust" | "zero-trust" | "zt" => Ok(ProtocolType::ZeroTrust),
        "mosh" => Ok(ProtocolType::Mosh),
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
        ProtocolType::Serial => 0,
        ProtocolType::Kubernetes => 0,
        ProtocolType::ZeroTrust => 22,
        ProtocolType::Web => 443,
    }
}
