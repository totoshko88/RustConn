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

pub mod alert;
mod app;
pub mod async_utils;
#[cfg(feature = "rdp-audio")]
pub mod audio;
pub mod automation;
pub mod dialogs;
pub mod display;
pub mod embedded;
pub mod embedded_rdp;
pub mod embedded_spice;
pub mod embedded_trait;
pub mod embedded_vnc;
pub mod embedded_vnc_types;
pub mod embedded_vnc_ui;
pub mod external_window;
pub mod i18n;
pub mod monitoring;
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

pub mod error;

// CLI startup override, set in `main()` and consumed in `build_ui()`.
// Uses `RefCell` because GTK is single-threaded and the value
// is written once before `app.run()` and read once inside `connect_activate`.
std::thread_local! {
    static CLI_STARTUP_OVERRIDE: std::cell::RefCell<Option<rustconn_core::config::StartupAction>> =
        const { std::cell::RefCell::new(None) };
}

/// Stores a CLI-provided startup action for `build_ui` to consume.
pub fn set_cli_startup_override(action: rustconn_core::config::StartupAction) {
    CLI_STARTUP_OVERRIDE.with(|cell| {
        *cell.borrow_mut() = Some(action);
    });
}

/// Takes the CLI startup override (if any), leaving `None` behind.
pub fn take_cli_startup_override() -> Option<rustconn_core::config::StartupAction> {
    CLI_STARTUP_OVERRIDE.with(|cell| cell.borrow_mut().take())
}

/// Parses CLI arguments for the GUI binary.
///
/// Supported flags:
/// - `--shell` — open a local shell on startup
/// - `--connect <name-or-uuid>` — connect to a saved connection
/// - `--help` / `-h` — print usage and exit
/// - `--version` / `-V` — print version and exit
fn parse_cli_args() -> Option<rustconn_core::config::StartupAction> {
    use rustconn_core::config::StartupAction;

    let args: Vec<String> = std::env::args().collect();
    let mut i = 1; // skip binary name
    while i < args.len() {
        match args[i].as_str() {
            "--shell" => return Some(StartupAction::LocalShell),
            "--connect" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Error: --connect requires a connection name or UUID");
                    std::process::exit(1);
                }
                let value = &args[i];
                // Try UUID first, then search by name
                if let Ok(uuid) = uuid::Uuid::parse_str(value) {
                    return Some(StartupAction::Connection(uuid));
                }
                // Defer name lookup — config isn't loaded yet. Store the name
                // and resolve in build_ui after state is created.
                // We use a special marker: store name in a second thread-local.
                set_cli_connect_name(value.clone());
                return None; // Signal that name resolution is needed
            }
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            "--version" | "-V" => {
                println!("RustConn {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            _ => {
                // Ignore unknown args (GTK may pass its own)
            }
        }
        i += 1;
    }
    None
}

// Thread-local for `--connect <name>` that needs deferred resolution.
std::thread_local! {
    static CLI_CONNECT_NAME: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

/// Stores a connection name from `--connect` for deferred resolution.
pub fn set_cli_connect_name(name: String) {
    CLI_CONNECT_NAME.with(|cell| {
        *cell.borrow_mut() = Some(name);
    });
}

/// Takes the CLI connect name (if any).
pub fn take_cli_connect_name() -> Option<String> {
    CLI_CONNECT_NAME.with(|cell| cell.borrow_mut().take())
}

fn print_usage() {
    println!(
        "Usage: rustconn [OPTIONS]\n\n\
         Options:\n  \
           --shell              Open a local shell on startup\n  \
           --connect <NAME|UUID> Connect to a saved connection\n  \
           -h, --help           Print this help message\n  \
           -V, --version        Print version"
    );
}

fn main() -> gtk4::glib::ExitCode {
    // Initialize internationalization (gettext)
    i18n::init();

    // Apply saved language from config BEFORE GTK starts.
    // This must happen early so that all gettext() calls during UI construction
    // use the correct locale. The LANGUAGE env var must be set before any
    // translatable string is evaluated.
    i18n::apply_language_from_config();

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

    // Ensure ssh-agent is running so that child processes (Dolphin,
    // mc, ssh-add) inherit SSH_AUTH_SOCK. On some DEs (KDE on
    // openSUSE Tumbleweed) ssh-agent is not started by default.
    if let Some(info) = rustconn_core::sftp::ensure_ssh_agent() {
        rustconn_core::sftp::set_agent_info(info);
    } else {
        tracing::warn!(
            "Could not ensure ssh-agent is running; \
             SFTP via file managers may require manual setup"
        );
    }

    // Initialize Tokio runtime for async operations
    // Note: Runtime creation failure at startup is unrecoverable
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime required for async ops");
    let _guard = runtime.enter();

    // Parse CLI arguments for startup overrides (--shell, --connect)
    if let Some(action) = parse_cli_args() {
        set_cli_startup_override(action);
    }

    app::run()
}
