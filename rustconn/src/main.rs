//! `RustConn` - Modern Connection Manager for Linux
//!
//! A GTK4-based connection manager supporting SSH, RDP, and VNC protocols
//! with Wayland-native support and `KeePassXC` integration.

// Allow common clippy lints for GUI code
#![allow(clippy::too_many_lines)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::type_complexity)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::option_if_let_else)]
#![allow(clippy::redundant_clone)]
#![allow(clippy::needless_pass_by_value)]
#![allow(clippy::unused_self)]
#![allow(clippy::match_same_arms)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::wildcard_imports)]
#![allow(clippy::manual_let_else)]
#![allow(clippy::branches_sharing_code)]
#![allow(clippy::uninlined_format_args)]
#![allow(clippy::needless_borrows_for_generic_args)]
#![allow(clippy::redundant_closure_for_method_calls)]
#![allow(clippy::map_unwrap_or)]
#![allow(clippy::cloned_instead_of_copied)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::only_used_in_recursion)]
#![allow(clippy::unnecessary_wraps)]
#![allow(clippy::assigning_clones)]
#![allow(clippy::inefficient_to_string)]
#![allow(clippy::no_effect_underscore_binding)]
#![allow(clippy::manual_map)]

pub mod adaptive_tabs;
mod app;
#[cfg(feature = "rdp-audio")]
pub mod audio;
pub mod automation;
pub mod dashboard;
pub mod dialogs;
pub mod embedded;
pub mod embedded_rdp;
pub mod embedded_rdp_buffer;
pub mod embedded_rdp_detect;
pub mod embedded_rdp_launcher;
pub mod embedded_rdp_thread;
pub mod embedded_rdp_types;
pub mod embedded_rdp_ui;
pub mod embedded_spice;
pub mod embedded_vnc;
pub mod embedded_vnc_types;
pub mod embedded_vnc_ui;
pub mod external_window;
pub mod floating_controls;
pub mod session;
mod sidebar;
mod sidebar_types;
mod sidebar_ui;
pub mod split_view;
mod state;
mod terminal;
pub mod tray;
pub mod wayland_surface;
mod window;
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

fn main() -> gtk4::glib::ExitCode {
    // Initialize logging with environment filter (RUST_LOG)
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    app::run()
}
