//! RDP client boundary for domain types and optional embedded sessions
//!
//! This module always exposes RDP configuration, events, command types, backend
//! detection, and argument helpers that are useful to headless callers. The
//! actual embedded RDP runtime is compiled only with the `rdp-embedded` feature,
//! so a minimal `rustconn-core` build does not pull IronRDP or client runtime
//! dependencies.
//!
//! # Architecture
//!
//! When enabled, the RDP client runs in a background thread with its own Tokio runtime and
//! communicates with the GUI through channels:
//! - `RdpClientEvent` channel: framebuffer updates, resolution changes, etc.
//! - `RdpClientCommand` channel: keyboard/mouse input, disconnect requests
//!
//! This follows the same pattern as the VNC client (`vnc_client` module).
//!
//! # Graphics Pipeline
//!
//! When the `gfx-h264` feature is enabled, the client registers an EGFX
//! dynamic virtual channel (`ironrdp-egfx`) for the GFX pipeline with
//! H.264/AVC decoding via OpenH264 (loaded at runtime via `dlopen`).
//! The pipeline auto-selects the best mode: GfxAvc444 > GfxH264 > Gfx >
//! RemoteFX > Legacy. See [`gfx_handler`] and [`graphics`] for details.
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

// cast_possible_truncation, cast_precision_loss allowed at workspace level
#![allow(
    clippy::cast_sign_loss,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::missing_panics_doc,
    reason = "module-wide override for legacy code; refactored case by case"
)]

#[cfg(feature = "rdp-embedded")]
pub mod audio;
pub mod backend;
#[cfg(feature = "rdp-embedded")]
mod client;
#[cfg(feature = "rdp-embedded")]
pub mod clipboard;
mod config;
#[cfg(feature = "rdp-embedded")]
pub mod dir_watcher;
mod error;
mod event;
pub mod gateway;
#[cfg(feature = "gfx-h264")]
pub mod gfx_handler;
pub mod graphics;
pub mod input;
pub mod keyboard_layout;
pub mod multimonitor;
#[cfg(feature = "rdp-embedded")]
pub mod rdpdr;
pub mod reconnect;

pub mod quick_actions;

pub use backend::{BackendDetectionResult, RdpBackend, RdpBackendSelector};
#[cfg(feature = "rdp-embedded")]
pub use client::{RdpClient, RdpClientState, RdpCommandSender, RdpEventReceiver};
pub use config::{
    ConfigValidationError, RdpClientConfig, RdpSecurityProtocol, RemoteAppConfig, SharedFolder,
};
pub use error::RdpClientError;
pub use event::{
    AudioFormatInfo, ClipboardFileInfo, ClipboardFormatInfo, PixelFormat, RdpClientCommand,
    RdpClientEvent, RdpRect, convert_to_bgra, create_frame_update,
    create_frame_update_with_conversion,
};
pub use gateway::{GatewayAuthMethod, GatewayConfig, GatewayError, GatewayState};
pub use graphics::{
    FrameStatistics, GraphicsError, GraphicsMode, GraphicsQuality, ServerGraphicsCapabilities,
};
pub use multimonitor::{MonitorArrangement, MonitorDefinition, MonitorLayout};
pub use reconnect::{ConnectionQuality, DisconnectReason, ReconnectPolicy, ReconnectState};

pub use quick_actions::{
    QUICK_ACTIONS, QuickAction, build_enter_sequence, build_hotkey_sequence, build_open_run_dialog,
    run_command_for,
};

pub use keyboard_layout::{LAYOUT_US_ENGLISH, detect_keyboard_layout, xkb_name_to_klid};

/// Check if embedded RDP support is available
///
/// Returns true if the `rdp-embedded` feature is enabled, which means
/// the native `IronRDP` client can be used. When false, the GUI should
/// fall back to `FreeRDP` subprocess.
#[must_use]
pub const fn is_embedded_rdp_available() -> bool {
    cfg!(feature = "rdp-embedded")
}

// Re-export key mapping functions for keyboard input handling
pub use input::{keycode_to_scancode, keyval_to_scancode, keyval_to_unicode};
