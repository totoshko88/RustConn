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

mod components;
mod detection;
mod download;
mod extract;
mod install;
mod install_cloud;
mod install_custom;
mod install_pip;
mod uninstall;
mod update;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use thiserror::Error;

/// HTTP timeout for CLI download index/redirect lookups.
///
/// 15 seconds covers the worst-case latency for upstream metadata
/// endpoints (GitHub releases, AWS S3, Tailscale package index) under
/// modest network conditions while still failing fast in CI sandboxes.
pub(crate) const HTTP_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15);

pub use self::components::{
    DOWNLOADABLE_COMPONENTS, get_available_components, get_component, get_components_by_category,
    get_installation_status, get_pinned_versions,
};
pub use self::detection::{PackageManager, detect_package_manager, get_system_install_command};
pub use self::extract::find_binary_recursive;

use self::install::install_download_component;
use self::install_pip::install_pip_component;

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

    /// Not running in a confined sandbox (snap or Flatpak)
    #[error("CLI download is only available inside a snap or Flatpak sandbox")]
    NotSandboxed,

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
        extract::find_binary_recursive(&install_dir, self.binary_name, 5)
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
    /// On aarch64, returns the ARM64-specific URL. If no ARM64 URL is
    /// available, returns `None` — the component cannot be downloaded on
    /// this architecture. On x86_64, returns the default download URL.
    #[must_use]
    pub fn download_url_for_arch(&self) -> Option<&'static str> {
        if cfg!(target_arch = "aarch64") {
            self.aarch64_url
        } else {
            self.download_url
        }
    }

    /// Returns `true` if this component has a download available for the
    /// current CPU architecture.
    ///
    /// Pip and system-package components are always architecture-compatible.
    /// Download-based components require an explicit URL for the target arch.
    #[must_use]
    pub fn is_available_for_current_arch(&self) -> bool {
        match self.install_method {
            InstallMethod::Download | InstallMethod::CustomScript => {
                self.download_url_for_arch().is_some()
            }
            // pip and system packages are arch-independent
            InstallMethod::Pip | InstallMethod::SystemPackage { .. } => true,
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

/// Get the CLI installation directory
///
/// Returns a writable per-application directory where external CLI tools are
/// installed at runtime. Strict sandboxes do not expose host binaries, so the
/// app downloads them here on demand.
///
/// - Flatpak: `~/.var/app/io.github.totoshko88.RustConn/cli/`
/// - snap: `$SNAP_USER_DATA/cli/`
///
/// Returns `None` outside a sandbox.
#[must_use]
pub fn get_cli_install_dir() -> Option<PathBuf> {
    if crate::flatpak::is_flatpak() {
        // In Flatpak, XDG_DATA_HOME points to ~/.var/app/<app-id>/data
        // We want ~/.var/app/<app-id>/cli
        return std::env::var("XDG_DATA_HOME").ok().map(|data_home| {
            PathBuf::from(data_home)
                .parent()
                .map(|p| p.join("cli"))
                .unwrap_or_else(|| PathBuf::from("cli"))
        });
    }

    // In a snap, use the writable per-user data directory.
    crate::snap::get_cli_install_dir()
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

    let path_subdirs = [
        "python/bin",
        "aws-cli/bin",
        "aws-cli/v2/current/bin",
        "ssm-plugin/usr/local/sessionmanagerplugin/bin",
        "google-cloud-sdk/bin",
        "teleport",
        "tailscale",
        "cloudflared",
        "boundary",
        "bitwarden",
        "1password",
        "tigervnc/usr/bin",
        "kubectl",
        "hoop",
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

    // On macOS, GUI apps have minimal PATH. Add Homebrew and common tool dirs.
    #[cfg(target_os = "macos")]
    let extra_paths: Vec<&str> = {
        let macos_dirs: &[&str] = &[
            "/opt/homebrew/bin",
            "/opt/homebrew/sbin",
            "/usr/local/bin",
            "/Applications/KeePassXC.app/Contents/MacOS",
        ];
        macos_dirs
            .iter()
            .filter(|dir| std::path::Path::new(*dir).exists() && !current_path.contains(*dir))
            .copied()
            .collect()
    };

    #[cfg(not(target_os = "macos"))]
    let extra_paths: &[&str] = &[];

    let cli_path: String = cli_dirs
        .iter()
        .filter_map(|p| p.to_str())
        .chain(extra_paths.iter().copied())
        .collect::<Vec<_>>()
        .join(":");

    if cli_path.is_empty() {
        current_path
    } else if current_path.is_empty() {
        cli_path
    } else {
        format!("{cli_path}:{current_path}")
    }
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
    if !crate::is_sandboxed() {
        return Err(CliDownloadError::NotSandboxed);
    }

    if component.is_installed() {
        return Err(CliDownloadError::AlreadyInstalled);
    }

    if !component.is_downloadable() {
        return Err(CliDownloadError::NotAvailable(component.name.to_string()));
    }

    let cli_dir = get_cli_install_dir().ok_or(CliDownloadError::NotSandboxed)?;

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
            install_custom::install_custom_component(
                component,
                &cli_dir,
                progress_callback,
                cancel_token,
            )
            .await
        }
        InstallMethod::SystemPackage { .. } => Err(CliDownloadError::NotAvailable(
            "System packages must be installed via the system \
                 package manager"
                .to_string(),
        )),
    }
}

/// Uninstall a component
///
/// # Errors
///
/// Returns error if removal fails.
pub async fn uninstall_component(component: &DownloadableComponent) -> CliDownloadResult<()> {
    uninstall::uninstall_component_impl(component).await
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
    update::update_component_impl(component, progress_callback, cancel_token).await
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
        CliDownloadError::NotSandboxed => {
            "This feature is only available in a snap or Flatpak.".to_string()
        }
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
                InstallMethod::Download | InstallMethod::CustomScript => {}
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
        let protocol = get_components_by_category(ComponentCategory::ProtocolClient);
        for c in &protocol {
            assert!(
                !c.works_in_sandbox,
                "Protocol client {} should not work in sandbox",
                c.id,
            );
        }

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
        if !crate::flatpak::is_flatpak() {
            let all = get_available_components();
            let expected = DOWNLOADABLE_COMPONENTS
                .iter()
                .filter(|c| c.is_available_for_current_arch())
                .count();
            assert_eq!(all.len(), expected);
        }
    }

    #[test]
    fn test_cli_install_dir_not_flatpak() {
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
