//! GTK4 Application setup and initialization
//!
//! This module provides the main application entry point and configuration
//! for the `RustConn` GTK4 application, including state management and
//! action setup.

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;
use std::cell::RefCell;
use std::rc::Rc;

use crate::state::{SharedAppState, create_shared_state};
use crate::tray::{TrayManager, TrayMessage};
use crate::window::MainWindow;
use gettextrs::gettext;
use rustconn_core::config::ColorScheme;

/// Applies a color scheme to GTK/libadwaita settings
pub fn apply_color_scheme(scheme: ColorScheme) {
    // For libadwaita applications, use StyleManager instead of GTK Settings
    let style_manager = adw::StyleManager::default();

    match scheme {
        ColorScheme::System => {
            style_manager.set_color_scheme(adw::ColorScheme::Default);
        }
        ColorScheme::Light => {
            style_manager.set_color_scheme(adw::ColorScheme::ForceLight);
        }
        ColorScheme::Dark => {
            style_manager.set_color_scheme(adw::ColorScheme::ForceDark);
        }
    }
}

/// Application ID for `RustConn`
pub const APP_ID: &str = "io.github.totoshko88.RustConn";

/// Shared tray manager type
type SharedTrayManager = Rc<RefCell<Option<TrayManager>>>;

/// Creates and configures the GTK4 Application
///
/// Sets up the application with Wayland-native configuration and
/// connects the activate signal to create the main window.
#[must_use]
pub fn create_application() -> adw::Application {
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::default())
        .build();

    // Create shared tray manager (will be initialized in build_ui)
    let tray_manager: SharedTrayManager = Rc::new(RefCell::new(None));

    app.connect_activate(move |app| {
        build_ui(app, tray_manager.clone());
    });

    // Keep the application running even when all windows are closed (for tray icon)
    app.set_accels_for_action("app.quit", &["<Control>q"]);

    app
}

/// Builds the main UI when the application is activated
fn build_ui(app: &adw::Application, tray_manager: SharedTrayManager) {
    // Load CSS styles for split view panes
    load_css_styles();

    // Force Adwaita icon theme on non-GNOME desktops (KDE, XFCE, etc.).
    // Libadwaita already renders all widgets in Adwaita style, but icon lookups
    // still go through the system icon theme. Many standard symbolic icons are
    // missing in Breeze/other themes.
    // GtkSettings::gtk-icon-theme-name controls the icon theme globally and
    // unlike IconTheme::set_theme_name, works on the display singleton.
    if let Some(display) = gtk4::gdk::Display::default() {
        let settings = gtk4::Settings::for_display(&display);
        let current = settings.gtk_icon_theme_name().unwrap_or_default();
        if current != "Adwaita" {
            settings.set_gtk_icon_theme_name(Some("Adwaita"));
            tracing::info!(
                previous_theme = %current,
                "Forced Adwaita icon theme for consistent icon availability"
            );
        }

        // Suppress the libadwaita warning about gtk-application-prefer-dark-theme.
        // Some desktop environments (KDE, XFCE) set this property globally.
        // Libadwaita uses AdwStyleManager::color-scheme instead, so we clear
        // the deprecated property to avoid the runtime warning.
        if settings.is_gtk_application_prefer_dark_theme() {
            settings.set_gtk_application_prefer_dark_theme(false);
            tracing::debug!(
                "Cleared deprecated gtk-application-prefer-dark-theme (using AdwStyleManager)"
            );
        }
    }

    // Create shared application state (fast — secret backends deferred)
    let state = match create_shared_state() {
        Ok(state) => state,
        Err(e) => {
            tracing::error!(%e, "Failed to initialize application state");
            show_error_dialog(app, &gettext("Initialization Error"), &e);
            return;
        }
    };

    // Apply saved color scheme from settings
    apply_saved_color_scheme(&state);

    // Apply saved language from settings
    apply_saved_language(&state);

    // Create main window with state
    let window = MainWindow::new(app, state.clone());

    // Initialize tray icon if enabled in settings
    let enable_tray = state.borrow().settings().ui.enable_tray_icon;
    if enable_tray && let Some(tray) = TrayManager::new() {
        // Update tray with initial state
        let mut initial_cache = TrayStateCache::default();
        update_tray_state(&tray, &state, &mut initial_cache);
        *tray_manager.borrow_mut() = Some(tray);
    }

    // Set up application actions
    setup_app_actions(app, &window, &state, tray_manager.clone());

    // Set up tray message polling
    setup_tray_polling(app, &window, state.clone(), tray_manager);

    // Connect shutdown signal to flush persistence
    let state_shutdown = state.clone();
    app.connect_shutdown(move |_| {
        if let Err(e) = state_shutdown.borrow().flush_persistence() {
            tracing::error!(%e, "Failed to flush persistence on shutdown");
        }
    });

    // Present window immediately — no waiting for secret backends
    window.present();

    // Execute startup action (CLI override takes precedence over settings)
    {
        use rustconn_core::config::StartupAction;

        // 1. Check CLI override (--shell or --connect <uuid>)
        let cli_action = crate::take_cli_startup_override();

        // 2. Check CLI --connect <name> (deferred name resolution)
        let cli_name_action = crate::take_cli_connect_name().and_then(|name| {
            let state_ref = state.borrow();
            state_ref
                .find_connection_by_name(&name)
                .map(|conn| StartupAction::Connection(conn.id))
                .or_else(|| {
                    tracing::warn!(name, "CLI --connect: connection not found by name");
                    None
                })
        });

        // CLI args override persisted setting
        let action = cli_action
            .or(cli_name_action)
            .unwrap_or_else(|| state.borrow().settings().ui.startup_action.clone());

        window.execute_startup_action(&action);
    }

    // Initialize secret backends in a background thread after the window is visible.
    // Decryption is fast and runs on the main thread; the slow Bitwarden vault
    // unlock runs in a background thread to avoid blocking the GTK main loop.
    let state_for_secrets = state.clone();
    let sidebar_for_secrets = window.sidebar_rc();
    let window_for_secrets = window.gtk_window().downgrade();
    glib::idle_add_local_once(move || {
        // Phase 1: Decrypt stored credentials (fast, needs &mut self)
        {
            let mut state_ref = state_for_secrets.borrow_mut();
            let settings = &mut state_ref.settings_mut().secrets;

            if settings.bitwarden_password_encrypted.is_some()
                && settings.decrypt_bitwarden_password()
            {
                tracing::info!("Bitwarden password restored from encrypted storage");
            }

            if settings.bitwarden_use_api_key
                && (settings.bitwarden_client_id_encrypted.is_some()
                    || settings.bitwarden_client_secret_encrypted.is_some())
                && settings.decrypt_bitwarden_api_credentials()
            {
                tracing::info!("Bitwarden API credentials restored from encrypted storage");
            }
        }

        // Phase 2: Check if Bitwarden auto-unlock is needed
        let needs_bitwarden = {
            let state_ref = state_for_secrets.borrow();
            matches!(
                state_ref.settings().secrets.preferred_backend,
                rustconn_core::config::SecretBackendType::Bitwarden
            )
        };

        if needs_bitwarden {
            // Clone settings for the background thread (Send + 'static)
            let secret_settings = state_for_secrets.borrow().settings().secrets.clone();

            // Resolve bw CLI path before background unlock (probes Flatpak dirs, PATH)
            let _ = rustconn_core::secret::resolve_bw_cmd();

            // Channel to receive the result on the GTK main thread
            let (tx, rx) = std::sync::mpsc::channel::<bool>();

            // Run slow Bitwarden unlock in a background thread
            std::thread::spawn(move || {
                let rt = match tokio::runtime::Runtime::new() {
                    Ok(rt) => rt,
                    Err(e) => {
                        tracing::warn!("Failed to create runtime for Bitwarden unlock: {e}");
                        let _ = tx.send(false);
                        return;
                    }
                };

                match rt.block_on(rustconn_core::secret::auto_unlock(&secret_settings)) {
                    Ok(_) => {
                        tracing::info!("Bitwarden vault unlocked at startup");
                        let _ = tx.send(true);
                    }
                    Err(e) => {
                        tracing::warn!("Bitwarden auto-unlock at startup failed: {e}");
                        let _ = tx.send(false);
                    }
                }
            });

            // Poll for the result on the GTK main thread (non-blocking)
            let state_for_poll = state_for_secrets.clone();
            let sidebar_for_poll = sidebar_for_secrets.clone();
            let window_for_poll = window_for_secrets.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                match rx.try_recv() {
                    Ok(_) => {
                        state_for_poll.borrow_mut().refresh_secret_backend_cache();
                        refresh_sidebar_secret_status(&state_for_poll, &sidebar_for_poll);
                        check_secret_backend_available(&state_for_poll, &window_for_poll);
                        tracing::info!("Secret backends initialized after window presentation");
                        glib::ControlFlow::Break
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        state_for_poll.borrow_mut().refresh_secret_backend_cache();
                        refresh_sidebar_secret_status(&state_for_poll, &sidebar_for_poll);
                        check_secret_backend_available(&state_for_poll, &window_for_poll);
                        tracing::warn!("Bitwarden unlock thread disconnected");
                        glib::ControlFlow::Break
                    }
                }
            });
        } else {
            // No Bitwarden — just refresh sidebar status immediately
            state_for_secrets
                .borrow_mut()
                .refresh_secret_backend_cache();
            refresh_sidebar_secret_status(&state_for_secrets, &sidebar_for_secrets);
            check_secret_backend_available(&state_for_secrets, &window_for_secrets);
            tracing::info!("Secret backends initialized after window presentation");
        }
    });
}

/// Updates the tray icon state from the application state
///
/// Only updates if state has actually changed to avoid unnecessary work.
fn update_tray_state(tray: &TrayManager, state: &SharedAppState, last_state: &mut TrayStateCache) {
    let state_ref = state.borrow();

    // Update active session count only if changed
    let session_count = state_ref.active_sessions().len();
    #[allow(clippy::cast_possible_truncation)]
    let session_count_u32 = session_count as u32;

    if last_state.session_count != session_count_u32 {
        tray.set_active_sessions(session_count_u32);
        last_state.session_count = session_count_u32;
    }

    // Update recent connections only if connection list has changed
    // Use a simple hash of connection count + last_connected timestamps as dirty check
    let connections_hash = state_ref
        .list_connections()
        .iter()
        .filter(|c| c.last_connected.is_some())
        .map(|c| c.last_connected.map_or(0, |t| t.timestamp()))
        .sum::<i64>();

    if last_state.connections_hash != connections_hash {
        let mut connections: Vec<_> = state_ref
            .list_connections()
            .iter()
            .filter(|c| c.last_connected.is_some())
            .map(|c| (c.id, c.name.clone(), c.last_connected))
            .collect();
        connections.sort_by(|a, b| b.2.cmp(&a.2));
        let recent: Vec<_> = connections
            .into_iter()
            .take(10)
            .map(|(id, name, _)| (id, name))
            .collect();
        tray.set_recent_connections(recent);
        last_state.connections_hash = connections_hash;
    }
}

/// Cache for tray state to avoid unnecessary updates
#[derive(Default)]
struct TrayStateCache {
    session_count: u32,
    connections_hash: i64,
}

/// Sets up event-driven tray message handling and periodic state sync.
///
/// Tray messages (user clicks) are checked every 50ms via lightweight
/// `try_recv()`. Tray state (session count, recent connections) is synced
/// every 2 seconds with dirty-flag tracking to minimize D-Bus calls.
fn setup_tray_polling(
    app: &adw::Application,
    window: &MainWindow,
    state: SharedAppState,
    tray_manager: SharedTrayManager,
) {
    let app_weak = app.downgrade();
    let window_weak = window.gtk_window().downgrade();

    // --- Fast message polling (50ms) ---
    // Only checks try_recv() which is non-blocking and very cheap.
    let tray_for_msgs = tray_manager.clone();
    let app_for_msgs = app_weak;
    let window_for_msgs = window_weak;
    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        let Some(app) = app_for_msgs.upgrade() else {
            return glib::ControlFlow::Break;
        };

        let tray_ref = tray_for_msgs.borrow();
        let Some(tray) = tray_ref.as_ref() else {
            return glib::ControlFlow::Continue;
        };

        while let Some(msg) = tray.try_recv() {
            match msg {
                TrayMessage::ShowWindow => {
                    if let Some(win) = window_for_msgs.upgrade() {
                        win.present();
                    }
                    tray.set_window_visible(true);
                }
                TrayMessage::HideWindow => {
                    if let Some(win) = window_for_msgs.upgrade() {
                        win.set_visible(false);
                    }
                    tray.set_window_visible(false);
                }
                TrayMessage::ToggleWindow => {
                    if let Some(win) = window_for_msgs.upgrade() {
                        if win.is_visible() {
                            win.set_visible(false);
                            tray.set_window_visible(false);
                        } else {
                            win.present();
                            tray.set_window_visible(true);
                        }
                    }
                }
                TrayMessage::Connect(conn_id) => {
                    if let Some(win) = window_for_msgs.upgrade() {
                        win.present();
                        tray.set_window_visible(true);
                        let _ = gtk4::prelude::WidgetExt::activate_action(
                            &win,
                            "connect",
                            Some(&conn_id.to_string().to_variant()),
                        );
                    }
                }
                TrayMessage::QuickConnect => {
                    if let Some(win) = window_for_msgs.upgrade() {
                        win.present();
                        tray.set_window_visible(true);
                        let _ =
                            gtk4::prelude::WidgetExt::activate_action(&win, "quick-connect", None);
                    }
                }
                TrayMessage::LocalShell => {
                    if let Some(win) = window_for_msgs.upgrade() {
                        win.present();
                        tray.set_window_visible(true);
                        let _ =
                            gtk4::prelude::WidgetExt::activate_action(&win, "local-shell", None);
                    }
                }
                TrayMessage::About => {
                    if let Some(win) = window_for_msgs.upgrade() {
                        win.present();
                        tray.set_window_visible(true);
                    }
                    gio::prelude::ActionGroupExt::activate_action(&app, "about", None);
                }
                TrayMessage::Quit => {
                    app.quit();
                }
            }
        }

        glib::ControlFlow::Continue
    });

    // --- Slow state sync (2 seconds) ---
    // Updates session count and recent connections with dirty-flag tracking.
    let state_clone = state;
    let tray_for_state = tray_manager;
    let state_cache = std::rc::Rc::new(std::cell::RefCell::new(TrayStateCache::default()));

    glib::timeout_add_local(std::time::Duration::from_secs(2), move || {
        let tray_ref = tray_for_state.borrow();
        let Some(tray) = tray_ref.as_ref() else {
            return glib::ControlFlow::Continue;
        };
        update_tray_state(tray, &state_clone, &mut state_cache.borrow_mut());
        glib::ControlFlow::Continue
    });
}

/// Refreshes the sidebar secret backend status indicator.
fn refresh_sidebar_secret_status(
    state: &SharedAppState,
    sidebar: &std::rc::Rc<crate::sidebar::ConnectionSidebar>,
) {
    let state_ref = state.borrow();
    let settings = state_ref.settings();
    let backend = settings.secrets.preferred_backend;
    let (enabled, database_exists) = match backend {
        rustconn_core::config::SecretBackendType::LibSecret
        | rustconn_core::config::SecretBackendType::Bitwarden
        | rustconn_core::config::SecretBackendType::OnePassword
        | rustconn_core::config::SecretBackendType::Passbolt
        | rustconn_core::config::SecretBackendType::Pass => (true, true),
        rustconn_core::config::SecretBackendType::KeePassXc
        | rustconn_core::config::SecretBackendType::KdbxFile => {
            let kdbx_enabled = settings.secrets.kdbx_enabled;
            let db_exists = settings
                .secrets
                .kdbx_path
                .as_ref()
                .is_some_and(|p| p.exists());
            (kdbx_enabled, db_exists)
        }
    };
    sidebar.update_keepass_status(enabled, database_exists);
}

/// Shows a one-time warning toast if the preferred secret backend is unavailable.
fn check_secret_backend_available(
    state: &SharedAppState,
    window_weak: &glib::WeakRef<adw::ApplicationWindow>,
) {
    let state_ref = state.borrow();
    let backend = state_ref.settings().secrets.preferred_backend;

    // Only warn for non-default backends (LibSecret is always the fallback)
    if matches!(backend, rustconn_core::config::SecretBackendType::LibSecret) {
        return;
    }

    let available = state_ref.has_secret_backend();
    drop(state_ref);

    if !available && let Some(win) = window_weak.upgrade() {
        let backend_name = format!("{backend:?}");
        let msg = crate::i18n::i18n_f("{} backend unavailable. Using fallback.", &[&backend_name]);
        tracing::warn!(backend = %backend_name, "Preferred secret backend unavailable at startup");
        crate::toast::show_toast_on_window(&win, &msg, crate::toast::ToastType::Warning);
    }
}

/// Loads CSS styles for the application from external stylesheet
fn load_css_styles() {
    let provider = gtk4::CssProvider::new();
    provider.load_from_string(include_str!("../assets/style.css"));

    // Use safe display access
    if !crate::utils::add_css_provider(&provider, gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION) {
        tracing::warn!("Failed to add CSS provider - no display available");
    }
}

/// Sets up application-level actions
fn setup_app_actions(
    app: &adw::Application,
    window: &MainWindow,
    state: &SharedAppState,
    _tray_manager: SharedTrayManager,
) {
    // Quit action - save expanded groups state before quitting
    let quit_action = gio::SimpleAction::new("quit", None);
    let app_weak = app.downgrade();
    let state_clone = state.clone();
    let sidebar_rc = window.sidebar_rc();
    quit_action.connect_activate(move |_, _| {
        // Save expanded groups state
        let expanded = sidebar_rc.get_expanded_groups();
        if let Ok(mut state_ref) = state_clone.try_borrow_mut() {
            let _ = state_ref.update_expanded_groups(expanded);
        }
        if let Some(app) = app_weak.upgrade() {
            app.quit();
        }
    });
    app.add_action(&quit_action);

    // About action
    let about_action = gio::SimpleAction::new("about", None);
    let window_weak = window.gtk_window().downgrade();
    about_action.connect_activate(move |_, _| {
        if let Some(window) = window_weak.upgrade() {
            show_about_dialog(&window);
        }
    });
    app.add_action(&about_action);

    // Keyboard shortcuts action
    let shortcuts_action = gio::SimpleAction::new("shortcuts", None);
    let window_weak = window.gtk_window().downgrade();
    shortcuts_action.connect_activate(move |_, _| {
        if let Some(window) = window_weak.upgrade() {
            let dialog = crate::dialogs::ShortcutsDialog::new(Some(&window));
            dialog.show();
        }
    });
    app.add_action(&shortcuts_action);

    // Set up keyboard shortcuts dynamically from settings
    apply_keybindings(app, state);
}

/// Applies keyboard shortcuts from settings, falling back to defaults.
///
/// Reads the keybinding registry from `rustconn_core::default_keybindings()` and
/// applies user overrides from `AppSettings.keybindings`. This is the single
/// source of truth for all application keyboard shortcuts.
///
/// Note: Enter, Delete, Ctrl+E, Ctrl+D are NOT registered globally to avoid
/// intercepting keys when VTE terminal or embedded viewers have focus.
/// These are handled by the sidebar's `EventControllerKey` instead.
/// See: <https://github.com/totoshko88/RustConn/issues/4>
pub fn apply_keybindings(app: &adw::Application, state: &SharedAppState) {
    let keybinding_settings = state.borrow().settings().keybindings.clone();
    let defaults = rustconn_core::default_keybindings();

    for def in &defaults {
        let accel_str = keybinding_settings.get_accel(def);
        let accels: Vec<&str> = accel_str.split('|').collect();
        app.set_accels_for_action(&def.action, &accels);
    }
}

/// Shows the about dialog
fn show_about_dialog(parent: &adw::ApplicationWindow) {
    let description = gettext(
        "Modern connection manager for Linux with a \
GTK4/Wayland-native interface. Manage SSH, RDP, VNC, SPICE, Telnet, \
Serial, Kubernetes, and Zero Trust connections from a single application.",
    );

    // Build debug info for troubleshooting
    let debug_info = format!(
        "RustConn {version}\n\
         GTK {gtk_major}.{gtk_minor}.{gtk_micro}\n\
         libadwaita {adw_major}.{adw_minor}.{adw_micro}\n\
         Rust {rust_version}\n\
         OS: {os}",
        version = env!("CARGO_PKG_VERSION"),
        gtk_major = gtk4::major_version(),
        gtk_minor = gtk4::minor_version(),
        gtk_micro = gtk4::micro_version(),
        adw_major = adw::major_version(),
        adw_minor = adw::minor_version(),
        adw_micro = adw::micro_version(),
        rust_version = env!("CARGO_PKG_RUST_VERSION"),
        os = std::env::consts::OS,
    );

    let about = adw::AboutDialog::builder()
        .application_name("RustConn")
        .developer_name("Anton Isaiev")
        .version(env!("CARGO_PKG_VERSION"))
        .comments(&description)
        .website("https://github.com/totoshko88/RustConn")
        .issue_url("https://github.com/totoshko88/rustconn/issues")
        .support_url("https://ko-fi.com/totoshko88")
        .license_type(gtk4::License::Gpl30)
        .developers(vec!["Anton Isaiev <totoshko88@gmail.com>"])
        .copyright("© 2024-2026 Anton Isaiev")
        .application_icon("io.github.totoshko88.RustConn")
        // Translators: Replace this with your name and language, e.g. "John Doe (German)"
        .translator_credits(&gettext("translator-credits"))
        .debug_info(&debug_info)
        .debug_info_filename("rustconn-debug-info.txt")
        .build();

    // Documentation & resources links
    about.add_link(
        &gettext("User Guide"),
        "https://github.com/totoshko88/RustConn/blob/main/docs/USER_GUIDE.md",
    );
    about.add_link(
        &gettext("Installation"),
        "https://github.com/totoshko88/RustConn/blob/main/docs/INSTALL.md",
    );
    about.add_link(
        &gettext("Releases"),
        "https://github.com/totoshko88/RustConn/releases",
    );
    about.add_link(
        &gettext("Changelog"),
        "https://github.com/totoshko88/RustConn/blob/main/CHANGELOG.md",
    );

    // Support/sponsorship links
    about.add_link("Ko-Fi", "https://ko-fi.com/totoshko88");
    about.add_link("PayPal", "https://www.paypal.com/paypalme/totoshko88");
    about.add_link("Monobank", "https://send.monobank.ua/jar/2UgaGcQ3JC");

    // Acknowledgments
    about.add_acknowledgement_section(
        Some(&gettext("Special Thanks")),
        &[
            "GTK4 and the GNOME project https://www.gtk.org",
            "The Rust community https://www.rust-lang.org",
            "IronRDP project https://github.com/Devolutions/IronRDP",
            "FreeRDP project https://www.freerdp.com",
            "Midnight Commander https://midnight-commander.org",
            "virt-manager / virt-viewer https://virt-manager.org",
            "TigerVNC project https://tigervnc.org",
            "vnc-rs project https://github.com/niclas3640/vnc-rs",
            "KeePassXC project https://keepassxc.org",
            "VTE terminal library https://wiki.gnome.org/Apps/Terminal/VTE",
        ],
    );
    about.add_acknowledgement_section(
        Some(&gettext("Made in Ukraine")),
        &[&gettext("All contributors and supporters")],
    );

    // Legal sections for key dependencies
    about.add_legal_section(
        "GTK4, libadwaita & VTE",
        Some("© The GNOME Project"),
        gtk4::License::Lgpl21,
        None,
    );
    about.add_legal_section(
        "IronRDP",
        Some("© Devolutions Inc."),
        gtk4::License::MitX11,
        None,
    );

    about.present(Some(parent));
}

/// Shows an error dialog
fn show_error_dialog(app: &adw::Application, title: &str, message: &str) {
    let dialog = adw::AlertDialog::new(Some(title), Some(message));
    dialog.add_response("ok", "OK");
    dialog.set_default_response(Some("ok"));

    // Create a temporary window to show the dialog
    let window = adw::ApplicationWindow::builder().application(app).build();

    dialog.present(Some(&window));
}

/// Runs the GTK4 application
///
/// This is the main entry point that initializes GTK and runs the event loop.
///
/// # Returns
///
/// Returns `glib::ExitCode::FAILURE` if libadwaita initialization fails,
/// otherwise returns the application's exit code.
pub fn run() -> glib::ExitCode {
    // Initialize libadwaita before creating the application
    if let Err(e) = adw::init() {
        tracing::error!(%e, "Failed to initialize libadwaita");
        return glib::ExitCode::FAILURE;
    }

    let app = create_application();
    app.run()
}

/// Applies the saved color scheme from settings to GTK
fn apply_saved_color_scheme(state: &SharedAppState) {
    let color_scheme = {
        let state_ref = state.borrow();
        state_ref.settings().ui.color_scheme
    };

    apply_color_scheme(color_scheme);
}

/// Applies the saved language from settings to gettext
fn apply_saved_language(state: &SharedAppState) {
    let language = {
        let state_ref = state.borrow();
        state_ref.settings().ui.language.clone()
    };

    crate::i18n::apply_language(&language);
}
