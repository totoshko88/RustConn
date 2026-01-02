//! Pure Rust RDP client for embedded RDP sessions
//!
//! This module provides an RDP client implementation using the `ironrdp` crate,
//! enabling true embedded RDP sessions in GTK4 without external processes.
//!
//! # Architecture
//!
//! The RDP client runs in a background thread with its own Tokio runtime and
//! communicates with the GUI through channels:
//! - `RdpClientEvent` channel: framebuffer updates, resolution changes, etc.
//! - `RdpClientCommand` channel: keyboard/mouse input, disconnect requests
//!
//! This follows the same pattern as the VNC client (`vnc_client` module).
//!
//! # Feature Flag
//!
//! The embedded RDP client requires the `rdp-embedded` feature flag:
//!
//! ```toml
//! [dependencies]
//! rustconn-core = { version = "0.1", features = ["rdp-embedded"] }
//! ```
//!
//! When the feature is disabled, the module still provides the types and
//! configuration, but the `RdpClient` struct is not available. In this case,
//! the GUI falls back to `FreeRDP` subprocess (wlfreerdp/xfreerdp).
//!
//! # Requirements Coverage
//!
//! - Requirement 1.1: Native RDP embedding as GTK widget
//! - Requirement 1.2: Mouse coordinate forwarding
//! - Requirement 1.3: Keyboard event forwarding
//! - Requirement 1.4: Ctrl+Alt+Del support
//! - Requirement 1.5: Fallback to `FreeRDP`
//! - Requirement 1.6: Resource cleanup on disconnect
//! - Requirement 10.1: Follow VNC client architecture pattern

// Allow cast warnings - RDP protocol uses various integer sizes
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::missing_panics_doc)]

#[cfg(feature = "rdp-embedded")]
pub mod audio;
#[cfg(feature = "rdp-embedded")]
mod client;
#[cfg(feature = "rdp-embedded")]
pub mod clipboard;
mod config;
#[cfg(feature = "rdp-embedded")]
pub mod dir_watcher;
mod error;
mod event;
pub mod input;
#[cfg(feature = "rdp-embedded")]
pub mod rdpdr;

#[cfg(feature = "rdp-embedded")]
pub use audio::AudioFormatInfo;
#[cfg(feature = "rdp-embedded")]
pub use client::{RdpClient, RdpClientState, RdpCommandSender, RdpEventReceiver};
pub use config::{RdpClientConfig, RdpSecurityProtocol, SharedFolder};
pub use error::RdpClientError;
pub use event::{
    convert_to_bgra, create_frame_update, create_frame_update_with_conversion, ClipboardFileInfo,
    ClipboardFormatInfo, PixelFormat, RdpClientCommand, RdpClientEvent, RdpRect,
};

/// Check if embedded RDP support is available
///
/// Returns true if the `rdp-embedded` feature is enabled, which means
/// the native `IronRDP` client can be used. When false, the GUI should
/// fall back to `FreeRDP` subprocess.
#[must_use]
pub const fn is_embedded_rdp_available() -> bool {
    cfg!(feature = "rdp-embedded")
}

// Re-export keyval_to_unicode for keyboard input handling
pub use input::keyval_to_unicode;
