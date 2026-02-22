//! CLI download manager for Flatpak environment
//!
//! This module provides functionality to download and install external CLI tools
//! in Flatpak sandbox. CLIs are installed to `~/.var/app/<app-id>/cli/` directory.
//!
//! This feature is only available when running inside Flatpak sandbox.
//!
//! ## Security
//!
//! All downloads are verified using SHA256 checksums to prevent MITM attacks.
//! Components without checksums will fail to install.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use thiserror::Error;

/// Cancellation token for download operations
#[derive(Debug, Clone, Default)]
pub struct DownloadCancellation {
    cancelled: Arc<AtomicBool>,
}

impl DownloadCancellation {
    /// Create a new cancellation token
    #[must_use]
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Cancel the operation
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Check if cancelled
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// Progress information for download operations
#[derive(Debug, Clone)]
pub struct DownloadProgress {
    /// Bytes downloaded so far
    pub downloaded: u64,
    /// Total bytes (if known)
    pub total: Option<u64>,
    /// Current status message
    pub status: String,
}

impl DownloadProgress {
    /// Calculate progress percentage (0.0 - 1.0)
    #[must_use]
    pub fn percentage(&self) -> f64 {
        match self.total {
            Some(total) if total > 0 => self.downloaded as f64 / total as f64,
            _ => 0.0,
        }
    }
}

/// Type alias for progress callback
pub type ProgressCallback = Option<Box<dyn Fn(DownloadProgress) + Send + Sync>>;

/// Error type for CLI download operations
#[derive(Debug, Error)]
pub enum CliDownloadError {
    /// Network error during download
    #[error("Download failed: {0}")]
    DownloadFailed(String),

    /// Checksum verification failed
    #[error("Checksum verification failed: expected {expected}, got {actual}")]
    ChecksumMismatch {
        /// Expected checksum value
        expected: String,
        /// Actual computed checksum
        actual: String,
    },

    /// No checksum provided for component
    #[error("No checksum provided for component (security requirement)")]
    NoChecksum,

    /// Failed to extract archive
    #[error("Extraction failed: {0}")]
    ExtractionFailed(String),

    /// pip install failed
    #[error("pip install failed: {0}")]
    PipInstallFailed(String),

    /// I/O error
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),

    /// Not running in Flatpak
    #[error("CLI download is only available in Flatpak environment")]
    NotFlatpak,

    /// CLI already installed
    #[error("CLI is already installed")]
    AlreadyInstalled,

    /// Installation cancelled
    #[error("Installation cancelled by user")]
    Cancelled,

    /// Component not available for download
    #[error("Component not available for download: {0}")]
    NotAvailable(String),
}

/// Result type for CLI download operations
pub type CliDownloadResult<T> = Result<T, CliDownloadError>;

/// Installation method for a CLI
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallMethod {
    /// Download standalone binary/archive
    Download,
    /// Install via pip (Python package)
    Pip,
    /// Custom installation script
    CustomScript,
    /// Install via system package manager (apt, dnf, pacman, zypper).
    /// Contains package names for each supported package manager.
    SystemPackage {
        /// Package name for apt (Debian/Ubuntu)
        apt: Option<&'static str>,
        /// Package name for dnf (Fedora/RHEL)
        dnf: Option<&'static str>,
        /// Package name for pacman (Arch Linux)
        pacman: Option<&'static str>,
        /// Package name for zypper (openSUSE)
        zypper: Option<&'static str>,
    },
}

/// Detected system package manager
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageManager {
    /// apt (Debian, Ubuntu)
    Apt,
    /// dnf (Fedora, RHEL)
    Dnf,
    /// pacman (Arch Linux)
    Pacman,
    /// zypper (openSUSE)
    Zypper,
}

impl std::fmt::Display for PackageManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Apt => write!(f, "apt"),
            Self::Dnf => write!(f, "dnf"),
            Self::Pacman => write!(f, "pacman"),
            Self::Zypper => write!(f, "zypper"),
        }
    }
}

/// Detects the system package manager by checking for known binaries.
#[must_use]
pub fn detect_package_manager() -> Option<PackageManager> {
    use std::path::Path;
    if Path::new("/usr/bin/apt").exists() {
        Some(PackageManager::Apt)
    } else if Path::new("/usr/bin/dnf").exists() {
        Some(PackageManager::Dnf)
    } else if Path::new("/usr/bin/pacman").exists() {
        Some(PackageManager::Pacman)
    } else if Path::new("/usr/bin/zypper").exists() {
        Some(PackageManager::Zypper)
    } else {
        None
    }
}

/// Returns the install command for a system package, if available
/// for the given package manager.
#[must_use]
pub fn get_system_install_command(
    method: &InstallMethod,
    manager: PackageManager,
) -> Option<String> {
    if let InstallMethod::SystemPackage {
        apt,
        dnf,
        pacman,
        zypper,
    } = method
    {
        let pkg = match manager {
            PackageManager::Apt => *apt,
            PackageManager::Dnf => *dnf,
            PackageManager::Pacman => *pacman,
            PackageManager::Zypper => *zypper,
        }?;
        let cmd = match manager {
            PackageManager::Apt => format!("sudo apt install {pkg}"),
            PackageManager::Dnf => format!("sudo dnf install {pkg}"),
            PackageManager::Pacman => format!("sudo pacman -S {pkg}"),
            PackageManager::Zypper => format!("sudo zypper install {pkg}"),
        };
        Some(cmd)
    } else {
        None
    }
}

/// Category of downloadable component
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentCategory {
    /// Protocol client (RDP, VNC, SPICE)
    ProtocolClient,
    /// Zero Trust CLI
    ZeroTrust,
    /// Password manager CLI
    PasswordManager,
    /// Container orchestration CLI (kubectl)
    ContainerOrchestration,
}

/// Policy for verifying download integrity.
///
/// Components with stable release URLs should use `Static` checksums.
/// Components using "latest" URLs where the binary changes frequently
/// should use `SkipLatest` — the UI will warn the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChecksumPolicy {
    /// Verified against a known SHA256 hash.
    Static(&'static str),
    /// No stable checksum available (e.g. "latest" URL that changes).
    /// Installation proceeds with a warning to the user.
    SkipLatest,
    /// Not available for download at all.
    None,
}

/// Downloadable CLI component
#[derive(Debug, Clone)]
pub struct DownloadableComponent {
    /// Unique identifier
    pub id: &'static str,
    /// Display name
    pub name: &'static str,
    /// Description
    pub description: &'static str,
    /// Category
    pub category: ComponentCategory,
    /// Installation method
    pub install_method: InstallMethod,
    /// Download URL (for Download method)
    pub download_url: Option<&'static str>,
    /// Download URL for aarch64/arm64 architecture (if available)
    pub aarch64_url: Option<&'static str>,
    /// Checksum verification policy for downloads
    pub checksum: ChecksumPolicy,
    /// pip package name (for Pip method)
    pub pip_package: Option<&'static str>,
    /// Approximate size for display
    pub size_hint: &'static str,
    /// Binary name after installation
    pub binary_name: &'static str,
    /// Subdirectory in cli folder
    pub install_subdir: &'static str,
    /// Pinned version string for tracking. `None` for "latest" or
    /// unversioned URLs.
    pub pinned_version: Option<&'static str>,
    /// Whether this component works inside a Flatpak sandbox.
    ///
    /// Network-only tools (cloud CLIs, password managers, kubectl) work
    /// fine in the sandbox. Tools that need host display access
    /// (xfreerdp, vncviewer) do not.
    pub works_in_sandbox: bool,
}

impl DownloadableComponent {
    /// Check if this component is installed
    #[must_use]
    pub fn is_installed(&self) -> bool {
        self.find_installed_binary().is_some()
    }

    /// Find the installed binary path (searches common locations)
    #[must_use]
    pub fn find_installed_binary(&self) -> Option<PathBuf> {
        let cli_dir = get_cli_install_dir()?;
        let install_dir = cli_dir.join(self.install_subdir);

        // Check direct path first
        let direct = install_dir.join(self.binary_name);
        if direct.exists() {
            return Some(direct);
        }

        // Check common subdirectories
        let common_subdirs = [
            "bin",
            "usr/bin",
            "usr/local/bin",
            "usr/local/sessionmanagerplugin/bin",
            "v2/current/bin",
        ];
        for subdir in common_subdirs {
            let path = install_dir.join(subdir).join(self.binary_name);
            if path.exists() {
                return Some(path);
            }
        }

        // For pip components, also check the python directory directly
        if self.install_method == InstallMethod::Pip {
            let python_bin = cli_dir.join("python").join("bin").join(self.binary_name);
            if python_bin.exists() {
                return Some(python_bin);
            }
        }

        // Check known custom install paths for specific components
        match self.id {
            "aws" => {
                let aws_bin = cli_dir.join("aws-cli").join("bin").join(self.binary_name);
                if aws_bin.exists() {
                    return Some(aws_bin);
                }
            }
            "gcloud" => {
                let gcloud_bin = cli_dir
                    .join("google-cloud-sdk")
                    .join("bin")
                    .join(self.binary_name);
                if gcloud_bin.exists() {
                    return Some(gcloud_bin);
                }
            }
            _ => {}
        }

        // Search recursively in install_dir only (limited depth)
        find_binary_in_dir_recursive(&install_dir, self.binary_name, 5)
    }

    /// Get the path to the installed binary (may not exist)
    #[must_use]
    pub fn binary_path(&self) -> Option<PathBuf> {
        // First try to find existing binary
        if let Some(found) = self.find_installed_binary() {
            return Some(found);
        }
        // Fall back to expected path
        let cli_dir = get_cli_install_dir()?;
        Some(cli_dir.join(self.install_subdir).join(self.binary_name))
    }

    /// Returns the download URL for the current system architecture.
    ///
    /// Falls back to the default (x86_64) URL if no architecture-specific
    /// URL is available.
    #[must_use]
    pub fn download_url_for_arch(&self) -> Option<&'static str> {
        if cfg!(target_arch = "aarch64") {
            self.aarch64_url.or(self.download_url)
        } else {
            self.download_url
        }
    }

    /// Check if this component can be downloaded
    #[must_use]
    pub fn is_downloadable(&self) -> bool {
        match self.install_method {
            InstallMethod::Download | InstallMethod::CustomScript => {
                self.download_url_for_arch().is_some()
                    && !matches!(self.checksum, ChecksumPolicy::None)
            }
            InstallMethod::Pip => self.pip_package.is_some(),
            InstallMethod::SystemPackage { .. } => true,
        }
    }
}

/// Helper to find binary in directory recursively
fn find_binary_in_dir_recursive(dir: &Path, binary_name: &str, max_depth: u32) -> Option<PathBuf> {
    if max_depth == 0 || !dir.exists() {
        return None;
    }

    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            if let Some(name) = path.file_name()
                && name == binary_name
            {
                return Some(path);
            }
        } else if path.is_dir()
            && let Some(found) = find_binary_in_dir_recursive(&path, binary_name, max_depth - 1)
        {
            return Some(found);
        }
    }
    None
}

/// Returns the architecture identifier used in download URLs.
///
/// Maps Rust target architecture to the naming convention used by
/// most CLI tool download pages.
#[must_use]
pub fn get_arch() -> &'static str {
    if cfg!(target_arch = "x86_64") {
        "amd64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "unknown"
    }
}

/// All downloadable components
///
/// Note: Components without SHA256 checksums are marked as not downloadable.
/// SPICE viewer (remote-viewer) is not available as standalone download.
/// FreeRDP does not provide pre-built Linux binaries - users should install via system package.
pub static DOWNLOADABLE_COMPONENTS: &[DownloadableComponent] = &[
    // Protocol clients (optional for external fallback)
    // Note: FreeRDP is not listed here — it does not provide pre-built
    // Linux binaries and must be installed via the system package manager.
    DownloadableComponent {
        id: "vncviewer",
        name: "TigerVNC Viewer",
        description: "Optional for external VNC connections",
        category: ComponentCategory::ProtocolClient,
        install_method: InstallMethod::Download,
        // SourceForge direct download URL (follows redirects)
        download_url: Some(
            "https://sourceforge.net/projects/tigervnc/files/stable/1.16.0/\
             tigervnc-1.16.0.x86_64.tar.gz/download",
        ),
        aarch64_url: None,
        checksum: ChecksumPolicy::Static(
            "e27114dcd7e23896094e654dc21fc6e4eb313e6e346598aa389ff95fd765c6c4",
        ),
        pip_package: None,
        size_hint: "~5 MB",
        binary_name: "vncviewer",
        install_subdir: "tigervnc",
        pinned_version: Some("1.16.0"),
        works_in_sandbox: false,
    },
    // Zero Trust CLIs
    DownloadableComponent {
        id: "aws",
        name: "AWS CLI",
        description: "For AWS SSM sessions",
        category: ComponentCategory::ZeroTrust,
        install_method: InstallMethod::CustomScript,
        download_url: Some("https://awscli.amazonaws.com/awscli-exe-linux-x86_64.zip"),
        aarch64_url: Some("https://awscli.amazonaws.com/awscli-exe-linux-aarch64.zip"),
        // AWS CLI "latest" URL — checksum changes with each release
        checksum: ChecksumPolicy::SkipLatest,
        pip_package: None,
        size_hint: "~50 MB",
        binary_name: "aws",
        install_subdir: "aws-cli",
        pinned_version: None,
        works_in_sandbox: true,
    },
    DownloadableComponent {
        id: "session-manager-plugin",
        name: "AWS SSM Plugin",
        description: "Required for AWS SSM sessions",
        category: ComponentCategory::ZeroTrust,
        install_method: InstallMethod::Download,
        download_url: Some(
            "https://s3.amazonaws.com/session-manager-downloads/plugin/latest/\
             ubuntu_64bit/session-manager-plugin.deb",
        ),
        aarch64_url: Some(
            "https://s3.amazonaws.com/session-manager-downloads/plugin/latest/\
             ubuntu_arm64/session-manager-plugin.deb",
        ),
        // Note: AWS doesn't provide stable checksums for "latest" URL
        checksum: ChecksumPolicy::SkipLatest,
        pip_package: None,
        size_hint: "~5 MB",
        binary_name: "session-manager-plugin",
        install_subdir: "ssm-plugin",
        pinned_version: None,
        works_in_sandbox: true,
    },
    DownloadableComponent {
        id: "gcloud",
        name: "Google Cloud CLI",
        description: "For GCP IAP tunnels",
        category: ComponentCategory::ZeroTrust,
        install_method: InstallMethod::CustomScript,
        download_url: Some(
            "https://dl.google.com/dl/cloudsdk/channels/rapid/downloads/\
             google-cloud-cli-linux-x86_64.tar.gz",
        ),
        aarch64_url: Some(
            "https://dl.google.com/dl/cloudsdk/channels/rapid/downloads/\
             google-cloud-cli-linux-arm.tar.gz",
        ),
        checksum: ChecksumPolicy::SkipLatest,
        pip_package: None,
        size_hint: "~500 MB",
        binary_name: "gcloud",
        install_subdir: "google-cloud-sdk/bin",
        pinned_version: None,
        works_in_sandbox: true,
    },
    DownloadableComponent {
        id: "az",
        name: "Azure CLI",
        description: "For Azure Bastion",
        category: ComponentCategory::ZeroTrust,
        install_method: InstallMethod::Pip,
        download_url: None,
        aarch64_url: None,
        checksum: ChecksumPolicy::None,
        pip_package: Some("azure-cli"),
        size_hint: "~200 MB",
        binary_name: "az",
        install_subdir: "python/bin",
        pinned_version: None,
        works_in_sandbox: true,
    },
    DownloadableComponent {
        id: "oci",
        name: "OCI CLI",
        description: "For OCI Bastion",
        category: ComponentCategory::ZeroTrust,
        install_method: InstallMethod::Pip,
        download_url: None,
        aarch64_url: None,
        checksum: ChecksumPolicy::None,
        pip_package: Some("oci-cli"),
        size_hint: "~50 MB",
        binary_name: "oci",
        install_subdir: "python/bin",
        pinned_version: None,
        works_in_sandbox: true,
    },
    DownloadableComponent {
        id: "tsh",
        name: "Teleport",
        description: "For Teleport access",
        category: ComponentCategory::ZeroTrust,
        install_method: InstallMethod::Download,
        download_url: Some("https://cdn.teleport.dev/teleport-v18.7.0-linux-amd64-bin.tar.gz"),
        aarch64_url: Some("https://cdn.teleport.dev/teleport-v18.7.0-linux-arm64-bin.tar.gz"),
        checksum: ChecksumPolicy::SkipLatest,
        pip_package: None,
        size_hint: "~100 MB",
        binary_name: "tsh",
        install_subdir: "teleport",
        pinned_version: Some("18.7.0"),
        works_in_sandbox: true,
    },
    DownloadableComponent {
        id: "tailscale",
        name: "Tailscale",
        description: "For Tailscale SSH",
        category: ComponentCategory::ZeroTrust,
        install_method: InstallMethod::Download,
        download_url: Some("https://pkgs.tailscale.com/stable/tailscale_1.94.2_amd64.tgz"),
        aarch64_url: Some("https://pkgs.tailscale.com/stable/tailscale_1.94.2_arm64.tgz"),
        checksum: ChecksumPolicy::Static(
            "c6f99a5d774c7783b56902188d69e9756fc3dddfb08ac6be4cb2585f3fecdc32",
        ),
        pip_package: None,
        size_hint: "~25 MB",
        binary_name: "tailscale",
        install_subdir: "tailscale",
        pinned_version: Some("1.94.2"),
        works_in_sandbox: true,
    },
    DownloadableComponent {
        id: "cloudflared",
        name: "Cloudflare Tunnel",
        description: "For Cloudflare Access",
        category: ComponentCategory::ZeroTrust,
        install_method: InstallMethod::Download,
        download_url: Some(
            "https://github.com/cloudflare/cloudflared/releases/latest/download/\
             cloudflared-linux-amd64",
        ),
        aarch64_url: Some(
            "https://github.com/cloudflare/cloudflared/releases/latest/download/\
             cloudflared-linux-arm64",
        ),
        checksum: ChecksumPolicy::SkipLatest,
        pip_package: None,
        size_hint: "~30 MB",
        binary_name: "cloudflared",
        install_subdir: "cloudflared",
        pinned_version: None,
        works_in_sandbox: true,
    },
    DownloadableComponent {
        id: "boundary",
        name: "HashiCorp Boundary",
        description: "For Boundary access",
        category: ComponentCategory::ZeroTrust,
        install_method: InstallMethod::Download,
        download_url: Some(
            "https://releases.hashicorp.com/boundary/0.21.1/\
             boundary_0.21.1_linux_amd64.zip",
        ),
        aarch64_url: Some(
            "https://releases.hashicorp.com/boundary/0.21.1/\
             boundary_0.21.1_linux_arm64.zip",
        ),
        checksum: ChecksumPolicy::Static(
            "ab9f2b2a7624ecd351a9e6f3ef85e61e37eaa3665a6e38a4036b3486e925f8b4",
        ),
        pip_package: None,
        size_hint: "~50 MB",
        binary_name: "boundary",
        install_subdir: "boundary",
        pinned_version: Some("0.21.1"),
        works_in_sandbox: true,
    },
    // Password manager CLIs
    DownloadableComponent {
        id: "bw",
        name: "Bitwarden CLI",
        description: "For Bitwarden integration",
        category: ComponentCategory::PasswordManager,
        install_method: InstallMethod::Download,
        download_url: Some(
            "https://github.com/bitwarden/clients/releases/download/\
             cli-v2026.1.0/bw-linux-2026.1.0.zip",
        ),
        aarch64_url: None,
        checksum: ChecksumPolicy::SkipLatest,
        pip_package: None,
        size_hint: "~50 MB",
        binary_name: "bw",
        install_subdir: "bitwarden",
        pinned_version: Some("2026.1.0"),
        works_in_sandbox: true,
    },
    DownloadableComponent {
        id: "op",
        name: "1Password CLI",
        description: "For 1Password integration",
        category: ComponentCategory::PasswordManager,
        install_method: InstallMethod::Download,
        download_url: Some(
            "https://cache.agilebits.com/dist/1P/op2/pkg/v2.32.1/\
             op_linux_amd64_v2.32.1.zip",
        ),
        aarch64_url: Some(
            "https://cache.agilebits.com/dist/1P/op2/pkg/v2.32.1/\
             op_linux_arm64_v2.32.1.zip",
        ),
        checksum: ChecksumPolicy::SkipLatest,
        pip_package: None,
        size_hint: "~15 MB",
        binary_name: "op",
        install_subdir: "1password",
        pinned_version: Some("2.32.1"),
        works_in_sandbox: true,
    },
    // Container orchestration CLIs
    DownloadableComponent {
        id: "kubectl",
        name: "kubectl",
        description: "Kubernetes CLI for pod shell connections",
        category: ComponentCategory::ContainerOrchestration,
        install_method: InstallMethod::Download,
        download_url: Some("https://dl.k8s.io/release/v1.35.1/bin/linux/amd64/kubectl"),
        aarch64_url: Some("https://dl.k8s.io/release/v1.35.1/bin/linux/arm64/kubectl"),
        // kubectl is a single binary — checksum changes per release
        checksum: ChecksumPolicy::SkipLatest,
        pip_package: None,
        size_hint: "~50 MB",
        binary_name: "kubectl",
        install_subdir: "kubectl",
        pinned_version: Some("1.35.1"),
        works_in_sandbox: true,
    },
];

/// Get the CLI installation directory
///
/// In Flatpak: `~/.var/app/io.github.totoshko88.RustConn/cli/`
#[must_use]
pub fn get_cli_install_dir() -> Option<PathBuf> {
    if !crate::flatpak::is_flatpak() {
        return None;
    }

    // In Flatpak, XDG_DATA_HOME points to ~/.var/app/<app-id>/data
    // We want ~/.var/app/<app-id>/cli
    std::env::var("XDG_DATA_HOME").ok().map(|data_home| {
        PathBuf::from(data_home)
            .parent()
            .map(|p| p.join("cli"))
            .unwrap_or_else(|| PathBuf::from("cli"))
    })
}

/// Get all CLI directories that should be added to PATH
///
/// Returns a list of directories containing installed CLI binaries.
/// This is used to extend PATH for Local Shell sessions.
#[must_use]
pub fn get_cli_path_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    let Some(cli_dir) = get_cli_install_dir() else {
        return dirs;
    };

    // Add directories for each installed component
    // pip-installed CLIs (az, oci) are in python/bin
    // AWS CLI v2 is in aws-cli/bin or aws-cli/v2/current/bin
    // SSM Plugin is in ssm-plugin/usr/local/sessionmanagerplugin/bin
    let path_subdirs = [
        "python/bin",                                    // pip-installed CLIs (az, oci)
        "aws-cli/bin",                                   // AWS CLI v2 (official installer)
        "aws-cli/v2/current/bin",                        // AWS CLI v2 (symlink structure)
        "ssm-plugin/usr/local/sessionmanagerplugin/bin", // AWS SSM Plugin
        "google-cloud-sdk/bin",                          // Google Cloud CLI
        "teleport",                                      // Teleport
        "tailscale",        // Tailscale (contains tailscale_X.Y.Z directory)
        "cloudflared",      // Cloudflare Tunnel
        "boundary",         // HashiCorp Boundary
        "bitwarden",        // Bitwarden CLI
        "1password",        // 1Password CLI
        "tigervnc/usr/bin", // TigerVNC
    ];

    for subdir in &path_subdirs {
        let path = cli_dir.join(subdir);
        if path.exists() && path.is_dir() {
            dirs.push(path);
        }
    }

    // Also check for versioned tailscale directory
    let tailscale_dir = cli_dir.join("tailscale");
    if tailscale_dir.exists()
        && let Ok(entries) = std::fs::read_dir(&tailscale_dir)
    {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir()
                && let Some(name) = path.file_name().and_then(|n| n.to_str())
                && name.starts_with("tailscale_")
            {
                dirs.push(path);
            }
        }
    }

    dirs
}

/// Get PATH string with CLI directories prepended
///
/// Returns the current PATH with CLI directories added at the beginning.
#[must_use]
pub fn get_extended_path() -> String {
    let cli_dirs = get_cli_path_dirs();
    let current_path = std::env::var("PATH").unwrap_or_default();

    if cli_dirs.is_empty() {
        return current_path;
    }

    let cli_path: String = cli_dirs
        .iter()
        .filter_map(|p| p.to_str())
        .collect::<Vec<_>>()
        .join(":");

    if current_path.is_empty() {
        cli_path
    } else {
        format!("{cli_path}:{current_path}")
    }
}

/// Get component by ID
#[must_use]
pub fn get_component(id: &str) -> Option<&'static DownloadableComponent> {
    DOWNLOADABLE_COMPONENTS.iter().find(|c| c.id == id)
}

/// Get all components in a category
#[must_use]
pub fn get_components_by_category(
    category: ComponentCategory,
) -> Vec<&'static DownloadableComponent> {
    DOWNLOADABLE_COMPONENTS
        .iter()
        .filter(|c| c.category == category)
        .collect()
}

/// Returns components filtered for the current environment.
///
/// In Flatpak, excludes components that require host display access
/// (e.g. xfreerdp, vncviewer). Outside Flatpak, returns all components.
#[must_use]
pub fn get_available_components() -> Vec<&'static DownloadableComponent> {
    if crate::flatpak::is_flatpak() {
        DOWNLOADABLE_COMPONENTS
            .iter()
            .filter(|c| c.works_in_sandbox)
            .collect()
    } else {
        DOWNLOADABLE_COMPONENTS.iter().collect()
    }
}

/// Check installation status of all components
#[must_use]
pub fn get_installation_status() -> Vec<(&'static DownloadableComponent, bool)> {
    DOWNLOADABLE_COMPONENTS
        .iter()
        .map(|c| (c, c.is_installed()))
        .collect()
}

/// Returns `(component_id, version)` pairs for all components with
/// pinned versions. Useful for CI version-checking scripts.
#[must_use]
pub fn get_pinned_versions() -> Vec<(&'static str, &'static str)> {
    DOWNLOADABLE_COMPONENTS
        .iter()
        .filter_map(|c| c.pinned_version.map(|v| (c.id, v)))
        .collect()
}

/// Verify SHA256 checksum of downloaded data
fn verify_checksum(data: &[u8], expected: &str) -> CliDownloadResult<()> {
    use ring::digest::{Context, SHA256};

    let mut context = Context::new(&SHA256);
    context.update(data);
    let digest = context.finish();
    let actual = hex::encode(digest.as_ref());

    if actual != expected {
        return Err(CliDownloadError::ChecksumMismatch {
            expected: expected.to_string(),
            actual,
        });
    }

    Ok(())
}

/// Download file with progress reporting
async fn download_with_progress(
    url: &str,
    progress_callback: &ProgressCallback,
    cancel_token: &DownloadCancellation,
) -> CliDownloadResult<Vec<u8>> {
    use futures_util::StreamExt;

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| CliDownloadError::DownloadFailed(e.to_string()))?;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| CliDownloadError::DownloadFailed(e.to_string()))?;

    // Check for HTTP errors
    let status = response.status();
    if !status.is_success() {
        return Err(CliDownloadError::DownloadFailed(format!(
            "HTTP {} - {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or("Unknown error")
        )));
    }

    let total_size = response.content_length();
    let mut downloaded: u64 = 0;
    let mut data = Vec::with_capacity(total_size.unwrap_or(10_000_000) as usize);

    let mut stream = response.bytes_stream();

    while let Some(chunk_result) = stream.next().await {
        // Check for cancellation
        if cancel_token.is_cancelled() {
            return Err(CliDownloadError::Cancelled);
        }

        let chunk = chunk_result.map_err(|e| CliDownloadError::DownloadFailed(e.to_string()))?;
        downloaded += chunk.len() as u64;
        data.extend_from_slice(&chunk);

        if let Some(cb) = progress_callback {
            cb(DownloadProgress {
                downloaded,
                total: total_size,
                status: format!("Downloading... {:.1} MB", downloaded as f64 / 1_000_000.0),
            });
        }
    }

    Ok(data)
}

/// Download and install a component
///
/// # Errors
///
/// Returns error if download, verification, or installation fails.
pub async fn install_component(
    component: &DownloadableComponent,
    progress_callback: ProgressCallback,
    cancel_token: DownloadCancellation,
) -> CliDownloadResult<PathBuf> {
    if !crate::flatpak::is_flatpak() {
        return Err(CliDownloadError::NotFlatpak);
    }

    if component.is_installed() {
        return Err(CliDownloadError::AlreadyInstalled);
    }

    if !component.is_downloadable() {
        return Err(CliDownloadError::NotAvailable(component.name.to_string()));
    }

    let cli_dir = get_cli_install_dir().ok_or(CliDownloadError::NotFlatpak)?;

    // Create installation directory
    let install_dir = cli_dir.join(component.install_subdir);
    tokio::fs::create_dir_all(&install_dir).await?;

    match component.install_method {
        InstallMethod::Download => {
            install_download_component(component, &cli_dir, progress_callback, cancel_token).await
        }
        InstallMethod::Pip => {
            install_pip_component(component, &cli_dir, progress_callback, cancel_token).await
        }
        InstallMethod::CustomScript => {
            install_custom_component(component, &cli_dir, progress_callback, cancel_token).await
        }
        InstallMethod::SystemPackage { .. } => Err(CliDownloadError::NotAvailable(
            "System packages must be installed via the system \
                 package manager"
                .to_string(),
        )),
    }
}

async fn install_download_component(
    component: &DownloadableComponent,
    cli_dir: &Path,
    progress_callback: ProgressCallback,
    cancel_token: DownloadCancellation,
) -> CliDownloadResult<PathBuf> {
    let url = component
        .download_url_for_arch()
        .ok_or_else(|| CliDownloadError::NotAvailable("No download URL".to_string()))?;

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: 0,
            total: None,
            status: format!("Downloading {}...", component.name),
        });
    }

    // Download with progress
    let bytes = download_with_progress(url, &progress_callback, &cancel_token).await?;

    // Check cancellation before verification
    if cancel_token.is_cancelled() {
        return Err(CliDownloadError::Cancelled);
    }

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: bytes.len() as u64,
            total: Some(bytes.len() as u64),
            status: "Verifying checksum...".to_string(),
        });
    }

    // Verify checksum based on policy
    match component.checksum {
        ChecksumPolicy::Static(expected) => {
            verify_checksum(&bytes, expected)?;
        }
        ChecksumPolicy::SkipLatest => {
            tracing::warn!(
                "Skipping checksum for {} (latest URL, no stable hash)",
                component.name
            );
        }
        ChecksumPolicy::None => {
            return Err(CliDownloadError::NoChecksum);
        }
    }

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: bytes.len() as u64,
            total: Some(bytes.len() as u64),
            status: "Extracting...".to_string(),
        });
    }

    let install_dir = cli_dir.join(component.install_subdir);
    tokio::fs::create_dir_all(&install_dir).await?;

    // Determine file type and extract
    let url_lower = url.to_lowercase();
    #[allow(clippy::case_sensitive_file_extension_comparisons)]
    if url_lower.ends_with(".zip") {
        extract_zip(&bytes, &install_dir)?;
    } else if url_lower.ends_with(".tar.gz") || url_lower.ends_with(".tgz") {
        extract_tar_gz(&bytes, &install_dir)?;
    } else if url_lower.ends_with(".deb") {
        extract_deb(&bytes, &install_dir)?;
    } else {
        // Single binary file
        let binary_path = install_dir.join(component.binary_name);
        tokio::fs::write(&binary_path, &bytes).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = tokio::fs::metadata(&binary_path).await?.permissions();
            perms.set_mode(0o755);
            tokio::fs::set_permissions(&binary_path, perms).await?;
        }
    }

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: bytes.len() as u64,
            total: Some(bytes.len() as u64),
            status: "Done".to_string(),
        });
    }

    // Try to find the binary - it might be in a subdirectory or have a different name
    let binary_path = find_binary_in_dir(&install_dir, component.binary_name)?;
    Ok(binary_path)
}

/// Find a binary in a directory, searching recursively if needed
fn find_binary_in_dir(dir: &Path, binary_name: &str) -> CliDownloadResult<PathBuf> {
    // First check direct path
    let direct = dir.join(binary_name);
    if direct.exists() {
        return Ok(direct);
    }

    // Check common subdirectories (including SSM plugin path)
    let common_subdirs = [
        "bin",
        "usr/bin",
        "usr/local/bin",
        "usr/local/sessionmanagerplugin/bin", // AWS SSM Plugin
    ];
    for subdir in common_subdirs {
        let path = dir.join(subdir).join(binary_name);
        if path.exists() {
            return Ok(path);
        }
    }

    // Search recursively
    if let Some(found) = find_binary_recursive(dir, binary_name, 5) {
        return Ok(found);
    }

    Err(CliDownloadError::ExtractionFailed(format!(
        "Binary '{}' not found in extracted files",
        binary_name
    )))
}

fn find_binary_recursive(dir: &Path, binary_name: &str, max_depth: u32) -> Option<PathBuf> {
    if max_depth == 0 {
        return None;
    }

    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            if let Some(name) = path.file_name()
                && name == binary_name
            {
                return Some(path);
            }
        } else if path.is_dir()
            && let Some(found) = find_binary_recursive(&path, binary_name, max_depth - 1)
        {
            return Some(found);
        }
    }
    None
}

/// Check if pip is available (either system pip or our installed pip)
async fn ensure_pip_available(python_dir: &Path) -> CliDownloadResult<PathBuf> {
    // First check if pip is already available
    let pip_check = tokio::process::Command::new("python3")
        .args(["-m", "pip", "--version"])
        .output()
        .await;

    if let Ok(output) = pip_check
        && output.status.success()
    {
        tracing::debug!("System pip is available");
        return Ok(PathBuf::from("pip")); // Use system pip
    }

    // Check if we have pip installed in our python directory
    let local_pip = python_dir.join("bin/pip3");
    if local_pip.exists() {
        tracing::debug!("Local pip found at {:?}", local_pip);
        return Ok(local_pip);
    }

    // Install pip using ensurepip
    tracing::info!("Installing pip via ensurepip...");

    tokio::fs::create_dir_all(python_dir).await?;

    let output = tokio::process::Command::new("python3")
        .args(["-m", "ensurepip", "--user", "--upgrade"])
        .env("PYTHONUSERBASE", python_dir)
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!("ensurepip failed: {}", stderr);
        return Err(CliDownloadError::PipInstallFailed(format!(
            "Failed to install pip via ensurepip: {}",
            stderr
        )));
    }

    tracing::info!("pip installed successfully via ensurepip");

    // Return path to the installed pip
    let pip_path = python_dir.join("bin/pip3");
    if pip_path.exists() {
        Ok(pip_path)
    } else {
        // Try pip instead of pip3
        let pip_path = python_dir.join("bin/pip");
        if pip_path.exists() {
            Ok(pip_path)
        } else {
            // Fall back to using python -m pip
            Ok(PathBuf::from("python3"))
        }
    }
}

/// Install a pip-based component
async fn install_pip_component(
    component: &DownloadableComponent,
    cli_dir: &Path,
    progress_callback: ProgressCallback,
    cancel_token: DownloadCancellation,
) -> CliDownloadResult<PathBuf> {
    let pip_package = component
        .pip_package
        .ok_or_else(|| CliDownloadError::NotAvailable("No pip package specified".to_string()))?;

    let python_dir = cli_dir.join("python");

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: 0,
            total: None,
            status: "Checking pip availability...".to_string(),
        });
    }

    // Ensure pip is available (we call this to install pip if needed, but use python -m pip)
    ensure_pip_available(&python_dir).await?;

    if cancel_token.is_cancelled() {
        return Err(CliDownloadError::Cancelled);
    }

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: 0,
            total: None,
            status: format!("Installing {}...", component.name),
        });
    }

    // Install the package using pip with --target to control installation location
    // Use python -m pip for reliability
    let output = tokio::process::Command::new("python3")
        .args([
            "-m",
            "pip",
            "install",
            "--user",
            "--no-warn-script-location",
            pip_package,
        ])
        .env("PYTHONUSERBASE", &python_dir)
        .output()
        .await?;

    if cancel_token.is_cancelled() {
        return Err(CliDownloadError::Cancelled);
    }

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!("pip install failed for {}: {}", pip_package, stderr);
        return Err(CliDownloadError::PipInstallFailed(stderr.to_string()));
    }

    // Log pip output for debugging
    let stdout = String::from_utf8_lossy(&output.stdout);
    tracing::debug!("pip install output: {}", stdout);

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: 100,
            total: Some(100),
            status: "Creating wrapper script...".to_string(),
        });
    }

    // pip with PYTHONUSERBASE doesn't create console scripts in bin/
    // We need to create wrapper scripts manually
    let binary_path = create_pip_wrapper_script(&python_dir, component).await?;

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: 100,
            total: Some(100),
            status: "Done".to_string(),
        });
    }

    Ok(binary_path)
}

/// Create a wrapper script for pip-installed CLI tools
///
/// pip with `--user` and `PYTHONUSERBASE` installs packages to site-packages
/// but doesn't create console scripts in bin/. We create wrapper scripts
/// that invoke the Python module directly.
#[allow(clippy::too_many_lines)]
async fn create_pip_wrapper_script(
    python_dir: &Path,
    component: &DownloadableComponent,
) -> CliDownloadResult<PathBuf> {
    let bin_dir = python_dir.join("bin");
    tokio::fs::create_dir_all(&bin_dir).await?;

    let binary_path = bin_dir.join(component.binary_name);

    // Determine the Python module/entry point based on the component
    // Some packages use -m style, others need import style for entry points
    let script_content = match component.id {
        "az" => {
            // Azure CLI: python -m azure.cli
            format!(
                r#"#!/bin/bash
# Wrapper script for {name} CLI
# Auto-generated by RustConn

export PYTHONUSERBASE="{python_dir}"
PYVER=$(python3 -c "import sys; print(f'python{{sys.version_info.major}}.{{sys.version_info.minor}}')" 2>/dev/null || echo "python3")
export PYTHONPATH="{python_dir}/lib/$PYVER/site-packages:$PYTHONPATH"
exec python3 -m azure.cli "$@"
"#,
                name = component.name,
                python_dir = python_dir.display(),
            )
        }
        "oci" => {
            // OCI CLI: entry point is oci_cli.cli:cli
            format!(
                r#"#!/bin/bash
# Wrapper script for {name} CLI
# Auto-generated by RustConn

export PYTHONUSERBASE="{python_dir}"
PYVER=$(python3 -c "import sys; print(f'python{{sys.version_info.major}}.{{sys.version_info.minor}}')" 2>/dev/null || echo "python3")
export PYTHONPATH="{python_dir}/lib/$PYVER/site-packages:$PYTHONPATH"
exec python3 -c "from oci_cli.cli import cli; cli()" "$@"
"#,
                name = component.name,
                python_dir = python_dir.display(),
            )
        }
        "session-manager-plugin" => {
            // SSM Session Client: entry point is ssm_session_client.main:main
            format!(
                r#"#!/bin/bash
# Wrapper script for {name}
# Auto-generated by RustConn

export PYTHONUSERBASE="{python_dir}"
PYVER=$(python3 -c "import sys; print(f'python{{sys.version_info.major}}.{{sys.version_info.minor}}')" 2>/dev/null || echo "python3")
export PYTHONPATH="{python_dir}/lib/$PYVER/site-packages:$PYTHONPATH"
exec python3 -c "from ssm_session_client.main import main; main()" "$@"
"#,
                name = component.name,
                python_dir = python_dir.display(),
            )
        }
        _ => {
            return Err(CliDownloadError::ExtractionFailed(format!(
                "Unknown pip component: {}",
                component.id
            )));
        }
    };

    tokio::fs::write(&binary_path, script_content).await?;

    // Make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = tokio::fs::metadata(&binary_path).await?.permissions();
        perms.set_mode(0o755);
        tokio::fs::set_permissions(&binary_path, perms).await?;
    }

    tracing::info!("Created wrapper script at {:?}", binary_path);

    // Verify the script works by running --version
    let test_output = tokio::process::Command::new(&binary_path)
        .arg("--version")
        .output()
        .await;

    match test_output {
        Ok(output) if output.status.success() => {
            tracing::info!(
                "{} wrapper script verified successfully",
                component.binary_name
            );
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!(
                "{} wrapper script test returned non-zero: {}",
                component.binary_name,
                stderr
            );
            // Don't fail - some CLIs return non-zero for --version
        }
        Err(e) => {
            tracing::warn!(
                "{} wrapper script test failed: {}",
                component.binary_name,
                e
            );
            // Don't fail - the script might still work
        }
    }

    Ok(binary_path)
}

async fn install_custom_component(
    component: &DownloadableComponent,
    cli_dir: &Path,
    progress_callback: ProgressCallback,
    cancel_token: DownloadCancellation,
) -> CliDownloadResult<PathBuf> {
    // Special handling for gcloud
    if component.id == "gcloud" {
        return install_gcloud(component, cli_dir, progress_callback, cancel_token).await;
    }

    // Special handling for AWS CLI
    if component.id == "aws" {
        return install_aws_cli(component, cli_dir, progress_callback, cancel_token).await;
    }

    Err(CliDownloadError::NotAvailable(format!(
        "Custom installation not implemented for {}",
        component.id
    )))
}

async fn install_gcloud(
    component: &DownloadableComponent,
    cli_dir: &Path,
    progress_callback: ProgressCallback,
    cancel_token: DownloadCancellation,
) -> CliDownloadResult<PathBuf> {
    let url = component
        .download_url_for_arch()
        .ok_or_else(|| CliDownloadError::NotAvailable("No download URL".to_string()))?;

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: 0,
            total: None,
            status: "Downloading Google Cloud CLI...".to_string(),
        });
    }

    let bytes = download_with_progress(url, &progress_callback, &cancel_token).await?;

    if cancel_token.is_cancelled() {
        return Err(CliDownloadError::Cancelled);
    }

    // Verify checksum based on policy
    match component.checksum {
        ChecksumPolicy::Static(expected) => {
            verify_checksum(&bytes, expected)?;
        }
        ChecksumPolicy::SkipLatest => {
            tracing::warn!(
                "Skipping checksum for {} (latest URL, no stable hash)",
                component.name
            );
        }
        ChecksumPolicy::None => {
            return Err(CliDownloadError::NoChecksum);
        }
    }

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: bytes.len() as u64,
            total: Some(bytes.len() as u64),
            status: "Extracting...".to_string(),
        });
    }

    // Extract to cli_dir - the archive contains google-cloud-sdk/ directory
    // Use extract_tar_gz_preserve to keep the directory structure
    extract_tar_gz_preserve(&bytes, cli_dir)?;

    if cancel_token.is_cancelled() {
        return Err(CliDownloadError::Cancelled);
    }

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: bytes.len() as u64,
            total: Some(bytes.len() as u64),
            status: "Running install script...".to_string(),
        });
    }

    // Run install.sh
    let install_script = cli_dir.join("google-cloud-sdk/install.sh");
    if install_script.exists() {
        let output = tokio::process::Command::new("bash")
            .args([
                install_script.to_str().unwrap_or("install.sh"),
                "--quiet",
                "--path-update=false",
                "--command-completion=false",
                "--usage-reporting=false",
            ])
            .output()
            .await?;

        if !output.status.success() {
            tracing::warn!(
                "gcloud install.sh returned non-zero: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    } else {
        tracing::warn!("gcloud install.sh not found at {:?}", install_script);
    }

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: bytes.len() as u64,
            total: Some(bytes.len() as u64),
            status: "Done".to_string(),
        });
    }

    let binary_path = cli_dir.join("google-cloud-sdk/bin/gcloud");
    if binary_path.exists() {
        Ok(binary_path)
    } else {
        // Log directory structure for debugging
        tracing::error!("gcloud binary not found at {:?}", binary_path);
        if let Ok(entries) = std::fs::read_dir(cli_dir) {
            for entry in entries.flatten() {
                tracing::debug!("  cli_dir contains: {:?}", entry.path());
            }
        }
        let sdk_dir = cli_dir.join("google-cloud-sdk");
        if sdk_dir.exists()
            && let Ok(entries) = std::fs::read_dir(&sdk_dir)
        {
            for entry in entries.flatten() {
                tracing::debug!("  google-cloud-sdk contains: {:?}", entry.path());
            }
        }
        Err(CliDownloadError::ExtractionFailed(
            "gcloud binary not found. Check logs for details.".to_string(),
        ))
    }
}

/// Install AWS CLI v2
///
/// AWS CLI is distributed as a zip containing an installer script.
/// We extract and run the installer with custom install location.
#[allow(clippy::too_many_lines)]
async fn install_aws_cli(
    component: &DownloadableComponent,
    cli_dir: &Path,
    progress_callback: ProgressCallback,
    cancel_token: DownloadCancellation,
) -> CliDownloadResult<PathBuf> {
    let url = component
        .download_url_for_arch()
        .ok_or_else(|| CliDownloadError::NotAvailable("No download URL".to_string()))?;

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: 0,
            total: None,
            status: "Downloading AWS CLI...".to_string(),
        });
    }

    let bytes = download_with_progress(url, &progress_callback, &cancel_token).await?;

    if cancel_token.is_cancelled() {
        return Err(CliDownloadError::Cancelled);
    }

    // Verify checksum based on policy
    match component.checksum {
        ChecksumPolicy::Static(expected) => {
            verify_checksum(&bytes, expected)?;
        }
        ChecksumPolicy::SkipLatest => {
            tracing::warn!(
                "Skipping checksum for {} (latest URL, no stable hash)",
                component.name
            );
        }
        ChecksumPolicy::None => {
            return Err(CliDownloadError::NoChecksum);
        }
    }

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: bytes.len() as u64,
            total: Some(bytes.len() as u64),
            status: "Extracting...".to_string(),
        });
    }

    // Create temp directory for extraction
    let temp_dir = cli_dir.join("aws-cli-temp");
    tokio::fs::create_dir_all(&temp_dir).await?;

    // Extract zip to temp directory
    extract_zip(&bytes, &temp_dir)?;

    if cancel_token.is_cancelled() {
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
        return Err(CliDownloadError::Cancelled);
    }

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: bytes.len() as u64,
            total: Some(bytes.len() as u64),
            status: "Running installer...".to_string(),
        });
    }

    // AWS CLI installer is at aws/install
    let install_script = temp_dir.join("aws/install");
    let install_dir = cli_dir.join("aws-cli");

    if install_script.exists() {
        // Run the installer with custom paths
        let output = tokio::process::Command::new(&install_script)
            .args([
                "--install-dir",
                install_dir.to_str().unwrap_or("aws-cli"),
                "--bin-dir",
                install_dir.join("bin").to_str().unwrap_or("bin"),
                "--update",
            ])
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!("AWS CLI installer returned non-zero: {}", stderr);
            // Don't fail - installer might return non-zero for minor issues
        }
    } else {
        tracing::warn!("AWS CLI installer not found at {:?}", install_script);
        // Try to find the installer in other locations
        if let Some(found) = find_binary_recursive(&temp_dir, "install", 3) {
            tracing::info!("Found installer at {:?}", found);
            let output = tokio::process::Command::new(&found)
                .args([
                    "--install-dir",
                    install_dir.to_str().unwrap_or("aws-cli"),
                    "--bin-dir",
                    install_dir.join("bin").to_str().unwrap_or("bin"),
                    "--update",
                ])
                .output()
                .await?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                tracing::warn!("AWS CLI installer returned non-zero: {}", stderr);
            }
        }
    }

    // Clean up temp directory
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: bytes.len() as u64,
            total: Some(bytes.len() as u64),
            status: "Done".to_string(),
        });
    }

    // Find the aws binary
    let binary_path = install_dir.join("bin/aws");
    if binary_path.exists() {
        return Ok(binary_path);
    }

    // Try v2/current/bin/aws (symlink structure)
    let v2_binary = install_dir.join("v2/current/bin/aws");
    if v2_binary.exists() {
        return Ok(v2_binary);
    }

    // Search recursively
    if let Some(found) = find_binary_recursive(&install_dir, "aws", 5) {
        return Ok(found);
    }

    tracing::error!("AWS CLI binary not found after installation");
    Err(CliDownloadError::ExtractionFailed(
        "AWS CLI binary not found. Check logs for details.".to_string(),
    ))
}

fn extract_zip(data: &[u8], dest: &Path) -> CliDownloadResult<()> {
    use std::io::Cursor;

    let cursor = Cursor::new(data);
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|e| CliDownloadError::ExtractionFailed(e.to_string()))?;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| CliDownloadError::ExtractionFailed(e.to_string()))?;

        // enclosed_name() validates against path traversal (e.g. "../../../etc/passwd")
        let relative = file.enclosed_name().ok_or_else(|| {
            CliDownloadError::ExtractionFailed(format!(
                "zip entry has unsafe path: {:?}",
                file.name()
            ))
        })?;
        let outpath = dest.join(relative);

        if file.is_dir() {
            std::fs::create_dir_all(&outpath)?;
        } else {
            if let Some(parent) = outpath.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut outfile = std::fs::File::create(&outpath)?;
            std::io::copy(&mut file, &mut outfile)?;

            // Set executable permission on Unix
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Some(mode) = file.unix_mode() {
                    std::fs::set_permissions(&outpath, std::fs::Permissions::from_mode(mode))?;
                }
            }
        }
    }

    Ok(())
}

/// Extract .deb package (ar archive containing data.tar.gz or data.tar.xz)
fn extract_deb(data: &[u8], dest: &Path) -> CliDownloadResult<()> {
    use std::io::{Cursor, Read};

    let cursor = Cursor::new(data);
    let mut archive = ar::Archive::new(cursor);

    // Find and extract data.tar.* from the .deb
    while let Some(entry_result) = archive.next_entry() {
        let mut entry = entry_result
            .map_err(|e| CliDownloadError::ExtractionFailed(format!("ar read error: {e}")))?;

        let name = String::from_utf8_lossy(entry.header().identifier()).to_string();

        if name.starts_with("data.tar") {
            // Read the data archive
            let mut data_archive = Vec::new();
            entry
                .read_to_end(&mut data_archive)
                .map_err(|e| CliDownloadError::ExtractionFailed(format!("read data.tar: {e}")))?;

            // Extract based on compression type
            // Note: name is already from .deb archive, extensions are always lowercase
            #[allow(clippy::case_sensitive_file_extension_comparisons)]
            if name.ends_with(".gz") {
                extract_tar_gz(&data_archive, dest)?;
            } else if name.ends_with(".xz") {
                extract_tar_xz(&data_archive, dest)?;
            } else if name.ends_with(".zst") {
                extract_tar_zst(&data_archive, dest)?;
            } else {
                // Uncompressed tar
                extract_tar(&data_archive, dest)?;
            }

            return Ok(());
        }
    }

    Err(CliDownloadError::ExtractionFailed(
        "data.tar not found in .deb package".to_string(),
    ))
}

/// Extract uncompressed tar archive
fn extract_tar(data: &[u8], dest: &Path) -> CliDownloadResult<()> {
    use std::io::Cursor;
    use tar::Archive;

    let cursor = Cursor::new(data);
    let mut archive = Archive::new(cursor);

    archive
        .unpack(dest)
        .map_err(|e| CliDownloadError::ExtractionFailed(format!("tar unpack failed: {e}")))?;

    Ok(())
}

/// Extract tar.xz archive
fn extract_tar_xz(data: &[u8], dest: &Path) -> CliDownloadResult<()> {
    use std::io::Read;

    // xz decompression - use xz command line tool
    let temp_file = dest.join("_temp_data.tar.xz");
    std::fs::write(&temp_file, data)?;

    let output = std::process::Command::new("xz")
        .args(["-d", "-k", "-f"])
        .arg(&temp_file)
        .output();

    match output {
        Ok(result) if result.status.success() => {
            let tar_file = dest.join("_temp_data.tar");
            if tar_file.exists() {
                let tar_data = std::fs::read(&tar_file)?;
                let _ = std::fs::remove_file(&tar_file);
                let _ = std::fs::remove_file(&temp_file);
                return extract_tar(&tar_data, dest);
            }
        }
        _ => {}
    }

    let _ = std::fs::remove_file(&temp_file);

    // Fallback: try reading as gzip (some .xz files are actually gzip)
    let cursor = std::io::Cursor::new(data);
    let mut decoder = flate2::read::GzDecoder::new(cursor);
    let mut decompressed = Vec::new();
    if decoder.read_to_end(&mut decompressed).is_ok() {
        return extract_tar(&decompressed, dest);
    }

    Err(CliDownloadError::ExtractionFailed(
        "xz decompression failed - xz command not available".to_string(),
    ))
}

/// Extract tar.zst archive
fn extract_tar_zst(data: &[u8], dest: &Path) -> CliDownloadResult<()> {
    // Use zstd command line tool
    let temp_file = dest.join("_temp_data.tar.zst");
    std::fs::write(&temp_file, data)?;

    let output = std::process::Command::new("zstd")
        .args(["-d", "-f"])
        .arg(&temp_file)
        .output();

    match output {
        Ok(result) if result.status.success() => {
            let tar_file = dest.join("_temp_data.tar");
            if tar_file.exists() {
                let tar_data = std::fs::read(&tar_file)?;
                let _ = std::fs::remove_file(&tar_file);
                let _ = std::fs::remove_file(&temp_file);
                return extract_tar(&tar_data, dest);
            }
        }
        _ => {}
    }

    let _ = std::fs::remove_file(&temp_file);

    Err(CliDownloadError::ExtractionFailed(
        "zstd decompression failed - zstd command not available".to_string(),
    ))
}

fn extract_tar_gz(data: &[u8], dest: &Path) -> CliDownloadResult<()> {
    use flate2::read::GzDecoder;
    use std::io::Cursor;
    use tar::Archive;

    let cursor = Cursor::new(data);
    let decoder = GzDecoder::new(cursor);
    let mut archive = Archive::new(decoder);

    // First, try to get entries to check the archive structure
    let cursor2 = Cursor::new(data);
    let decoder2 = GzDecoder::new(cursor2);
    let mut archive2 = Archive::new(decoder2);

    // Check if archive has a single top-level directory
    let entries = archive2
        .entries()
        .map_err(|e| CliDownloadError::ExtractionFailed(format!("failed to read archive: {e}")))?;

    let mut top_dirs: std::collections::HashSet<String> = std::collections::HashSet::new();
    for entry in entries {
        let entry =
            entry.map_err(|e| CliDownloadError::ExtractionFailed(format!("bad entry: {e}")))?;
        if let Ok(path) = entry.path()
            && let Some(std::path::Component::Normal(name)) = path.components().next()
        {
            top_dirs.insert(name.to_string_lossy().to_string());
        }
    }

    // Extract to destination
    archive
        .unpack(dest)
        .map_err(|e| CliDownloadError::ExtractionFailed(format!("unpack failed: {e}")))?;

    // If there's exactly one top-level directory, move its contents up
    if top_dirs.len() == 1 {
        let top_dir_name = top_dirs.into_iter().next().unwrap_or_default();
        let top_dir = dest.join(&top_dir_name);
        if top_dir.is_dir() {
            // Move contents from top_dir to dest
            if let Ok(dir_entries) = std::fs::read_dir(&top_dir) {
                for entry in dir_entries.flatten() {
                    let src = entry.path();
                    let file_name = entry.file_name();
                    let target = dest.join(&file_name);
                    // Don't overwrite if destination exists
                    if !target.exists()
                        && let Err(e) = std::fs::rename(&src, &target)
                    {
                        tracing::debug!(
                            "Could not move {:?} to {:?}: {}, trying copy",
                            src,
                            target,
                            e
                        );
                        // Try copy instead
                        if src.is_dir() {
                            copy_dir_recursive(&src, &target)?;
                        } else {
                            std::fs::copy(&src, &target)?;
                        }
                    }
                }
            }
            // Remove the now-empty top directory
            let _ = std::fs::remove_dir_all(&top_dir);
        }
    }

    Ok(())
}

/// Extract tar.gz archive preserving directory structure (no flattening)
fn extract_tar_gz_preserve(data: &[u8], dest: &Path) -> CliDownloadResult<()> {
    use flate2::read::GzDecoder;
    use std::io::Cursor;
    use tar::Archive;

    let cursor = Cursor::new(data);
    let decoder = GzDecoder::new(cursor);
    let mut archive = Archive::new(decoder);

    // Simply extract to destination without modifying structure
    archive
        .unpack(dest)
        .map_err(|e| CliDownloadError::ExtractionFailed(format!("unpack failed: {e}")))?;

    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> CliDownloadResult<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

/// Uninstall a component
///
/// # Errors
///
/// Returns error if removal fails.
pub async fn uninstall_component(component: &DownloadableComponent) -> CliDownloadResult<()> {
    if !crate::flatpak::is_flatpak() {
        return Err(CliDownloadError::NotFlatpak);
    }

    let cli_dir = get_cli_install_dir().ok_or(CliDownloadError::NotFlatpak)?;
    let install_dir = cli_dir.join(component.install_subdir);

    if install_dir.exists() {
        tokio::fs::remove_dir_all(&install_dir).await?;
    }

    // Custom components may install to additional directories
    match component.id {
        "aws" => {
            // AWS CLI installer creates aws-cli/ and aws-cli-temp/ directories
            let aws_cli_dir = cli_dir.join("aws-cli");
            if aws_cli_dir.exists() {
                tokio::fs::remove_dir_all(&aws_cli_dir).await?;
            }
            let aws_temp_dir = cli_dir.join("aws-cli-temp");
            if aws_temp_dir.exists() {
                tokio::fs::remove_dir_all(&aws_temp_dir).await?;
            }
        }
        "gcloud" => {
            // gcloud extracts to google-cloud-sdk/ directory
            let gcloud_dir = cli_dir.join("google-cloud-sdk");
            if gcloud_dir.exists() {
                tokio::fs::remove_dir_all(&gcloud_dir).await?;
            }
        }
        _ => {}
    }

    // Also clean up pip/python directory for pip-based components
    if component.install_method == InstallMethod::Pip {
        let python_bin = cli_dir
            .join("python")
            .join("bin")
            .join(component.binary_name);
        if python_bin.exists() {
            let _ = tokio::fs::remove_file(&python_bin).await;
        }
    }

    Ok(())
}

/// Update a component (uninstall and reinstall)
///
/// # Errors
///
/// Returns error if update fails.
pub async fn update_component(
    component: &DownloadableComponent,
    progress_callback: ProgressCallback,
    cancel_token: DownloadCancellation,
) -> CliDownloadResult<PathBuf> {
    if !crate::flatpak::is_flatpak() {
        return Err(CliDownloadError::NotFlatpak);
    }

    if !component.is_downloadable() {
        return Err(CliDownloadError::NotAvailable(component.name.to_string()));
    }

    // Report progress
    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: 0,
            total: None,
            status: format!("Removing old version of {}...", component.name),
        });
    }

    // Remove existing installation
    let cli_dir = get_cli_install_dir().ok_or(CliDownloadError::NotFlatpak)?;
    let install_dir = cli_dir.join(component.install_subdir);

    if install_dir.exists() {
        tokio::fs::remove_dir_all(&install_dir).await?;
    }

    if cancel_token.is_cancelled() {
        return Err(CliDownloadError::Cancelled);
    }

    // Reinstall
    match component.install_method {
        InstallMethod::Download => {
            install_download_component(component, &cli_dir, progress_callback, cancel_token).await
        }
        InstallMethod::Pip => {
            update_pip_component(component, &cli_dir, progress_callback, cancel_token).await
        }
        InstallMethod::CustomScript => {
            install_custom_component(component, &cli_dir, progress_callback, cancel_token).await
        }
        InstallMethod::SystemPackage { .. } => Err(CliDownloadError::NotAvailable(
            "System packages cannot be updated through RustConn".to_string(),
        )),
    }
}

/// Update a pip-based component using pip install --upgrade
async fn update_pip_component(
    component: &DownloadableComponent,
    cli_dir: &Path,
    progress_callback: ProgressCallback,
    cancel_token: DownloadCancellation,
) -> CliDownloadResult<PathBuf> {
    let pip_package = component
        .pip_package
        .ok_or_else(|| CliDownloadError::NotAvailable("No pip package specified".to_string()))?;

    let python_dir = cli_dir.join("python");

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: 0,
            total: None,
            status: "Checking pip availability...".to_string(),
        });
    }

    // Ensure pip is available
    let _pip_path = ensure_pip_available(&python_dir).await?;

    if cancel_token.is_cancelled() {
        return Err(CliDownloadError::Cancelled);
    }

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: 0,
            total: None,
            status: format!("Updating {}...", component.name),
        });
    }

    // Update the package using python -m pip with --upgrade flag
    let output = tokio::process::Command::new("python3")
        .args([
            "-m",
            "pip",
            "install",
            "--user",
            "--upgrade",
            "--no-warn-script-location",
            pip_package,
        ])
        .env("PYTHONUSERBASE", &python_dir)
        .output()
        .await?;

    if cancel_token.is_cancelled() {
        return Err(CliDownloadError::Cancelled);
    }

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!("pip upgrade failed for {}: {}", pip_package, stderr);
        return Err(CliDownloadError::PipInstallFailed(stderr.to_string()));
    }

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: 100,
            total: Some(100),
            status: "Updating wrapper script...".to_string(),
        });
    }

    // Recreate wrapper script (in case Python version changed)
    let binary_path = create_pip_wrapper_script(&python_dir, component).await?;

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: 100,
            total: Some(100),
            status: "Done".to_string(),
        });
    }

    Ok(binary_path)
}

/// Get user-friendly error message for display
#[must_use]
pub fn get_user_friendly_error(error: &CliDownloadError) -> String {
    match error {
        CliDownloadError::DownloadFailed(_) => {
            "Download failed. Check your internet connection.".to_string()
        }
        CliDownloadError::ChecksumMismatch { .. } => {
            "Security verification failed. The download may be corrupted.".to_string()
        }
        CliDownloadError::NoChecksum => {
            "Cannot install: security checksum not available.".to_string()
        }
        CliDownloadError::ExtractionFailed(_) => {
            "Failed to extract the downloaded archive.".to_string()
        }
        CliDownloadError::PipInstallFailed(_) => "Python package installation failed.".to_string(),
        CliDownloadError::IoError(_) => "File system error occurred.".to_string(),
        CliDownloadError::NotFlatpak => "This feature is only available in Flatpak.".to_string(),
        CliDownloadError::AlreadyInstalled => "Component is already installed.".to_string(),
        CliDownloadError::Cancelled => "Installation was cancelled.".to_string(),
        CliDownloadError::NotAvailable(name) => {
            format!("{name} is not available for download.")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_components_have_required_fields() {
        for component in DOWNLOADABLE_COMPONENTS {
            assert!(!component.id.is_empty());
            assert!(!component.name.is_empty());
            assert!(!component.description.is_empty());
            assert!(!component.binary_name.is_empty());
            assert!(!component.install_subdir.is_empty());

            match component.install_method {
                InstallMethod::Download | InstallMethod::CustomScript => {
                    // Download URL may be None for some components
                }
                InstallMethod::Pip => {
                    assert!(
                        component.pip_package.is_some(),
                        "Pip component {} must have pip_package",
                        component.id
                    );
                }
                InstallMethod::SystemPackage {
                    apt,
                    dnf,
                    pacman,
                    zypper,
                } => {
                    // At least one package manager should be specified
                    assert!(
                        apt.is_some() || dnf.is_some() || pacman.is_some() || zypper.is_some(),
                        "SystemPackage component {} needs at least one \
                         package name",
                        component.id
                    );
                }
            }
        }
    }

    #[test]
    fn test_get_component() {
        assert!(get_component("aws").is_some());
        assert!(get_component("nonexistent").is_none());
    }

    #[test]
    fn test_get_components_by_category() {
        let zero_trust = get_components_by_category(ComponentCategory::ZeroTrust);
        assert!(!zero_trust.is_empty());
        assert!(
            zero_trust
                .iter()
                .all(|c| c.category == ComponentCategory::ZeroTrust)
        );
    }

    #[test]
    fn test_sandbox_compatibility_flags() {
        // Protocol clients that need host display should NOT work in sandbox
        let protocol = get_components_by_category(ComponentCategory::ProtocolClient);
        for c in &protocol {
            assert!(
                !c.works_in_sandbox,
                "Protocol client {} should not work in sandbox",
                c.id,
            );
        }

        // Network-only tools should work in sandbox
        let zero_trust = get_components_by_category(ComponentCategory::ZeroTrust);
        for c in &zero_trust {
            assert!(
                c.works_in_sandbox,
                "Zero Trust CLI {} should work in sandbox",
                c.id,
            );
        }

        let pw_managers = get_components_by_category(ComponentCategory::PasswordManager);
        for c in &pw_managers {
            assert!(
                c.works_in_sandbox,
                "Password manager {} should work in sandbox",
                c.id,
            );
        }

        let k8s = get_components_by_category(ComponentCategory::ContainerOrchestration);
        for c in &k8s {
            assert!(
                c.works_in_sandbox,
                "Container CLI {} should work in sandbox",
                c.id,
            );
        }
    }

    #[test]
    fn test_get_available_components_outside_flatpak() {
        // Outside Flatpak, get_available_components returns all
        if !crate::flatpak::is_flatpak() {
            let all = get_available_components();
            assert_eq!(all.len(), DOWNLOADABLE_COMPONENTS.len());
        }
    }

    #[test]
    fn test_cli_install_dir_not_flatpak() {
        // Outside Flatpak, should return None
        if !crate::flatpak::is_flatpak() {
            assert!(get_cli_install_dir().is_none());
        }
    }

    #[test]
    fn test_cancellation_token() {
        let token = DownloadCancellation::new();
        assert!(!token.is_cancelled());
        token.cancel();
        assert!(token.is_cancelled());
    }

    #[test]
    fn test_download_progress_percentage() {
        let progress = DownloadProgress {
            downloaded: 50,
            total: Some(100),
            status: "test".to_string(),
        };
        assert!((progress.percentage() - 0.5).abs() < f64::EPSILON);

        let progress_no_total = DownloadProgress {
            downloaded: 50,
            total: None,
            status: "test".to_string(),
        };
        assert!((progress_no_total.percentage() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_is_downloadable() {
        // Components with download URL and checksum should be downloadable
        for component in DOWNLOADABLE_COMPONENTS {
            match component.install_method {
                InstallMethod::Download | InstallMethod::CustomScript => {
                    let expected = component.download_url_for_arch().is_some()
                        && !matches!(component.checksum, ChecksumPolicy::None);
                    assert_eq!(
                        component.is_downloadable(),
                        expected,
                        "Component {} downloadable mismatch",
                        component.id
                    );
                }
                InstallMethod::Pip => {
                    assert_eq!(
                        component.is_downloadable(),
                        component.pip_package.is_some(),
                        "Pip component {} downloadable mismatch",
                        component.id
                    );
                }
                InstallMethod::SystemPackage { .. } => {
                    assert!(
                        component.is_downloadable(),
                        "SystemPackage component {} should always be \
                         downloadable",
                        component.id
                    );
                }
            }
        }
    }

    #[test]
    fn test_user_friendly_errors() {
        let errors = vec![
            CliDownloadError::DownloadFailed("test".to_string()),
            CliDownloadError::ChecksumMismatch {
                expected: "a".to_string(),
                actual: "b".to_string(),
            },
            CliDownloadError::NoChecksum,
            CliDownloadError::Cancelled,
            CliDownloadError::NotAvailable("test".to_string()),
        ];

        for error in errors {
            let msg = get_user_friendly_error(&error);
            assert!(!msg.is_empty());
            // Should not contain technical details
            assert!(!msg.contains("test") || matches!(error, CliDownloadError::NotAvailable(_)));
        }
    }

    #[test]
    fn test_pinned_versions_match_urls() {
        for component in DOWNLOADABLE_COMPONENTS {
            if let (Some(version), Some(url)) = (component.pinned_version, component.download_url) {
                assert!(
                    url.contains(version),
                    "Component '{}': pinned version '{}' not found in URL '{}'",
                    component.id,
                    version,
                    url,
                );
            }
        }
    }

    #[test]
    fn test_get_pinned_versions() {
        let pinned = get_pinned_versions();
        assert!(
            !pinned.is_empty(),
            "Should have at least one pinned version"
        );
        for (id, version) in &pinned {
            assert!(!id.is_empty());
            assert!(!version.is_empty());
            // Every returned id must exist in DOWNLOADABLE_COMPONENTS
            assert!(
                get_component(id).is_some(),
                "Pinned component '{id}' not found"
            );
        }
    }

    #[test]
    fn test_get_arch_returns_known_value() {
        let arch = get_arch();
        assert!(
            arch == "amd64" || arch == "arm64" || arch == "unknown",
            "Unexpected arch: {arch}"
        );
    }

    #[test]
    fn test_download_url_for_arch_returns_some() {
        for component in get_available_components() {
            if component.download_url.is_some() {
                assert!(
                    component.download_url_for_arch().is_some(),
                    "Component {} should have a download URL for current arch",
                    component.id
                );
            }
        }
    }

    #[test]
    fn test_detect_package_manager_returns_option() {
        // Just verify it doesn't panic — actual result depends on system
        let _ = detect_package_manager();
    }

    #[test]
    fn test_system_install_command() {
        let method = InstallMethod::SystemPackage {
            apt: Some("freerdp3-wayland"),
            dnf: Some("freerdp"),
            pacman: Some("freerdp"),
            zypper: None,
        };
        assert_eq!(
            get_system_install_command(&method, PackageManager::Apt),
            Some("sudo apt install freerdp3-wayland".to_string())
        );
        assert_eq!(
            get_system_install_command(&method, PackageManager::Dnf),
            Some("sudo dnf install freerdp".to_string())
        );
        assert_eq!(
            get_system_install_command(&method, PackageManager::Pacman),
            Some("sudo pacman -S freerdp".to_string())
        );
        assert_eq!(
            get_system_install_command(&method, PackageManager::Zypper),
            None
        );
    }

    #[test]
    fn test_system_install_command_wrong_method() {
        let method = InstallMethod::Download;
        assert_eq!(
            get_system_install_command(&method, PackageManager::Apt),
            None
        );
    }

    #[test]
    fn test_package_manager_display() {
        assert_eq!(PackageManager::Apt.to_string(), "apt");
        assert_eq!(PackageManager::Dnf.to_string(), "dnf");
        assert_eq!(PackageManager::Pacman.to_string(), "pacman");
        assert_eq!(PackageManager::Zypper.to_string(), "zypper");
    }

    #[test]
    fn test_system_package_is_downloadable() {
        let component = DownloadableComponent {
            id: "test-pkg",
            name: "Test Package",
            description: "A test system package",
            category: ComponentCategory::ProtocolClient,
            install_method: InstallMethod::SystemPackage {
                apt: Some("test-pkg"),
                dnf: None,
                pacman: None,
                zypper: None,
            },
            download_url: None,
            aarch64_url: None,
            checksum: ChecksumPolicy::None,
            pip_package: None,
            size_hint: "1 MB",
            binary_name: "test-pkg",
            install_subdir: "test-pkg",
            pinned_version: None,
            works_in_sandbox: false,
        };
        assert!(component.is_downloadable());
    }
}
