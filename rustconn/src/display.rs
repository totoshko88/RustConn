//! Display server detection and capabilities
//!
//! This module provides unified display server detection for the application,
//! consolidating the detection logic that was previously duplicated across
//! `embedded.rs` and `wayland_surface.rs`.
//!
//! # Detection Order
//!
//! 1. GDK display type (most reliable when available)
//! 2. `GDK_BACKEND` environment variable (explicit override)
//! 3. `XDG_SESSION_TYPE` environment variable
//! 4. `WAYLAND_DISPLAY` environment variable
//! 5. `DISPLAY` environment variable (X11 fallback)
//!
//! # Usage
//!
//! ```ignore
//! use crate::display::DisplayServer;
//!
//! let server = DisplayServer::detect();
//! if server.is_wayland() {
//!     // Use native Wayland features
//! } else {
//!     // Fall back to X11 or Cairo rendering
//! }
//! ```

use gtk4::glib::object::Cast;
use std::sync::OnceLock;

/// Cached display server detection result
static DISPLAY_SERVER: OnceLock<DisplayServer> = OnceLock::new();

/// Display server type detected at runtime
///
/// This enum represents the display server the application is running on.
/// Detection is performed once and cached for the lifetime of the application.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DisplayServer {
    /// Wayland display server
    ///
    /// Supports:
    /// - Native subsurface integration for embedded sessions
    /// - Modern compositor features
    /// - Better security model
    Wayland,

    /// X11 display server
    ///
    /// Supports:
    /// - XEmbed protocol for embedding
    /// - Legacy window positioning
    /// - Cairo fallback rendering
    X11,

    /// Unknown display server
    ///
    /// Falls back to:
    /// - Cairo rendering
    /// - External window mode for sessions
    #[default]
    Unknown,
}

impl DisplayServer {
    /// Detects the current display server (cached)
    ///
    /// This function performs detection once and caches the result.
    /// Subsequent calls return the cached value.
    #[must_use]
    pub fn detect() -> Self {
        *DISPLAY_SERVER.get_or_init(Self::detect_impl)
    }

    /// Internal detection implementation
    fn detect_impl() -> Self {
        // Try GDK display detection first (most reliable when GTK is initialized)
        #[cfg(feature = "wayland-native")]
        {
            if let Some(display) = gtk4::gdk::Display::default() {
                // Check if it's a Wayland display
                if display
                    .downcast_ref::<gdk4_wayland::WaylandDisplay>()
                    .is_some()
                {
                    return Self::Wayland;
                }
                // If we have a display but it's not Wayland, it's likely X11
                // (GDK doesn't expose X11Display in the same way)
            }
        }

        // Check GDK_BACKEND environment variable (explicit override)
        if let Ok(backend) = std::env::var("GDK_BACKEND") {
            let backend_lower = backend.to_lowercase();
            if backend_lower.contains("wayland") {
                return Self::Wayland;
            }
            if backend_lower.contains("x11") {
                return Self::X11;
            }
        }

        // Check XDG_SESSION_TYPE environment variable
        if let Ok(session_type) = std::env::var("XDG_SESSION_TYPE") {
            let session_lower = session_type.to_lowercase();
            if session_lower == "wayland" {
                return Self::Wayland;
            }
            if session_lower == "x11" {
                return Self::X11;
            }
        }

        // Check for Wayland display socket
        if std::env::var("WAYLAND_DISPLAY").is_ok() {
            return Self::Wayland;
        }

        // Check for X11 display
        if std::env::var("DISPLAY").is_ok() {
            return Self::X11;
        }

        Self::Unknown
    }

    /// Returns `true` if running on Wayland
    #[must_use]
    pub const fn is_wayland(&self) -> bool {
        matches!(self, Self::Wayland)
    }

    /// Returns `true` if running on X11
    #[must_use]
    pub const fn is_x11(&self) -> bool {
        matches!(self, Self::X11)
    }

    /// Returns `true` if the display server is unknown
    #[must_use]
    pub const fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown)
    }

    /// Returns whether native Wayland subsurface integration is supported
    ///
    /// Subsurfaces allow embedding RDP/VNC sessions directly in the GTK widget
    /// hierarchy without going through X11 embedding protocols.
    #[must_use]
    pub const fn supports_subsurface(&self) -> bool {
        matches!(self, Self::Wayland)
    }

    /// Returns whether XEmbed protocol is supported for embedding
    ///
    /// XEmbed allows embedding external windows (like xfreerdp) into GTK sockets.
    /// This is only available on X11.
    #[must_use]
    pub const fn supports_xembed(&self) -> bool {
        matches!(self, Self::X11)
    }

    /// Returns whether any embedding protocol is supported
    ///
    /// This returns true if either Wayland subsurfaces or X11 XEmbed is available.
    #[must_use]
    pub const fn supports_embedding(&self) -> bool {
        matches!(self, Self::Wayland | Self::X11)
    }

    /// Returns whether Cairo fallback rendering should be used
    ///
    /// Cairo fallback is used when native Wayland subsurfaces are not available.
    #[must_use]
    pub const fn use_cairo_fallback(&self) -> bool {
        matches!(self, Self::X11 | Self::Unknown)
    }

    /// Returns whether the native Wayland feature is compiled in
    #[must_use]
    pub const fn has_native_wayland_support() -> bool {
        cfg!(feature = "wayland-native")
    }

    /// Returns a human-readable name for the display server
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Wayland => "Wayland",
            Self::X11 => "X11",
            Self::Unknown => "Unknown",
        }
    }
}

impl std::fmt::Display for DisplayServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_server_default_is_unknown() {
        assert_eq!(DisplayServer::default(), DisplayServer::Unknown);
    }

    #[test]
    fn display_server_name() {
        assert_eq!(DisplayServer::Wayland.name(), "Wayland");
        assert_eq!(DisplayServer::X11.name(), "X11");
        assert_eq!(DisplayServer::Unknown.name(), "Unknown");
    }

    #[test]
    fn display_server_capabilities() {
        assert!(DisplayServer::Wayland.supports_subsurface());
        assert!(!DisplayServer::X11.supports_subsurface());
        assert!(!DisplayServer::Unknown.supports_subsurface());

        assert!(!DisplayServer::Wayland.supports_xembed());
        assert!(DisplayServer::X11.supports_xembed());
        assert!(!DisplayServer::Unknown.supports_xembed());

        assert!(!DisplayServer::Wayland.use_cairo_fallback());
        assert!(DisplayServer::X11.use_cairo_fallback());
        assert!(DisplayServer::Unknown.use_cairo_fallback());
    }

    #[test]
    fn display_server_display_trait() {
        assert_eq!(format!("{}", DisplayServer::Wayland), "Wayland");
        assert_eq!(format!("{}", DisplayServer::X11), "X11");
        assert_eq!(format!("{}", DisplayServer::Unknown), "Unknown");
    }
}
