//! Optional embedded VNC client integration boundary
//!
//! A headless build keeps this module as an availability boundary only. The
//! `vnc-rs` runtime, VNC config, events, and client types are compiled only with
//! the `vnc-embedded` feature.
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
pub use config::{VncClientConfig, VncEncoding};
#[cfg(feature = "vnc-embedded")]
pub use error::VncClientError;
#[cfg(feature = "vnc-embedded")]
pub use event::{VncClientCommand, VncClientEvent, VncRect};

/// Check if embedded VNC support is available
#[must_use]
pub const fn is_embedded_vnc_available() -> bool {
    cfg!(feature = "vnc-embedded")
}
