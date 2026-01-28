//! `RustConn` - Modern Connection Manager for Linux
//!
//! A GTK4-based connection manager supporting SSH, RDP, and VNC protocols
//! with Wayland-native support and `KeePassXC` integration.

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
    let filter = tracing_subscriber::EnvFilter::from_default_env()
        .add_directive("zbus=warn".parse().expect("valid directive"));

    tracing_subscriber::fmt().with_env_filter(filter).init();

    app::run()
}
