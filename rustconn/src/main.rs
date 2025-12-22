//! `RustConn` - Modern Connection Manager for Linux
//!
//! A GTK4-based connection manager supporting SSH, RDP, VNC, and SPICE protocols
//! with Wayland-native support and `KeePassXC` integration.
//!
//! # Supported Protocols
//!
//! - **SSH** - Embedded VTE terminal with full PTY support
//! - **RDP** - Via `FreeRDP` (`xfreerdp`/`xfreerdp3`) or embedded `IronRDP`
//! - **VNC** - Via TigerVNC/TightVNC or embedded `vnc-rs`
//! - **SPICE** - Via `remote-viewer` or embedded `spice-client`
//!
//! # Architecture
//!
//! The application follows a three-crate workspace structure:
//! - `rustconn` (this crate) - GTK4 GUI application
//! - `rustconn-core` - Business logic, models, protocols (GUI-free)
//! - `rustconn-cli` - Command-line interface

// =============================================================================
// GUI-specific lint configuration
// =============================================================================
// GTK4 applications have specific patterns that trigger clippy warnings:
// - Widget fields stored to prevent dropping (dead_code)
// - Complex callback signatures (type_complexity)
// - RefCell borrows in callbacks (significant_drop_tightening)
// - i32/f64 casts for GTK dimensions (cast_* lints)
// - Match arms for future protocol expansion (match_same_arms)

// Dead code: GTK widgets must be stored to prevent dropping
#![allow(dead_code)]
// Complexity: GTK callbacks and setup functions are inherently complex
#![allow(clippy::too_many_lines)]
#![allow(clippy::type_complexity)]
#![allow(clippy::cognitive_complexity)]
// Casts: GTK uses i32 for dimensions, f64 for some values
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_sign_loss)]
// Borrows: RefCell patterns in GTK callbacks
#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::redundant_clone)]
// Style: Readability preferences for GUI code
#![allow(clippy::match_same_arms)]
#![allow(clippy::option_if_let_else)]
#![allow(clippy::manual_let_else)]
#![allow(clippy::if_not_else)]
#![allow(clippy::single_match_else)]
// Documentation: Internal GUI code
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::must_use_candidate)]
// Minor optimizations not critical for GUI
#![allow(clippy::needless_pass_by_value)]
#![allow(clippy::needless_borrows_for_generic_args)]
#![allow(clippy::redundant_closure_for_method_calls)]
#![allow(clippy::cloned_instead_of_copied)]
#![allow(clippy::assigning_clones)]
#![allow(clippy::uninlined_format_args)]
#![allow(clippy::unused_self)]
#![allow(clippy::no_effect_underscore_binding)]
#![allow(clippy::branches_sharing_code)]
#![allow(clippy::unnecessary_wraps)]

pub mod adaptive_tabs;
mod app;
pub mod dashboard;
pub mod dialogs;
pub mod embedded;
pub mod embedded_rdp;
pub mod embedded_spice;
pub mod embedded_vnc;
pub mod error;
pub mod external_window;
pub mod floating_controls;
pub mod session;
mod sidebar;
pub mod split_view;
mod state;
mod terminal;
pub mod tray;
pub mod wayland_surface;
mod window;

fn main() -> gtk4::glib::ExitCode {
    app::run()
}
