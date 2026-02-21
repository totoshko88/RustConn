//! Client detection utilities for protocol handlers
//!
//! This module provides functionality to detect installed protocol clients
//! (SSH, RDP, VNC) and retrieve their version information.

use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

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
    /// Minimum required version for compatibility (if known)
    pub min_version: Option<&'static str>,
    /// Whether the detected version meets the minimum requirement
    pub version_compatible: bool,
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
            min_version: None,
            version_compatible: true,
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
            min_version: None,
            version_compatible: false,
        }
    }

    /// Checks if the detected version meets the minimum requirement.
    ///
    /// Returns `true` if no minimum version is set, or if the detected
    /// version is greater than or equal to the minimum.
    #[must_use]
    pub fn check_version_compatible(&self) -> bool {
        let Some(min_str) = self.min_version else {
            return true;
        };
        let Some(ref detected) = self.version else {
            return false;
        };
        let Some(min_ver) = parse_semver(min_str) else {
            return true;
        };
        let Some(det_ver) = parse_semver(detected) else {
            return false;
        };
        det_ver >= min_ver
    }

    /// Sets the minimum version requirement and updates compatibility.
    #[must_use]
    pub fn with_min_version(mut self, min: &'static str) -> Self {
        self.min_version = Some(min);
        self.version_compatible = self.check_version_compatible();
        self
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
    /// SPICE client information
    pub spice: ClientInfo,
    /// Telnet client information
    pub telnet: ClientInfo,
}

impl ClientDetectionResult {
    /// Detects all protocol clients
    #[must_use]
    pub fn detect_all() -> Self {
        Self {
            ssh: detect_ssh_client(),
            rdp: detect_rdp_client(),
            vnc: detect_vnc_client(),
            spice: detect_spice_client(),
            telnet: detect_telnet_client(),
        }
    }

    /// Returns a cached result if available and fresh (< 5 minutes),
    /// otherwise runs full detection and caches the result.
    #[must_use]
    pub fn detect_cached() -> Self {
        static CACHE: OnceLock<Mutex<Option<(ClientDetectionResult, Instant)>>> = OnceLock::new();
        let cache = CACHE.get_or_init(|| Mutex::new(None));

        let mut guard = cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((ref result, ref ts)) = *guard
            && ts.elapsed() < Duration::from_secs(300)
        {
            return result.clone();
        }
        let result = Self::detect_all();
        *guard = Some((result.clone(), Instant::now()));
        result
    }
}

/// Result of detecting all Zero Trust CLI clients
#[derive(Debug, Clone)]
pub struct ZeroTrustDetectionResult {
    /// AWS CLI (SSM)
    pub aws: ClientInfo,
    /// Google Cloud CLI
    pub gcloud: ClientInfo,
    /// Azure CLI
    pub azure: ClientInfo,
    /// OCI CLI
    pub oci: ClientInfo,
    /// Cloudflare CLI
    pub cloudflared: ClientInfo,
    /// Teleport CLI
    pub teleport: ClientInfo,
    /// Tailscale CLI
    pub tailscale: ClientInfo,
    /// Boundary CLI
    pub boundary: ClientInfo,
}

impl ZeroTrustDetectionResult {
    /// Detects all Zero Trust CLI clients
    #[must_use]
    pub fn detect_all() -> Self {
        Self {
            aws: detect_aws_cli(),
            gcloud: detect_gcloud_cli(),
            azure: detect_azure_cli(),
            oci: detect_oci_cli(),
            cloudflared: detect_cloudflared(),
            teleport: detect_teleport(),
            tailscale: detect_tailscale(),
            boundary: detect_boundary(),
        }
    }

    /// Returns all clients as a vector for iteration
    #[must_use]
    pub fn as_vec(&self) -> Vec<&ClientInfo> {
        vec![
            &self.aws,
            &self.gcloud,
            &self.azure,
            &self.oci,
            &self.cloudflared,
            &self.teleport,
            &self.tailscale,
            &self.boundary,
        ]
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
        "Install openssh-client (openssh-clients) package",
    )
}

/// Detects the RDP client on the system
///
/// Checks for FreeRDP 3.x, FreeRDP 2.x, or rdesktop binaries and extracts version information.
/// Priority: xfreerdp3/wlfreerdp3 (FreeRDP 3.x) > xfreerdp/wlfreerdp (FreeRDP 2.x) > rdesktop
#[must_use]
pub fn detect_rdp_client() -> ClientInfo {
    // Try FreeRDP 3.x first (preferred)
    // wlfreerdp3 for Wayland, xfreerdp3 for X11
    if let Some(info) = try_detect_client("FreeRDP 3", "wlfreerdp3", &["--version"]) {
        return info.with_min_version("3.0.0");
    }
    if let Some(info) = try_detect_client("FreeRDP 3", "xfreerdp3", &["--version"]) {
        return info.with_min_version("3.0.0");
    }

    // Try FreeRDP 2.x
    // wlfreerdp for Wayland, xfreerdp for X11
    if let Some(info) = try_detect_client("FreeRDP 2", "wlfreerdp", &["--version"]) {
        return info.with_min_version("3.0.0");
    }
    if let Some(info) = try_detect_client("FreeRDP 2", "xfreerdp", &["--version"]) {
        return info.with_min_version("3.0.0");
    }

    // Try rdesktop as legacy fallback
    if let Some(info) = try_detect_client("rdesktop", "rdesktop", &["--version"]) {
        return info;
    }

    ClientInfo::not_installed("RDP Client", "Install freerdp3-wayland (freerdp) package")
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
        "Install tigervnc-viewer (tigervnc) package. Alternatives: gvncviewer, remmina, krdc",
    )
}

/// Detects the SPICE client on the system
///
/// Checks for `remote-viewer` binary (from virt-viewer package).
#[must_use]
pub fn detect_spice_client() -> ClientInfo {
    if let Some(info) = try_detect_client("remote-viewer", "remote-viewer", &["--version"]) {
        return info;
    }
    ClientInfo::not_installed("SPICE Client", "Install virt-viewer package")
}

/// Detects the installed Telnet client
pub fn detect_telnet_client() -> ClientInfo {
    if let Some(info) = try_detect_client("Telnet", "telnet", &[]) {
        return info;
    }
    ClientInfo::not_installed("Telnet", "Install telnet (inetutils-telnet) package")
}

/// Returns the path to the first available VNC viewer binary
///
/// VNC viewer binaries in order of preference.
const VNC_VIEWERS: &[&str] = &[
    "vncviewer",   // TigerVNC, TightVNC - most common and feature-rich
    "tigervnc",    // TigerVNC specific binary name
    "gvncviewer",  // GTK-VNC viewer
    "xvnc4viewer", // RealVNC
    "vinagre",     // GNOME Vinagre (deprecated but still available)
    "remmina",     // Remmina (supports VNC)
    "krdc",        // KDE Remote Desktop Client
];

/// Returns the path to the first available VNC viewer
///
/// This function checks for VNC viewers in order of preference and returns
/// the path to the first one found. Returns `None` if no viewer is installed.
///
/// # Returns
/// `Some(PathBuf)` with the path to the VNC viewer binary, or `None` if not found
#[must_use]
pub fn detect_vnc_viewer_path() -> Option<PathBuf> {
    VNC_VIEWERS.iter().find_map(|viewer| which_binary(viewer))
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
    VNC_VIEWERS
        .iter()
        .find(|viewer| which_binary(viewer).is_some())
        .map(|viewer| (*viewer).to_string())
}

// ============================================================================
// Zero Trust CLI Detection
// ============================================================================

/// Detects AWS CLI v2 for SSM Session Manager
#[must_use]
pub fn detect_aws_cli() -> ClientInfo {
    if let Some(info) = try_detect_client("AWS CLI (SSM)", "aws", &["--version"]) {
        return info;
    }
    ClientInfo::not_installed("AWS CLI (SSM)", "Install awscli package")
}

/// Detects Google Cloud CLI for IAP tunneling
#[must_use]
pub fn detect_gcloud_cli() -> ClientInfo {
    if let Some(info) = try_detect_client("Google Cloud CLI", "gcloud", &["--version"]) {
        return info;
    }
    ClientInfo::not_installed("Google Cloud CLI", "Install google-cloud-cli package")
}

/// Detects Azure CLI for Bastion and SSH
#[must_use]
pub fn detect_azure_cli() -> ClientInfo {
    if let Some(info) = try_detect_client("Azure CLI", "az", &["--version"]) {
        return info;
    }
    ClientInfo::not_installed("Azure CLI", "Install azure-cli package")
}

/// Detects OCI CLI for Oracle Cloud Bastion
#[must_use]
pub fn detect_oci_cli() -> ClientInfo {
    if let Some(info) = try_detect_client("OCI CLI", "oci", &["--version"]) {
        return info;
    }
    ClientInfo::not_installed("OCI CLI", "Install oci-cli package")
}

/// Detects Cloudflare Access tunnel client
#[must_use]
pub fn detect_cloudflared() -> ClientInfo {
    if let Some(info) = try_detect_client("Cloudflare CLI", "cloudflared", &["--version"]) {
        return info;
    }
    ClientInfo::not_installed("Cloudflare CLI", "Install cloudflared package")
}

/// Detects Teleport SSH client
#[must_use]
pub fn detect_teleport() -> ClientInfo {
    if let Some(info) = try_detect_client("Teleport CLI", "tsh", &["version"]) {
        return info;
    }
    ClientInfo::not_installed("Teleport CLI", "Install teleport package")
}

/// Detects Tailscale CLI
#[must_use]
pub fn detect_tailscale() -> ClientInfo {
    if let Some(info) = try_detect_client("Tailscale CLI", "tailscale", &["--version"]) {
        return info;
    }
    ClientInfo::not_installed("Tailscale CLI", "Install tailscale package")
}

/// Detects `HashiCorp` Boundary client
#[must_use]
pub fn detect_boundary() -> ClientInfo {
    if let Some(info) = try_detect_client("Boundary CLI", "boundary", &["version"]) {
        return info;
    }
    ClientInfo::not_installed("Boundary CLI", "Install boundary package")
}

/// Detects kubectl (Kubernetes CLI)
#[must_use]
pub fn detect_kubectl() -> ClientInfo {
    if let Some(info) = try_detect_client("kubectl", "kubectl", &["version", "--client", "--short"])
    {
        return info;
    }
    ClientInfo::not_installed("kubectl", "Install kubectl package")
}

/// Detects picocom (serial terminal client)
#[must_use]
pub fn detect_picocom() -> ClientInfo {
    if let Some(info) = try_detect_client("picocom", "picocom", &["--help"]) {
        return info;
    }
    ClientInfo::not_installed("picocom", "Install picocom package")
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
    // In snap environment, check SNAP directory first for bundled clients
    if let Ok(snap_dir) = std::env::var("SNAP") {
        // Check common snap binary locations
        let snap_paths = [
            format!("{snap_dir}/usr/bin/{binary}"),
            format!("{snap_dir}/bin/{binary}"),
            format!("{snap_dir}/usr/local/bin/{binary}"),
        ];

        for snap_path in &snap_paths {
            let path = PathBuf::from(snap_path);
            if path.exists() && path.is_file() {
                return Some(path);
            }
        }
    }

    // Use `which` command to find the binary in PATH
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

/// CLI version check timeout (6 seconds)
///
/// Some CLIs (gcloud, az, oci) load Python runtimes and can take 3-5 seconds.
/// This timeout prevents a single slow CLI from blocking the entire detection.
const VERSION_CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(6);

/// Gets version information from a binary with a timeout
fn get_version(binary: &str, args: &[&str]) -> Option<String> {
    let mut child = Command::new(binary)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .ok()?;

    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => break,
            Ok(None) => {
                if start.elapsed() >= VERSION_CHECK_TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Some("installed (timeout)".to_string());
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(_) => return None,
        }
    }

    let output = child.wait_with_output().ok()?;

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

        // Check special formats first (before generic patterns)

        // Azure CLI format: "azure-cli                         2.82.0 *"
        if line.starts_with("azure-cli") {
            return parse_azure_cli_version(line);
        }

        // Teleport format: "Teleport v18.6.5 git:v18.6.5-0-g4bc3277 go1.24.12"
        if line.starts_with("Teleport v") {
            return parse_teleport_version(line);
        }

        // Look for common version patterns
        // SSH: "OpenSSH_8.9p1 Ubuntu-3ubuntu0.1, OpenSSL 3.0.2 15 Mar 2022"
        // FreeRDP: "This is FreeRDP version 2.10.0"
        // rdesktop: "rdesktop 1.9.0"
        // vncviewer: "TigerVNC Viewer 64-bit v1.12.0"
        // remote-viewer: "remote-viewer version 11.0"

        // Return the first meaningful line as version info
        if line.contains("version")
            || line.contains("OpenSSH")
            || line.contains("FreeRDP")
            || line.contains("rdesktop")
            || line.contains("VNC")
            || line.contains("TigerVNC")
            || line.contains("TightVNC")
            || line.contains("remote-viewer")
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

/// Parses Azure CLI version from its specific output format
/// Input: "azure-cli                         2.82.0 *"
/// Output: "2.82.0"
fn parse_azure_cli_version(line: &str) -> Option<String> {
    // Split by whitespace and find the version number
    for part in line.split_whitespace() {
        // Skip "azure-cli" and "*" markers
        if part == "azure-cli" || part == "*" {
            continue;
        }
        // Check if it looks like a version number (starts with digit)
        if part.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            return Some(part.to_string());
        }
    }
    None
}

/// Parses Teleport version from its specific output format
/// Input: "Teleport v18.6.5 git:v18.6.5-0-g4bc3277 go1.24.12"
/// Output: "v18.6.5"
fn parse_teleport_version(line: &str) -> Option<String> {
    // Split by whitespace and find the version (second word starting with 'v')
    for part in line.split_whitespace() {
        // Look for version like "v18.6.5"
        if part.starts_with('v') && part.chars().nth(1).is_some_and(|c| c.is_ascii_digit()) {
            return Some(part.to_string());
        }
    }
    None
}

/// Parses a version string into (major, minor, patch) tuple.
///
/// Handles common version formats: "3.0.0", "v3.0.0", "3.0",
/// "`FreeRDP` version 3.0.0", etc.
fn parse_semver(version_str: &str) -> Option<(u32, u32, u32)> {
    // Extract version-like pattern from the string
    let re_like = version_str
        .split(|c: char| !c.is_ascii_digit() && c != '.')
        .find(|s| s.contains('.') && s.chars().next().is_some_and(|c| c.is_ascii_digit()))?;

    let parts: Vec<&str> = re_like.split('.').collect();
    let major = parts.first()?.parse().ok()?;
    let minor = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let patch = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
    Some((major, minor, patch))
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

    #[test]
    fn test_parse_azure_cli_version() {
        let line = "azure-cli                         2.82.0 *";
        let version = parse_azure_cli_version(line);
        assert_eq!(version, Some("2.82.0".to_string()));
    }

    #[test]
    fn test_parse_azure_cli_version_no_star() {
        let line = "azure-cli                         2.82.0";
        let version = parse_azure_cli_version(line);
        assert_eq!(version, Some("2.82.0".to_string()));
    }

    #[test]
    fn test_parse_version_azure_cli_output() {
        let output = "azure-cli                         2.82.0 *\ncore                              2.82.0 *\ntelemetry                          1.1.0";
        let version = parse_version(output);
        assert_eq!(version, Some("2.82.0".to_string()));
    }

    #[test]
    fn test_parse_teleport_version() {
        let line = "Teleport v18.6.5 git:v18.6.5-0-g4bc3277 go1.24.12";
        let version = parse_teleport_version(line);
        assert_eq!(version, Some("v18.6.5".to_string()));
    }

    #[test]
    fn test_parse_version_teleport_output() {
        let output = "Teleport v18.6.5 git:v18.6.5-0-g4bc3277 go1.24.12";
        let version = parse_version(output);
        assert_eq!(version, Some("v18.6.5".to_string()));
    }

    #[test]
    fn test_parse_semver() {
        assert_eq!(parse_semver("3.0.0"), Some((3, 0, 0)));
        assert_eq!(parse_semver("v3.0.0"), Some((3, 0, 0)));
        assert_eq!(parse_semver("FreeRDP version 3.0.0"), Some((3, 0, 0)));
        assert_eq!(parse_semver("2.11.7"), Some((2, 11, 7)));
        assert_eq!(parse_semver("1.0"), Some((1, 0, 0)));
        assert_eq!(parse_semver(""), None);
        assert_eq!(parse_semver("no version"), None);
    }

    #[test]
    fn test_version_compatible() {
        let info = ClientInfo::installed(
            "FreeRDP",
            PathBuf::from("/usr/bin/xfreerdp"),
            Some("3.0.0".to_string()),
        )
        .with_min_version("3.0.0");
        assert!(info.version_compatible);

        let info = ClientInfo::installed(
            "FreeRDP",
            PathBuf::from("/usr/bin/xfreerdp"),
            Some("2.11.7".to_string()),
        )
        .with_min_version("3.0.0");
        assert!(!info.version_compatible);

        let info = ClientInfo::installed(
            "FreeRDP",
            PathBuf::from("/usr/bin/xfreerdp"),
            Some("3.1.0".to_string()),
        )
        .with_min_version("3.0.0");
        assert!(info.version_compatible);
    }
}
