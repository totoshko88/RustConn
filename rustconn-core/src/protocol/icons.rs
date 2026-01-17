//! Cloud provider icon cache for `ZeroTrust` connections
//!
//! This module provides icon caching and provider detection for `ZeroTrust` CLI connections.
//! It supports AWS, GCP, Azure, OCI, and other cloud providers with appropriate icon fallbacks.
//!
//! Also provides protocol-specific icon names for the sidebar display.

use crate::models::ProtocolType;
use std::path::PathBuf;

/// Cloud provider type for icon selection
///
/// Represents the major cloud providers and tools that can be detected
/// from `ZeroTrust` CLI commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CloudProvider {
    /// Amazon Web Services (AWS SSM, etc.)
    Aws,
    /// Google Cloud Platform (GCP IAP, etc.)
    Gcloud,
    /// Microsoft Azure (Bastion, SSH, etc.)
    Azure,
    /// Oracle Cloud Infrastructure (OCI Bastion)
    Oci,
    /// Cloudflare Access
    Cloudflare,
    /// Teleport
    Teleport,
    /// Tailscale
    Tailscale,
    /// `HashiCorp` Boundary
    Boundary,
    /// Generic/unknown provider
    #[default]
    Generic,
}

impl CloudProvider {
    /// Returns the icon name for this provider
    ///
    /// These are standard GTK/Adwaita symbolic icon names that are guaranteed to exist
    /// in all icon themes. Each provider has a unique icon - no duplicates with base
    /// protocols (SSH, RDP, VNC, SPICE).
    ///
    /// Icons must match sidebar.rs `get_protocol_icon()` for consistency.
    #[must_use]
    pub const fn icon_name(self) -> &'static str {
        match self {
            Self::Aws => "network-workgroup-symbolic", // AWS - workgroup
            Self::Gcloud => "weather-overcast-symbolic", // GCP - cloud
            Self::Azure => "weather-few-clouds-symbolic", // Azure - clouds
            Self::Oci => "drive-harddisk-symbolic",    // OCI - harddisk
            Self::Cloudflare => "security-high-symbolic", // Cloudflare - security
            Self::Teleport => "emblem-system-symbolic", // Teleport - system/gear
            Self::Tailscale => "network-vpn-symbolic", // Tailscale - VPN
            Self::Boundary => "dialog-password-symbolic", // Boundary - password/lock
            Self::Generic => "system-run-symbolic",    // Generic - run command
        }
    }

    /// Returns the display name for this provider
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Aws => "AWS",
            Self::Gcloud => "Google Cloud",
            Self::Azure => "Azure",
            Self::Oci => "Oracle Cloud",
            Self::Cloudflare => "Cloudflare",
            Self::Teleport => "Teleport",
            Self::Tailscale => "Tailscale",
            Self::Boundary => "Boundary",
            Self::Generic => "Cloud",
        }
    }

    /// Returns all available providers
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::Aws,
            Self::Gcloud,
            Self::Azure,
            Self::Oci,
            Self::Cloudflare,
            Self::Teleport,
            Self::Tailscale,
            Self::Boundary,
            Self::Generic,
        ]
    }
}

/// Returns the icon name for a given protocol type
///
/// Each protocol has a distinct icon to help users quickly identify connection types.
/// This implements Requirements 7.1, 7.2, 7.3, 7.4 for protocol-specific icons.
///
/// # Arguments
/// * `protocol` - The protocol type to get an icon for
///
/// # Returns
/// A symbolic icon name suitable for GTK icon themes
///
/// # Example
/// ```
/// use rustconn_core::models::ProtocolType;
/// use rustconn_core::protocol::icons::get_protocol_icon;
///
/// assert_eq!(get_protocol_icon(ProtocolType::Ssh), "utilities-terminal-symbolic");
/// assert_eq!(get_protocol_icon(ProtocolType::Rdp), "computer-symbolic");
/// assert_eq!(get_protocol_icon(ProtocolType::Vnc), "video-display-symbolic");
/// assert_eq!(get_protocol_icon(ProtocolType::Spice), "preferences-desktop-remote-desktop-symbolic");
/// ```
#[must_use]
pub const fn get_protocol_icon(protocol: ProtocolType) -> &'static str {
    match protocol {
        ProtocolType::Ssh => "utilities-terminal-symbolic",
        ProtocolType::Rdp => "computer-symbolic",
        ProtocolType::Vnc => "video-display-symbolic",
        ProtocolType::Spice => "preferences-desktop-remote-desktop-symbolic",
        ProtocolType::ZeroTrust => "cloud-symbolic",
    }
}

/// Returns all protocol types with their corresponding icon names
///
/// Useful for testing that all protocols have distinct icons.
#[must_use]
pub const fn all_protocol_icons() -> &'static [(ProtocolType, &'static str)] {
    &[
        (ProtocolType::Ssh, "utilities-terminal-symbolic"),
        (ProtocolType::Rdp, "computer-symbolic"),
        (ProtocolType::Vnc, "video-display-symbolic"),
        (
            ProtocolType::Spice,
            "preferences-desktop-remote-desktop-symbolic",
        ),
        (ProtocolType::ZeroTrust, "cloud-symbolic"),
    ]
}

/// Returns the icon name for a `ZeroTrust` provider
///
/// Each provider has a unique icon that is associative with its branding:
/// - AWS SSM: package/cube (AWS logo shape)
/// - GCP IAP: cloud (Google Cloud)
/// - Azure: window (Microsoft Windows)
/// - OCI: harddisk (Oracle database)
/// - Cloudflare: security shield
/// - Teleport: gear/cog (Teleport logo)
/// - Tailscale: VPN network
/// - Boundary: secure channel/lock
/// - Generic: terminal
#[must_use]
pub const fn get_zero_trust_provider_icon(
    provider: crate::models::ZeroTrustProvider,
) -> &'static str {
    provider.icon_name()
}

/// Detect cloud provider from a CLI command string
///
/// Analyzes the command to determine which cloud provider it belongs to.
/// Returns `CloudProvider::Generic` if no specific provider is detected.
///
/// # Arguments
/// * `command` - The CLI command string to analyze
///
/// # Returns
/// The detected `CloudProvider` based on command content
///
/// # Example
/// ```
/// use rustconn_core::protocol::icons::detect_provider;
/// use rustconn_core::protocol::icons::CloudProvider;
///
/// assert_eq!(detect_provider("aws ssm start-session"), CloudProvider::Aws);
/// assert_eq!(detect_provider("gcloud compute ssh"), CloudProvider::Gcloud);
/// assert_eq!(detect_provider("az network bastion"), CloudProvider::Azure);
/// ```
#[must_use]
pub fn detect_provider(command: &str) -> CloudProvider {
    let cmd_lower = command.to_lowercase();

    // Helper to check if command contains a tool name as a word boundary
    // Handles: "tool args", "/path/to/tool args", "env tool args"
    let contains_tool = |tool: &str| -> bool {
        // Direct start
        cmd_lower.starts_with(&format!("{tool} "))
            // After path separator (e.g., /usr/bin/tool)
            || cmd_lower.contains(&format!("/{tool} "))
            // After space (e.g., env tool)
            || cmd_lower.contains(&format!(" {tool} "))
            // Tool at end after path
            || cmd_lower.ends_with(&format!("/{tool}"))
            // Tool at end after space
            || cmd_lower.ends_with(&format!(" {tool}"))
    };

    // Check for AWS commands - enhanced detection for SSM
    // Patterns: "aws ssm", "aws-ssm", "ssm start-session", instance IDs (i-*, mi-*)
    if contains_tool("aws")
        || cmd_lower.contains("aws ssm")
        || cmd_lower.contains("aws-ssm")
        || cmd_lower.contains("ssm start-session")
        || cmd_lower.contains("ssm-plugin")
        || contains_ec2_instance_id(command)
        || contains_managed_instance_id(command)
    {
        return CloudProvider::Aws;
    }

    // Check for GCP commands - enhanced detection
    // Patterns: "gcloud", "iap-tunnel", "compute ssh"
    if contains_tool("gcloud")
        || cmd_lower.contains("iap-tunnel")
        || cmd_lower.contains("compute ssh")
        || cmd_lower.contains("--tunnel-through-iap")
    {
        return CloudProvider::Gcloud;
    }

    // Check for Tailscale commands BEFORE Azure
    // (tailscale commands may contain "az" as arguments, e.g., "tailscale az @")
    if contains_tool("tailscale") {
        return CloudProvider::Tailscale;
    }

    // Check for Teleport commands BEFORE Azure
    // (tsh commands may contain "az" as arguments, e.g., "tsh az a")
    if contains_tool("tsh") || cmd_lower.contains("teleport") {
        return CloudProvider::Teleport;
    }

    // Check for Boundary commands BEFORE Azure
    // (boundary commands may contain "az" as arguments, e.g., "boundary az -")
    if contains_tool("boundary") || cmd_lower.contains("hashicorp") {
        return CloudProvider::Boundary;
    }

    // Check for Cloudflare commands BEFORE Azure
    // (cloudflared commands may contain "az" as arguments, e.g., "cloudflared az tunnel")
    if contains_tool("cloudflared") || cmd_lower.contains("cloudflare") {
        return CloudProvider::Cloudflare;
    }

    // Check for Azure commands - enhanced detection
    // Patterns: "az ", "azure", "bastion"
    if contains_tool("az")
        || cmd_lower.contains("azure")
        || cmd_lower.contains("bastion ssh")
        || cmd_lower.contains("az ssh")
        || cmd_lower.contains("az network bastion")
    {
        return CloudProvider::Azure;
    }

    // Check for OCI commands
    if contains_tool("oci") || cmd_lower.contains("oracle") {
        return CloudProvider::Oci;
    }

    CloudProvider::Generic
}

/// Checks if the command contains an EC2 instance ID pattern (i-xxxxxxxxxxxxxxxxx)
///
/// EC2 instance IDs start with "i-" followed by 8 or 17 hexadecimal characters.
fn contains_ec2_instance_id(command: &str) -> bool {
    // Look for patterns like "i-" followed by hex characters
    // EC2 instance IDs are either 8 chars (old) or 17 chars (new)
    let cmd_lower = command.to_lowercase();

    // Check for --target i- pattern (common in SSM commands)
    if cmd_lower.contains("--target i-") || cmd_lower.contains("--target=i-") {
        return true;
    }

    // Check for standalone i- pattern with hex characters
    for (idx, _) in cmd_lower.match_indices("i-") {
        let after = &cmd_lower[idx + 2..];
        // Check if followed by at least 8 hex characters
        let hex_count = after.chars().take_while(char::is_ascii_hexdigit).count();
        if hex_count >= 8 {
            return true;
        }
    }

    false
}

/// Checks if the command contains a managed instance ID pattern (mi-xxxxxxxxxxxxxxxxx)
///
/// Managed instance IDs (for on-premises servers registered with SSM) start with "mi-"
/// followed by 17 hexadecimal characters.
fn contains_managed_instance_id(command: &str) -> bool {
    let cmd_lower = command.to_lowercase();

    // Check for --target mi- pattern
    if cmd_lower.contains("--target mi-") || cmd_lower.contains("--target=mi-") {
        return true;
    }

    // Check for standalone mi- pattern with hex characters
    for (idx, _) in cmd_lower.match_indices("mi-") {
        let after = &cmd_lower[idx + 3..];
        // Check if followed by at least 17 hex characters
        let hex_count = after.chars().take_while(char::is_ascii_hexdigit).count();
        if hex_count >= 17 {
            return true;
        }
    }

    false
}

/// Cloud provider icon cache
///
/// Manages cached icons for cloud providers in the user's cache directory.
/// Icons are stored at `~/.cache/rustconn/icons/`.
#[derive(Debug, Clone)]
pub struct ProviderIconCache {
    /// Directory where icons are cached
    cache_dir: PathBuf,
}

impl Default for ProviderIconCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ProviderIconCache {
    /// Create a new provider icon cache
    ///
    /// Uses the XDG cache directory (`~/.cache/rustconn/icons/`) by default.
    #[must_use]
    pub fn new() -> Self {
        let cache_dir = dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from(".cache"))
            .join("rustconn")
            .join("icons");
        Self { cache_dir }
    }

    /// Create a provider icon cache with a custom cache directory
    #[must_use]
    pub const fn with_cache_dir(cache_dir: PathBuf) -> Self {
        Self { cache_dir }
    }

    /// Get the cache directory path
    #[must_use]
    pub const fn cache_dir(&self) -> &PathBuf {
        &self.cache_dir
    }

    /// Get the icon path for a provider
    ///
    /// Returns the path where the icon should be cached.
    /// Note: This does not check if the icon actually exists.
    #[must_use]
    pub fn get_icon_path(&self, provider: CloudProvider) -> PathBuf {
        let icon_name = match provider {
            CloudProvider::Aws => "aws-logo.svg",
            CloudProvider::Gcloud => "gcloud-logo.svg",
            CloudProvider::Azure => "azure-logo.svg",
            CloudProvider::Oci => "oci-logo.svg",
            CloudProvider::Cloudflare => "cloudflare-logo.svg",
            CloudProvider::Teleport => "teleport-logo.svg",
            CloudProvider::Tailscale => "tailscale-logo.svg",
            CloudProvider::Boundary => "boundary-logo.svg",
            CloudProvider::Generic => "cloud-symbolic.svg",
        };
        self.cache_dir.join(icon_name)
    }

    /// Check if an icon is cached for a provider
    #[must_use]
    pub fn has_cached_icon(&self, provider: CloudProvider) -> bool {
        self.get_icon_path(provider).exists()
    }

    /// Ensure the cache directory exists
    ///
    /// # Errors
    /// Returns an error if the directory cannot be created.
    pub fn ensure_cache_dir(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.cache_dir)
    }

    /// Get the GTK icon name for a provider
    ///
    /// Returns the symbolic icon name that can be used with GTK icon themes.
    /// Falls back to "cloud-symbolic" if no specific icon is available.
    #[must_use]
    pub const fn get_gtk_icon_name(&self, provider: CloudProvider) -> &'static str {
        // For now, we use the symbolic icon names
        // In the future, we could check if custom icons are cached
        provider.icon_name()
    }

    /// Detect provider from command and get the appropriate icon name
    ///
    /// Convenience method that combines provider detection with icon lookup.
    #[must_use]
    pub fn get_icon_for_command(&self, command: &str) -> &'static str {
        let provider = detect_provider(command);
        self.get_gtk_icon_name(provider)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_provider_aws() {
        // Basic AWS SSM commands
        assert_eq!(detect_provider("aws ssm start-session"), CloudProvider::Aws);
        assert_eq!(
            detect_provider("aws ssm start-session --target i-123"),
            CloudProvider::Aws
        );
        assert_eq!(
            detect_provider("/usr/bin/aws ssm start-session"),
            CloudProvider::Aws
        );

        // AWS SSM with instance ID patterns
        assert_eq!(
            detect_provider("aws ssm start-session --target i-0123456789abcdef0"),
            CloudProvider::Aws
        );
        assert_eq!(detect_provider("--target i-abcdef12"), CloudProvider::Aws);
        assert_eq!(
            detect_provider("ssm start-session --target mi-01234567890abcdef0"),
            CloudProvider::Aws
        );

        // AWS-SSM hyphenated form
        assert_eq!(detect_provider("aws-ssm-plugin"), CloudProvider::Aws);
        assert_eq!(detect_provider("aws-ssm start-session"), CloudProvider::Aws);

        // SSM plugin patterns
        assert_eq!(
            detect_provider("session-manager-plugin"),
            CloudProvider::Generic
        ); // This shouldn't match without explicit SSM
        assert_eq!(
            detect_provider("ssm-plugin --target i-12345678"),
            CloudProvider::Aws
        );
    }

    #[test]
    fn test_detect_provider_gcloud() {
        // Basic gcloud commands
        assert_eq!(detect_provider("gcloud compute ssh"), CloudProvider::Gcloud);
        assert_eq!(
            detect_provider("gcloud compute ssh instance --zone us-central1-a"),
            CloudProvider::Gcloud
        );

        // IAP tunnel patterns
        assert_eq!(
            detect_provider("gcloud compute start-iap-tunnel"),
            CloudProvider::Gcloud
        );
        assert_eq!(
            detect_provider("iap-tunnel --project my-project"),
            CloudProvider::Gcloud
        );
        assert_eq!(
            detect_provider("--tunnel-through-iap"),
            CloudProvider::Gcloud
        );

        // Compute SSH patterns
        assert_eq!(
            detect_provider("compute ssh my-instance --zone us-west1-b"),
            CloudProvider::Gcloud
        );
    }

    #[test]
    fn test_detect_provider_azure() {
        // Basic az commands
        assert_eq!(
            detect_provider("az network bastion ssh"),
            CloudProvider::Azure
        );
        assert_eq!(detect_provider("az ssh vm"), CloudProvider::Azure);

        // Azure-specific patterns
        assert_eq!(
            detect_provider("az network bastion ssh --name mybastion"),
            CloudProvider::Azure
        );
        assert_eq!(detect_provider("azure cli command"), CloudProvider::Azure);
        assert_eq!(
            detect_provider("bastion ssh --resource-group mygroup"),
            CloudProvider::Azure
        );

        // Azure SSH patterns
        assert_eq!(
            detect_provider("az ssh vm --name myvm --resource-group myrg"),
            CloudProvider::Azure
        );
    }

    #[test]
    fn test_detect_provider_oci() {
        assert_eq!(
            detect_provider("oci bastion session create"),
            CloudProvider::Oci
        );
    }

    #[test]
    fn test_detect_provider_cloudflare() {
        assert_eq!(
            detect_provider("cloudflared access ssh"),
            CloudProvider::Cloudflare
        );
    }

    #[test]
    fn test_detect_provider_teleport() {
        assert_eq!(
            detect_provider("tsh ssh user@host"),
            CloudProvider::Teleport
        );
    }

    #[test]
    fn test_detect_provider_tailscale() {
        assert_eq!(
            detect_provider("tailscale ssh user@host"),
            CloudProvider::Tailscale
        );
    }

    #[test]
    fn test_detect_provider_boundary() {
        assert_eq!(
            detect_provider("boundary connect ssh"),
            CloudProvider::Boundary
        );
    }

    #[test]
    fn test_detect_provider_generic() {
        assert_eq!(detect_provider("ssh user@host"), CloudProvider::Generic);
        assert_eq!(detect_provider("custom-command"), CloudProvider::Generic);
        assert_eq!(detect_provider(""), CloudProvider::Generic);
    }

    #[test]
    fn test_cloud_provider_icon_name() {
        // Using standard GTK icons that exist in all themes
        // Each provider has a unique icon - no duplicates with base protocols
        assert_eq!(CloudProvider::Aws.icon_name(), "network-workgroup-symbolic");
        assert_eq!(
            CloudProvider::Gcloud.icon_name(),
            "weather-overcast-symbolic"
        );
        assert_eq!(
            CloudProvider::Azure.icon_name(),
            "weather-few-clouds-symbolic"
        );
        assert_eq!(CloudProvider::Generic.icon_name(), "system-run-symbolic");
    }

    #[test]
    fn test_cloud_provider_display_name() {
        assert_eq!(CloudProvider::Aws.display_name(), "AWS");
        assert_eq!(CloudProvider::Gcloud.display_name(), "Google Cloud");
        assert_eq!(CloudProvider::Azure.display_name(), "Azure");
        assert_eq!(CloudProvider::Generic.display_name(), "Cloud");
    }

    #[test]
    fn test_provider_icon_cache_new() {
        let cache = ProviderIconCache::new();
        assert!(cache.cache_dir().ends_with("rustconn/icons"));
    }

    #[test]
    fn test_provider_icon_cache_get_icon_path() {
        let cache = ProviderIconCache::with_cache_dir(PathBuf::from("/tmp/test"));
        assert_eq!(
            cache.get_icon_path(CloudProvider::Aws),
            PathBuf::from("/tmp/test/aws-logo.svg")
        );
        assert_eq!(
            cache.get_icon_path(CloudProvider::Generic),
            PathBuf::from("/tmp/test/cloud-symbolic.svg")
        );
    }

    #[test]
    fn test_provider_icon_cache_get_icon_for_command() {
        let cache = ProviderIconCache::new();
        // Using standard GTK icons that exist in all themes
        assert_eq!(
            cache.get_icon_for_command("aws ssm start-session"),
            "network-workgroup-symbolic"
        );
        assert_eq!(
            cache.get_icon_for_command("gcloud compute ssh"),
            "weather-overcast-symbolic"
        );
        assert_eq!(
            cache.get_icon_for_command("ssh user@host"),
            "system-run-symbolic"
        );
    }

    #[test]
    fn test_cloud_provider_all() {
        let all = CloudProvider::all();
        assert!(all.contains(&CloudProvider::Aws));
        assert!(all.contains(&CloudProvider::Gcloud));
        assert!(all.contains(&CloudProvider::Azure));
        assert!(all.contains(&CloudProvider::Generic));
        assert_eq!(all.len(), 9);
    }
}
