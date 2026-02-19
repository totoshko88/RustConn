//! `FreeRDP` command builder for external mode RDP connections
//!
//! This module provides functions to build `FreeRDP` command-line arguments
//! for external mode RDP connections. It supports window decorations,
//! geometry persistence, and various RDP options.
//!
//! # Requirements Coverage
//!
//! - Requirement 6.1: `/decorations` flag for window controls
//! - Requirement 6.2: Window geometry persistence (save on close)
//! - Requirement 6.3: Window geometry restoration (load on start)
//! - Requirement 6.4: Respect `remember_window_position` setting

use crate::models::WindowGeometry;
use std::path::PathBuf;

/// A shared folder for RDP drive redirection
#[derive(Debug, Clone)]
pub struct SharedFolder {
    /// Local directory path to share
    pub local_path: PathBuf,
    /// Share name visible in the remote session
    pub share_name: String,
}

/// Configuration for `FreeRDP` external mode
#[derive(Debug, Clone, Default)]
pub struct FreeRdpConfig {
    /// Target hostname or IP address
    pub host: String,
    /// Target port (default: 3389)
    pub port: u16,
    /// Username for authentication
    pub username: Option<String>,
    /// Password for authentication
    pub password: Option<String>,
    /// Domain for authentication
    pub domain: Option<String>,
    /// Desired width in pixels
    pub width: u32,
    /// Desired height in pixels
    pub height: u32,
    /// Enable clipboard sharing
    pub clipboard_enabled: bool,
    /// Shared folders for drive redirection
    pub shared_folders: Vec<SharedFolder>,
    /// Additional `FreeRDP` arguments
    pub extra_args: Vec<String>,
    /// Window geometry for external mode
    pub window_geometry: Option<WindowGeometry>,
    /// Whether to remember window position
    pub remember_window_position: bool,
}

impl FreeRdpConfig {
    /// Creates a new `FreeRDP` configuration with default settings
    #[must_use]
    pub fn new(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port: 3389,
            username: None,
            password: None,
            domain: None,
            width: 1280,
            height: 720,
            clipboard_enabled: true,
            shared_folders: Vec::new(),
            extra_args: Vec::new(),
            window_geometry: None,
            remember_window_position: true,
        }
    }

    /// Sets the port
    #[must_use]
    pub const fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Sets the username
    #[must_use]
    pub fn with_username(mut self, username: impl Into<String>) -> Self {
        self.username = Some(username.into());
        self
    }

    /// Sets the password
    #[must_use]
    pub fn with_password(mut self, password: impl Into<String>) -> Self {
        self.password = Some(password.into());
        self
    }

    /// Sets the domain
    #[must_use]
    pub fn with_domain(mut self, domain: impl Into<String>) -> Self {
        self.domain = Some(domain.into());
        self
    }

    /// Sets the resolution
    #[must_use]
    pub const fn with_resolution(mut self, width: u32, height: u32) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    /// Enables or disables clipboard sharing
    #[must_use]
    pub const fn with_clipboard(mut self, enabled: bool) -> Self {
        self.clipboard_enabled = enabled;
        self
    }

    /// Sets shared folders for drive redirection
    #[must_use]
    pub fn with_shared_folders(mut self, folders: Vec<SharedFolder>) -> Self {
        self.shared_folders = folders;
        self
    }

    /// Adds extra `FreeRDP` arguments
    #[must_use]
    pub fn with_extra_args(mut self, args: Vec<String>) -> Self {
        self.extra_args = args;
        self
    }

    /// Sets the window geometry for external mode
    #[must_use]
    pub const fn with_window_geometry(mut self, geometry: WindowGeometry) -> Self {
        self.window_geometry = Some(geometry);
        self
    }

    /// Sets whether to remember window position
    #[must_use]
    pub const fn with_remember_window_position(mut self, remember: bool) -> Self {
        self.remember_window_position = remember;
        self
    }
}

/// Builds `FreeRDP` command-line arguments from configuration
///
/// This function generates the command-line arguments for `FreeRDP` (xfreerdp/wlfreerdp)
/// based on the provided configuration. It includes:
/// - Authentication options (username, password, domain)
/// - Display options (resolution, dynamic resolution)
/// - Window options (decorations, geometry)
/// - Feature options (clipboard)
///
/// # Arguments
///
/// * `config` - The `FreeRDP` configuration
///
/// # Returns
///
/// A vector of command-line arguments for `FreeRDP`
///
/// # Requirements Coverage
///
/// - Requirement 6.1: Always includes `/decorations` flag
/// - Requirement 6.3: Includes `/x:` and `/y:` if geometry is set and `remember_window_position` is true
/// - Requirement 6.4: Skips geometry if `remember_window_position` is false
#[must_use]
pub fn build_freerdp_args(config: &FreeRdpConfig) -> Vec<String> {
    let mut args = Vec::new();

    // Domain
    if let Some(ref domain) = config.domain {
        if !domain.is_empty() {
            args.push(format!("/d:{domain}"));
        }
    }

    // Username
    if let Some(ref username) = config.username {
        args.push(format!("/u:{username}"));
    }

    // Password — use /from-stdin to avoid /proc/PID/cmdline exposure
    if config.password.as_ref().is_some_and(|p| !p.is_empty()) {
        args.push("/from-stdin".to_string());
    }

    // Resolution
    args.push(format!("/w:{}", config.width));
    args.push(format!("/h:{}", config.height));

    // Certificate handling
    args.push("/cert:ignore".to_string());

    // Dynamic resolution
    args.push("/dynamic-resolution".to_string());

    // Decorations flag for window controls (Requirement 6.1)
    args.push("/decorations".to_string());

    // Window geometry (Requirements 6.2, 6.3, 6.4)
    if config.remember_window_position {
        if let Some(ref geometry) = config.window_geometry {
            args.push(format!("/x:{}", geometry.x));
            args.push(format!("/y:{}", geometry.y));
        }
    }

    // Clipboard
    if config.clipboard_enabled {
        args.push("+clipboard".to_string());
    }

    // Shared folders (drive redirection)
    for folder in &config.shared_folders {
        if folder.local_path.exists() {
            // FreeRDP format: /drive:share_name,/path/to/folder
            args.push(format!(
                "/drive:{},{}",
                folder.share_name,
                folder.local_path.display()
            ));
        }
    }

    // Extra arguments
    for arg in &config.extra_args {
        args.push(arg.clone());
    }

    // Server address (must be last)
    if config.port == 3389 {
        args.push(format!("/v:{}", config.host));
    } else {
        args.push(format!("/v:{}:{}", config.host, config.port));
    }

    args
}

/// Checks if the `FreeRDP` arguments contain the decorations flag
///
/// # Arguments
///
/// * `args` - The `FreeRDP` command-line arguments
///
/// # Returns
///
/// `true` if the `/decorations` flag is present
#[must_use]
pub fn has_decorations_flag(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "/decorations")
}

/// Extracts window geometry from `FreeRDP` arguments
///
/// # Arguments
///
/// * `args` - The `FreeRDP` command-line arguments
///
/// # Returns
///
/// The extracted window geometry if both `/x:` and `/y:` are present
#[must_use]
pub fn extract_geometry_from_args(args: &[String]) -> Option<(i32, i32)> {
    let mut x = None;
    let mut y = None;

    for arg in args {
        if let Some(val) = arg.strip_prefix("/x:") {
            x = val.parse().ok();
        } else if let Some(val) = arg.strip_prefix("/y:") {
            y = val.parse().ok();
        }
    }

    match (x, y) {
        (Some(x_val), Some(y_val)) => Some((x_val, y_val)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_freerdp_args_basic() {
        let config = FreeRdpConfig::new("server.example.com");
        let args = build_freerdp_args(&config);

        assert!(args.contains(&"/w:1280".to_string()));
        assert!(args.contains(&"/h:720".to_string()));
        assert!(args.contains(&"/decorations".to_string()));
        assert!(args.contains(&"/v:server.example.com".to_string()));
    }

    #[test]
    fn test_build_freerdp_args_with_credentials() {
        let config = FreeRdpConfig::new("server.example.com")
            .with_username("admin")
            .with_password("secret")
            .with_domain("CORP");
        let args = build_freerdp_args(&config);

        assert!(args.contains(&"/u:admin".to_string()));
        assert!(args.contains(&"/from-stdin".to_string()));
        assert!(!args.iter().any(|a| a.starts_with("/p:")));
        assert!(args.contains(&"/d:CORP".to_string()));
    }

    #[test]
    fn test_build_freerdp_args_with_geometry() {
        let geometry = WindowGeometry::new(100, 200, 1920, 1080);
        let config = FreeRdpConfig::new("server.example.com")
            .with_window_geometry(geometry)
            .with_remember_window_position(true);
        let args = build_freerdp_args(&config);

        assert!(args.contains(&"/x:100".to_string()));
        assert!(args.contains(&"/y:200".to_string()));
    }

    #[test]
    fn test_build_freerdp_args_geometry_disabled() {
        let geometry = WindowGeometry::new(100, 200, 1920, 1080);
        let config = FreeRdpConfig::new("server.example.com")
            .with_window_geometry(geometry)
            .with_remember_window_position(false);
        let args = build_freerdp_args(&config);

        // Geometry should NOT be included when remember_window_position is false
        assert!(!args.iter().any(|a| a.starts_with("/x:")));
        assert!(!args.iter().any(|a| a.starts_with("/y:")));
    }

    #[test]
    fn test_has_decorations_flag() {
        let args_with = vec!["/decorations".to_string(), "/v:host".to_string()];
        let args_without = vec!["/v:host".to_string()];

        assert!(has_decorations_flag(&args_with));
        assert!(!has_decorations_flag(&args_without));
    }

    #[test]
    fn test_extract_geometry_from_args() {
        let args = vec![
            "/x:100".to_string(),
            "/y:200".to_string(),
            "/v:host".to_string(),
        ];
        let geometry = extract_geometry_from_args(&args);
        assert_eq!(geometry, Some((100, 200)));

        let args_partial = vec!["/x:100".to_string(), "/v:host".to_string()];
        let geometry_partial = extract_geometry_from_args(&args_partial);
        assert_eq!(geometry_partial, None);
    }

    #[test]
    fn test_build_freerdp_args_custom_port() {
        let config = FreeRdpConfig::new("server.example.com").with_port(3390);
        let args = build_freerdp_args(&config);

        assert!(args.contains(&"/v:server.example.com:3390".to_string()));
    }

    #[test]
    fn test_build_freerdp_args_clipboard_disabled() {
        let config = FreeRdpConfig::new("server.example.com").with_clipboard(false);
        let args = build_freerdp_args(&config);

        assert!(!args.contains(&"+clipboard".to_string()));
    }

    #[test]
    fn test_build_freerdp_args_with_shared_folders() {
        // Create a temp directory that exists for the test
        let temp_dir = std::env::temp_dir();

        let folders = vec![
            SharedFolder {
                share_name: "Documents".to_string(),
                local_path: temp_dir.clone(),
            },
            SharedFolder {
                share_name: "Downloads".to_string(),
                local_path: temp_dir,
            },
        ];

        let config = FreeRdpConfig::new("server.example.com").with_shared_folders(folders);
        let args = build_freerdp_args(&config);

        // Check that drive arguments are present
        let drive_args: Vec<_> = args.iter().filter(|a| a.starts_with("/drive:")).collect();
        assert_eq!(drive_args.len(), 2);

        // Verify format: /drive:share_name,/path
        assert!(drive_args[0].starts_with("/drive:Documents,"));
        assert!(drive_args[1].starts_with("/drive:Downloads,"));
    }

    #[test]
    fn test_build_freerdp_args_shared_folders_nonexistent_path() {
        let folders = vec![SharedFolder {
            share_name: "NonExistent".to_string(),
            local_path: PathBuf::from("/nonexistent/path/that/does/not/exist"),
        }];

        let config = FreeRdpConfig::new("server.example.com").with_shared_folders(folders);
        let args = build_freerdp_args(&config);

        // Non-existent paths should be skipped
        assert!(!args.iter().any(|a| a.starts_with("/drive:")));
    }
}
