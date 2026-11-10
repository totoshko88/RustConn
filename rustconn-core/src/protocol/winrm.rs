//! WinRM protocol handler
//!
//! Sessions run `evil-winrm` in a VTE terminal. It takes a password only through
//! `-p`, where every local process could read it, or at its own prompt when `-p`
//! is absent; RustConn always uses the prompt.

use super::{Protocol, ProtocolCapabilities, ProtocolResult};
use crate::error::ProtocolError;
use crate::models::{Connection, ProtocolConfig};

/// The external client program.
pub const WINRM_CLIENT: &str = "evil-winrm";

/// The text the client prints when it asks for the password.
pub const WINRM_PASSWORD_PROMPT: &str = "Enter Password:";

/// Options that would put a secret on the client's command line.
const SECRET_OPTIONS: &[&str] = &["-p", "--password", "-H", "--hash"];

/// WinRM protocol handler
#[derive(Debug)]
pub struct WinRmProtocol;

impl WinRmProtocol {
    /// Creates a new WinRM protocol handler
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for WinRmProtocol {
    fn default() -> Self {
        Self::new()
    }
}

fn is_secret_option(arg: &str) -> bool {
    SECRET_OPTIONS
        .iter()
        .any(|option| arg == *option || arg.starts_with(&format!("{option}=")))
}

/// The account name the client logs in with: `DOMAIN\\user` when a domain is set
/// and the name does not already carry one.
pub(crate) fn account_name(connection: &Connection) -> Option<String> {
    let user = connection.username.as_deref()?.trim();
    if user.is_empty() {
        return None;
    }
    match connection.domain.as_deref().map(str::trim) {
        Some(domain) if !domain.is_empty() && !user.contains(['\\', '@']) => {
            Some(format!("{domain}\\{user}"))
        }
        _ => Some(user.to_string()),
    }
}

impl Protocol for WinRmProtocol {
    fn protocol_id(&self) -> &'static str {
        "winrm"
    }

    fn display_name(&self) -> &'static str {
        "WinRM"
    }

    fn default_port(&self) -> u16 {
        5985
    }

    fn validate_connection(&self, connection: &Connection) -> ProtocolResult<()> {
        if connection.host.is_empty() || connection.host.starts_with('-') {
            return Err(ProtocolError::InvalidConfig(
                "Host is not valid".to_string(),
            ));
        }
        if connection.port == 0 {
            return Err(ProtocolError::InvalidConfig("Port cannot be 0".to_string()));
        }
        if let ProtocolConfig::WinRm(config) = &connection.protocol_config
            && config.custom_args.iter().any(|arg| is_secret_option(arg))
        {
            return Err(ProtocolError::InvalidConfig(
                "Custom arguments cannot hold a password or hash".to_string(),
            ));
        }
        Ok(())
    }

    fn capabilities(&self) -> ProtocolCapabilities {
        ProtocolCapabilities::terminal()
    }

    fn build_command(&self, connection: &Connection) -> Option<Vec<String>> {
        let ProtocolConfig::WinRm(config) = &connection.protocol_config else {
            return None;
        };
        let mut cmd = vec![
            WINRM_CLIENT.to_string(),
            "-i".to_string(),
            connection.host.clone(),
            "-P".to_string(),
            connection.port.to_string(),
        ];
        if let Some(account) = account_name(connection) {
            cmd.push("-u".to_string());
            cmd.push(account);
        }
        if config.use_ssl {
            cmd.push("-S".to_string());
        }
        let mut args = config.custom_args.iter();
        while let Some(arg) = args.next() {
            if is_secret_option(arg) {
                tracing::warn!("Skipping a WinRM custom argument that holds a secret");
                if !arg.contains('=') {
                    args.next();
                }
            } else if !arg.contains(['\0', '\n']) {
                cmd.push(arg.clone());
            }
        }
        Some(cmd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::WinRmConfig;

    fn connection(config: WinRmConfig) -> Connection {
        let mut conn = Connection::new(
            "Windows".to_string(),
            "win.lan".to_string(),
            5985,
            ProtocolConfig::WinRm(config),
        );
        conn.username = Some("alice".to_string());
        conn
    }

    #[test]
    fn the_command_names_the_host_port_and_user() {
        let cmd = WinRmProtocol::new()
            .build_command(&connection(WinRmConfig::default()))
            .unwrap();
        assert_eq!(
            cmd,
            ["evil-winrm", "-i", "win.lan", "-P", "5985", "-u", "alice"]
        );
    }

    #[test]
    fn https_and_a_domain_are_passed() {
        let mut conn = connection(WinRmConfig {
            use_ssl: true,
            ..WinRmConfig::default()
        });
        conn.domain = Some("CORP".to_string());
        let cmd = WinRmProtocol::new().build_command(&conn).unwrap();
        assert!(cmd.iter().any(|a| a == "-S"));
        assert!(cmd.iter().any(|a| a == "CORP\\alice"));
    }

    #[test]
    fn a_name_already_qualified_keeps_its_form() {
        let mut conn = connection(WinRmConfig::default());
        conn.username = Some("alice@corp.example".to_string());
        conn.domain = Some("CORP".to_string());
        let cmd = WinRmProtocol::new().build_command(&conn).unwrap();
        assert!(cmd.iter().any(|a| a == "alice@corp.example"));
    }

    #[test]
    fn a_password_never_reaches_the_command_line() {
        let conn = connection(WinRmConfig {
            custom_args: ["-p", "hunter2", "--hash=abc", "-n", "--password", "x"]
                .map(String::from)
                .to_vec(),
            ..WinRmConfig::default()
        });
        let cmd = WinRmProtocol::new().build_command(&conn).unwrap();
        assert!(
            !cmd.iter()
                .any(|a| a.contains("hunter2") || a.contains("abc"))
        );
        assert!(!cmd.iter().any(|a| a == "x"));
        assert!(cmd.iter().any(|a| a == "-n"), "other options are kept");
        assert!(WinRmProtocol::new().validate_connection(&conn).is_err());
    }

    #[test]
    fn invalid_targets_are_refused() {
        let protocol = WinRmProtocol::new();
        let mut conn = connection(WinRmConfig::default());
        assert!(protocol.validate_connection(&conn).is_ok());
        conn.host = "-x".to_string();
        assert!(protocol.validate_connection(&conn).is_err());
        conn.host = "win.lan".to_string();
        conn.port = 0;
        assert!(protocol.validate_connection(&conn).is_err());
    }

    #[test]
    fn the_prompt_is_recognized_as_a_password_prompt() {
        assert!(crate::connection::looks_like_password_prompt(
            WINRM_PASSWORD_PROMPT
        ));
    }

    #[test]
    fn the_configuration_round_trips_through_the_stored_format() {
        let config = ProtocolConfig::WinRm(WinRmConfig {
            use_ssl: true,
            custom_args: vec!["-n".to_string()],
            ..WinRmConfig::default()
        });
        let text = serde_json::to_string(&config).unwrap();
        assert_eq!(
            serde_json::from_str::<ProtocolConfig>(&text).unwrap(),
            config
        );
    }
}
