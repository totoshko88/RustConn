//! WinRM protocol handler
//!
//! Sessions run `evil-winrm` in a VTE terminal. It takes a password only through
//! `-p`, where every local process could read it, or at its own prompt when `-p`
//! is absent; RustConn always uses the prompt.

use std::path::PathBuf;

use super::{Protocol, ProtocolCapabilities, ProtocolResult};
use crate::error::ProtocolError;
use crate::models::{Connection, ProtocolConfig, WinRmConfig};

/// The external client program.
pub const WINRM_CLIENT: &str = "evil-winrm";

/// Where the WinRM client was found.
///
/// The Flatpak runtime has no Ruby, so the client usually lives on the host.
/// Either way the password never travels with the launch: it is typed at the
/// client's prompt in the terminal's PTY, which `flatpak-spawn` hands to the
/// host process, and is not passed in argv or the environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WinRmClient {
    /// On this environment's `PATH`.
    Local(PathBuf),
    /// On the Flatpak host, started through `flatpak-spawn --host`.
    Host(PathBuf),
}

/// Finds the WinRM client: on `PATH` first, then, in Flatpak, on the host.
///
/// The host probe is bounded to two seconds and is skipped outside Flatpak.
#[must_use]
pub fn locate_winrm_client() -> Option<WinRmClient> {
    crate::which::find_in_path(WINRM_CLIENT)
        .map(WinRmClient::Local)
        .or_else(|| crate::which::find_on_host(WINRM_CLIENT).map(WinRmClient::Host))
}

/// The host shell command a [`WinRmClient::Host`] runs: `exec` its `$0` (the
/// client's path) with its `"$@"` (the arguments), each passed as its own
/// argument, so none of them is parsed by the shell.
const HOST_LOGIN_EXEC: &str = r#"exec "$0" "$@""#;

impl WinRmClient {
    /// The argv that runs `built`, a [`WinRmProtocol::build_command`] result
    /// whose first element is [`WINRM_CLIENT`], with this client.
    ///
    /// A host client starts in a host *login* shell (`sh -lc`), the one
    /// [`locate_winrm_client`] found it with, so a Ruby manager such as rbenv
    /// or rvm that the profile sets up is in place for the launch too; the
    /// session helper's own environment has neither its `PATH` nor its
    /// `GEM_HOME`. Only `TERM` is passed, because the host process does not
    /// inherit the sandbox's environment and the client needs a terminal type;
    /// nothing else, and no secret, goes on the `flatpak-spawn` line.
    ///
    /// Only a leading [`WINRM_CLIENT`] is replaced. A `built` without it is kept
    /// whole, so no argument is ever dropped (this used to be a debug-only
    /// assertion, and release builds silently lost the first argument).
    #[must_use]
    pub fn command(&self, built: Vec<String>) -> Vec<String> {
        let skip = usize::from(built.first().is_some_and(|first| first == WINRM_CLIENT));
        let rest = built.into_iter().skip(skip);
        match self {
            Self::Local(path) => std::iter::once(path.to_string_lossy().into_owned())
                .chain(rest)
                .collect(),
            Self::Host(path) => [
                "flatpak-spawn",
                "--host",
                "--watch-bus",
                "--env=TERM=xterm-256color",
                "--",
                "sh",
                "-lc",
                HOST_LOGIN_EXEC,
            ]
            .into_iter()
            .map(str::to_string)
            .chain(std::iter::once(path.to_string_lossy().into_owned()))
            .chain(rest)
            .collect(),
        }
    }
}

/// The text the client prints when it asks for the password.
pub const WINRM_PASSWORD_PROMPT: &str = "Enter Password:";

/// Long options that would put a secret on the client's command line, without
/// their leading `--`.
const SECRET_LONG_OPTIONS: &[&str] = &["password", "hash"];

/// Short options that would put a secret on the client's command line.
const SECRET_SHORT_OPTIONS: &[char] = &['p', 'H'];

/// Where a custom argument that sets a secret option keeps its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SecretOption {
    /// The value is part of the same argument: `-phunter2`, `--password=x`.
    Attached,
    /// The value is the next argument: `-p x`, `-np x`, `--pass x`.
    NextArgument,
}

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

/// Recognizes every form in which evil-winrm's Ruby `OptionParser` takes
/// `-p`/`--password` or `-H`/`--hash`: the exact name, a value after `=`, a
/// prefix of the long name in any case (`--pass`, `--HA`), a short option with
/// its value attached (`-phunter2`) and a short option inside a cluster (`-np`).
///
/// It errs towards refusing. A short cluster counts when it holds `p` or `H`
/// anywhere, even where an earlier option would take the rest as its value, and
/// an ambiguous prefix such as `--p` counts too.
fn secret_option(arg: &str) -> Option<SecretOption> {
    if let Some(long) = arg.strip_prefix("--") {
        let (name, attached) = long
            .split_once('=')
            .map_or((long, false), |(n, _)| (n, true));
        let name = name.to_ascii_lowercase();
        let secret = !name.is_empty()
            && SECRET_LONG_OPTIONS
                .iter()
                .any(|option| option.starts_with(name.as_str()));
        return secret.then_some(if attached {
            SecretOption::Attached
        } else {
            SecretOption::NextArgument
        });
    }
    let cluster = arg.strip_prefix('-')?;
    let at = cluster.find(SECRET_SHORT_OPTIONS)?;
    // Both letters are ASCII, so `at + 1` is the next character's offset.
    Some(if at + 1 == cluster.len() {
        SecretOption::NextArgument
    } else {
        SecretOption::Attached
    })
}

/// The secret option in `arg` as the user typed it, for an error message:
/// `--` and the long name without what follows `=`, or a dash and the matched
/// short letter. Never holds a value attached to the option.
fn secret_option_name(arg: &str) -> Option<String> {
    secret_option(arg)?;
    if let Some(long) = arg.strip_prefix("--") {
        let name = long.split_once('=').map_or(long, |(name, _)| name);
        return Some(format!("--{name}"));
    }
    let cluster = arg.strip_prefix('-')?;
    let letter = cluster.chars().find(|c| SECRET_SHORT_OPTIONS.contains(c))?;
    Some(format!("-{letter}"))
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
            && let Some(name) = config
                .custom_args
                .iter()
                .find_map(|arg| secret_option_name(arg))
        {
            return Err(ProtocolError::InvalidConfig(format!(
                "Custom arguments cannot hold a password or hash ({name})"
            )));
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
        cmd.extend(launch_custom_args(config));
        Some(cmd)
    }
}

/// The custom arguments the client is given: `config.custom_args` without a
/// password or hash option (and its value) and without an argument holding a
/// NUL or a newline.
#[must_use]
pub fn launch_custom_args(config: &WinRmConfig) -> Vec<String> {
    let mut kept = Vec::new();
    let mut args = config.custom_args.iter();
    while let Some(arg) = args.next() {
        match secret_option(arg) {
            Some(form) => {
                tracing::warn!("Skipping a WinRM custom argument that holds a secret");
                if form == SecretOption::NextArgument {
                    args.next();
                }
            }
            None if !arg.contains(['\0', '\n']) => kept.push(arg.clone()),
            None => {}
        }
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn launch_custom_args_drops_secret_options() {
        let config = WinRmConfig {
            custom_args: ["-n", "-p", "hunter2", "-Habc", "--spn", "HTTP", "bad\narg"]
                .map(String::from)
                .to_vec(),
            ..WinRmConfig::default()
        };
        assert_eq!(launch_custom_args(&config), ["-n", "--spn", "HTTP"]);
    }

    #[test]
    fn abbreviated_attached_and_clustered_secret_options_are_recognized() {
        for arg in [
            "-p",
            "-H",
            "--password",
            "--hash",
            "--pass",
            "--pa",
            "--ha",
            "--HASH",
            "-np",
            "-nH",
            "--p",
        ] {
            assert_eq!(
                secret_option(arg),
                Some(SecretOption::NextArgument),
                "{arg}"
            );
        }
        for arg in ["-phunter2", "-Habc", "-nphunter2", "--pass=x", "--Hash=abc"] {
            assert_eq!(secret_option(arg), Some(SecretOption::Attached), "{arg}");
        }
        for arg in [
            "-n",
            "-N",
            "-S",
            "-P",
            "-l",
            "--port",
            "--help",
            "--no-colors",
            "--",
            "-",
            "plain",
        ] {
            assert_eq!(secret_option(arg), None, "{arg}");
        }
    }

    #[test]
    fn the_refusal_names_the_option_without_its_value() {
        for (arg, name) in [
            ("-phunter2", "-p"),
            ("--pass=hunter2", "--pass"),
            ("--HA", "--HA"),
            ("-nH", "-H"),
            ("-uHank", "-H"),
        ] {
            let conn = connection(WinRmConfig {
                custom_args: vec!["-n".to_string(), arg.to_string()],
                ..WinRmConfig::default()
            });
            let message = WinRmProtocol::new()
                .validate_connection(&conn)
                .unwrap_err()
                .to_string();
            assert!(message.contains(&format!("({name})")), "{arg}: {message}");
            assert!(!message.contains("hunter2"), "{arg}: {message}");
        }
        assert_eq!(secret_option_name("-n"), None);
        assert_eq!(secret_option_name("--port=1"), None);
    }

    #[test]
    fn abbreviated_and_attached_secrets_never_reach_the_command_line() {
        let conn = connection(WinRmConfig {
            custom_args: [
                "-phunter2",
                "--pass",
                "s3cret",
                "-np",
                "t0ken",
                "--HA=abc",
                "-N",
                "--no-colors",
            ]
            .map(String::from)
            .to_vec(),
            ..WinRmConfig::default()
        });
        let cmd = WinRmProtocol::new().build_command(&conn).unwrap();
        for secret in ["hunter2", "s3cret", "t0ken", "abc"] {
            assert!(!cmd.iter().any(|a| a.contains(secret)), "{secret}");
        }
        assert!(cmd.iter().any(|a| a == "-N"), "other options are kept");
        assert!(cmd.iter().any(|a| a == "--no-colors"));
        assert!(WinRmProtocol::new().validate_connection(&conn).is_err());
        for arg in ["-phunter2", "--pass", "-np", "--HA=abc"] {
            let single = connection(WinRmConfig {
                custom_args: vec![arg.to_string()],
                ..WinRmConfig::default()
            });
            assert!(
                WinRmProtocol::new().validate_connection(&single).is_err(),
                "{arg}"
            );
        }
    }

    fn built() -> Vec<String> {
        WinRmProtocol::new()
            .build_command(&connection(WinRmConfig::default()))
            .unwrap()
    }

    #[test]
    fn a_local_client_runs_by_its_path() {
        let client = WinRmClient::Local(PathBuf::from("/usr/bin/evil-winrm"));
        assert_eq!(
            client.command(built()),
            [
                "/usr/bin/evil-winrm",
                "-i",
                "win.lan",
                "-P",
                "5985",
                "-u",
                "alice"
            ]
        );
    }

    #[test]
    fn an_argv_without_the_client_name_loses_no_argument() {
        let client = WinRmClient::Local(PathBuf::from("/usr/bin/evil-winrm"));
        let mut args = built();
        args.remove(0);
        assert_eq!(
            client.command(args.clone()),
            std::iter::once("/usr/bin/evil-winrm".to_string())
                .chain(args)
                .collect::<Vec<_>>()
        );
        assert_eq!(client.command(Vec::new()), ["/usr/bin/evil-winrm"]);
    }

    #[test]
    fn a_host_client_runs_through_flatpak_spawn() {
        let client = WinRmClient::Host(PathBuf::from("/home/a/.gem/bin/evil-winrm"));
        let argv = client.command(built());
        assert_eq!(
            argv,
            [
                "flatpak-spawn",
                "--host",
                "--watch-bus",
                "--env=TERM=xterm-256color",
                "--",
                "sh",
                "-lc",
                r#"exec "$0" "$@""#,
                "/home/a/.gem/bin/evil-winrm",
                "-i",
                "win.lan",
                "-P",
                "5985",
                "-u",
                "alice"
            ]
        );
        assert!(!argv.iter().any(|a| a == "-p"), "no password option");
        assert!(
            argv.iter()
                .filter(|a| a.starts_with("--env="))
                .all(|a| a == "--env=TERM=xterm-256color"),
            "only TERM is passed to the host"
        );
    }

    #[test]
    fn the_host_shell_passes_the_path_and_arguments_through_unparsed() {
        // `-c` rather than the launch's `-lc`, so the test reads no profile.
        let output = std::process::Command::new("sh")
            .args([
                "-c",
                HOST_LOGIN_EXEC,
                "printf",
                "%s\n",
                "a b",
                "$(x);y",
                "*",
            ])
            .output()
            .expect("sh runs");
        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout), "a b\n$(x);y\n*\n");
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
