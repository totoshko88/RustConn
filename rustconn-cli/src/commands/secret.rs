//! Secret backend management commands.

use std::path::Path;
use std::sync::Arc;

use crate::cli::SecretCommands;
use crate::error::CliError;
#[cfg(feature = "secret-management")]
use crate::util::{create_config_manager, find_connection};

/// Prints a one-line cue on stderr when a `keepassxc-cli` run blocks on a
/// hardware-key touch.
///
/// KeePassXC's own "touch your YubiKey" prompt never reaches the user because
/// RustConn runs the CLI with `-q` and reads its output only after it exits
/// (issue #350). This observer turns the start of a touch-waiting run into a
/// visible line. It writes only when stderr is a terminal, so a scripted
/// `rustconn-cli secret …` in a pipe stays quiet.
struct StderrTouchCue;

impl rustconn_core::secret::TouchObserver for StderrTouchCue {
    fn touch_started(&self) {
        use std::io::IsTerminal;
        if std::io::stderr().is_terminal() {
            eprintln!("👆 Touch your hardware key to unlock the KeePass database…");
        }
    }

    fn touch_finished(&self) {}
}

/// Returns the one Tokio runtime shared by every async secret backend call in
/// this process.
///
/// The `secret get`/`set`/`delete` commands each touch one backend, often
/// through several `block_on` calls; building a fresh `Runtime` for each was
/// wasteful. A single current-thread runtime, created on first use and reused,
/// is enough — the CLI runs one command and never needs a worker pool.
///
/// # Errors
///
/// Returns [`CliError::Secret`] if the runtime cannot be created.
#[cfg(feature = "secret-management")]
fn secret_runtime() -> Result<&'static tokio::runtime::Runtime, CliError> {
    use std::sync::OnceLock;
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    if let Some(rt) = RUNTIME.get() {
        return Ok(rt);
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| CliError::Secret(format!("Runtime error: {e}")))?;
    // Another caller may have initialised it between the get and here; keep
    // whichever won and drop ours.
    Ok(RUNTIME.get_or_init(|| rt))
}

/// Creates a `PassBackend` from the current app settings.
#[cfg(feature = "secret-management")]
fn create_pass_backend(
    settings: &rustconn_core::config::AppSettings,
) -> rustconn_core::secret::PassBackend {
    rustconn_core::secret::PassBackend::from_app_settings(settings)
}

/// Where a connection's credential lives in a backend other than KeePass.
///
/// `primary` is the key the GUI saves and resolves under —
/// `RustConn/{group path}/{name} ({protocol})` for the system keyring,
/// `rustconn/{name}` for everything else. Until 0.23.17 this command used
/// `{name} ({protocol})` for all of them, and the connection UUID for a
/// 1Password store and for Passbolt. So `secret get` never found a password the
/// GUI had saved, and a connect never found one `secret set` had saved.
///
/// `legacy` is the rest of the GUI's key list for the connection
/// ([`rustconn_core::secret::vault_keys_for_connection`]), plus the keys older
/// releases of this command wrote, so their credentials are still read and still
/// deleted. Every backend's `delete` matches only RustConn's own entries, so a
/// legacy key that was never written deletes nothing.
#[cfg(feature = "secret-management")]
#[derive(Debug, PartialEq, Eq)]
pub(super) struct VaultKeys {
    primary: String,
    legacy: Vec<String>,
}

#[cfg(feature = "secret-management")]
fn vault_keys(
    connection: &rustconn_core::models::Connection,
    groups: &[rustconn_core::models::ConnectionGroup],
    backend: rustconn_core::config::SecretBackendType,
) -> VaultKeys {
    use rustconn_core::config::SecretBackendType;

    // The GUI's list, the same function its save, rename and cleanup use.
    let mut keys = rustconn_core::secret::vault_keys_for_connection(
        groups,
        connection,
        // The protocol the GUI keys by. `connection.protocol` is a second
        // persisted copy that nothing keeps in step on load.
        connection.protocol_config.protocol_type().as_str(),
        backend,
    );
    // Plus what this command alone used to write, on the backends it wrote it
    // to. 1Password and Passbolt are left out: the old command stored those
    // under the UUID, which the GUI list already ends with.
    if !matches!(
        backend,
        SecretBackendType::OnePassword | SecretBackendType::Passbolt
    ) {
        keys.push(format!(
            "{} ({})",
            connection.name,
            connection.protocol.as_str()
        ));
    }
    let primary = keys.remove(0);
    let mut legacy: Vec<String> = Vec::with_capacity(keys.len());
    for key in keys {
        if key != primary && !legacy.contains(&key) {
            legacy.push(key);
        }
    }
    VaultKeys { primary, legacy }
}

/// The other connections whose credential lives under `primary` too.
///
/// `rustconn/{name}` carries no group or protocol, so same-named connections
/// share it — in the GUI as well. A `set` or `delete` here changes it for all
/// of them, and the user only named one.
#[cfg(feature = "secret-management")]
fn connections_sharing_key<'a>(
    primary: &str,
    target: &rustconn_core::models::Connection,
    connections: &'a [rustconn_core::models::Connection],
    groups: &[rustconn_core::models::ConnectionGroup],
    backend: rustconn_core::config::SecretBackendType,
) -> Vec<&'a rustconn_core::models::Connection> {
    connections
        .iter()
        .filter(|c| c.id != target.id && vault_keys(c, groups, backend).primary == primary)
        .collect()
}

/// Tells the user, on stderr, that a `set`/`delete` also affects the
/// connections that share the key. Not an error: the GUI shares it the same way.
///
/// Silent for KeePass: its entry path carries the group and the protocol, so
/// same-named connections do not share it, and `primary` (`rustconn/{name}`) is
/// not the key KeePass is addressed by.
#[cfg(feature = "secret-management")]
fn warn_shared_key(
    backend: rustconn_core::config::SecretBackendType,
    primary: &str,
    others: &[&rustconn_core::models::Connection],
) {
    use rustconn_core::config::SecretBackendType;

    if others.is_empty()
        || matches!(
            backend,
            SecretBackendType::KdbxFile | SecretBackendType::KeePassXc
        )
    {
        return;
    }
    let names: Vec<String> = others
        .iter()
        .map(|c| format!("'{}' ({})", c.name, c.protocol.as_str()))
        .collect();
    eprintln!(
        "Warning: the vault key '{primary}' is shared with {}; this changes their \
         stored credential too.",
        names.join(", ")
    );
}

/// The key plus the connection's name and host, so a backend searching from
/// the vault root can match an entry titled by either — the read a connect does
/// (issue #353). A plain `retrieve(key)` gives root search nothing to match but
/// the key itself.
#[cfg(feature = "secret-management")]
fn identity_of<'a>(
    key: &'a str,
    connection: &'a rustconn_core::models::Connection,
) -> rustconn_core::secret::LookupIdentity<'a> {
    let non_blank = |s: &'a str| Some(s).filter(|s| !s.trim().is_empty());
    rustconn_core::secret::LookupIdentity {
        key,
        name: non_blank(&connection.name),
        host: non_blank(&connection.host),
    }
}

/// Reads a connection's credential: the GUI's key first, root search included,
/// then each legacy key by plain lookup.
#[cfg(feature = "secret-management")]
async fn retrieve_connection_credentials(
    backend: &dyn rustconn_core::secret::SecretBackend,
    keys: &VaultKeys,
    connection: &rustconn_core::models::Connection,
) -> rustconn_core::error::SecretResult<Option<rustconn_core::models::Credentials>> {
    if let Some(creds) = backend
        .retrieve_identity(identity_of(&keys.primary, connection))
        .await?
    {
        return Ok(Some(creds));
    }
    for key in &keys.legacy {
        if let Some(creds) = backend.retrieve(key).await? {
            return Ok(Some(creds));
        }
    }
    Ok(None)
}

/// Deletes a connection's credential under the GUI's key, then under every
/// legacy key.
///
/// The primary key's error fails the command. A legacy key's does not: most of
/// them hold nothing, and the credential the user asked about is the primary.
#[cfg(feature = "secret-management")]
async fn delete_connection_credentials(
    backend: &dyn rustconn_core::secret::SecretBackend,
    keys: &VaultKeys,
) -> rustconn_core::error::SecretResult<()> {
    backend.delete(&keys.primary).await?;
    for key in &keys.legacy {
        if let Err(e) = backend.delete(key).await {
            tracing::debug!(%key, error = %e, "secret delete: legacy key not deleted");
        }
    }
    Ok(())
}

/// Secret command handler
///
/// # Errors
///
/// Returns:
/// - [`CliError::Config`] when configuration cannot be read
/// - [`CliError::ConnectionNotFound`] when the targeted connection does not exist
/// - [`CliError::Secret`] when the configured secret backend is unreachable
///   or the requested operation (get / set / delete / status) fails
pub fn cmd_secret(config_path: Option<&Path>, subcmd: SecretCommands) -> Result<(), CliError> {
    // Every secret subcommand can open a KeePass database, and any of them may
    // block on a hardware-key touch. Install the stderr cue once for all of them.
    rustconn_core::secret::set_touch_observer(Some(Arc::new(StderrTouchCue)));

    match subcmd {
        #[cfg(feature = "secret-management")]
        SecretCommands::Status => cmd_secret_status(config_path),
        #[cfg(feature = "secret-management")]
        SecretCommands::Get {
            connection,
            backend,
        } => cmd_secret_get(config_path, &connection, backend.as_deref()),
        #[cfg(feature = "secret-management")]
        SecretCommands::Set {
            connection,
            user,
            password,
            password_stdin,
            backend,
        } => cmd_secret_set(
            config_path,
            &connection,
            user.as_deref(),
            password,
            password_stdin,
            backend.as_deref(),
        ),
        #[cfg(feature = "secret-management")]
        SecretCommands::Delete {
            connection,
            backend,
        } => cmd_secret_delete(config_path, &connection, backend.as_deref()),
        SecretCommands::VerifyKeepass {
            database,
            key_file,
            yubikey,
        } => cmd_secret_verify_keepass(
            config_path,
            &database,
            key_file.as_deref(),
            yubikey.as_deref(),
        ),
    }
}

#[cfg(feature = "secret-management")]
fn cmd_secret_status(config_path: Option<&Path>) -> Result<(), CliError> {
    use rustconn_core::secret::KeePassStatus;

    println!("Secret Backend Status");
    println!("=====================\n");

    let libsecret_available = rustconn_core::which::is_available("secret-tool");
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
#[cfg(feature = "secret-management")]
fn parse_backend(b: &str) -> Result<rustconn_core::config::SecretBackendType, CliError> {
    use rustconn_core::config::SecretBackendType;
    match b.to_lowercase().as_str() {
        "keyring" | "libsecret" => Ok(SecretBackendType::LibSecret),
        "keepass" | "kdbx" | "keepassxc" => Ok(SecretBackendType::KdbxFile),
        "bitwarden" | "bw" => Ok(SecretBackendType::Bitwarden),
        "1password" | "onepassword" | "op" => Ok(SecretBackendType::OnePassword),
        "passbolt" => Ok(SecretBackendType::Passbolt),
        "pass" => Ok(SecretBackendType::Pass),
        // Both file backends already have handlers in get/set/delete; until now
        // there was simply no spelling that reached them.
        "encrypted-file" | "encrypted_file" | "file" => Ok(SecretBackendType::EncryptedFile),
        "portable" | "portable-file" | "portable_file" => {
            Ok(SecretBackendType::PortableEncryptedFile)
        }
        _ => Err(CliError::Secret(format!(
            "Unknown backend: {b}. Use: keyring, keepass, bitwarden, \
             1password, passbolt, pass, encrypted-file, or portable"
        ))),
    }
}

#[cfg(feature = "secret-management")]
#[expect(
    clippy::too_many_lines,
    reason = "get handler dispatches across every backend kind with backend-specific error \
              translation; splitting per backend would duplicate the connection lookup"
)]
fn cmd_secret_get(
    config_path: Option<&Path>,
    connection_name: &str,
    backend: Option<&str>,
) -> Result<(), CliError> {
    use rustconn_core::config::SecretBackendType;
    use rustconn_core::models::Credentials;
    use rustconn_core::secret::{KeePassHierarchy, KeePassStatus};

    let config_manager = create_config_manager(config_path)?;

    let connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let groups = config_manager
        .load_groups()
        .map_err(|e| CliError::Config(format!("Failed to load groups: {e}")))?;

    let connection = find_connection(&connections, connection_name)?;
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
    let keys = vault_keys(connection, &groups, backend_type);

    match backend_type {
        SecretBackendType::LibSecret | SecretBackendType::MacOsKeychain => {
            let rt = secret_runtime()?;

            // macOS routes the keyring backend to the system Keychain;
            // LibSecretBackend is not compiled there (R10.1, R10.2).
            #[cfg(target_os = "macos")]
            let backend = rustconn_core::secret::MacOsKeychainBackend::new();
            #[cfg(not(target_os = "macos"))]
            let backend = rustconn_core::secret::LibSecretBackend::new("rustconn");
            let result: Result<Option<Credentials>, _> =
                rt.block_on(retrieve_connection_credentials(&backend, &keys, connection));

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

            let mut result = KeePassStatus::get_password_from_kdbx_with_key(
                std::path::Path::new(kdbx_path),
                settings.secrets.kdbx_password.as_ref(),
                key_file,
                &keepass_key,
                Some(connection.protocol.as_str()),
                settings.secrets.kdbx_yubikey_slot.as_deref(),
            );

            // Root-search fallback, mirroring the GUI resolver
            // (`resolve_credentials_blocking_once`): when the user enabled
            // "Search from vault root" and the scoped `RustConn/…` lookup missed,
            // widen to a whole-database search keyed by the connection's name and
            // host as well as the scoped key (issue #353). Read-widening only; it
            // never writes and never prepends `RustConn/`. Keeping `secret get`
            // in step with the resolver means the CLI can diagnose exactly what a
            // connect would find.
            if settings.secrets.kdbx_root_search && matches!(result, Ok(None)) {
                result = KeePassStatus::get_password_from_kdbx_root(
                    std::path::Path::new(kdbx_path),
                    settings.secrets.kdbx_password.as_ref(),
                    key_file,
                    &[
                        keepass_key.as_str(),
                        connection.host.as_str(),
                        connection.name.as_str(),
                    ],
                    settings.secrets.kdbx_yubikey_slot.as_deref(),
                );
            }

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

            let rt = secret_runtime()?;

            let backend = BitwardenBackend::new().with_settings_toggles(&settings.secrets);
            let result: Result<Option<Credentials>, _> =
                rt.block_on(retrieve_connection_credentials(&backend, &keys, connection));

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

            let rt = secret_runtime()?;

            // Token plus the read-only / root-search toggles from settings.
            let backend = OnePasswordBackend::from_secret_settings(&settings.secrets);
            let result: Result<Option<Credentials>, _> =
                rt.block_on(retrieve_connection_credentials(&backend, &keys, connection));

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

            let rt = secret_runtime()?;

            // Server, passphrase and the read-only / root-search toggles.
            let backend = PassboltBackend::from_secret_settings(&settings.secrets);
            let result: Result<Option<Credentials>, _> =
                rt.block_on(retrieve_connection_credentials(&backend, &keys, connection));

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
            let rt = secret_runtime()?;

            let backend = create_pass_backend(&settings);
            let result: Result<Option<Credentials>, _> =
                rt.block_on(retrieve_connection_credentials(&backend, &keys, connection));

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
        SecretBackendType::EncryptedFile => {
            use rustconn_core::secret::EncryptedFileBackend;

            let rt = secret_runtime()?;

            let backend = EncryptedFileBackend::new();
            let result: Result<Option<Credentials>, _> =
                rt.block_on(retrieve_connection_credentials(&backend, &keys, connection));

            match result {
                Ok(Some(creds)) => {
                    println!("Connection: {}", connection.name);
                    if let Some(ref user) = creds.username {
                        println!("Username:   {user}");
                    }
                    if creds.expose_password().is_some() {
                        println!(
                            "Password:   ******** \
                             (stored in encrypted file)"
                        );
                    } else {
                        println!("Password:   (not set)");
                    }
                    Ok(())
                }
                Ok(None) => Err(CliError::Secret(format!(
                    "No credentials found in encrypted file for '{}'",
                    connection.name
                ))),
                Err(e) => Err(CliError::Secret(format!("Encrypted file error: {e}"))),
            }
        }
        SecretBackendType::PortableEncryptedFile => {
            let backend = open_portable_backend(&settings.secrets)?;
            let rt = secret_runtime()?;
            let result: Result<Option<Credentials>, _> =
                rt.block_on(retrieve_connection_credentials(&backend, &keys, connection));

            match result {
                Ok(Some(creds)) => {
                    println!("Connection: {}", connection.name);
                    if let Some(ref user) = creds.username {
                        println!("Username:   {user}");
                    }
                    if creds.expose_password().is_some() {
                        println!(
                            "Password:   ******** \
                             (stored in portable encrypted file)"
                        );
                    } else {
                        println!("Password:   (not set)");
                    }
                    Ok(())
                }
                Ok(None) => Err(CliError::Secret(format!(
                    "No credentials found in the portable encrypted file for '{}'",
                    connection.name
                ))),
                Err(e) => Err(CliError::Secret(format!("Portable file error: {e}"))),
            }
        }
    }
}

/// Stores a connection's credential in the resolved backend, under the GUI's
/// vault key, and prints the one-line confirmation.
///
/// This is the single store path shared by `secret set` and `add
/// --password-stdin`, so a password provisioned at `add` time lands exactly
/// where the GUI's resolver and a later `secret get` look for it. The
/// `password` is consumed (moved into the backend's `Credentials`) so it is not
/// copied past this call.
///
/// # Errors
///
/// Returns [`CliError::Secret`] when the backend is unreachable, not configured
/// (KeePass without a database), or rejects the write.
#[cfg(feature = "secret-management")]
#[expect(
    clippy::too_many_lines,
    reason = "store dispatches across every backend kind with backend-specific storage \
              paths and construction; splitting per backend would only relocate the arms"
)]
pub(super) fn store_connection_credentials_in_backend(
    settings: &rustconn_core::config::AppSettings,
    backend_type: rustconn_core::config::SecretBackendType,
    keys: &VaultKeys,
    keepass_key: &str,
    connection: &rustconn_core::models::Connection,
    username_value: &str,
    password_value: secrecy::SecretString,
) -> Result<(), CliError> {
    use rustconn_core::config::SecretBackendType;
    use rustconn_core::secret::KeePassStatus;

    match backend_type {
        SecretBackendType::LibSecret | SecretBackendType::MacOsKeychain => {
            use rustconn_core::models::Credentials;
            use rustconn_core::secret::SecretBackend;

            let rt = secret_runtime()?;

            // macOS routes the keyring backend to the system Keychain;
            // LibSecretBackend is not compiled there (R10.1, R10.2).
            #[cfg(target_os = "macos")]
            let backend = rustconn_core::secret::MacOsKeychainBackend::new();
            #[cfg(not(target_os = "macos"))]
            let backend = rustconn_core::secret::LibSecretBackend::new("rustconn");
            let creds = Credentials {
                username: Some(username_value.to_string()),
                password: Some(password_value),
                key_passphrase: None,
                domain: connection.domain.clone(),
            };

            rt.block_on(backend.store(&keys.primary, &creds))
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
                settings.secrets.kdbx_password.as_ref(),
                key_file,
                keepass_key,
                username_value,
                &password_value,
                Some(&format!(
                    "{}://{}:{}",
                    connection.protocol.as_str().to_lowercase(),
                    connection.host,
                    connection.port
                )),
                settings.secrets.kdbx_yubikey_slot.as_deref(),
                settings.secrets.kdbx_read_only,
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

            let rt = secret_runtime()?;

            let backend = BitwardenBackend::new().with_settings_toggles(&settings.secrets);
            let creds = Credentials {
                username: Some(username_value.to_string()),
                password: Some(password_value),
                key_passphrase: None,
                domain: connection.domain.clone(),
            };

            rt.block_on(backend.store(&keys.primary, &creds))
                .map_err(|e| CliError::Secret(format!("Bitwarden error: {e}")))?;

            println!(
                "Stored credentials for '{}' in Bitwarden (user: {})",
                connection.name, username_value
            );
            Ok(())
        }
        SecretBackendType::OnePassword => {
            use rustconn_core::models::Credentials;
            use rustconn_core::secret::{OnePasswordBackend, SecretBackend};

            let rt = secret_runtime()?;

            // Token plus the read-only / root-search toggles from settings.
            let backend = OnePasswordBackend::from_secret_settings(&settings.secrets);
            let creds = Credentials {
                username: Some(username_value.to_string()),
                password: Some(password_value),
                key_passphrase: None,
                domain: connection.domain.clone(),
            };

            rt.block_on(backend.store(&keys.primary, &creds))
                .map_err(|e| CliError::Secret(format!("1Password error: {e}")))?;

            println!(
                "Stored credentials for '{}' in 1Password (user: {})",
                connection.name, username_value
            );
            Ok(())
        }
        SecretBackendType::Passbolt => {
            use rustconn_core::models::Credentials;
            use rustconn_core::secret::{PassboltBackend, SecretBackend};

            let rt = secret_runtime()?;

            // Server, passphrase and the read-only / root-search toggles.
            let backend = PassboltBackend::from_secret_settings(&settings.secrets);
            let creds = Credentials {
                username: Some(username_value.to_string()),
                password: Some(password_value),
                key_passphrase: None,
                domain: connection.domain.clone(),
            };

            rt.block_on(backend.store(&keys.primary, &creds))
                .map_err(|e| CliError::Secret(format!("Passbolt error: {e}")))?;

            println!(
                "Stored credentials for '{}' in Passbolt (user: {})",
                connection.name, username_value
            );
            Ok(())
        }
        SecretBackendType::Pass => {
            use rustconn_core::models::Credentials;
            use rustconn_core::secret::SecretBackend;

            let rt = secret_runtime()?;

            let backend = create_pass_backend(settings);
            let creds = Credentials {
                username: Some(username_value.to_string()),
                password: Some(password_value),
                key_passphrase: None,
                domain: connection.domain.clone(),
            };

            rt.block_on(backend.store(&keys.primary, &creds))
                .map_err(|e| CliError::Secret(format!("Pass error: {e}")))?;

            println!(
                "Stored credentials for '{}' in Pass (user: {})",
                connection.name, username_value
            );
            Ok(())
        }
        SecretBackendType::EncryptedFile => {
            use rustconn_core::models::Credentials;
            use rustconn_core::secret::{EncryptedFileBackend, SecretBackend};

            let rt = secret_runtime()?;

            let backend = EncryptedFileBackend::new();
            let creds = Credentials {
                username: Some(username_value.to_string()),
                password: Some(password_value),
                key_passphrase: None,
                domain: connection.domain.clone(),
            };

            rt.block_on(backend.store(&keys.primary, &creds))
                .map_err(|e| CliError::Secret(format!("Encrypted file error: {e}")))?;

            println!(
                "Stored credentials for '{}' in encrypted file (user: {})",
                connection.name, username_value
            );
            Ok(())
        }
        SecretBackendType::PortableEncryptedFile => {
            use rustconn_core::models::Credentials;
            use rustconn_core::secret::SecretBackend;

            let rt = secret_runtime()?;

            let backend = open_portable_backend(&settings.secrets)?;
            let creds = Credentials {
                username: Some(username_value.to_string()),
                password: Some(password_value),
                key_passphrase: None,
                domain: connection.domain.clone(),
            };

            rt.block_on(backend.store(&keys.primary, &creds))
                .map_err(|e| CliError::Secret(format!("Portable file error: {e}")))?;

            println!(
                "Stored credentials for '{}' in the portable encrypted file (user: {})",
                connection.name, username_value
            );
            Ok(())
        }
    }
}

/// Promotes a connection's `password_source` to `Vault` when it is still `None`.
///
/// Storing a vault secret for a connection whose source is `None` is pointless:
/// a connect never reads the vault, so the credential just saved would never be
/// used (found while testing issue #353). Only `None` is promoted — an explicit
/// `Prompt`, `Variable`, `Inherit` or `Script` is the user's choice and is left
/// untouched. Shared by `secret set` and `add --password-stdin`.
///
/// # Errors
///
/// Returns [`CliError::Config`] when the connection list cannot be re-saved.
#[cfg(feature = "secret-management")]
pub(super) fn promote_password_source_to_vault(
    config_manager: &rustconn_core::config::ConfigManager,
    connection_id: uuid::Uuid,
) -> Result<bool, CliError> {
    let mut connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;
    if let Some(conn) = connections.iter_mut().find(|c| c.id == connection_id)
        && conn.password_source == rustconn_core::models::PasswordSource::None
    {
        conn.password_source = rustconn_core::models::PasswordSource::Vault;
        config_manager
            .save_connections(&connections)
            .map_err(|e| CliError::Config(format!("Failed to save connection: {e}")))?;
        return Ok(true);
    }
    Ok(false)
}

/// Resolves the vault key material (GUI primary + legacy keys, and the KeePass
/// entry key) for a connection, so `add --password-stdin` stores under exactly
/// the keys `secret set` and the GUI use.
#[cfg(feature = "secret-management")]
pub(super) fn vault_keys_and_keepass_key(
    connection: &rustconn_core::models::Connection,
    groups: &[rustconn_core::models::ConnectionGroup],
    backend_type: rustconn_core::config::SecretBackendType,
) -> (VaultKeys, String) {
    use rustconn_core::secret::KeePassHierarchy;
    let keepass_base = KeePassHierarchy::build_entry_path(connection, groups);
    let keepass_key = format!(
        "{} ({})",
        keepass_base
            .strip_prefix("RustConn/")
            .unwrap_or(&keepass_base),
        connection.protocol.as_str().to_lowercase()
    );
    (vault_keys(connection, groups, backend_type), keepass_key)
}

/// Parses an optional backend name, defaulting to the preferred backend.
///
/// # Errors
///
/// Returns [`CliError::Secret`] when the name is not a known backend.
#[cfg(feature = "secret-management")]
pub(super) fn resolve_backend_type(
    backend: Option<&str>,
    settings: &rustconn_core::config::AppSettings,
) -> Result<rustconn_core::config::SecretBackendType, CliError> {
    Ok(backend
        .map(parse_backend)
        .transpose()?
        .unwrap_or(settings.secrets.preferred_backend))
}

#[cfg(feature = "secret-management")]
fn cmd_secret_set(
    config_path: Option<&Path>,
    connection_name: &str,
    username: Option<&str>,
    password_arg: Option<String>,
    password_stdin: bool,
    backend: Option<&str>,
) -> Result<(), CliError> {
    use rustconn_core::secret::KeePassHierarchy;
    use zeroize::Zeroizing;

    if password_arg.is_some() {
        eprintln!(
            "Warning: --password is deprecated and insecure (visible in \
             /proc/cmdline). Use --password-stdin or interactive prompt instead."
        );
    }

    // Wrap the argv password in Zeroizing immediately to minimize plain-heap lifetime.
    let password_zeroizing: Option<Zeroizing<String>> = password_arg.map(Zeroizing::new);

    let config_manager = create_config_manager(config_path)?;

    let connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let groups = config_manager
        .load_groups()
        .map_err(|e| CliError::Config(format!("Failed to load groups: {e}")))?;

    let connection = find_connection(&connections, connection_name)?;
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
    let keys = vault_keys(connection, &groups, backend_type);
    warn_shared_key(
        backend_type,
        &keys.primary,
        &connections_sharing_key(
            &keys.primary,
            connection,
            &connections,
            &groups,
            backend_type,
        ),
    );

    let password_value = if let Some(pwd) = password_zeroizing {
        secrecy::SecretString::from(pwd.as_str())
    } else if password_stdin {
        use std::io::BufRead;
        let stdin = std::io::stdin();
        let line = Zeroizing::new(
            stdin
                .lock()
                .lines()
                .next()
                .ok_or_else(|| CliError::Secret("No input on stdin".to_string()))?
                .map_err(|e| CliError::Secret(format!("Failed to read stdin: {e}")))?,
        );
        secrecy::SecretString::from(line.as_str())
    } else {
        eprint!("Enter password for '{}': ", connection.name);
        let prompted = Zeroizing::new(
            rpassword::read_password()
                .map_err(|e| CliError::Secret(format!("Failed to read password: {e}")))?,
        );
        secrecy::SecretString::from(prompted.as_str())
    };

    let username_value = username
        .map(String::from)
        .or_else(|| connection.username.clone())
        .unwrap_or_default();

    // Needed after the store to mark the connection as vault-backed.
    let connection_id = connection.id;

    store_connection_credentials_in_backend(
        &settings,
        backend_type,
        &keys,
        &keepass_key,
        connection,
        &username_value,
        password_value,
    )?;

    if promote_password_source_to_vault(&config_manager, connection_id)? {
        println!("Set password source to Vault for this connection.");
    }

    Ok(())
}

#[cfg(feature = "secret-management")]
#[expect(
    clippy::too_many_lines,
    reason = "delete handler dispatches across every backend kind; splitting per backend \
              would duplicate the connection lookup and confirmation prompt"
)]
fn cmd_secret_delete(
    config_path: Option<&Path>,
    connection_name: &str,
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
    let keys = vault_keys(connection, &groups, backend_type);
    warn_shared_key(
        backend_type,
        &keys.primary,
        &connections_sharing_key(
            &keys.primary,
            connection,
            &connections,
            &groups,
            backend_type,
        ),
    );

    match backend_type {
        SecretBackendType::LibSecret | SecretBackendType::MacOsKeychain => {
            let rt = secret_runtime()?;

            // macOS routes the keyring backend to the system Keychain;
            // LibSecretBackend is not compiled there (R10.1, R10.2).
            #[cfg(target_os = "macos")]
            let backend = rustconn_core::secret::MacOsKeychainBackend::new();
            #[cfg(not(target_os = "macos"))]
            let backend = rustconn_core::secret::LibSecretBackend::new("rustconn");
            rt.block_on(delete_connection_credentials(&backend, &keys))
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
                settings.secrets.kdbx_password.as_ref(),
                key_file,
                &keepass_entry_path,
                settings.secrets.kdbx_yubikey_slot.as_deref(),
                settings.secrets.kdbx_read_only,
            )
            .map_err(|e| CliError::Secret(format!("KeePass error: {e}")))?;

            println!("Deleted credentials for '{}' from KeePass", connection.name);
            Ok(())
        }
        SecretBackendType::Bitwarden => {
            use rustconn_core::secret::BitwardenBackend;

            let rt = secret_runtime()?;

            let backend = BitwardenBackend::new().with_settings_toggles(&settings.secrets);
            rt.block_on(delete_connection_credentials(&backend, &keys))
                .map_err(|e| CliError::Secret(format!("Bitwarden error: {e}")))?;

            println!(
                "Deleted credentials for '{}' from Bitwarden",
                connection.name
            );
            Ok(())
        }
        SecretBackendType::OnePassword => {
            use rustconn_core::secret::OnePasswordBackend;

            let rt = secret_runtime()?;

            // Token plus the read-only / root-search toggles from settings.
            let backend = OnePasswordBackend::from_secret_settings(&settings.secrets);
            rt.block_on(delete_connection_credentials(&backend, &keys))
                .map_err(|e| CliError::Secret(format!("1Password error: {e}")))?;

            println!(
                "Deleted credentials for '{}' from 1Password",
                connection.name
            );
            Ok(())
        }
        SecretBackendType::Passbolt => {
            use rustconn_core::secret::PassboltBackend;

            let rt = secret_runtime()?;

            // Server, passphrase and the read-only / root-search toggles.
            let backend = PassboltBackend::from_secret_settings(&settings.secrets);
            rt.block_on(delete_connection_credentials(&backend, &keys))
                .map_err(|e| CliError::Secret(format!("Passbolt error: {e}")))?;

            println!(
                "Deleted credentials for '{}' from Passbolt",
                connection.name
            );
            Ok(())
        }
        SecretBackendType::Pass => {
            let rt = secret_runtime()?;

            let backend = create_pass_backend(&settings);
            rt.block_on(delete_connection_credentials(&backend, &keys))
                .map_err(|e| CliError::Secret(format!("Pass error: {e}")))?;

            println!("Deleted credentials for '{}' from Pass", connection.name);
            Ok(())
        }
        SecretBackendType::EncryptedFile => {
            use rustconn_core::secret::EncryptedFileBackend;

            let rt = secret_runtime()?;

            let backend = EncryptedFileBackend::new();
            rt.block_on(delete_connection_credentials(&backend, &keys))
                .map_err(|e| CliError::Secret(format!("Encrypted file error: {e}")))?;

            println!(
                "Deleted credentials for '{}' from encrypted file",
                connection.name
            );
            Ok(())
        }
        SecretBackendType::PortableEncryptedFile => {
            let backend = open_portable_backend(&settings.secrets)?;
            let rt = secret_runtime()?;
            rt.block_on(delete_connection_credentials(&backend, &keys))
                .map_err(|e| CliError::Secret(format!("Portable file error: {e}")))?;

            println!(
                "Deleted credentials for '{}' from the portable encrypted file",
                connection.name
            );
            Ok(())
        }
    }
}

/// Builds an unlocked portable backend from settings, prompting if needed.
///
/// Resolution order mirrors what the GUI does at startup: the machine-local
/// encrypted copy first, then the system keyring, then an interactive prompt.
/// The passphrase is verified against the store before the caller uses it, so a
/// typo fails with "incorrect passphrase" instead of creating a second key
/// inside a file that already has one.
///
/// # Errors
/// Returns [`CliError::Secret`] if no passphrase can be obtained (for example a
/// non-interactive shell with nothing persisted) or if it does not open the file.
#[cfg(feature = "secret-management")]
fn open_portable_backend(
    settings: &rustconn_core::config::SecretSettings,
) -> Result<rustconn_core::secret::PortableEncryptedFileBackend, CliError> {
    use rustconn_core::secret::{
        PortableEncryptedFileBackend, resolve_portable_store_path, verify_portable_passphrase,
    };
    use zeroize::Zeroizing;

    let path = resolve_portable_store_path(settings.portable_file_path.as_deref());

    // Where the passphrase came from decides what a rejection means. One the user
    // just typed is a typo; one restored from disk is a stale saved copy, and
    // telling that user their passphrase is "incorrect" names something they never
    // entered and offers nothing to do about it.
    let mut passphrase_was_restored = true;
    let passphrase = if let Some(ref existing) = settings.portable_passphrase {
        existing.clone()
    } else if let Some(restored) = restore_portable_passphrase(settings) {
        restored
    } else {
        passphrase_was_restored = false;
        if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            return Err(CliError::Secret(
                "The portable credential file needs a passphrase and there is no terminal \
                 to ask on. Store it with \"Save passphrase\" in Settings ▸ Secrets, or run \
                 this command interactively."
                    .to_string(),
            ));
        }
        eprint!("Enter passphrase for '{}': ", path.display());
        let prompted = Zeroizing::new(
            rpassword::read_password()
                .map_err(|e| CliError::Secret(format!("Failed to read passphrase: {e}")))?,
        );

        // Creating a new store means whatever was typed becomes the passphrase,
        // with nothing to check it against. A typo there is unrecoverable: the
        // file is written, the intended passphrase never opens it, and there is
        // no recovery path. The GUI guards this with a confirmation entry; the
        // CLI has to ask twice for the same reason.
        if !path.exists() {
            eprintln!(
                "'{}' does not exist yet and will be created. There is no way to recover \
                 this passphrase if it is lost.",
                path.display()
            );
            eprint!("Confirm passphrase: ");
            let confirm = Zeroizing::new(
                rpassword::read_password()
                    .map_err(|e| CliError::Secret(format!("Failed to read passphrase: {e}")))?,
            );
            if confirm.as_str() != prompted.as_str() {
                return Err(CliError::Secret(
                    "The two passphrases do not match; nothing was written.".to_string(),
                ));
            }
        }

        secrecy::SecretString::from(prompted.as_str())
    };

    verify_portable_passphrase(&path, &passphrase).map_err(|e| {
        // The remembered copy is only rewritten when Settings is saved, so a
        // passphrase changed in the GUI and not yet saved lands here on every
        // machine that remembered the old one. That is the common cause, and
        // "incorrect passphrase" describes it from the file's point of view
        // rather than the user's.
        if passphrase_was_restored
            && matches!(e, rustconn_core::error::SecretError::IncorrectPassphrase)
        {
            return CliError::Secret(format!(
                "The saved passphrase no longer opens '{}'. If it was changed in \
                 Settings ▸ Secrets ▸ Change Passphrase, save the settings there so the \
                 copy kept on this computer is updated too.",
                path.display()
            ));
        }
        CliError::Secret(format!("Cannot open portable credential file: {e}"))
    })?;

    let backend = PortableEncryptedFileBackend::with_path(path);
    backend.set_passphrase(passphrase);
    Ok(backend)
}

/// Recovers a stored portable passphrase without prompting.
///
/// Tries the machine-local encrypted copy, then the system keyring under the
/// same 5-second ceiling the GUI uses, so an unresponsive Secret Service falls
/// through to the prompt instead of hanging the command.
#[cfg(feature = "secret-management")]
fn restore_portable_passphrase(
    settings: &rustconn_core::config::SecretSettings,
) -> Option<secrecy::SecretString> {
    let mut probe = settings.clone();
    if probe.decrypt_portable_passphrase() {
        return probe.portable_passphrase;
    }
    if !settings.portable_save_to_keyring {
        return None;
    }

    let rt = secret_runtime().ok()?;
    match rt.block_on(async {
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            rustconn_core::secret::get_portable_passphrase_from_keyring(),
        )
        .await
    }) {
        Ok(Ok(found)) => found,
        Ok(Err(e)) => {
            tracing::debug!(error = %e, "Portable passphrase lookup failed");
            None
        }
        Err(_elapsed) => {
            tracing::warn!("Keyring query for the portable passphrase timed out after 5s");
            None
        }
    }
}

fn cmd_secret_verify_keepass(
    _config_path: Option<&Path>,
    database: &Path,
    key_file: Option<&Path>,
    yubikey: Option<&str>,
) -> Result<(), CliError> {
    use rustconn_core::secret::KeePassStatus;

    KeePassStatus::validate_kdbx_path(database)
        .map_err(|e| CliError::Secret(format!("Invalid database: {e}")))?;

    if yubikey.is_some() {
        println!("Touch your YubiKey when it blinks…");
    }

    // A YubiKey slot alone (no key file) still authenticates the database, so a
    // key-file-less run is only password-only when there is also no YubiKey.
    if let Some(kf) = key_file {
        if !kf.exists() {
            return Err(CliError::Secret(format!(
                "Key file not found: {}",
                kf.display()
            )));
        }

        KeePassStatus::verify_kdbx_credentials(database, None, Some(kf), yubikey)
            .map_err(|e| CliError::Secret(format!("Verification failed: {e}")))?;

        println!(
            "✓ KeePass database verified successfully \
             (using key file)"
        );
        println!("  Database: {}", database.display());
        println!("  Key file: {}", kf.display());
    } else if yubikey.is_some() {
        // YubiKey-only (no key file): try without a password first; the slot is
        // the second (or only) factor.
        KeePassStatus::verify_kdbx_credentials(database, None, None, yubikey)
            .map_err(|e| CliError::Secret(format!("Verification failed: {e}")))?;

        println!("✓ KeePass database verified successfully (using YubiKey)");
        println!("  Database: {}", database.display());
    } else {
        eprint!("Enter database password: ");
        let password = rpassword::read_password()
            .map_err(|e| CliError::Secret(format!("Failed to read password: {e}")))?;
        let password = secrecy::SecretString::from(password);

        KeePassStatus::verify_kdbx_credentials(database, Some(&password), None, yubikey)
            .map_err(|e| CliError::Secret(format!("Verification failed: {e}")))?;

        println!("✓ KeePass database verified successfully");
        println!("  Database: {}", database.display());
    }

    Ok(())
}
#[cfg(all(test, feature = "secret-management"))]
mod vault_key_tests {
    use rustconn_core::config::SecretBackendType;
    use rustconn_core::models::{Connection, ConnectionGroup};

    use super::{VaultKeys, connections_sharing_key, vault_keys};

    /// The reporter's connection from issue #353: `hanfile03` in groups
    /// `AME/DE/11`.
    fn grouped_rdp() -> (Connection, Vec<ConnectionGroup>) {
        let ame = ConnectionGroup::new("AME".to_string());
        let de = ConnectionGroup::with_parent("DE".to_string(), ame.id);
        let eleven = ConnectionGroup::with_parent("11".to_string(), de.id);
        let mut conn = Connection::new_rdp(
            "hanfile03".to_string(),
            "hanfile03.aag.local".to_string(),
            3389,
        );
        conn.group_id = Some(eleven.id);
        (conn, vec![ame, de, eleven])
    }

    /// `secret get` reads, and `secret set` writes, under the key the GUI saves
    /// and resolves with — `rustconn/{name}` — and not under `{name} (rdp)`,
    /// which only this command ever used.
    #[test]
    fn cli_backends_use_the_guis_flat_key() {
        let (conn, groups) = grouped_rdp();
        for backend in [
            SecretBackendType::Bitwarden,
            SecretBackendType::OnePassword,
            SecretBackendType::Passbolt,
            SecretBackendType::Pass,
            SecretBackendType::EncryptedFile,
            SecretBackendType::PortableEncryptedFile,
        ] {
            assert_eq!(
                vault_keys(&conn, &groups, backend).primary,
                "rustconn/hanfile03",
                "{backend:?}"
            );
        }
    }

    /// Credentials an older release wrote stay reachable: the GUI's UUID key,
    /// and the `{name} ({protocol})` key the old `secret set` used.
    #[test]
    fn older_cli_keys_are_kept_as_legacy() {
        let (conn, groups) = grouped_rdp();
        let keys = vault_keys(&conn, &groups, SecretBackendType::Bitwarden);
        assert_eq!(
            keys,
            VaultKeys {
                primary: "rustconn/hanfile03".to_string(),
                legacy: vec![conn.id.to_string(), "hanfile03 (rdp)".to_string()],
            }
        );
    }

    /// The old command never wrote `{name} ({protocol})` to 1Password or
    /// Passbolt — it used the UUID — so `delete` must not go looking for it
    /// there.
    #[test]
    fn onepassword_and_passbolt_get_no_name_protocol_key() {
        let (conn, groups) = grouped_rdp();
        for backend in [SecretBackendType::OnePassword, SecretBackendType::Passbolt] {
            assert_eq!(
                vault_keys(&conn, &groups, backend).legacy,
                vec![conn.id.to_string()],
                "{backend:?}"
            );
        }
    }

    /// The keyring key is the one the GUI saves under (group path, no import
    /// sanitising), and the legacy list is the GUI's cleanup list.
    #[test]
    fn keyring_uses_the_guis_key_list() {
        let (conn, groups) = grouped_rdp();
        let keys = vault_keys(&conn, &groups, SecretBackendType::LibSecret);
        assert_eq!(keys.primary, "RustConn/AME/DE/11/hanfile03 (rdp)");
        assert_eq!(
            keys.legacy,
            vec![
                "hanfile03 (rdp)".to_string(),
                "rustconn/hanfile03".to_string(),
                conn.id.to_string(),
            ]
        );
    }

    /// Two connections with one name share `rustconn/{name}`, so `set`/`delete`
    /// on one must say it reaches the other. A different name does not.
    #[test]
    fn same_named_connections_are_reported_as_sharing_the_key() {
        let (conn, groups) = grouped_rdp();
        let twin = Connection::new_ssh("hanfile03".to_string(), "other".to_string(), 22);
        let other = Connection::new_ssh("web".to_string(), "web".to_string(), 22);
        let all = vec![conn.clone(), twin.clone(), other];
        let keys = vault_keys(&conn, &groups, SecretBackendType::Bitwarden);
        let sharing = connections_sharing_key(
            &keys.primary,
            &conn,
            &all,
            &groups,
            SecretBackendType::Bitwarden,
        );
        assert_eq!(sharing.len(), 1);
        assert_eq!(sharing[0].id, twin.id);
    }
}
