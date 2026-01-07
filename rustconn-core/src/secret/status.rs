//! `KeePass` integration status detection
//!
//! This module provides functionality to detect the status of `KeePass` integration,
//! including `KeePassXC` installation detection, version parsing, and KDBX file validation.

// Allow missing errors documentation - status detection functions have straightforward errors
#![allow(clippy::missing_errors_doc)]

use std::path::Path;
use std::process::Command;

/// Status of `KeePass` integration
///
/// This struct provides information about the current state of `KeePass` integration,
/// including whether `KeePassXC` is installed, its version, and KDBX file accessibility.
#[derive(Debug, Clone, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct KeePassStatus {
    /// Whether `KeePassXC` application is installed
    pub keepassxc_installed: bool,
    /// `KeePassXC` version if installed
    pub keepassxc_version: Option<String>,
    /// Path to `KeePassXC` CLI binary
    pub keepassxc_path: Option<std::path::PathBuf>,
    /// Whether KDBX file is configured
    pub kdbx_configured: bool,
    /// Whether KDBX file exists and is accessible
    pub kdbx_accessible: bool,
    /// Whether integration is currently active (unlocked)
    pub integration_active: bool,
}

impl KeePassStatus {
    /// Detects current `KeePass` status by checking for `KeePassXC` installation
    ///
    /// This method searches for the `keepassxc-cli` binary in common locations
    /// and attempts to determine its version.
    #[must_use]
    pub fn detect() -> Self {
        let mut status = Self::default();

        // Try to find keepassxc-cli in PATH or common locations
        if let Some(path) = Self::find_keepassxc_cli() {
            status.keepassxc_installed = true;
            status.keepassxc_path = Some(path.clone());

            // Try to get version
            if let Some(version) = Self::get_keepassxc_version(&path) {
                status.keepassxc_version = Some(version);
            }
        }

        status
    }

    /// Detects status with a configured KDBX path
    ///
    /// # Arguments
    /// * `kdbx_path` - Optional path to the KDBX database file
    #[must_use]
    pub fn detect_with_kdbx(kdbx_path: Option<&Path>) -> Self {
        let mut status = Self::detect();

        if let Some(path) = kdbx_path {
            status.kdbx_configured = true;
            status.kdbx_accessible = path.exists() && path.is_file();
        }

        status
    }

    /// Validates a KDBX file path
    ///
    /// # Arguments
    /// * `path` - Path to validate
    ///
    /// # Returns
    /// * `Ok(())` if the path is valid (ends with .kdbx and file exists)
    /// * `Err(String)` with a description of the validation failure
    ///
    /// # Errors
    /// Returns an error if:
    /// - The path does not have a .kdbx extension (case-insensitive)
    /// - The file does not exist
    /// - The path points to a directory instead of a file
    pub fn validate_kdbx_path(path: &Path) -> Result<(), String> {
        // Check extension (case-insensitive)
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_lowercase);

        if extension.as_deref() != Some("kdbx") {
            return Err("File must have .kdbx extension".to_string());
        }

        // Check if file exists
        if !path.exists() {
            return Err(format!("File does not exist: {}", path.display()));
        }

        // Check if it's a file (not a directory)
        if !path.is_file() {
            return Err(format!("Path is not a file: {}", path.display()));
        }

        Ok(())
    }

    /// Finds the `keepassxc-cli` binary
    ///
    /// Searches in PATH and common installation locations.
    fn find_keepassxc_cli() -> Option<std::path::PathBuf> {
        // First, try to find in PATH using `which`
        if let Ok(output) = Command::new("which").arg("keepassxc-cli").output() {
            if output.status.success() {
                let path_str = String::from_utf8_lossy(&output.stdout);
                let path = std::path::PathBuf::from(path_str.trim());
                if path.exists() {
                    return Some(path);
                }
            }
        }

        // Check common installation paths
        let common_paths = [
            "/usr/bin/keepassxc-cli",
            "/usr/local/bin/keepassxc-cli",
            "/snap/bin/keepassxc-cli",
            "/var/lib/flatpak/exports/bin/org.keepassxc.KeePassXC.cli",
        ];

        for path_str in &common_paths {
            let path = std::path::PathBuf::from(path_str);
            if path.exists() {
                return Some(path);
            }
        }

        None
    }

    /// Gets the `KeePassXC` version from the CLI
    ///
    /// # Arguments
    /// * `cli_path` - Path to the `keepassxc-cli` binary
    fn get_keepassxc_version(cli_path: &Path) -> Option<String> {
        let output = Command::new(cli_path).arg("--version").output().ok()?;

        if output.status.success() {
            let version_output = String::from_utf8_lossy(&output.stdout);
            parse_keepassxc_version(&version_output)
        } else {
            // Some versions output to stderr
            let version_output = String::from_utf8_lossy(&output.stderr);
            parse_keepassxc_version(&version_output)
        }
    }

    /// Retrieves a password from KDBX database using `keepassxc-cli`
    ///
    /// # Arguments
    /// * `kdbx_path` - Path to the KDBX database file
    /// * `db_password` - Password to unlock the database
    /// * `entry_name` - Name of the entry to look up (connection name or host)
    ///
    /// # Returns
    /// * `Ok(Some(String))` if the password is found
    /// * `Ok(None)` if the entry is not found
    /// * `Err(String)` with error description if retrieval fails
    ///
    /// # Errors
    /// Returns an error if:
    /// - `keepassxc-cli` is not installed
    /// - The KDBX file path is invalid
    /// - The database password is incorrect
    pub fn get_password_from_kdbx(
        kdbx_path: &Path,
        db_password: &str,
        entry_name: &str,
    ) -> Result<Option<String>, String> {
        use std::io::Write as IoWrite;
        use std::process::Stdio;

        // First validate the path
        Self::validate_kdbx_path(kdbx_path)?;

        // Find keepassxc-cli
        let cli_path = Self::find_keepassxc_cli()
            .ok_or_else(|| "keepassxc-cli not found. Please install KeePassXC.".to_string())?;

        // Use keepassxc-cli show command to get the password
        // Format: keepassxc-cli show -s <database> <entry>
        let mut child = Command::new(&cli_path)
            .arg("show")
            .arg("-s") // Show password attribute
            .arg("-a")
            .arg("Password") // Get password attribute
            .arg(kdbx_path)
            .arg(entry_name)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to run keepassxc-cli: {e}"))?;

        // Write database password to stdin
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(db_password.as_bytes())
                .map_err(|e| format!("Failed to send password: {e}"))?;
            stdin
                .write_all(b"\n")
                .map_err(|e| format!("Failed to send password: {e}"))?;
        }

        let output = child
            .wait_with_output()
            .map_err(|e| format!("Failed to wait for keepassxc-cli: {e}"))?;

        if output.status.success() {
            let password = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if password.is_empty() {
                Ok(None)
            } else {
                Ok(Some(password))
            }
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("Could not find entry")
                || stderr.contains("Entry not found")
                || stderr.contains("No entry found")
            {
                Ok(None)
            } else if stderr.contains("Invalid credentials") || stderr.contains("wrong password") {
                Err("Invalid database password".to_string())
            } else {
                // Entry not found is not an error, just return None
                Ok(None)
            }
        }
    }

    /// Saves a password to KDBX database using `keepassxc-cli`
    ///
    /// # Arguments
    /// * `kdbx_path` - Path to the KDBX database file
    /// * `db_password` - Password to unlock the database (None if using key file)
    /// * `key_file` - Optional path to key file for authentication
    /// * `entry_name` - Name of the entry (connection name or host)
    /// * `username` - Username for the entry
    /// * `password` - Password to save
    /// * `url` - Optional URL for the entry
    ///
    /// # Returns
    /// * `Ok(())` if the password is saved successfully
    /// * `Err(String)` with error description if saving fails
    ///
    /// # Errors
    /// Returns an error if:
    /// - `keepassxc-cli` is not installed
    /// - The KDBX file path is invalid
    /// - The database password/key file is incorrect
    /// - The entry cannot be created
    ///
    /// Note: Entry names include protocol suffix to allow same name for different protocols.
    /// Format: `RustConn/{entry_name} ({protocol})` where protocol is extracted from URL.
    pub fn save_password_to_kdbx(
        kdbx_path: &Path,
        db_password: Option<&str>,
        key_file: Option<&Path>,
        entry_name: &str,
        username: &str,
        password: &str,
        url: Option<&str>,
    ) -> Result<(), String> {
        use std::io::Write as IoWrite;
        use std::process::Stdio;

        // First validate the path
        Self::validate_kdbx_path(kdbx_path)?;

        // Find keepassxc-cli
        let cli_path = Self::find_keepassxc_cli()
            .ok_or_else(|| "keepassxc-cli not found. Please install KeePassXC.".to_string())?;

        // Ensure RustConn group exists
        Self::ensure_rustconn_group(kdbx_path, db_password, key_file, &cli_path)?;

        // Build the entry path under RustConn group
        // entry_name should already include protocol suffix if needed (e.g., "server (rdp)")
        let entry_path = format!("RustConn/{entry_name}");

        // First, try to remove existing entry (ignore errors if it doesn't exist)
        let _ = Self::delete_kdbx_entry(kdbx_path, db_password, key_file, &entry_path);

        // Build command arguments for keepassxc-cli add
        // Format: keepassxc-cli add [options] <database> <entry>
        // -p/--password-prompt prompts for entry password via stdin (after db password)
        let mut args = vec!["add".to_string(), "-q".to_string()];

        // If using key file without password, add --no-password flag
        if db_password.is_none() && key_file.is_some() {
            args.push("--no-password".to_string());
        }

        // Add key file if provided
        if let Some(kf) = key_file {
            args.push("--key-file".to_string());
            args.push(kf.display().to_string());
        }

        // Add username if not empty
        if !username.is_empty() {
            args.push("-u".to_string());
            args.push(username.to_string());
        }

        // Add URL if provided
        if let Some(u) = url {
            if !u.is_empty() {
                args.push("--url".to_string());
                args.push(u.to_string());
            }
        }

        // Add password prompt flag - this tells keepassxc-cli to read entry password from stdin
        args.push("-p".to_string());

        // Add database path and entry name
        args.push(kdbx_path.display().to_string());
        args.push(entry_path);

        tracing::debug!("Running keepassxc-cli with args: {args:?}");

        let mut child = Command::new(&cli_path)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to run keepassxc-cli: {e}"))?;

        // Write passwords to stdin
        // When using --no-password (key file only): only entry password is needed
        // When using password: database password first, then entry password
        if let Some(mut stdin) = child.stdin.take() {
            // Database password (only if not using --no-password)
            if let Some(db_pwd) = db_password {
                stdin
                    .write_all(db_pwd.as_bytes())
                    .map_err(|e| format!("Failed to send database password: {e}"))?;
                stdin
                    .write_all(b"\n")
                    .map_err(|e| format!("Failed to send newline: {e}"))?;
            }

            // Entry password (prompted by -p flag)
            tracing::debug!("DEBUG: Sending entry password");
            stdin
                .write_all(password.as_bytes())
                .map_err(|e| format!("Failed to send entry password: {e}"))?;
            stdin
                .write_all(b"\n")
                .map_err(|e| format!("Failed to send newline: {e}"))?;

            // Close stdin to signal end of input
            drop(stdin);
        }

        let output = child
            .wait_with_output()
            .map_err(|e| format!("Failed to wait for keepassxc-cli: {e}"))?;

        tracing::debug!(
            "keepassxc-cli exit code: {:?}, stdout: '{}', stderr: '{}'",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stderr.contains("Invalid credentials")
                || stderr.contains("wrong password")
                || stderr.contains("Error while reading the database")
            {
                Err("Invalid database password or key file".to_string())
            } else if stderr.contains("Could not find group") {
                Err("RustConn group not found in database. Please create a group named 'RustConn' in your KeePass database.".to_string())
            } else if stderr.contains("already exists") {
                // Entry already exists - try to edit it instead
                Err(format!("Entry '{entry_name}' already exists"))
            } else if stderr.is_empty() && stdout.is_empty() {
                Err(format!(
                    "Failed to save password to KeePass database (exit code: {:?}). \
                     Try running: keepassxc-cli add -p {} 'RustConn/{}'",
                    output.status.code(),
                    kdbx_path.display(),
                    entry_name
                ))
            } else {
                let error_msg = if stderr.is_empty() { stdout } else { stderr };
                Err(format!("KeePass error: {}", error_msg.trim()))
            }
        }
    }

    /// Ensures the `RustConn` group exists in the database
    fn ensure_rustconn_group(
        kdbx_path: &Path,
        db_password: Option<&str>,
        key_file: Option<&Path>,
        cli_path: &Path,
    ) -> Result<(), String> {
        use std::io::Write as IoWrite;
        use std::process::Stdio;

        tracing::debug!("Checking if RustConn group exists...");

        // First check if RustConn group exists using ls command
        let mut args = vec!["ls".to_string(), "-q".to_string()];

        // If using key file without password, add --no-password flag
        if db_password.is_none() && key_file.is_some() {
            args.push("--no-password".to_string());
        }

        if let Some(kf) = key_file {
            args.push("--key-file".to_string());
            args.push(kf.display().to_string());
        }

        args.push(kdbx_path.display().to_string());
        args.push("RustConn".to_string());

        let mut child = Command::new(cli_path)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to run keepassxc-cli: {e}"))?;

        // Only send password if we have one
        if let Some(mut stdin) = child.stdin.take() {
            if let Some(db_pwd) = db_password {
                stdin.write_all(db_pwd.as_bytes()).ok();
                stdin.write_all(b"\n").ok();
            }
        }

        let output = child.wait_with_output().ok();

        // If group exists, we're done
        if let Some(ref o) = output {
            tracing::debug!(
                "ls RustConn result: exit={:?}, stdout='{}', stderr='{}'",
                o.status.code(),
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            );
            if o.status.success() {
                tracing::debug!("RustConn group exists");
                return Ok(());
            }
        }

        tracing::debug!("RustConn group doesn't exist, creating...");

        // Group doesn't exist, create it using mkdir command
        let mut args = vec!["mkdir".to_string(), "-q".to_string()];

        // If using key file without password, add --no-password flag
        if db_password.is_none() && key_file.is_some() {
            args.push("--no-password".to_string());
        }

        if let Some(kf) = key_file {
            args.push("--key-file".to_string());
            args.push(kf.display().to_string());
        }

        args.push(kdbx_path.display().to_string());
        args.push("RustConn".to_string());

        let mut child = Command::new(cli_path)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to run keepassxc-cli mkdir: {e}"))?;

        // Only send password if we have one
        if let Some(mut stdin) = child.stdin.take() {
            if let Some(db_pwd) = db_password {
                stdin.write_all(db_pwd.as_bytes()).ok();
                stdin.write_all(b"\n").ok();
            }
        }

        let output = child
            .wait_with_output()
            .map_err(|e| format!("Failed to wait for keepassxc-cli: {e}"))?;

        tracing::debug!(
            "mkdir RustConn result: exit={:?}, stdout='{}', stderr='{}'",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        if output.status.success() {
            tracing::debug!("RustConn group created successfully");
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // If group already exists, that's fine
            if stderr.contains("already exists") {
                tracing::debug!("RustConn group already exists");
                Ok(())
            } else if stderr.contains("Invalid credentials") || stderr.contains("wrong password") {
                Err("Invalid database password or key file".to_string())
            } else {
                // Don't fail if we can't create the group - the add command will give a better error
                tracing::debug!("Failed to create group, but continuing: {stderr}");
                Ok(())
            }
        }
    }

    /// Deletes an entry from KDBX database
    fn delete_kdbx_entry(
        kdbx_path: &Path,
        db_password: Option<&str>,
        key_file: Option<&Path>,
        entry_path: &str,
    ) -> Result<(), String> {
        use std::io::Write as IoWrite;
        use std::process::Stdio;

        let cli_path =
            Self::find_keepassxc_cli().ok_or_else(|| "keepassxc-cli not found".to_string())?;

        let mut args = vec!["rm".to_string(), "-q".to_string()];

        // If using key file without password, add --no-password flag
        if db_password.is_none() && key_file.is_some() {
            args.push("--no-password".to_string());
        }

        if let Some(kf) = key_file {
            args.push("--key-file".to_string());
            args.push(kf.display().to_string());
        }

        args.push(kdbx_path.display().to_string());
        args.push(entry_path.to_string());

        let mut child = Command::new(&cli_path)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to run keepassxc-cli: {e}"))?;

        // Only send password if we have one
        if let Some(mut stdin) = child.stdin.take() {
            if let Some(db_pwd) = db_password {
                stdin.write_all(db_pwd.as_bytes()).ok();
                stdin.write_all(b"\n").ok();
            }
        }

        let _ = child.wait_with_output();
        Ok(())
    }

    /// Retrieves a password from KDBX database using `keepassxc-cli` with key file support
    ///
    /// # Arguments
    /// * `kdbx_path` - Path to the KDBX database file
    /// * `db_password` - Password to unlock the database (None if using key file only)
    /// * `key_file` - Optional path to key file for authentication
    /// * `entry_name` - Name of the entry to look up (connection name or host)
    /// * `protocol` - Optional protocol (ssh, rdp, vnc, spice) for more specific lookup
    ///
    /// # Returns
    /// * `Ok(Some(String))` if the password is found
    /// * `Ok(None)` if the entry is not found
    /// * `Err(String)` with error description if retrieval fails
    ///
    /// Note: Searches in order: `RustConn/{name}`, `RustConn/{base_name}` (without protocol suffix), `{name}`
    pub fn get_password_from_kdbx_with_key(
        kdbx_path: &Path,
        db_password: Option<&str>,
        key_file: Option<&Path>,
        entry_name: &str,
        protocol: Option<&str>,
    ) -> Result<Option<String>, String> {
        use std::io::Write as IoWrite;
        use std::process::Stdio;

        // First validate the path
        Self::validate_kdbx_path(kdbx_path)?;

        // Find keepassxc-cli
        let cli_path = Self::find_keepassxc_cli()
            .ok_or_else(|| "keepassxc-cli not found. Please install KeePassXC.".to_string())?;

        // Build list of paths to try, prioritizing exact match then legacy formats
        let mut entry_paths = Vec::new();

        // First try exact entry name (may already include protocol suffix)
        entry_paths.push(format!("RustConn/{entry_name}"));

        // If entry_name contains protocol suffix like "name (ssh)", also try without it (legacy)
        // This handles migration from old format where entries were stored without protocol
        if let Some(base_name) = entry_name
            .strip_suffix(')')
            .and_then(|s| s.rfind(" (").map(|pos| &entry_name[..pos]))
        {
            entry_paths.push(format!("RustConn/{base_name}"));
        }

        // If protocol provided separately, try with it (for backward compatibility)
        if let Some(proto) = protocol {
            entry_paths.push(format!("RustConn/{entry_name} ({proto})"));
        }

        // Finally try direct entry name without RustConn prefix
        entry_paths.push(entry_name.to_string());

        tracing::debug!(
            "get_password: entry_name='{}', protocol={:?}, has_password={}, has_key_file={}",
            entry_name,
            protocol,
            db_password.is_some(),
            key_file.is_some()
        );

        for entry_path in &entry_paths {
            let mut args = vec![
                "show".to_string(),
                "-s".to_string(),
                "-a".to_string(),
                "Password".to_string(),
            ];

            // If using key file without password, add --no-password flag
            if db_password.is_none() && key_file.is_some() {
                args.push("--no-password".to_string());
            }

            if let Some(kf) = key_file {
                args.push("--key-file".to_string());
                args.push(kf.display().to_string());
            }

            args.push(kdbx_path.display().to_string());
            args.push(entry_path.clone());

            tracing::debug!("get_password: trying path '{entry_path}'");

            let mut child = Command::new(&cli_path)
                .args(&args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| format!("Failed to run keepassxc-cli: {e}"))?;

            // Only send password if we have one (not using --no-password)
            if let Some(mut stdin) = child.stdin.take() {
                if let Some(db_pwd) = db_password {
                    stdin
                        .write_all(db_pwd.as_bytes())
                        .map_err(|e| format!("Failed to send password: {e}"))?;
                    stdin
                        .write_all(b"\n")
                        .map_err(|e| format!("Failed to send password: {e}"))?;
                }
            }

            let output = child
                .wait_with_output()
                .map_err(|e| format!("Failed to wait for keepassxc-cli: {e}"))?;

            tracing::debug!(
                "get_password: exit={:?}, stderr='{}'",
                output.status.code(),
                String::from_utf8_lossy(&output.stderr)
            );

            if output.status.success() {
                let password = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !password.is_empty() {
                    tracing::debug!("get_password: found password at '{entry_path}'");
                    return Ok(Some(password));
                }
            }
        }

        tracing::debug!("get_password: password not found");
        Ok(None)
    }

    /// Verifies a KDBX database password using `keepassxc-cli`
    ///
    /// # Arguments
    /// * `kdbx_path` - Path to the KDBX database file
    /// * `password` - Password to verify
    ///
    /// # Returns
    /// * `Ok(())` if the password is correct
    /// * `Err(String)` with error description if verification fails
    ///
    /// # Errors
    /// Returns an error if:
    /// - `keepassxc-cli` is not installed
    /// - The KDBX file path is invalid
    /// - The password is incorrect
    /// - The database cannot be opened
    pub fn verify_kdbx_password(kdbx_path: &Path, password: &str) -> Result<(), String> {
        Self::verify_kdbx_credentials(kdbx_path, Some(password), None)
    }

    /// Verifies KDBX database credentials (password and/or key file) using `keepassxc-cli`
    ///
    /// # Arguments
    /// * `kdbx_path` - Path to the KDBX database file
    /// * `password` - Password to verify (None if using key file only)
    /// * `key_file` - Optional path to key file
    ///
    /// # Returns
    /// * `Ok(())` if the credentials are correct
    /// * `Err(String)` with error description if verification fails
    pub fn verify_kdbx_credentials(
        kdbx_path: &Path,
        password: Option<&str>,
        key_file: Option<&Path>,
    ) -> Result<(), String> {
        use std::io::Write as IoWrite;
        use std::process::Stdio;

        // First validate the path
        Self::validate_kdbx_path(kdbx_path)?;

        // Find keepassxc-cli
        let cli_path = Self::find_keepassxc_cli()
            .ok_or_else(|| "keepassxc-cli not found. Please install KeePassXC.".to_string())?;

        // Build command arguments
        let mut args = vec!["ls".to_string()];

        if let Some(kf) = key_file {
            args.push("--key-file".to_string());
            args.push(kf.display().to_string());
        }

        args.push(kdbx_path.display().to_string());

        let mut child = Command::new(&cli_path)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to run keepassxc-cli: {e}"))?;

        // Write password to stdin
        if let Some(mut stdin) = child.stdin.take() {
            if let Some(pwd) = password {
                stdin
                    .write_all(pwd.as_bytes())
                    .map_err(|e| format!("Failed to send password: {e}"))?;
            }
            stdin
                .write_all(b"\n")
                .map_err(|e| format!("Failed to send password: {e}"))?;
        }

        let output = child
            .wait_with_output()
            .map_err(|e| format!("Failed to wait for keepassxc-cli: {e}"))?;

        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("Invalid credentials")
                || stderr.contains("wrong password")
                || stderr.contains("Error while reading the database")
            {
                Err("Invalid password or key file".to_string())
            } else if stderr.is_empty() {
                Err("Failed to open database. Check your credentials.".to_string())
            } else {
                Err(format!("Database error: {}", stderr.trim()))
            }
        }
    }

    /// Validates a key file path
    ///
    /// # Arguments
    /// * `path` - Path to validate
    ///
    /// # Returns
    /// * `Ok(())` if the path is valid
    /// * `Err(String)` with a description of the validation failure
    ///
    /// Note: `KeePassXC` creates key files without extension by default,
    /// so we don't require a specific extension.
    pub fn validate_key_file_path(path: &Path) -> Result<(), String> {
        // Check if file exists
        if !path.exists() {
            return Err(format!("Key file does not exist: {}", path.display()));
        }

        // Check if it's a file (not a directory)
        if !path.is_file() {
            return Err(format!("Path is not a file: {}", path.display()));
        }

        Ok(())
    }
}

/// Parses a version string from `KeePassXC` CLI output
///
/// The output format is typically: "keepassxc-cli 2.7.6"
/// or just "2.7.6" on some systems.
///
/// # Arguments
/// * `output` - The raw output from `keepassxc-cli --version`
///
/// # Returns
/// * `Some(String)` containing the version number if found
/// * `None` if no valid version could be extracted
#[must_use]
pub fn parse_keepassxc_version(output: &str) -> Option<String> {
    let output = output.trim();

    if output.is_empty() {
        return None;
    }

    // Try to find a version pattern (digits and dots)
    // Common formats:
    // - "keepassxc-cli 2.7.6"
    // - "2.7.6"
    // - "KeePassXC 2.7.6"

    // Split by whitespace and look for version-like strings
    for part in output.split_whitespace() {
        // Check if this part looks like a version (starts with digit, contains dots)
        if part.chars().next().is_some_and(|c| c.is_ascii_digit())
            && part.contains('.')
            && part.chars().all(|c| c.is_ascii_digit() || c == '.')
        {
            return Some(part.to_string());
        }
    }

    // If no version found with dots, try to find any digit sequence
    // This handles edge cases like "2" or "2.7"
    for part in output.split_whitespace() {
        if part.chars().next().is_some_and(|c| c.is_ascii_digit())
            && part.chars().all(|c| c.is_ascii_digit() || c == '.')
        {
            return Some(part.to_string());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_kdbx_path_valid_extension() {
        // Create a temp file with .kdbx extension
        let temp_dir = tempfile::tempdir().unwrap();
        let kdbx_path = temp_dir.path().join("test.kdbx");
        std::fs::write(&kdbx_path, b"dummy content").unwrap();

        assert!(KeePassStatus::validate_kdbx_path(&kdbx_path).is_ok());
    }

    #[test]
    fn test_validate_kdbx_path_uppercase_extension() {
        let temp_dir = tempfile::tempdir().unwrap();
        let kdbx_path = temp_dir.path().join("test.KDBX");
        std::fs::write(&kdbx_path, b"dummy content").unwrap();

        assert!(KeePassStatus::validate_kdbx_path(&kdbx_path).is_ok());
    }

    #[test]
    fn test_validate_kdbx_path_wrong_extension() {
        let temp_dir = tempfile::tempdir().unwrap();
        let txt_path = temp_dir.path().join("test.txt");
        std::fs::write(&txt_path, b"dummy content").unwrap();

        let result = KeePassStatus::validate_kdbx_path(&txt_path);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains(".kdbx extension"));
    }

    #[test]
    fn test_validate_kdbx_path_nonexistent() {
        let path = std::path::PathBuf::from("/nonexistent/path/test.kdbx");
        let result = KeePassStatus::validate_kdbx_path(&path);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("does not exist"));
    }

    #[test]
    fn test_validate_kdbx_path_directory() {
        let temp_dir = tempfile::tempdir().unwrap();
        // Create a directory with .kdbx name
        let dir_path = temp_dir.path().join("test.kdbx");
        std::fs::create_dir(&dir_path).unwrap();

        let result = KeePassStatus::validate_kdbx_path(&dir_path);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not a file"));
    }

    #[test]
    fn test_parse_version_standard_format() {
        assert_eq!(
            parse_keepassxc_version("keepassxc-cli 2.7.6"),
            Some("2.7.6".to_string())
        );
    }

    #[test]
    fn test_parse_version_just_number() {
        assert_eq!(parse_keepassxc_version("2.7.6"), Some("2.7.6".to_string()));
    }

    #[test]
    fn test_parse_version_with_prefix() {
        assert_eq!(
            parse_keepassxc_version("KeePassXC 2.7.6"),
            Some("2.7.6".to_string())
        );
    }

    #[test]
    fn test_parse_version_empty() {
        assert_eq!(parse_keepassxc_version(""), None);
    }

    #[test]
    fn test_parse_version_whitespace() {
        assert_eq!(parse_keepassxc_version("   "), None);
    }

    #[test]
    fn test_parse_version_no_version() {
        assert_eq!(parse_keepassxc_version("keepassxc-cli"), None);
    }

    #[test]
    fn test_parse_version_with_newline() {
        assert_eq!(
            parse_keepassxc_version("keepassxc-cli 2.7.6\n"),
            Some("2.7.6".to_string())
        );
    }

    #[test]
    fn test_default_status() {
        let status = KeePassStatus::default();
        assert!(!status.keepassxc_installed);
        assert!(status.keepassxc_version.is_none());
        assert!(status.keepassxc_path.is_none());
        assert!(!status.kdbx_configured);
        assert!(!status.kdbx_accessible);
        assert!(!status.integration_active);
    }
}
