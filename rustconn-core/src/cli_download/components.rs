use super::{ChecksumPolicy, ComponentCategory, DownloadableComponent, InstallMethod};

/// All downloadable components
///
/// Note: Components without SHA256 checksums are marked as not downloadable.
/// SPICE viewer (remote-viewer) is not available as standalone download.
/// FreeRDP does not provide pre-built Linux binaries - users should install via system package.
pub static DOWNLOADABLE_COMPONENTS: &[DownloadableComponent] = &[
    // Protocol clients (optional for external fallback)
    DownloadableComponent {
        id: "vncviewer",
        name: "TigerVNC Viewer",
        description: "Optional for external VNC connections",
        category: ComponentCategory::ProtocolClient,
        install_method: InstallMethod::Download,
        download_url: Some(
            "https://sourceforge.net/projects/tigervnc/files/stable/1.16.2/\
             tigervnc-1.16.2.x86_64.tar.gz/download",
        ),
        aarch64_url: None,
        checksum: ChecksumPolicy::Static(
            "5b70c84baefc09a030cfc78315c34ccb55b2a0dde4092b7da67a1962c5f0dea6",
        ),
        pip_package: None,
        size_hint: "~5 MB",
        binary_name: "vncviewer",
        install_subdir: "tigervnc",
        pinned_version: Some("1.16.2"),
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
        install_method: InstallMethod::CustomScript,
        download_url: Some("https://api.github.com/repos/gravitational/teleport/releases/latest"),
        aarch64_url: Some("https://api.github.com/repos/gravitational/teleport/releases/latest"),
        checksum: ChecksumPolicy::SkipLatest,
        pip_package: None,
        size_hint: "~100 MB",
        binary_name: "tsh",
        install_subdir: "teleport",
        pinned_version: None,
        works_in_sandbox: true,
    },
    DownloadableComponent {
        id: "tailscale",
        name: "Tailscale",
        description: "For Tailscale SSH",
        category: ComponentCategory::ZeroTrust,
        install_method: InstallMethod::CustomScript,
        download_url: Some("https://pkgs.tailscale.com/stable/"),
        aarch64_url: Some("https://pkgs.tailscale.com/stable/"),
        checksum: ChecksumPolicy::SkipLatest,
        pip_package: None,
        size_hint: "~25 MB",
        binary_name: "tailscale",
        install_subdir: "tailscale",
        pinned_version: None,
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
        install_method: InstallMethod::CustomScript,
        download_url: Some("https://checkpoint-api.hashicorp.com/v1/check/boundary"),
        aarch64_url: Some("https://checkpoint-api.hashicorp.com/v1/check/boundary"),
        checksum: ChecksumPolicy::SkipLatest,
        pip_package: None,
        size_hint: "~50 MB",
        binary_name: "boundary",
        install_subdir: "boundary",
        pinned_version: None,
        works_in_sandbox: true,
    },
    DownloadableComponent {
        id: "hoop",
        name: "Hoop.dev",
        description: "For Hoop.dev access",
        category: ComponentCategory::ZeroTrust,
        install_method: InstallMethod::CustomScript,
        download_url: Some("https://releases.hoop.dev/release/latest.txt"),
        aarch64_url: Some("https://releases.hoop.dev/release/latest.txt"),
        checksum: ChecksumPolicy::SkipLatest,
        pip_package: None,
        size_hint: "~30 MB",
        binary_name: "hoop",
        install_subdir: "hoop",
        pinned_version: None,
        works_in_sandbox: true,
    },
    // Password manager CLIs
    DownloadableComponent {
        id: "bw",
        name: "Bitwarden CLI",
        description: "For Bitwarden integration",
        category: ComponentCategory::PasswordManager,
        install_method: InstallMethod::CustomScript,
        download_url: Some("https://api.github.com/repos/bitwarden/clients/releases"),
        aarch64_url: None,
        checksum: ChecksumPolicy::SkipLatest,
        pip_package: None,
        size_hint: "~50 MB",
        binary_name: "bw",
        install_subdir: "bitwarden",
        pinned_version: None,
        works_in_sandbox: true,
    },
    DownloadableComponent {
        id: "op",
        name: "1Password CLI",
        description: "For 1Password integration",
        category: ComponentCategory::PasswordManager,
        install_method: InstallMethod::CustomScript,
        download_url: Some("https://app-updates.agilebits.com/check/1/0/CLI2/en/2.0.0/N"),
        aarch64_url: Some("https://app-updates.agilebits.com/check/1/0/CLI2/en/2.0.0/N"),
        checksum: ChecksumPolicy::SkipLatest,
        pip_package: None,
        size_hint: "~15 MB",
        binary_name: "op",
        install_subdir: "1password",
        pinned_version: None,
        works_in_sandbox: true,
    },
    // Container orchestration CLIs
    DownloadableComponent {
        id: "kubectl",
        name: "kubectl",
        description: "Kubernetes CLI for pod shell connections",
        category: ComponentCategory::ContainerOrchestration,
        install_method: InstallMethod::CustomScript,
        download_url: Some("https://dl.k8s.io/release/stable.txt"),
        aarch64_url: Some("https://dl.k8s.io/release/stable.txt"),
        checksum: ChecksumPolicy::SkipLatest,
        pip_package: None,
        size_hint: "~50 MB",
        binary_name: "kubectl",
        install_subdir: "kubectl",
        pinned_version: None,
        works_in_sandbox: true,
    },
];

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
/// In a sandbox (snap or Flatpak), excludes components that require host
/// display access (e.g. xfreerdp, vncviewer). On ARM64, excludes components
/// without an ARM64 download URL. Outside a sandbox on x86_64, returns all.
#[must_use]
pub fn get_available_components() -> Vec<&'static DownloadableComponent> {
    DOWNLOADABLE_COMPONENTS
        .iter()
        .filter(|c| {
            if crate::is_sandboxed() && !c.works_in_sandbox {
                return false;
            }
            c.is_available_for_current_arch()
        })
        .collect()
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
