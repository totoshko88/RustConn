//! Secret backend management commands.

use std::path::Path;

use crate::cli::SecretCommands;
use crate::error::CliError;
use crate::util::{create_config_manager, find_connection};

/// Creates a `PassBackend` from the current app settings.
fn create_pass_backend(
    settings: &rustconn_core::config::AppSettings,
) -> rustconn_core::secret::PassBackend {
    rustconn_core::secret::PassBackend::from_app_settings(settings)
}

/// Secret command handler
pub fn cmd_secret(config_path: Option<&Path>, subcmd: SecretCommands) -> Result<(), CliError> {
    match subcmd {
        SecretCommands::Status => cmd_secret_status(config_path),
        SecretCommands::Get {
            connection,
            backend,
        } => cmd_secret_get(config_path, &connection, backend.as_deref()),
        SecretCommands::Set {
            connection,
            user,
            password,
            backend,
        } => cmd_secret_set(
            config_path,
            &connection,
            user.as_deref(),
            password.as_deref(),
            backend.as_deref(),
        ),
        SecretCommands::Delete {
            connection,
            backend,
        } => cmd_secret_delete(config_path, &connection, backend.as_deref()),
        SecretCommands::VerifyKeepass { database, key_file } => {
            cmd_secret_verify_keepass(config_path, &database, key_file.as_deref())
        }
    }
}

#[allow(clippy::too_many_lines)]
fn cmd_secret_status(config_path: Option<&Path>) -> Result<(), CliError> {
    use rustconn_core::secret::KeePassStatus;

    println!("Secret Backend Status");
    println!("=====================\n");

    let libsecret_available = std::process::Command::new("which")
        .arg("secret-tool")
        .output()
        .is_ok_and(|o| o.status.success());
    println!(
        "Keyring (libsecret):  {}",
        if libsecret_available {
            "Available ✓"
        } else {
            "Not available (secret-tool not found)"
        }
    );

    let keepass_status = KeePassStatus::detect();
    if keepass_status.keepassxc_installed {
        let version = keepass_status
            .keepassxc_version
            .as_deref()
            .unwrap_or("unknown");
        println!("KeePassXC:            Available ✓ (version {version})");
        if let Some(ref path) = keepass_status.keepassxc_path {
            println!("  CLI path: {}", path.display());
        }
    } else {
        println!("KeePassXC:            Not installed");
    }

    let bw_output = std::process::Command::new("bw").arg("--version").output();
    if let Ok(output) = bw_output {
        if output.status.success() {
            let version = String::from_utf8_lossy(&output.stdout);
            println!(
                "Bitwarden CLI:        Available ✓ (version {})",
                version.trim()
            );
        } else {
            println!("Bitwarden CLI:        Not installed");
        }
    } else {
        println!("Bitwarden CLI:        Not installed");
    }

    let op_output = std::process::Command::new("op").arg("--version").output();
    if let Ok(output) = op_output {
        if output.status.success() {
            let version = String::from_utf8_lossy(&output.stdout);
            println!(
                "1Password CLI:        Available ✓ (version {})",
                version.trim()
            );
        } else {
            println!("1Password CLI:        Not installed");
        }
    } else {
        println!("1Password CLI:        Not installed");
    }

    let pb_output = std::process::Command::new("passbolt")
        .arg("--version")
        .output();
    if let Ok(output) = pb_output {
        if output.status.success() {
            let version = String::from_utf8_lossy(&output.stdout);
            println!(
                "Passbolt CLI:         Available ✓ (version {})",
                version.trim()
            );
        } else {
            println!("Passbolt CLI:         Not installed");
        }
    } else {
        println!("Passbolt CLI:         Not installed");
    }

    let pass_output = std::process::Command::new("pass").arg("--version").output();
    if let Ok(output) = pass_output {
        if output.status.success() {
            let version = String::from_utf8_lossy(&output.stdout);
            println!(
                "Pass (passwordstore):  Available ✓ (version {})",
                version.lines().next().unwrap_or("").trim()
            );
        } else {
            println!("Pass (passwordstore):  Not installed");
        }
    } else {
        println!("Pass (passwordstore):  Not installed");
    }

    let config_manager = create_config_manager(config_path)?;

    if let Ok(settings) = config_manager.load_settings() {
        println!("\nConfiguration:");
        println!(
            "  Preferred backend: {:?}",
            settings.secrets.preferred_backend
        );
        if settings.secrets.kdbx_enabled {
            if let Some(ref path) = settings.secrets.kdbx_path {
                println!("  KDBX database: {}", path.display());
            }
            if let Some(ref key) = settings.secrets.kdbx_key_file {
                println!("  KDBX key file: {}", key.display());
            }
        }
    }

    Ok(())
}

/// Parse backend string into `SecretBackendType`
fn parse_backend(b: &str) -> Result<rustconn_core::config::SecretBackendType, CliError> {
    use rustconn_core::config::SecretBackendType;
    match b.to_lowercase().as_str() {
        "keyring" | "libsecret" => Ok(SecretBackendType::LibSecret),
        "keepass" | "kdbx" | "keepassxc" => Ok(SecretBackendType::KdbxFile),
        "bitwarden" | "bw" => Ok(SecretBackendType::Bitwarden),
        "1password" | "onepassword" | "op" => Ok(SecretBackendType::OnePassword),
        "passbolt" => Ok(SecretBackendType::Passbolt),
        "pass" => Ok(SecretBackendType::Pass),
        _ => Err(CliError::Secret(format!(
            "Unknown backend: {b}. Use: keyring, keepass, bitwarden, \
             1password, passbolt, or pass"
        ))),
    }
}

#[allow(clippy::too_many_lines)]
fn cmd_secret_get(
    config_path: Option<&Path>,
    connection_name: &str,
    backend: Option<&str>,
) -> Result<(), CliError> {
    use rustconn_core::config::SecretBackendType;
    use rustconn_core::models::Credentials;
    use rustconn_core::secret::{KeePassHierarchy, KeePassStatus, LibSecretBackend, SecretBackend};

    let config_manager = create_config_manager(config_path)?;

    let connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let groups = config_manager
        .load_groups()
        .map_err(|e| CliError::Config(format!("Failed to load groups: {e}")))?;

    let connection = find_connection(&connections, connection_name)?;
    let lookup_key = format!("{} ({})", connection.name, connection.protocol.as_str());
    let keepass_base = KeePassHierarchy::build_entry_path(connection, &groups);
    let keepass_key = format!(
        "{} ({})",
        keepass_base
            .strip_prefix("RustConn/")
            .unwrap_or(&keepass_base),
        connection.protocol.as_str().to_lowercase()
    );

    let settings = config_manager
        .load_settings()
        .map_err(|e| CliError::Config(format!("Failed to load settings: {e}")))?;

    let backend_type = backend
        .map(parse_backend)
        .transpose()?
        .unwrap_or(settings.secrets.preferred_backend);

    match backend_type {
        SecretBackendType::LibSecret => {
            let rt = tokio::runtime::Runtime::new()
                .map_err(|e| CliError::Secret(format!("Runtime error: {e}")))?;

            let backend = LibSecretBackend::new("rustconn");
            let result: Result<Option<Credentials>, _> = rt.block_on(backend.retrieve(&lookup_key));

            match result {
                Ok(Some(creds)) => {
                    println!("Connection: {}", connection.name);
                    if let Some(ref user) = creds.username {
                        println!("Username:   {user}");
                    }
                    if creds.expose_password().is_some() {
                        println!("Password:   ********");
                        println!("\nUse 'secret-tool' to view actual value");
                    } else {
                        println!("Password:   (not set)");
                    }
                    Ok(())
                }
                Ok(None) => Err(CliError::Secret(format!(
                    "No credentials found for '{}'",
                    connection.name
                ))),
                Err(e) => Err(CliError::Secret(format!("Keyring error: {e}"))),
            }
        }
        SecretBackendType::KdbxFile | SecretBackendType::KeePassXc => {
            if !settings.secrets.kdbx_enabled {
                return Err(CliError::Secret(
                    "KeePass is not enabled in settings".into(),
                ));
            }
            let Some(ref kdbx_path) = settings.secrets.kdbx_path else {
                return Err(CliError::Secret("KeePass database not configured".into()));
            };

            let key_file = settings
                .secrets
                .kdbx_key_file
                .as_ref()
                .map(std::path::Path::new);

            let result = KeePassStatus::get_password_from_kdbx_with_key(
                std::path::Path::new(kdbx_path),
                None,
                key_file,
                &keepass_key,
                Some(connection.protocol.as_str()),
            );

            match result {
                Ok(Some(_)) => {
                    println!("Connection: {}", connection.name);
                    println!(
                        "Username:   {}",
                        connection.username.as_deref().unwrap_or("-")
                    );
                    println!("Password:   ******** (stored in KeePass)");
                    Ok(())
                }
                Ok(None) => Err(CliError::Secret(format!(
                    "No password found in KeePass for '{}'",
                    connection.name
                ))),
                Err(e) => Err(CliError::Secret(format!("KeePass error: {e}"))),
            }
        }
        SecretBackendType::Bitwarden => {
            use rustconn_core::secret::BitwardenBackend;

            let rt = tokio::runtime::Runtime::new()
                .map_err(|e| CliError::Secret(format!("Runtime error: {e}")))?;

            let backend = BitwardenBackend::new();
            let result: Result<Option<Credentials>, _> = rt.block_on(backend.retrieve(&lookup_key));

            match result {
                Ok(Some(creds)) => {
                    println!("Connection: {}", connection.name);
                    if let Some(ref user) = creds.username {
                        println!("Username:   {user}");
                    }
                    if creds.expose_password().is_some() {
                        println!(
                            "Password:   ******** \
                             (stored in Bitwarden)"
                        );
                    } else {
                        println!("Password:   (not set)");
                    }
                    Ok(())
                }
                Ok(None) => Err(CliError::Secret(format!(
                    "No credentials found in Bitwarden for '{}'",
                    connection.name
                ))),
                Err(e) => Err(CliError::Secret(format!("Bitwarden error: {e}"))),
            }
        }
        SecretBackendType::OnePassword => {
            use rustconn_core::secret::OnePasswordBackend;

            let rt = tokio::runtime::Runtime::new()
                .map_err(|e| CliError::Secret(format!("Runtime error: {e}")))?;

            let backend = OnePasswordBackend::new();
            let result: Result<Option<Credentials>, _> = rt.block_on(backend.retrieve(&lookup_key));

            match result {
                Ok(Some(creds)) => {
                    println!("Connection: {}", connection.name);
                    if let Some(ref user) = creds.username {
                        println!("Username:   {user}");
                    }
                    if creds.expose_password().is_some() {
                        println!(
                            "Password:   ******** \
                             (stored in 1Password)"
                        );
                    } else {
                        println!("Password:   (not set)");
                    }
                    Ok(())
                }
                Ok(None) => Err(CliError::Secret(format!(
                    "No credentials found in 1Password for '{}'",
                    connection.name
                ))),
                Err(e) => Err(CliError::Secret(format!("1Password error: {e}"))),
            }
        }
        SecretBackendType::Passbolt => {
            use rustconn_core::secret::PassboltBackend;

            let rt = tokio::runtime::Runtime::new()
                .map_err(|e| CliError::Secret(format!("Runtime error: {e}")))?;

            let backend = PassboltBackend::new();
            let pb_key = connection.id.to_string();
            let result: Result<Option<Credentials>, _> = rt.block_on(backend.retrieve(&pb_key));

            match result {
                Ok(Some(creds)) => {
                    println!("Connection: {}", connection.name);
                    if let Some(ref user) = creds.username {
                        println!("Username:   {user}");
                    }
                    if creds.expose_password().is_some() {
                        println!(
                            "Password:   ******** \
                             (stored in Passbolt)"
                        );
                    } else {
                        println!("Password:   (not set)");
                    }
                    Ok(())
                }
                Ok(None) => Err(CliError::Secret(format!(
                    "No credentials found in Passbolt for '{}'",
                    connection.name
                ))),
                Err(e) => Err(CliError::Secret(format!("Passbolt error: {e}"))),
            }
        }
        SecretBackendType::Pass => {
            let rt = tokio::runtime::Runtime::new()
                .map_err(|e| CliError::Secret(format!("Runtime error: {e}")))?;

            let backend = create_pass_backend(&settings);
            let result: Result<Option<Credentials>, _> = rt.block_on(backend.retrieve(&lookup_key));

            match result {
                Ok(Some(creds)) => {
                    println!("Connection: {}", connection.name);
                    if let Some(ref user) = creds.username {
                        println!("Username:   {user}");
                    }
                    if creds.expose_password().is_some() {
                        println!(
                            "Password:   ******** \
                             (stored in Pass)"
                        );
                    } else {
                        println!("Password:   (not set)");
                    }
                    Ok(())
                }
                Ok(None) => Err(CliError::Secret(format!(
                    "No credentials found in Pass for '{}'",
                    connection.name
                ))),
                Err(e) => Err(CliError::Secret(format!("Pass error: {e}"))),
            }
        }
    }
}

#[allow(clippy::too_many_lines)]
fn cmd_secret_set(
    config_path: Option<&Path>,
    connection_name: &str,
    username: Option<&str>,
    password: Option<&str>,
    backend: Option<&str>,
) -> Result<(), CliError> {
    use rustconn_core::config::SecretBackendType;
    use rustconn_core::secret::{KeePassHierarchy, KeePassStatus};

    let config_manager = create_config_manager(config_path)?;

    let connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let groups = config_manager
        .load_groups()
        .map_err(|e| CliError::Config(format!("Failed to load groups: {e}")))?;

    let connection = find_connection(&connections, connection_name)?;
    let lookup_key = format!("{} ({})", connection.name, connection.protocol.as_str());
    let keepass_base = KeePassHierarchy::build_entry_path(connection, &groups);
    let keepass_key = format!(
        "{} ({})",
        keepass_base
            .strip_prefix("RustConn/")
            .unwrap_or(&keepass_base),
        connection.protocol.as_str().to_lowercase()
    );

    let settings = config_manager
        .load_settings()
        .map_err(|e| CliError::Config(format!("Failed to load settings: {e}")))?;

    let backend_type = backend
        .map(parse_backend)
        .transpose()?
        .unwrap_or(settings.secrets.preferred_backend);

    let password_value = secrecy::SecretString::from(if let Some(pwd) = password {
        pwd.to_string()
    } else {
        eprint!("Enter password for '{}': ", connection.name);
        rpassword::read_password()
            .map_err(|e| CliError::Secret(format!("Failed to read password: {e}")))?
    });

    let username_value = username
        .map(String::from)
        .or_else(|| connection.username.clone())
        .unwrap_or_default();

    match backend_type {
        SecretBackendType::LibSecret => {
            use rustconn_core::models::Credentials;
            use rustconn_core::secret::{LibSecretBackend, SecretBackend};

            let rt = tokio::runtime::Runtime::new()
                .map_err(|e| CliError::Secret(format!("Runtime error: {e}")))?;

            let backend = LibSecretBackend::new("rustconn");
            let creds = Credentials {
                username: Some(username_value.clone()),
                password: Some(password_value.clone()),
                key_passphrase: None,
                domain: connection.domain.clone(),
            };

            rt.block_on(backend.store(&lookup_key, &creds))
                .map_err(|e| CliError::Secret(format!("Keyring error: {e}")))?;

            println!(
                "Stored credentials for '{}' in Keyring (user: {})",
                connection.name, username_value
            );
            Ok(())
        }
        SecretBackendType::KdbxFile | SecretBackendType::KeePassXc => {
            if !settings.secrets.kdbx_enabled {
                return Err(CliError::Secret(
                    "KeePass is not enabled in settings".into(),
                ));
            }
            let Some(ref kdbx_path) = settings.secrets.kdbx_path else {
                return Err(CliError::Secret("KeePass database not configured".into()));
            };

            let key_file = settings
                .secrets
                .kdbx_key_file
                .as_ref()
                .map(std::path::Path::new);

            KeePassStatus::save_password_to_kdbx(
                std::path::Path::new(kdbx_path),
                None,
                key_file,
                &keepass_key,
                &username_value,
                {
                    use secrecy::ExposeSecret;
                    password_value.expose_secret()
                },
                Some(&format!(
                    "{}://{}:{}",
                    connection.protocol.as_str().to_lowercase(),
                    connection.host,
                    connection.port
                )),
            )
            .map_err(|e| CliError::Secret(format!("KeePass error: {e}")))?;

            println!(
                "Stored credentials for '{}' in KeePass (user: {})",
                connection.name, username_value
            );
            Ok(())
        }
        SecretBackendType::Bitwarden => {
            use rustconn_core::models::Credentials;
            use rustconn_core::secret::{BitwardenBackend, SecretBackend};

            let rt = tokio::runtime::Runtime::new()
                .map_err(|e| CliError::Secret(format!("Runtime error: {e}")))?;

            let backend = BitwardenBackend::new();
            let creds = Credentials {
                username: Some(username_value.clone()),
                password: Some(password_value.clone()),
                key_passphrase: None,
                domain: connection.domain.clone(),
            };

            rt.block_on(backend.store(&lookup_key, &creds))
                .map_err(|e| CliError::Secret(format!("Bitwarden error: {e}")))?;

            println!(
                "Stored credentials for '{}' in Bitwarden \
                 (user: {})",
                connection.name, username_value
            );
            Ok(())
        }
        SecretBackendType::OnePassword => {
            use rustconn_core::models::Credentials;
            use rustconn_core::secret::{OnePasswordBackend, SecretBackend};

            let rt = tokio::runtime::Runtime::new()
                .map_err(|e| CliError::Secret(format!("Runtime error: {e}")))?;

            let backend = OnePasswordBackend::new();
            let creds = Credentials {
                username: Some(username_value.clone()),
                password: Some(password_value.clone()),
                key_passphrase: None,
                domain: connection.domain.clone(),
            };

            let op_key = connection.id.to_string();
            rt.block_on(backend.store(&op_key, &creds))
                .map_err(|e| CliError::Secret(format!("1Password error: {e}")))?;

            println!(
                "Stored credentials for '{}' in 1Password \
                 (user: {})",
                connection.name, username_value
            );
            Ok(())
        }
        SecretBackendType::Passbolt => {
            use rustconn_core::models::Credentials;
            use rustconn_core::secret::{PassboltBackend, SecretBackend};

            let rt = tokio::runtime::Runtime::new()
                .map_err(|e| CliError::Secret(format!("Runtime error: {e}")))?;

            let backend = PassboltBackend::new();
            let creds = Credentials {
                username: Some(username_value.clone()),
                password: Some(password_value.clone()),
                key_passphrase: None,
                domain: connection.domain.clone(),
            };

            let pb_key = connection.id.to_string();
            rt.block_on(backend.store(&pb_key, &creds))
                .map_err(|e| CliError::Secret(format!("Passbolt error: {e}")))?;

            println!(
                "Stored credentials for '{}' in Passbolt \
                 (user: {})",
                connection.name, username_value
            );
            Ok(())
        }
        SecretBackendType::Pass => {
            use rustconn_core::models::Credentials;
            use rustconn_core::secret::SecretBackend;

            let rt = tokio::runtime::Runtime::new()
                .map_err(|e| CliError::Secret(format!("Runtime error: {e}")))?;

            let backend = create_pass_backend(&settings);
            let creds = Credentials {
                username: Some(username_value.clone()),
                password: Some(password_value.clone()),
                key_passphrase: None,
                domain: connection.domain.clone(),
            };

            rt.block_on(backend.store(&lookup_key, &creds))
                .map_err(|e| CliError::Secret(format!("Pass error: {e}")))?;

            println!(
                "Stored credentials for '{}' in Pass \
                 (user: {})",
                connection.name, username_value
            );
            Ok(())
        }
    }
}

#[allow(clippy::too_many_lines)]
fn cmd_secret_delete(
    config_path: Option<&Path>,
    connection_name: &str,
    backend: Option<&str>,
) -> Result<(), CliError> {
    use rustconn_core::config::SecretBackendType;
    use rustconn_core::secret::{KeePassHierarchy, KeePassStatus, LibSecretBackend, SecretBackend};

    let config_manager = create_config_manager(config_path)?;

    let connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let groups = config_manager
        .load_groups()
        .map_err(|e| CliError::Config(format!("Failed to load groups: {e}")))?;

    let connection = find_connection(&connections, connection_name)?;
    let lookup_key = format!("{} ({})", connection.name, connection.protocol.as_str());
    let keepass_base = KeePassHierarchy::build_entry_path(connection, &groups);
    let keepass_entry_path = format!(
        "{} ({})",
        keepass_base,
        connection.protocol.as_str().to_lowercase()
    );

    let settings = config_manager
        .load_settings()
        .map_err(|e| CliError::Config(format!("Failed to load settings: {e}")))?;

    let backend_type = backend
        .map(parse_backend)
        .transpose()?
        .unwrap_or(settings.secrets.preferred_backend);

    match backend_type {
        SecretBackendType::LibSecret => {
            let rt = tokio::runtime::Runtime::new()
                .map_err(|e| CliError::Secret(format!("Runtime error: {e}")))?;

            let backend = LibSecretBackend::new("rustconn");
            rt.block_on(backend.delete(&lookup_key))
                .map_err(|e| CliError::Secret(format!("Keyring error: {e}")))?;

            println!("Deleted credentials for '{}' from Keyring", connection.name);
            Ok(())
        }
        SecretBackendType::KdbxFile | SecretBackendType::KeePassXc => {
            if !settings.secrets.kdbx_enabled {
                return Err(CliError::Secret(
                    "KeePass is not enabled in settings".into(),
                ));
            }
            let Some(ref kdbx_path) = settings.secrets.kdbx_path else {
                return Err(CliError::Secret("KeePass database not configured".into()));
            };

            let key_file = settings
                .secrets
                .kdbx_key_file
                .as_ref()
                .map(std::path::Path::new);

            KeePassStatus::delete_entry_from_kdbx(
                std::path::Path::new(kdbx_path),
                None,
                key_file,
                &keepass_entry_path,
            )
            .map_err(|e| CliError::Secret(format!("KeePass error: {e}")))?;

            println!("Deleted credentials for '{}' from KeePass", connection.name);
            Ok(())
        }
        SecretBackendType::Bitwarden => {
            use rustconn_core::secret::BitwardenBackend;

            let rt = tokio::runtime::Runtime::new()
                .map_err(|e| CliError::Secret(format!("Runtime error: {e}")))?;

            let backend = BitwardenBackend::new();
            rt.block_on(backend.delete(&lookup_key))
                .map_err(|e| CliError::Secret(format!("Bitwarden error: {e}")))?;

            println!(
                "Deleted credentials for '{}' from Bitwarden",
                connection.name
            );
            Ok(())
        }
        SecretBackendType::OnePassword => {
            use rustconn_core::secret::OnePasswordBackend;

            let rt = tokio::runtime::Runtime::new()
                .map_err(|e| CliError::Secret(format!("Runtime error: {e}")))?;

            let backend = OnePasswordBackend::new();
            let op_key = connection.id.to_string();
            rt.block_on(backend.delete(&op_key))
                .map_err(|e| CliError::Secret(format!("1Password error: {e}")))?;

            println!(
                "Deleted credentials for '{}' from 1Password",
                connection.name
            );
            Ok(())
        }
        SecretBackendType::Passbolt => {
            use rustconn_core::secret::PassboltBackend;

            let rt = tokio::runtime::Runtime::new()
                .map_err(|e| CliError::Secret(format!("Runtime error: {e}")))?;

            let backend = PassboltBackend::new();
            let pb_key = connection.id.to_string();
            rt.block_on(backend.delete(&pb_key))
                .map_err(|e| CliError::Secret(format!("Passbolt error: {e}")))?;

            println!(
                "Deleted credentials for '{}' from Passbolt",
                connection.name
            );
            Ok(())
        }
        SecretBackendType::Pass => {
            let rt = tokio::runtime::Runtime::new()
                .map_err(|e| CliError::Secret(format!("Runtime error: {e}")))?;

            let backend = create_pass_backend(&settings);
            rt.block_on(backend.delete(&lookup_key))
                .map_err(|e| CliError::Secret(format!("Pass error: {e}")))?;

            println!("Deleted credentials for '{}' from Pass", connection.name);
            Ok(())
        }
    }
}

fn cmd_secret_verify_keepass(
    _config_path: Option<&Path>,
    database: &Path,
    key_file: Option<&Path>,
) -> Result<(), CliError> {
    use rustconn_core::secret::KeePassStatus;

    KeePassStatus::validate_kdbx_path(database)
        .map_err(|e| CliError::Secret(format!("Invalid database: {e}")))?;

    if let Some(kf) = key_file {
        if !kf.exists() {
            return Err(CliError::Secret(format!(
                "Key file not found: {}",
                kf.display()
            )));
        }

        KeePassStatus::verify_kdbx_credentials(database, None, Some(kf))
            .map_err(|e| CliError::Secret(format!("Verification failed: {e}")))?;

        println!(
            "✓ KeePass database verified successfully \
             (using key file)"
        );
        println!("  Database: {}", database.display());
        println!("  Key file: {}", kf.display());
    } else {
        eprint!("Enter database password: ");
        let password = rpassword::read_password()
            .map_err(|e| CliError::Secret(format!("Failed to read password: {e}")))?;
        let password = secrecy::SecretString::from(password);

        KeePassStatus::verify_kdbx_credentials(database, Some(&password), None)
            .map_err(|e| CliError::Secret(format!("Verification failed: {e}")))?;

        println!("✓ KeePass database verified successfully");
        println!("  Database: {}", database.display());
    }

    Ok(())
}
