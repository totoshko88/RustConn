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
// Binary crate: pub items are internal helpers shared between modules, not a public API.
#![allow(
    unreachable_pub,
    reason = "binary crate — pub is for inter-module visibility only"
)]

pub mod activity_coordinator;
pub mod alert;
mod app;
pub mod async_utils;
#[cfg(feature = "rdp-audio")]
pub mod audio;
pub mod automation;
pub mod cairo_buffer;
pub mod dialogs;
pub mod display;
pub mod embedded;
pub mod embedded_rdp;
pub mod embedded_toolbar_overflow;
pub mod embedded_trait;
pub mod embedded_vnc;
pub mod embedded_vnc_types;
#[cfg(feature = "web-embedded")]
pub mod embedded_web;
pub mod external_session;
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

/// Detects if running inside a macOS .app bundle and returns the
/// bundle's Resources path for programmatic configuration.
///
/// Unlike the previous `setup_macos_bundle_env()` which used `exec()` to
/// re-launch with environment variables, this approach configures each
/// subsystem programmatically without re-exec. This preserves the macOS
/// LaunchServices "scene" registration which is required for NSStatusItem
/// (tray icon) to display correctly.
///
/// Environment variables that were previously set via re-exec:
/// - `LOCALEDIR` → now detected by `i18n::locale_dir()` via bundle path
/// - `XDG_DATA_DIRS` → icon paths added programmatically in `register_app_icon()`
/// - `GSETTINGS_SCHEMA_DIR` → schemas loaded via `configure_gsettings_schemas()`
/// - `PATH` → `get_extended_path()` already adds Homebrew paths per-spawn
#[cfg(target_os = "macos")]
fn configure_macos_bundle() {
    use std::env;

    // Detect bundle: executable is at .app/Contents/MacOS/rustconn
    let exe_path = match env::current_exe() {
        Ok(p) => p,
        Err(_) => return,
    };

    let macos_dir = exe_path.parent().unwrap_or(&exe_path);
    let contents_dir = macos_dir.parent().unwrap_or(macos_dir);
    let bundle_dir = contents_dir.parent().unwrap_or(contents_dir);

    // Verify this looks like a .app bundle
    let is_bundle = bundle_dir
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("app"));
    if !is_bundle {
        return;
    }

    // Store the bundle Resources path for use by other subsystems.
    // GSettings schemas: loaded later in app.rs via configure_gsettings_schemas().
    // LOCALEDIR: i18n::locale_dir() detects the bundle path automatically.
    // Icon paths: register_app_icon() in window/mod.rs already handles this.
    let resources_dir = contents_dir.join("Resources");
    MACOS_BUNDLE_RESOURCES.with(|cell| {
        *cell.borrow_mut() = Some(resources_dir);
    });

    tracing::debug!(
        bundle = %bundle_dir.display(),
        "Detected macOS .app bundle (no re-exec needed)"
    );
}

// Thread-local storage for the macOS bundle Resources path.
// Set by `configure_macos_bundle()` at startup, consumed by
// `configure_gsettings_schemas()` and other subsystems.
#[cfg(target_os = "macos")]
std::thread_local! {
    static MACOS_BUNDLE_RESOURCES: std::cell::RefCell<Option<std::path::PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

/// Returns the macOS bundle Resources path if running inside a .app bundle.
#[cfg(target_os = "macos")]
pub fn macos_bundle_resources_dir() -> Option<std::path::PathBuf> {
    MACOS_BUNDLE_RESOURCES.with(|cell| cell.borrow().clone())
}

/// Configures GSettings schema source from the macOS .app bundle.
///
/// GTK4/libadwaita on macOS needs compiled GSettings schemas for some
/// internal settings (cursor blink, font rendering hints). Without
/// `GSETTINGS_SCHEMA_DIR` or XDG_DATA_DIRS pointing to schemas, GTK
/// emits warnings but does NOT crash — it uses built-in defaults.
///
/// On macOS, the critical settings (dark mode, fonts) are handled by
/// native APIs (NSAppearance, CoreText) so missing GSettings schemas
/// only affects minor GTK internals like cursor-blink-time.
///
/// This function uses `gio::SettingsSchemaSource::from_directory()` to
/// validate schemas exist; the actual schema source initialization
/// relies on GTK's built-in XDG fallbacks. The bundle's `Resources/share`
/// is added to GTK's data path via `register_app_icon()` which also
/// helps GTK find schemas in `XDG_DATA_DIRS`-like locations.
///
/// Must be called AFTER `configure_macos_bundle()` and BEFORE GTK init.
#[cfg(target_os = "macos")]
pub fn configure_gsettings_schemas() {
    use gtk4::gio;

    let schema_dir = MACOS_BUNDLE_RESOURCES.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|resources| resources.join("share/glib-2.0/schemas"))
    });

    // Check if schemas are available somewhere
    let candidates = [
        schema_dir,
        Some(std::path::PathBuf::from(
            "/opt/homebrew/share/glib-2.0/schemas",
        )),
        Some(std::path::PathBuf::from(
            "/usr/local/share/glib-2.0/schemas",
        )),
    ];

    for candidate in candidates.into_iter().flatten() {
        if candidate.join("gschemas.compiled").exists() {
            // Verify schemas can be loaded from this directory.
            // Note: this creates a local schema source — GTK's internal
            // default source is initialized separately via XDG_DATA_DIRS.
            // On macOS this is acceptable: GTK uses native APIs for
            // critical settings, and GSettings is only used for minor
            // internal preferences that have sensible defaults.
            match gio::SettingsSchemaSource::from_directory(
                &candidate,
                gio::SettingsSchemaSource::default().as_ref(),
                false,
            ) {
                Ok(_source) => {
                    tracing::debug!(
                        path = %candidate.display(),
                        "GSettings schemas available at path"
                    );
                    return;
                }
                Err(e) => {
                    tracing::debug!(
                        path = %candidate.display(),
                        error = %e,
                        "Failed to load GSettings schemas (non-fatal)"
                    );
                }
            }
        }
    }

    tracing::debug!(
        "No GSettings schemas found — GTK will use defaults \
         (non-fatal on macOS: dark mode, fonts handled natively)"
    );
}

/// Installs a GLib log writer that drops the high-volume, harmless CSS
/// theme-parser warnings (libadwaita ≥1.9 stylesheet vs GTK4's CSS parser)
/// while forwarding every other GLib/GTK message to the default writer.
///
/// # Panics
///
/// Panics if a writer function was already installed; this is called exactly
/// once from `main`, so that cannot happen in practice.
fn install_glib_css_warning_filter() {
    use gtk4::glib;

    glib::log_set_writer_func(|level, fields| {
        let is_css_noise = fields.iter().any(|field| {
            field.key() == "MESSAGE"
                && field
                    .value_str()
                    .is_some_and(|msg| msg.contains("Theme parser") || msg.contains("gtk.css"))
        });
        if is_css_noise {
            glib::LogWriterOutput::Handled
        } else {
            glib::log_writer_default(level, fields)
        }
    });
}

fn main() -> gtk4::glib::ExitCode {
    // macOS .app bundle: detect bundle Resources path for programmatic
    // configuration of i18n, GSettings schemas, and icon paths.
    // Unlike the old approach, this does NOT re-exec the process —
    // preserving the LaunchServices scene needed for NSStatusItem (tray icon).
    // This MUST happen before i18n::init() which uses bundle detection.
    #[cfg(target_os = "macos")]
    configure_macos_bundle();

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

    // Drop the flood of harmless CSS theme-parser warnings GTK4 emits when it
    // reads the libadwaita ≥1.9 stylesheet (it uses CSS syntax the older GTK4
    // parser does not fully understand). They are non-fatal — documented in
    // docs/MACOS_BUILD.md — but at debug level they bury every other log line.
    install_glib_css_warning_filter();

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
