//! Secret backend detection and version checking
//!
//! This module provides utilities for detecting installed password managers
//! and their versions, useful for UI display and backend selection.

use std::path::PathBuf;
use std::sync::LazyLock;

use regex::Regex;
use tokio::process::Command;

/// Cached regex for version parsing: matches patterns like "1.2.3" or "v1.2.3"
static VERSION_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"v?(\d+\.\d+(?:\.\d+)?)").expect("VERSION_REGEX is a valid regex pattern")
});

/// Information about an installed password manager
#[derive(Debug, Clone)]
pub struct PasswordManagerInfo {
    /// Unique identifier
    pub id: &'static str,
    /// Display name
    pub name: &'static str,
    /// Version string (if detected)
    pub version: Option<String>,
    /// Whether the manager is installed/available
    pub installed: bool,
    /// Whether it's currently running (for socket-based backends)
    pub running: bool,
    /// Path to executable or database
    pub path: Option<PathBuf>,
    /// Additional status message
    pub status_message: Option<String>,
    /// Supported formats (e.g., "KDBX 4", "Secret Service API")
    pub formats: Vec<&'static str>,
}

/// Detects all available password managers on the system
pub async fn detect_password_managers() -> Vec<PasswordManagerInfo> {
    let (keepassxc, gnome_secrets, libsecret, bitwarden, onepassword, keepass, passbolt) = tokio::join!(
        detect_keepassxc(),
        detect_gnome_secrets(),
        detect_libsecret(),
        detect_bitwarden(),
        detect_onepassword(),
        detect_keepass(),
        detect_passbolt(),
    );

    vec![
        keepassxc,
        gnome_secrets,
        libsecret,
        bitwarden,
        onepassword,
        keepass,
        passbolt,
    ]
}

/// Detects KeePassXC installation and status
pub async fn detect_keepassxc() -> PasswordManagerInfo {
    let mut info = PasswordManagerInfo {
        id: "keepassxc",
        name: "KeePassXC",
        version: None,
        installed: false,
        running: false,
        path: None,
        status_message: None,
        formats: vec!["KDBX 3", "KDBX 4"],
    };

    // Check keepassxc-cli
    if let Ok(output) = Command::new("keepassxc-cli")
        .arg("--version")
        .output()
        .await
    {
        if output.status.success() {
            let version_str = String::from_utf8_lossy(&output.stdout);
            info.version = parse_version_line(&version_str);
            info.installed = true;
        }
    }

    // Check if KeePassXC is running (socket exists)
    let socket_path = std::env::var("XDG_RUNTIME_DIR")
        .map(|dir| PathBuf::from(dir).join("kpxc_server"))
        .unwrap_or_else(|_| PathBuf::from("/tmp/kpxc_server"));

    if socket_path.exists() {
        info.running = true;
        info.status_message = Some("Browser integration active".to_string());
    } else if info.installed {
        info.status_message = Some("Not running or browser integration disabled".to_string());
    }

    // Find executable path
    if let Ok(output) = Command::new("which").arg("keepassxc").output().await {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                info.path = Some(PathBuf::from(path));
            }
        }
    }

    info
}

/// Detects GNOME Secrets (Password Safe) installation
pub async fn detect_gnome_secrets() -> PasswordManagerInfo {
    let mut info = PasswordManagerInfo {
        id: "gnome-secrets",
        name: "GNOME Secrets",
        version: None,
        installed: false,
        running: false,
        path: None,
        status_message: None,
        formats: vec!["KDBX 4"],
    };

    // Check for flatpak installation
    if let Ok(output) = Command::new("flatpak")
        .args(["info", "org.gnome.World.Secrets"])
        .output()
        .await
    {
        if output.status.success() {
            let output_str = String::from_utf8_lossy(&output.stdout);
            info.version = parse_flatpak_version(&output_str);
            info.installed = true;
            info.path = Some(PathBuf::from("flatpak:org.gnome.World.Secrets"));
        }
    }

    // Check for native installation
    if !info.installed {
        if let Ok(output) = Command::new("which").arg("gnome-secrets").output().await {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    info.installed = true;
                    info.path = Some(PathBuf::from(path));
                }
            }
        }
    }

    // Also check for old name (gnome-passwordsafe)
    if !info.installed {
        if let Ok(output) = Command::new("which")
            .arg("gnome-passwordsafe")
            .output()
            .await
        {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    info.installed = true;
                    info.path = Some(PathBuf::from(path));
                }
            }
        }
    }

    if info.installed {
        info.status_message = Some("Uses KDBX format (compatible with KeePass)".to_string());
    }

    info
}

/// Detects libsecret/secret-tool availability
pub async fn detect_libsecret() -> PasswordManagerInfo {
    let mut info = PasswordManagerInfo {
        id: "libsecret",
        name: "GNOME Keyring / KDE Wallet",
        version: None,
        installed: false,
        running: false,
        path: None,
        status_message: None,
        formats: vec!["Secret Service API"],
    };

    // Check secret-tool
    if let Ok(output) = Command::new("secret-tool").arg("--version").output().await {
        if output.status.success() {
            let version_str = String::from_utf8_lossy(&output.stdout);
            info.version = parse_version_line(&version_str);
            info.installed = true;
        }
    }

    // Check if gnome-keyring-daemon is running
    if let Ok(output) = Command::new("pgrep").arg("gnome-keyring-d").output().await {
        if output.status.success() {
            info.running = true;
            info.status_message = Some("GNOME Keyring daemon running".to_string());
        }
    }

    // Check if kwalletd is running (KDE)
    if !info.running {
        if let Ok(output) = Command::new("pgrep").arg("kwalletd").output().await {
            if output.status.success() {
                info.running = true;
                info.status_message = Some("KDE Wallet daemon running".to_string());
            }
        }
    }

    if info.installed && !info.running {
        info.status_message = Some("No keyring daemon detected".to_string());
    }

    info
}

/// Detects Bitwarden CLI installation
pub async fn detect_bitwarden() -> PasswordManagerInfo {
    let mut info = PasswordManagerInfo {
        id: "bitwarden",
        name: "Bitwarden CLI",
        version: None,
        installed: false,
        running: false,
        path: None,
        status_message: None,
        formats: vec!["Cloud or self-hosted vault"],
    };

    // Try common paths for bw CLI
    let bw_paths = ["bw", "/usr/bin/bw", "/usr/local/bin/bw", "/snap/bin/bw"];

    let home = std::env::var("HOME").unwrap_or_default();
    let extra_paths = [
        format!("{home}/.local/bin/bw"),
        format!("{home}/.npm-global/bin/bw"),
        format!("{home}/bin/bw"),
        format!("{home}/.nvm/versions/node/*/bin/bw"),
    ];

    let mut bw_cmd: Option<String> = None;

    // Try standard paths first
    for path in &bw_paths {
        if let Ok(output) = Command::new(path).arg("--version").output().await {
            if output.status.success() {
                let version_str = String::from_utf8_lossy(&output.stdout);
                info.version = Some(version_str.trim().to_string());
                info.installed = true;
                bw_cmd = Some((*path).to_string());
                break;
            }
        }
    }

    // Try home-relative paths
    if !info.installed {
        for path in &extra_paths {
            // Skip glob patterns
            if path.contains('*') {
                continue;
            }
            if let Ok(output) = Command::new(path).arg("--version").output().await {
                if output.status.success() {
                    let version_str = String::from_utf8_lossy(&output.stdout);
                    info.version = Some(version_str.trim().to_string());
                    info.installed = true;
                    bw_cmd = Some(path.clone());
                    break;
                }
            }
        }
    }

    // Check login status
    if let Some(ref cmd) = bw_cmd {
        if let Ok(output) = Command::new(cmd).arg("status").output().await {
            if output.status.success() {
                let status_str = String::from_utf8_lossy(&output.stdout);
                if let Ok(status) = serde_json::from_str::<serde_json::Value>(&status_str) {
                    if let Some(status_val) = status.get("status").and_then(|v| v.as_str()) {
                        match status_val {
                            "unlocked" => {
                                info.running = true;
                                info.status_message = Some("Vault unlocked".to_string());
                            }
                            "locked" => {
                                info.status_message = Some("Vault locked".to_string());
                            }
                            "unauthenticated" => {
                                info.status_message = Some("Not logged in".to_string());
                            }
                            _ => {
                                info.status_message = Some(format!("Status: {status_val}"));
                            }
                        }
                    }
                }
            }
        }
        info.path = Some(PathBuf::from(cmd));
    }

    // If still not found, try which command
    if !info.installed {
        if let Ok(output) = Command::new("which").arg("bw").output().await {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    info.path = Some(PathBuf::from(&path));
                    // Try to get version from found path
                    if let Ok(ver_output) = Command::new(&path).arg("--version").output().await {
                        if ver_output.status.success() {
                            let version_str = String::from_utf8_lossy(&ver_output.stdout);
                            info.version = Some(version_str.trim().to_string());
                            info.installed = true;
                        }
                    }
                }
            }
        }
    }

    if !info.installed {
        info.status_message = Some("Login with 'bw login' in terminal first".to_string());
    }

    info
}

/// Detects original KeePass (via kpcli or keepass2)
pub async fn detect_keepass() -> PasswordManagerInfo {
    let mut info = PasswordManagerInfo {
        id: "keepass",
        name: "KeePass",
        version: None,
        installed: false,
        running: false,
        path: None,
        status_message: None,
        formats: vec!["KDBX 3", "KDBX 4", "KDB"],
    };

    // Check kpcli (Perl CLI for KeePass)
    if let Ok(output) = Command::new("kpcli").arg("--version").output().await {
        if output.status.success() {
            let version_str = String::from_utf8_lossy(&output.stdout);
            info.version = parse_version_line(&version_str);
            info.installed = true;
            info.status_message = Some("kpcli available".to_string());
        }
    }

    // Check keepass2 (Mono/.NET version)
    if !info.installed {
        if let Ok(output) = Command::new("which").arg("keepass2").output().await {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    info.installed = true;
                    info.path = Some(PathBuf::from(path));
                    info.status_message = Some("KeePass 2 (Mono) available".to_string());
                }
            }
        }
    }

    info
}

/// Detects 1Password CLI installation and status
pub async fn detect_onepassword() -> PasswordManagerInfo {
    let mut info = PasswordManagerInfo {
        id: "onepassword",
        name: "1Password CLI",
        version: None,
        installed: false,
        running: false,
        path: None,
        status_message: None,
        formats: vec!["Cloud or self-hosted vault"],
    };

    // Try common paths for op CLI
    let op_paths = ["op", "/usr/bin/op", "/usr/local/bin/op"];

    let home = std::env::var("HOME").unwrap_or_default();
    let extra_paths = [format!("{home}/.local/bin/op"), format!("{home}/bin/op")];

    let mut op_cmd: Option<String> = None;

    // Try standard paths first
    for path in &op_paths {
        if let Ok(output) = Command::new(path).arg("--version").output().await {
            if output.status.success() {
                let version_str = String::from_utf8_lossy(&output.stdout);
                info.version = Some(version_str.trim().to_string());
                info.installed = true;
                op_cmd = Some((*path).to_string());
                break;
            }
        }
    }

    // Try home-relative paths
    if !info.installed {
        for path in &extra_paths {
            if let Ok(output) = Command::new(path).arg("--version").output().await {
                if output.status.success() {
                    let version_str = String::from_utf8_lossy(&output.stdout);
                    info.version = Some(version_str.trim().to_string());
                    info.installed = true;
                    op_cmd = Some(path.clone());
                    break;
                }
            }
        }
    }

    // Check signin status using whoami
    if let Some(ref cmd) = op_cmd {
        if let Ok(output) = Command::new(cmd)
            .args(["whoami", "--format", "json"])
            .output()
            .await
        {
            if output.status.success() {
                info.running = true;
                let stdout = String::from_utf8_lossy(&output.stdout);
                if let Ok(whoami) = serde_json::from_str::<serde_json::Value>(&stdout) {
                    if let Some(email) = whoami.get("email").and_then(|v| v.as_str()) {
                        info.status_message = Some(format!("Signed in as {email}"));
                    } else {
                        info.status_message = Some("Signed in".to_string());
                    }
                } else {
                    info.status_message = Some("Signed in".to_string());
                }
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                if stderr.contains("not signed in") || stderr.contains("sign in") {
                    info.status_message = Some("Not signed in".to_string());
                } else if stderr.contains("session expired") {
                    info.status_message = Some("Session expired".to_string());
                } else {
                    info.status_message = Some("Not signed in".to_string());
                }
            }
        }
        info.path = Some(PathBuf::from(cmd));
    }

    // If still not found, try which command
    if !info.installed {
        if let Ok(output) = Command::new("which").arg("op").output().await {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    info.path = Some(PathBuf::from(&path));
                    // Try to get version from found path
                    if let Ok(ver_output) = Command::new(&path).arg("--version").output().await {
                        if ver_output.status.success() {
                            let version_str = String::from_utf8_lossy(&ver_output.stdout);
                            info.version = Some(version_str.trim().to_string());
                            info.installed = true;
                        }
                    }
                }
            }
        }
    }

    if !info.installed {
        info.status_message =
            Some("Install from https://1password.com/downloads/command-line".to_string());
    }

    info
}

/// Detects Passbolt CLI installation and status
pub async fn detect_passbolt() -> PasswordManagerInfo {
    let mut info = PasswordManagerInfo {
        id: "passbolt",
        name: "Passbolt CLI",
        version: None,
        installed: false,
        running: false,
        path: None,
        status_message: None,
        formats: vec!["Server-based team vault"],
    };

    let passbolt_paths = ["passbolt", "/usr/bin/passbolt", "/usr/local/bin/passbolt"];

    let home = std::env::var("HOME").unwrap_or_default();
    let extra_paths = [
        format!("{home}/.local/bin/passbolt"),
        format!("{home}/go/bin/passbolt"),
        format!("{home}/go/bin/go-passbolt-cli"),
    ];

    let mut pb_cmd: Option<String> = None;

    for path in &passbolt_paths {
        if let Ok(output) = Command::new(path).arg("--version").output().await {
            if output.status.success() {
                let version_str = String::from_utf8_lossy(&output.stdout);
                info.version = Some(version_str.trim().to_string());
                info.installed = true;
                pb_cmd = Some((*path).to_string());
                break;
            }
        }
    }

    if !info.installed {
        for path in &extra_paths {
            if let Ok(output) = Command::new(path).arg("--version").output().await {
                if output.status.success() {
                    let version_str = String::from_utf8_lossy(&output.stdout);
                    info.version = Some(version_str.trim().to_string());
                    info.installed = true;
                    pb_cmd = Some(path.clone());
                    break;
                }
            }
        }
    }

    // Check if configured by listing users
    if let Some(ref cmd) = pb_cmd {
        if let Ok(output) = Command::new(cmd)
            .args(["list", "user", "--json"])
            .output()
            .await
        {
            if output.status.success() {
                info.running = true;
                info.status_message = Some("Configured".to_string());
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                if stderr.contains("no configuration") {
                    info.status_message = Some("Not configured".to_string());
                } else if stderr.contains("authentication") || stderr.contains("passphrase") {
                    info.status_message = Some("Authentication failed".to_string());
                } else {
                    info.status_message = Some("Not configured".to_string());
                }
            }
        }
        info.path = Some(PathBuf::from(cmd));
    }

    // Try which as fallback
    if !info.installed {
        if let Ok(output) = Command::new("which").arg("passbolt").output().await {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    info.path = Some(PathBuf::from(&path));
                    if let Ok(ver_output) = Command::new(&path).arg("--version").output().await {
                        if ver_output.status.success() {
                            let version_str = String::from_utf8_lossy(&ver_output.stdout);
                            info.version = Some(version_str.trim().to_string());
                            info.installed = true;
                        }
                    }
                }
            }
        }
    }

    if !info.installed {
        info.status_message = Some(
            "Install from \
             https://github.com/passbolt/go-passbolt-cli"
                .to_string(),
        );
    }

    info
}

/// Parses version from a typical version output line
fn parse_version_line(output: &str) -> Option<String> {
    VERSION_REGEX
        .captures(output)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// Parses version from flatpak info output
fn parse_flatpak_version(output: &str) -> Option<String> {
    for line in output.lines() {
        if line.trim().starts_with("Version:") {
            return Some(line.trim().strip_prefix("Version:")?.trim().to_string());
        }
    }
    None
}

/// Returns the command to open the password manager application
///
/// # Arguments
/// * `backend` - The secret backend type
/// * `passbolt_server_url` - Optional Passbolt server URL from settings
///
/// # Returns
/// A tuple of (command, args) to launch the password manager, or None
pub fn get_password_manager_launch_command(
    backend: &crate::config::SecretBackendType,
    passbolt_server_url: Option<&str>,
) -> Option<(String, Vec<String>)> {
    match backend {
        crate::config::SecretBackendType::KeePassXc
        | crate::config::SecretBackendType::KdbxFile => {
            // Try KeePassXC first
            if std::process::Command::new("which")
                .arg("keepassxc")
                .output()
                .is_ok_and(|o| o.status.success())
            {
                return Some(("keepassxc".to_string(), vec![]));
            }
            // Try GNOME Secrets (flatpak)
            if std::process::Command::new("flatpak")
                .args(["info", "org.gnome.World.Secrets"])
                .output()
                .is_ok_and(|o| o.status.success())
            {
                return Some((
                    "flatpak".to_string(),
                    vec!["run".to_string(), "org.gnome.World.Secrets".to_string()],
                ));
            }
            // Try gnome-secrets native
            if std::process::Command::new("which")
                .arg("gnome-secrets")
                .output()
                .is_ok_and(|o| o.status.success())
            {
                return Some(("gnome-secrets".to_string(), vec![]));
            }
            // Try KeePass 2
            if std::process::Command::new("which")
                .arg("keepass2")
                .output()
                .is_ok_and(|o| o.status.success())
            {
                return Some(("keepass2".to_string(), vec![]));
            }
            None
        }
        crate::config::SecretBackendType::LibSecret => {
            // Open Seahorse (GNOME Passwords and Keys)
            if std::process::Command::new("which")
                .arg("seahorse")
                .output()
                .is_ok_and(|o| o.status.success())
            {
                return Some(("seahorse".to_string(), vec![]));
            }
            // Try GNOME Settings privacy section
            if std::process::Command::new("which")
                .arg("gnome-control-center")
                .output()
                .is_ok_and(|o| o.status.success())
            {
                return Some((
                    "gnome-control-center".to_string(),
                    vec!["privacy".to_string()],
                ));
            }
            // Try KDE Wallet Manager
            if std::process::Command::new("which")
                .arg("kwalletmanager5")
                .output()
                .is_ok_and(|o| o.status.success())
            {
                return Some(("kwalletmanager5".to_string(), vec![]));
            }
            None
        }
        crate::config::SecretBackendType::Bitwarden => {
            // Open Bitwarden web vault in default browser
            Some((
                "xdg-open".to_string(),
                vec!["https://vault.bitwarden.com".to_string()],
            ))
        }
        crate::config::SecretBackendType::OnePassword => {
            // Try 1Password desktop app first
            if std::process::Command::new("which")
                .arg("1password")
                .output()
                .is_ok_and(|o| o.status.success())
            {
                return Some(("1password".to_string(), vec![]));
            }
            // Try flatpak version
            if std::process::Command::new("flatpak")
                .args(["info", "com.onepassword.OnePassword"])
                .output()
                .is_ok_and(|o| o.status.success())
            {
                return Some((
                    "flatpak".to_string(),
                    vec!["run".to_string(), "com.onepassword.OnePassword".to_string()],
                ));
            }
            // Fallback to web vault
            Some((
                "xdg-open".to_string(),
                vec!["https://my.1password.com".to_string()],
            ))
        }
        crate::config::SecretBackendType::Passbolt => {
            // Passbolt is web-based, open configured server URL in browser
            let url = passbolt_server_url
                .filter(|u| !u.is_empty())
                .unwrap_or("https://passbolt.local");
            Some(("xdg-open".to_string(), vec![url.to_string()]))
        }
    }
}

/// Opens the password manager application for the given backend
///
/// # Arguments
/// * `backend` - The secret backend type
/// * `passbolt_server_url` - Optional Passbolt server URL from settings
///
/// # Returns
/// Ok(()) if launched successfully
///
/// # Errors
/// Returns error message if no password manager is found or launch fails
pub fn open_password_manager(
    backend: &crate::config::SecretBackendType,
    passbolt_server_url: Option<&str>,
) -> Result<(), String> {
    let Some((cmd, args)) = get_password_manager_launch_command(backend, passbolt_server_url)
    else {
        return Err("No password manager application found".to_string());
    };

    std::process::Command::new(&cmd)
        .args(&args)
        .spawn()
        .map_err(|e| format!("Failed to launch {cmd}: {e}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_version_line() {
        assert_eq!(
            parse_version_line("KeePassXC 2.7.6"),
            Some("2.7.6".to_string())
        );
        assert_eq!(
            parse_version_line("secret-tool 0.19.1"),
            Some("0.19.1".to_string())
        );
        assert_eq!(parse_version_line("v1.2.3"), Some("1.2.3".to_string()));
        assert_eq!(parse_version_line("no version"), None);
    }

    #[test]
    fn test_parse_flatpak_version() {
        let output = "ID: org.gnome.World.Secrets\nVersion: 9.0\nBranch: stable";
        assert_eq!(parse_flatpak_version(output), Some("9.0".to_string()));
    }
}
