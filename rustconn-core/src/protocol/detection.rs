//! Client detection utilities for protocol handlers
//!
//! This module provides functionality to detect installed protocol clients
//! (SSH, RDP, VNC) and retrieve their version information.

use std::path::PathBuf;
use std::process::Command;

/// Information about a detected protocol client
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientInfo {
    /// Display name of the client (e.g., "OpenSSH", "`FreeRDP`")
    pub name: String,
    /// Path to the client binary, if found
    pub path: Option<PathBuf>,
    /// Version string extracted from the client
    pub version: Option<String>,
    /// Whether the client is installed and accessible
    pub installed: bool,
    /// Installation hint for missing clients
    pub install_hint: Option<String>,
}

impl ClientInfo {
    /// Creates a new `ClientInfo` for an installed client
    #[must_use]
    pub fn installed(name: impl Into<String>, path: PathBuf, version: Option<String>) -> Self {
        Self {
            name: name.into(),
            path: Some(path),
            version,
            installed: true,
            install_hint: None,
        }
    }

    /// Creates a new `ClientInfo` for a missing client
    #[must_use]
    pub fn not_installed(name: impl Into<String>, install_hint: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            path: None,
            version: None,
            installed: false,
            install_hint: Some(install_hint.into()),
        }
    }
}

/// Result of detecting all protocol clients
#[derive(Debug, Clone)]
pub struct ClientDetectionResult {
    /// SSH client information
    pub ssh: ClientInfo,
    /// RDP client information
    pub rdp: ClientInfo,
    /// VNC client information
    pub vnc: ClientInfo,
}

impl ClientDetectionResult {
    /// Detects all protocol clients
    #[must_use]
    pub fn detect_all() -> Self {
        Self {
            ssh: detect_ssh_client(),
            rdp: detect_rdp_client(),
            vnc: detect_vnc_client(),
        }
    }
}

/// Detects the SSH client on the system
///
/// Checks for the `ssh` binary and extracts version information using `ssh -V`.
#[must_use]
pub fn detect_ssh_client() -> ClientInfo {
    detect_client(
        "OpenSSH",
        &["ssh"],
        &["-V"],
        "Install OpenSSH: sudo apt install openssh-client (Debian/Ubuntu) or sudo dnf install openssh-clients (Fedora)",
    )
}

/// Detects the RDP client on the system
///
/// Checks for `xfreerdp3`, `xfreerdp`, or `rdesktop` binaries and extracts version information.
#[must_use]
pub fn detect_rdp_client() -> ClientInfo {
    // Try xfreerdp3 first (FreeRDP 3.x)
    if let Some(info) = try_detect_client("FreeRDP", "xfreerdp3", &["--version"]) {
        return info;
    }

    // Try xfreerdp (FreeRDP 2.x)
    if let Some(info) = try_detect_client("FreeRDP", "xfreerdp", &["--version"]) {
        return info;
    }

    // Try rdesktop as fallback
    if let Some(info) = try_detect_client("rdesktop", "rdesktop", &["--version"]) {
        return info;
    }

    ClientInfo::not_installed(
        "RDP Client",
        "Install FreeRDP: sudo apt install freerdp2-x11 (Debian/Ubuntu) or sudo dnf install freerdp (Fedora)",
    )
}

/// Detects the VNC client on the system
///
/// Checks for various VNC viewer binaries and extracts version information.
/// Supported viewers: vncviewer (TigerVNC/TightVNC), gvncviewer, xvnc4viewer, vinagre, remmina, krdc
#[must_use]
pub fn detect_vnc_client() -> ClientInfo {
    // Try vncviewer (TigerVNC/TightVNC) - most common
    if let Some(info) = try_detect_client("VNC Viewer", "vncviewer", &["-h"]) {
        return info;
    }

    // Try tigervnc specifically
    if let Some(info) = try_detect_client("TigerVNC", "tigervnc", &["-h"]) {
        return info;
    }

    // Try gvncviewer (GTK-VNC viewer)
    if let Some(info) = try_detect_client("GTK-VNC Viewer", "gvncviewer", &["--help"]) {
        return info;
    }

    // Try xvnc4viewer (RealVNC)
    if let Some(info) = try_detect_client("RealVNC Viewer", "xvnc4viewer", &["-h"]) {
        return info;
    }

    // Try vinagre (GNOME Remote Desktop Viewer - deprecated but still available)
    if let Some(info) = try_detect_client("Vinagre", "vinagre", &["--version"]) {
        return info;
    }

    // Try remmina (supports VNC among other protocols)
    if let Some(info) = try_detect_client("Remmina", "remmina", &["--version"]) {
        return info;
    }

    // Try krdc (KDE Remote Desktop Client)
    if let Some(info) = try_detect_client("KRDC", "krdc", &["--version"]) {
        return info;
    }

    ClientInfo::not_installed(
        "VNC Client",
        "Install TigerVNC: sudo apt install tigervnc-viewer (Debian/Ubuntu) or sudo dnf install tigervnc (Fedora). Alternatives: gvncviewer, remmina, krdc",
    )
}

/// Returns the path to the first available VNC viewer binary
///
/// This function checks for VNC viewers in order of preference and returns
/// the path to the first one found. Returns `None` if no viewer is installed.
///
/// # Returns
/// `Some(PathBuf)` with the path to the VNC viewer binary, or `None` if not found
#[must_use]
pub fn detect_vnc_viewer_path() -> Option<PathBuf> {
    // VNC viewers in order of preference
    let viewers = [
        "vncviewer",   // TigerVNC, TightVNC - most common and feature-rich
        "tigervnc",    // TigerVNC specific binary name
        "gvncviewer",  // GTK-VNC viewer
        "xvnc4viewer", // RealVNC
        "vinagre",     // GNOME Vinagre (deprecated but still available)
        "remmina",     // Remmina (supports VNC)
        "krdc",        // KDE Remote Desktop Client
    ];

    for viewer in viewers {
        if let Some(path) = which_binary(viewer) {
            return Some(path);
        }
    }

    None
}

/// Returns the name of the first available VNC viewer
///
/// This function checks for VNC viewers in order of preference and returns
/// the binary name of the first one found. Returns `None` if no viewer is installed.
///
/// # Returns
/// `Some(String)` with the VNC viewer binary name, or `None` if not found
#[must_use]
pub fn detect_vnc_viewer_name() -> Option<String> {
    // VNC viewers in order of preference
    let viewers = [
        "vncviewer",   // TigerVNC, TightVNC - most common and feature-rich
        "tigervnc",    // TigerVNC specific binary name
        "gvncviewer",  // GTK-VNC viewer
        "xvnc4viewer", // RealVNC
        "vinagre",     // GNOME Vinagre (deprecated but still available)
        "remmina",     // Remmina (supports VNC)
        "krdc",        // KDE Remote Desktop Client
    ];

    for viewer in viewers {
        if which_binary(viewer).is_some() {
            return Some(viewer.to_string());
        }
    }

    None
}

// ============================================================================
// Zero Trust CLI Detection
// ============================================================================

/// Detects AWS CLI v2 for SSM Session Manager
#[must_use]
pub fn detect_aws_cli() -> ClientInfo {
    if let Some(info) = try_detect_client("AWS CLI", "aws", &["--version"]) {
        return info;
    }
    ClientInfo::not_installed(
        "AWS CLI",
        "Install AWS CLI v2: https://docs.aws.amazon.com/cli/latest/userguide/getting-started-install.html",
    )
}

/// Detects Google Cloud CLI for IAP tunneling
#[must_use]
pub fn detect_gcloud_cli() -> ClientInfo {
    if let Some(info) = try_detect_client("Google Cloud CLI", "gcloud", &["--version"]) {
        return info;
    }
    ClientInfo::not_installed(
        "Google Cloud CLI",
        "Install gcloud: https://cloud.google.com/sdk/docs/install",
    )
}

/// Detects Azure CLI for Bastion and SSH
#[must_use]
pub fn detect_azure_cli() -> ClientInfo {
    if let Some(info) = try_detect_client("Azure CLI", "az", &["--version"]) {
        return info;
    }
    ClientInfo::not_installed(
        "Azure CLI",
        "Install Azure CLI: https://docs.microsoft.com/en-us/cli/azure/install-azure-cli",
    )
}

/// Detects OCI CLI for Oracle Cloud Bastion
#[must_use]
pub fn detect_oci_cli() -> ClientInfo {
    if let Some(info) = try_detect_client("OCI CLI", "oci", &["--version"]) {
        return info;
    }
    ClientInfo::not_installed(
        "OCI CLI",
        "Install OCI CLI: https://docs.oracle.com/en-us/iaas/Content/API/SDKDocs/cliinstall.htm",
    )
}

/// Detects Cloudflare Access tunnel client
#[must_use]
pub fn detect_cloudflared() -> ClientInfo {
    if let Some(info) = try_detect_client("cloudflared", "cloudflared", &["--version"]) {
        return info;
    }
    ClientInfo::not_installed(
        "cloudflared",
        "Install cloudflared: https://developers.cloudflare.com/cloudflare-one/connections/connect-apps/install-and-setup/installation/",
    )
}

/// Detects Teleport SSH client
#[must_use]
pub fn detect_teleport() -> ClientInfo {
    if let Some(info) = try_detect_client("Teleport", "tsh", &["version"]) {
        return info;
    }
    ClientInfo::not_installed(
        "Teleport",
        "Install Teleport: https://goteleport.com/docs/installation/",
    )
}

/// Detects Tailscale CLI
#[must_use]
pub fn detect_tailscale() -> ClientInfo {
    if let Some(info) = try_detect_client("Tailscale", "tailscale", &["--version"]) {
        return info;
    }
    ClientInfo::not_installed(
        "Tailscale",
        "Install Tailscale: https://tailscale.com/download/linux",
    )
}

/// Detects `HashiCorp` Boundary client
#[must_use]
pub fn detect_boundary() -> ClientInfo {
    if let Some(info) = try_detect_client("Boundary", "boundary", &["version"]) {
        return info;
    }
    ClientInfo::not_installed(
        "Boundary",
        "Install Boundary: https://developer.hashicorp.com/boundary/downloads",
    )
}

/// Attempts to detect a specific client binary
fn try_detect_client(name: &str, binary: &str, version_args: &[&str]) -> Option<ClientInfo> {
    // First check if the binary exists in PATH
    let path = which_binary(binary)?;

    // Try to get version information
    let version = get_version(binary, version_args);

    Some(ClientInfo::installed(name, path, version))
}

/// Generic client detection with fallback
fn detect_client(
    name: &str,
    binaries: &[&str],
    version_args: &[&str],
    install_hint: &str,
) -> ClientInfo {
    for binary in binaries {
        if let Some(info) = try_detect_client(name, binary, version_args) {
            return info;
        }
    }

    ClientInfo::not_installed(name, install_hint)
}

/// Finds a binary in PATH
fn which_binary(binary: &str) -> Option<PathBuf> {
    // Use `which` command to find the binary
    let output = Command::new("which").arg(binary).output().ok()?;

    if output.status.success() {
        let path_str = String::from_utf8_lossy(&output.stdout);
        let path = path_str.trim();
        if !path.is_empty() {
            return Some(PathBuf::from(path));
        }
    }

    None
}

/// Gets version information from a binary
fn get_version(binary: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(binary).args(args).output().ok()?;

    // Version info might be in stdout or stderr depending on the tool
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Combine and parse version
    let combined = format!("{stdout}{stderr}");
    parse_version(&combined)
}

/// Parses version string from command output
fn parse_version(output: &str) -> Option<String> {
    // Get the first non-empty line that contains version-like information
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Look for common version patterns
        // SSH: "OpenSSH_8.9p1 Ubuntu-3ubuntu0.1, OpenSSL 3.0.2 15 Mar 2022"
        // FreeRDP: "This is FreeRDP version 2.10.0"
        // rdesktop: "rdesktop 1.9.0"
        // vncviewer: "TigerVNC Viewer 64-bit v1.12.0"

        // Return the first meaningful line as version info
        if line.contains("version")
            || line.contains("OpenSSH")
            || line.contains("FreeRDP")
            || line.contains("rdesktop")
            || line.contains("VNC")
            || line.contains("TigerVNC")
            || line.contains("TightVNC")
        {
            // Clean up the version string
            return Some(extract_version_string(line));
        }
    }

    // If no specific pattern found, return first non-empty line
    output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(extract_version_string)
}

/// Extracts a clean version string from a line
fn extract_version_string(line: &str) -> String {
    // Limit length and clean up
    let cleaned = line.chars().take(100).collect::<String>();

    cleaned.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_info_installed() {
        let info = ClientInfo::installed(
            "Test",
            PathBuf::from("/usr/bin/test"),
            Some("1.0".to_string()),
        );
        assert!(info.installed);
        assert_eq!(info.name, "Test");
        assert_eq!(info.path, Some(PathBuf::from("/usr/bin/test")));
        assert_eq!(info.version, Some("1.0".to_string()));
        assert!(info.install_hint.is_none());
    }

    #[test]
    fn test_client_info_not_installed() {
        let info = ClientInfo::not_installed("Test", "Install with: apt install test");
        assert!(!info.installed);
        assert_eq!(info.name, "Test");
        assert!(info.path.is_none());
        assert!(info.version.is_none());
        assert_eq!(
            info.install_hint,
            Some("Install with: apt install test".to_string())
        );
    }

    #[test]
    fn test_parse_version_openssh() {
        let output = "OpenSSH_8.9p1 Ubuntu-3ubuntu0.1, OpenSSL 3.0.2 15 Mar 2022";
        let version = parse_version(output);
        assert!(version.is_some());
        assert!(version.unwrap().contains("OpenSSH"));
    }

    #[test]
    fn test_parse_version_freerdp() {
        let output = "This is FreeRDP version 2.10.0 (2.10.0)";
        let version = parse_version(output);
        assert!(version.is_some());
        assert!(version.unwrap().contains("FreeRDP"));
    }

    #[test]
    fn test_parse_version_tigervnc() {
        let output = "TigerVNC Viewer 64-bit v1.12.0\nBuilt on: 2023-01-15";
        let version = parse_version(output);
        assert!(version.is_some());
        assert!(version.unwrap().contains("TigerVNC"));
    }

    #[test]
    fn test_parse_version_empty() {
        let output = "";
        let version = parse_version(output);
        assert!(version.is_none());
    }

    #[test]
    fn test_extract_version_string_truncates() {
        let long_line = "a".repeat(200);
        let result = extract_version_string(&long_line);
        assert_eq!(result.len(), 100);
    }
}
