//! `RustConn` - Modern Connection Manager for Linux
//!
//! A GTK4/libadwaita connection manager supporting SSH, RDP, VNC, SPICE,
//! Telnet, and Zero Trust protocols with embedded Rust implementations.
//! with Wayland-native support and `KeePassXC` integration.
//!
//! # GTK Widget Lifecycle Pattern
//!
//! Throughout this crate, you'll see struct fields marked with `#[allow(dead_code)]`.
//! These are **intentionally kept alive** for GTK widget lifecycle management:
//!
//! - **Signal handlers**: `connect_clicked()`, `connect_changed()`, etc. hold references
//! - **Event controllers**: Motion, key, and scroll controllers need widget references
//! - **Widget tree ownership**: Parent-child relationships require keeping references
//!
//! **⚠️ WARNING**: Removing these "unused" fields will cause **segmentation faults**
//! when GTK signals fire, because the signal handler closures capture these references.
//!
//! ## Example
//!
//! ```ignore
//! pub struct MyDialog {
//!     window: adw::Window,
//!     #[allow(dead_code)] // Kept alive for connect_clicked() handler
//!     save_button: gtk4::Button,
//! }
//! ```
//!
//! The `save_button` field appears unused, but removing it would cause the button's
//! click handler to crash when invoked.

// Global clippy lint configuration for GUI code
// Only truly necessary suppressions are kept globally; others should be applied per-function
#![allow(clippy::too_many_lines)] // GUI setup functions are inherently long
#![allow(clippy::type_complexity)] // GTK callback types are complex by design
#![allow(clippy::significant_drop_tightening)] // GTK widget drops are managed by GTK
#![allow(clippy::missing_errors_doc)] // Internal GUI functions don't need error docs
#![allow(clippy::missing_panics_doc)] // Internal GUI functions don't need panic docs

pub mod adaptive_tabs;
pub mod alert;
mod app;
pub mod async_utils;
#[cfg(feature = "rdp-audio")]
pub mod audio;
pub mod automation;
pub mod dashboard;
pub mod dialogs;
pub mod display;
pub mod embedded;
pub mod embedded_rdp;
pub mod embedded_rdp_buffer;
pub mod embedded_rdp_detect;
pub mod embedded_rdp_launcher;
pub mod embedded_rdp_thread;
pub mod embedded_rdp_types;
pub mod embedded_rdp_ui;
pub mod embedded_spice;
pub mod embedded_trait;
pub mod embedded_vnc;
pub mod embedded_vnc_types;
pub mod embedded_vnc_ui;
pub mod empty_state;
pub mod external_window;
pub mod floating_controls;
pub mod loading;
pub mod session;
mod sidebar;
mod sidebar_types;
mod sidebar_ui;
pub mod split_view;
mod state;
mod terminal;
pub mod toast;
pub mod tray;
pub mod utils;
pub mod validation;
pub mod wayland_surface;
mod window;

// Error display utilities
pub mod error;
pub mod error_display;
mod window_clusters;
mod window_connection_dialogs;
mod window_document_actions;
mod window_edit_dialogs;
mod window_groups;
mod window_operations;
mod window_protocols;
mod window_rdp_vnc;
mod window_sessions;
mod window_snippets;
mod window_sorting;
mod window_templates;
mod window_types;
mod window_ui;

fn main() -> gtk4::glib::ExitCode {
    // Initialize logging with environment filter (RUST_LOG)
    // Filter out noisy zbus debug messages (ProvideXdgActivationToken errors from ksni)
    //
    // Note: expect() is acceptable here because:
    // 1. "zbus=warn" is a compile-time constant directive that is always valid
    // 2. Runtime creation failure at startup is unrecoverable - the app cannot function
    let filter = tracing_subscriber::EnvFilter::from_default_env().add_directive(
        "zbus=warn"
            .parse()
            .expect("compile-time constant directive"),
    );

    tracing_subscriber::fmt().with_env_filter(filter).init();

    // Initialize Tokio runtime for async operations
    // Note: Runtime creation failure at startup is unrecoverable
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime required for async ops");
    let _guard = runtime.enter();

    app::run()
}
