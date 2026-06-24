//! Pure Rust VNC client for embedded VNC sessions
//!
//! This module provides a VNC client implementation using the `vnc-rs` crate,
//! enabling true embedded VNC sessions in GTK4 without external processes.
//!
//! # Architecture
//!
//! The VNC client runs in a background tokio task and communicates with the
//! GUI through channels:
//! - `VncEvent` channel: framebuffer updates, resolution changes, etc.
//! - `VncCommand` channel: keyboard/mouse input, disconnect requests

#[cfg(feature = "vnc-embedded")]
mod client;
#[cfg(feature = "vnc-embedded")]
mod config;
#[cfg(feature = "vnc-embedded")]
mod error;
#[cfg(feature = "vnc-embedded")]
mod event;

#[cfg(feature = "vnc-embedded")]
pub use client::{VncClient, VncCommandSender, VncEventReceiver};
#[cfg(feature = "vnc-embedded")]
pub use config::VncClientConfig;
#[cfg(feature = "vnc-embedded")]
pub use error::VncClientError;
#[cfg(feature = "vnc-embedded")]
pub use event::{VncClientCommand, VncClientEvent, VncRect};

/// Check if embedded VNC support is available
#[must_use]
pub const fn is_embedded_vnc_available() -> bool {
    cfg!(feature = "vnc-embedded")
}
