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
#![allow(
    clippy::too_many_lines,
    reason = "GUI setup functions are inherently long"
)]
#![allow(
    clippy::type_complexity,
    reason = "GTK callback types are complex by design"
)]
#![allow(
    clippy::significant_drop_tightening,
    reason = "GTK widget drops are managed by GTK"
)]
#![allow(
    clippy::missing_errors_doc,
    reason = "Internal GUI functions don't need error docs"
)]
#![allow(
    clippy::missing_panics_doc,
    reason = "Internal GUI functions don't need panic docs"
)]

pub mod activity_coordinator;
pub mod alert;
mod app;
pub mod async_utils;
#[cfg(feature = "rdp-audio")]
pub mod audio;
pub mod automation;
pub mod broadcast;
pub mod cairo_buffer;
pub mod dialogs;
pub mod display;
pub mod embedded;
pub mod embedded_rdp;
pub mod embedded_spice;
pub mod embedded_trait;
pub mod embedded_vnc;
pub mod embedded_vnc_types;
pub mod external_window;
pub mod i18n;
mod i18n_markers;
#[cfg(target_os = "macos")]
pub mod macos_pty;
pub mod monitoring;
pub mod session;
mod sidebar;
mod sidebar_types;
mod sidebar_ui;
pub mod smart_folder_ui;
pub mod split_view;
mod state;
mod terminal;
pub mod toast;
pub mod tray;
pub mod utils;
pub mod validation;
mod vault_ops;
mod window;

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
                // Check if argument is an .rdp file path
                if std::path::Path::new(&args[i])
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("rdp"))
                {
                    let path = std::path::PathBuf::from(&args[i]);
                    if path.exists() {
                        return Some(StartupAction::RdpFile(path));
                    }
                    eprintln!("Error: RDP file not found: {}", args[i]);
                    std::process::exit(1);
                }
                // Check if argument is a .vv file path (virt-viewer)
                if std::path::Path::new(&args[i])
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("vv"))
                {
                    let path = std::path::PathBuf::from(&args[i]);
                    if path.exists() {
                        return Some(StartupAction::VvFile(path));
                    }
                    eprintln!("Error: Virt-viewer file not found: {}", args[i]);
                    std::process::exit(1);
                }
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
        "Usage: rustconn [OPTIONS] [FILE.rdp|FILE.vv]\n\n\
         Options:\n  \
           --shell              Open a local shell on startup\n  \
           --connect <NAME|UUID> Connect to a saved connection\n  \
           -h, --help           Print this help message\n  \
           -V, --version        Print version\n\n\
         Arguments:\n  \
           FILE.rdp             Open and connect from an .rdp file\n  \
           FILE.vv              Open and connect from a virt-viewer .vv file"
    );
}

/// Falls back to the Cairo GSK renderer on pure X11 sessions.
///
/// GTK4's default NGL (OpenGL) renderer has known issues with popover
/// initial paint on some X11 compositors — menus appear blank until the
/// pointer hovers over them (#85, affects MATE, XFCE, older Mutter).
///
/// If `GSK_RENDERER` is not already set by the user and the session is
/// X11 (no `WAYLAND_DISPLAY`), this function re-executes the process
/// with `GSK_RENDERER=cairo`.  The re-exec happens before GTK or tokio
/// start, so it is safe.  A sentinel env var prevents infinite loops.
#[cfg(not(target_os = "macos"))]
fn ensure_x11_renderer_fallback() {
    use std::os::unix::process::CommandExt;

    // Skip if user explicitly chose a renderer
    if std::env::var("GSK_RENDERER").is_ok() {
        return;
    }

    // Skip on Wayland — NGL works fine there
    if std::env::var("WAYLAND_DISPLAY").is_ok() {
        return;
    }

    // Only act when running on X11
    if std::env::var("DISPLAY").is_err() {
        return;
    }

    // Sentinel: we already re-execed once
    if std::env::var("_RUSTCONN_GSK_SET").ok().as_deref() == Some("1") {
        return;
    }

    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(_) => return,
    };

    let args: Vec<String> = std::env::args().collect();

    // Replace the current process image with GSK_RENDERER=cairo.
    // exec() only returns on error — in that case we just continue
    // with the default renderer.
    let err = std::process::Command::new(exe)
        .args(&args[1..])
        .env("GSK_RENDERER", "cairo")
        .env("_RUSTCONN_GSK_SET", "1")
        .exec();

    // exec() only returns on error — fall through to default renderer
    tracing::warn!(?err, "GSK_RENDERER re-exec failed; using default renderer");
}

/// Detects if running inside a macOS .app bundle and re-execs with the
/// correct environment if needed. This avoids using a bash wrapper as
/// CFBundleExecutable (which breaks NSStatusItem / tray icon).
///
/// Uses the same re-exec pattern as `ensure_x11_renderer_fallback()`:
/// detect → set env → exec self. Only triggers once (guard variable).
#[cfg(target_os = "macos")]
fn setup_macos_bundle_env() {
    use std::env;
    use std::os::unix::process::CommandExt;

    // Guard: don't re-exec if we already did
    if env::var_os("_RUSTCONN_BUNDLE_ENV_SET").is_some() {
        return;
    }

    // Detect bundle: executable is at .app/Contents/MacOS/rustconn
    let exe_path = match env::current_exe() {
        Ok(p) => p,
        Err(_) => return,
    };

    let macos_dir = exe_path.parent().unwrap_or(&exe_path);
    let contents_dir = macos_dir.parent().unwrap_or(macos_dir);
    let bundle_dir = contents_dir.parent().unwrap_or(contents_dir);

    // Verify this looks like a .app bundle
    let bundle_name = bundle_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    if !bundle_name.ends_with(".app") {
        return;
    }

    // Check if env vars are already set (e.g. launched from terminal with wrapper)
    if env::var_os("GSETTINGS_SCHEMA_DIR").is_some() {
        return;
    }

    // Build environment and re-exec
    let resources_dir = contents_dir.join("Resources");
    let resources_share = resources_dir.join("share");

    let xdg = format!(
        "{}:/opt/homebrew/share:/usr/local/share:/usr/share",
        resources_share.display()
    );

    let schemas = resources_share.join("glib-2.0/schemas");
    let schemas_str = if schemas.exists() {
        schemas.to_string_lossy().into_owned()
    } else {
        "/opt/homebrew/share/glib-2.0/schemas".to_string()
    };

    let locale_dir = resources_dir.join("locale");
    let locale_str = locale_dir.to_string_lossy().into_owned();

    let path = format!(
        "/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:{}",
        env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin:/usr/sbin:/sbin".to_string())
    );

    let home = env::var("HOME").unwrap_or_else(|_| "/".to_string());

    // Collect current args
    let args: Vec<String> = env::args().collect();

    // Re-exec with bundle environment
    let err = std::process::Command::new(&exe_path)
        .args(&args[1..])
        .env("XDG_DATA_DIRS", &xdg)
        .env("GSETTINGS_SCHEMA_DIR", &schemas_str)
        .env("LOCALEDIR", &locale_str)
        .env("PATH", &path)
        .env("HOME", &home)
        .env("_RUSTCONN_BUNDLE_ENV_SET", "1")
        .current_dir(&home)
        .exec();

    // exec() only returns on error
    eprintln!("RustConn bundle re-exec failed: {err}");
}

fn main() -> gtk4::glib::ExitCode {
    // macOS .app bundle: detect bundle Resources path and set env vars.
    // When launched via LaunchServices (Finder/Dock/Launchpad), the process
    // has minimal environment. We detect the bundle by checking if our
    // executable lives inside a .app/Contents/MacOS/ directory.
    // This MUST happen before i18n::init() which reads LOCALEDIR.
    #[cfg(target_os = "macos")]
    setup_macos_bundle_env();

    // Initialize internationalization (gettext)
    i18n::init();

    // Work around blank popover/menu rendering on X11 with GTK4's default
    // NGL renderer.  On some X11 compositors (MATE, XFCE, older Mutter)
    // popovers appear empty until the pointer moves over them (#85).
    // Re-exec with GSK_RENDERER=cairo before GTK starts (same pattern as
    // the language re-exec in i18n.rs).  Wayland sessions are unaffected.
    // Skipped on macOS — no X11/Wayland there.
    #[cfg(not(target_os = "macos"))]
    ensure_x11_renderer_fallback();

    // Apply saved language from config BEFORE GTK starts.
    // This must happen early so that all gettext() calls during UI construction
    // use the correct locale. The LANGUAGE env var must be set before any
    // translatable string is evaluated.
    i18n::apply_language_from_config();

    // Initialize logging with environment filter (RUST_LOG)
    // Filter out noisy zbus debug messages (ProvideXdgActivationToken errors from ksni)
    // and IronRDP internal debug spam (Non-32 bpp compressed RLE_BITMAP_STREAM etc.)
    //
    // Note: expect() is acceptable here because:
    // 1. These are compile-time constant directives that are always valid
    // 2. Runtime creation failure at startup is unrecoverable - the app cannot function
    let filter = tracing_subscriber::EnvFilter::from_default_env()
        .add_directive(
            "zbus=warn"
                .parse()
                .expect("compile-time constant directive"),
        )
        .add_directive(
            "ironrdp=warn"
                .parse()
                .expect("compile-time constant directive"),
        )
        .add_directive(
            "ironrdp_session=warn"
                .parse()
                .expect("compile-time constant directive"),
        )
        .add_directive(
            "ironrdp_tokio=warn"
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

    // Initialize Tokio runtime for async operations.
    // Runtime creation can fail in extremely constrained environments
    // (no spare PIDs, ulimit reached, no /proc); exit gracefully with a clear
    // user-facing message rather than panicking (M-PANIC-IS-STOP — runtime
    // failure at startup is recoverable for the user, not a programming bug).
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("RustConn: failed to start async runtime: {err}");
            eprintln!(
                "Hint: this can happen if the process is hitting ulimits \
                 (file descriptors, threads) or if /proc is unavailable in \
                 a sandbox. Check `ulimit -a` and try again."
            );
            std::process::exit(2);
        }
    };
    let _guard = runtime.enter();

    // Parse CLI arguments for startup overrides (--shell, --connect)
    if let Some(action) = parse_cli_args() {
        set_cli_startup_override(action);
    }

    app::run()
}
