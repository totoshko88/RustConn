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

use crate::state::{create_shared_state, SharedAppState};
use crate::tray::{TrayManager, TrayMessage};
use crate::window::MainWindow;
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

    // Create shared application state
    let state = match create_shared_state() {
        Ok(state) => state,
        Err(e) => {
            eprintln!("Failed to initialize application state: {e}");
            show_error_dialog(app, "Initialization Error", &e);
            return;
        }
    };

    // Apply saved color scheme from settings
    apply_saved_color_scheme(&state);

    // Create main window with state
    let window = MainWindow::new(app, state.clone());

    // Initialize tray icon if enabled in settings
    let enable_tray = state.borrow().settings().ui.enable_tray_icon;
    if enable_tray {
        if let Some(tray) = TrayManager::new() {
            // Update tray with initial state
            let mut initial_cache = TrayStateCache::default();
            update_tray_state(&tray, &state, &mut initial_cache);
            *tray_manager.borrow_mut() = Some(tray);
        }
    }

    // Set up application actions
    setup_app_actions(app, &window, &state, tray_manager.clone());

    // Set up tray message polling
    setup_tray_polling(app, &window, state, tray_manager);

    window.present();
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

/// Sets up polling for tray messages
///
/// Uses a 250ms interval (reduced from 100ms) with dirty-flag tracking
/// to minimize CPU usage when idle.
fn setup_tray_polling(
    app: &adw::Application,
    window: &MainWindow,
    state: SharedAppState,
    tray_manager: SharedTrayManager,
) {
    let app_weak = app.downgrade();
    let window_weak = window.gtk_window().downgrade();
    let state_clone = state;
    let tray_manager_clone = tray_manager;

    // State cache to track changes and avoid unnecessary updates
    let state_cache = std::rc::Rc::new(std::cell::RefCell::new(TrayStateCache::default()));

    // Poll for tray messages every 250ms (increased from 100ms for better efficiency)
    // Message handling is still responsive enough for user interactions
    glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
        let Some(app) = app_weak.upgrade() else {
            return glib::ControlFlow::Break;
        };

        let tray_ref = tray_manager_clone.borrow();
        let Some(tray) = tray_ref.as_ref() else {
            return glib::ControlFlow::Continue;
        };

        // Process any pending tray messages
        while let Some(msg) = tray.try_recv() {
            match msg {
                TrayMessage::ShowWindow => {
                    if let Some(win) = window_weak.upgrade() {
                        win.present();
                    }
                    tray.set_window_visible(true);
                }
                TrayMessage::HideWindow => {
                    if let Some(win) = window_weak.upgrade() {
                        win.set_visible(false);
                    }
                    tray.set_window_visible(false);
                }
                TrayMessage::ToggleWindow => {
                    if let Some(win) = window_weak.upgrade() {
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
                    // Show window first
                    if let Some(win) = window_weak.upgrade() {
                        win.present();
                        tray.set_window_visible(true);
                        // Trigger connection via window action
                        let _ = gtk4::prelude::WidgetExt::activate_action(
                            &win,
                            "connect",
                            Some(&conn_id.to_string().to_variant()),
                        );
                    }
                }
                TrayMessage::QuickConnect => {
                    // Show window and trigger quick connect dialog
                    if let Some(win) = window_weak.upgrade() {
                        win.present();
                        tray.set_window_visible(true);
                        // Activate window action
                        let _ =
                            gtk4::prelude::WidgetExt::activate_action(&win, "quick-connect", None);
                    }
                }
                TrayMessage::LocalShell => {
                    // Show window and open local shell
                    if let Some(win) = window_weak.upgrade() {
                        win.present();
                        tray.set_window_visible(true);
                        // Activate window action
                        let _ =
                            gtk4::prelude::WidgetExt::activate_action(&win, "local-shell", None);
                    }
                }
                TrayMessage::About => {
                    // Show about dialog (app-level action)
                    if let Some(win) = window_weak.upgrade() {
                        win.present();
                        tray.set_window_visible(true);
                    }
                    // About is an app action
                    gio::prelude::ActionGroupExt::activate_action(&app, "about", None);
                }
                TrayMessage::Quit => {
                    app.quit();
                }
            }
        }

        // Update tray state only if changed (dirty-flag tracking)
        update_tray_state(tray, &state_clone, &mut state_cache.borrow_mut());

        glib::ControlFlow::Continue
    });
}

/// Loads CSS styles for the application
fn load_css_styles() {
    let provider = gtk4::CssProvider::new();
    provider.load_from_string(
        r"
        /* Split view pane styles */
        .focused-pane {
            border: 2px solid @accent_color;
            border-radius: 4px;
        }

        .unfocused-pane {
            border: 1px solid @borders;
            border-radius: 4px;
        }

        /* Pane placeholder styles */
        .dim-label {
            opacity: 0.6;
        }

        /* Session tab styles - adaptive tabs */
        .session-tab {
            padding: 4px 6px;
            border-radius: 4px;
            min-height: 24px;
        }

        .session-tab:hover {
            background-color: alpha(@theme_fg_color, 0.08);
        }

        .tab-icon {
            opacity: 0.8;
        }

        .tab-label {
            margin-left: 4px;
            margin-right: 4px;
        }

        .tab-label-disconnected {
            margin-left: 4px;
            margin-right: 4px;
            color: @error_color;
        }

        .tab-close-button {
            min-width: 20px;
            min-height: 20px;
            padding: 2px;
            opacity: 0.6;
        }

        .tab-close-button:hover {
            opacity: 1.0;
            background-color: alpha(@error_color, 0.15);
        }

        /* Notebook tab styling for many tabs */
        notebook > header > tabs > tab {
            min-width: 40px;
            padding: 4px 8px;
        }

        notebook > header > tabs > tab label {
            min-width: 0;
        }

        /* Quick Filter button styles */
        .filter-button {
            min-width: 48px;
            padding: 4px 8px;
            font-size: 0.9em;
            font-weight: 500;
        }

        .filter-button:hover {
            background-color: alpha(@theme_fg_color, 0.08);
        }

        .filter-button.suggested-action {
            background-color: @accent_color;
            color: @accent_fg_color;
        }

        .filter-button.suggested-action:hover {
            background-color: alpha(@accent_color, 0.8);
        }

        /* Multiple filter active state - shows when 2+ filters are selected */
        .filter-button.filter-active-multiple {
            background-color: @accent_color;
            color: @accent_fg_color;
            border: 2px solid alpha(@accent_color, 0.6);
            box-shadow: 0 0 0 1px alpha(@accent_color, 0.3);
        }

        .filter-button.filter-active-multiple:hover {
            background-color: alpha(@accent_color, 0.9);
            box-shadow: 0 0 0 2px alpha(@accent_color, 0.4);
        }

        /* Floating controls styles - Requirement 5.2 */
        .floating-controls {
            background-color: alpha(@window_bg_color, 0.85);
            border-radius: 8px;
            padding: 6px 12px;
            box-shadow: 0 2px 8px alpha(black, 0.3);
            border: 1px solid alpha(@borders, 0.5);
        }

        .floating-control-button {
            min-width: 36px;
            min-height: 36px;
            padding: 8px;
            border-radius: 6px;
            background-color: transparent;
            transition: background-color 150ms ease-in-out,
                        transform 100ms ease-in-out;
        }

        .floating-control-button:hover {
            background-color: alpha(@accent_color, 0.15);
            transform: scale(1.05);
        }

        .floating-control-button:active {
            background-color: alpha(@accent_color, 0.25);
            transform: scale(0.95);
        }

        .floating-control-button.destructive-action {
            color: @error_color;
        }

        .floating-control-button.destructive-action:hover {
            background-color: alpha(@error_color, 0.15);
        }

        .floating-control-button.destructive-action:active {
            background-color: alpha(@error_color, 0.25);
        }

        /* VNC display placeholder */
        .vnc-display {
            background-color: @view_bg_color;
        }

        /* Toast notification styles */
        .toast-container {
            background-color: alpha(@theme_bg_color, 0.95);
            border-radius: 8px;
            padding: 12px 16px;
            box-shadow: 0 2px 8px alpha(black, 0.3);
            border: 1px solid alpha(@borders, 0.5);
        }

        .toast-label {
            font-weight: 500;
        }

        .toast-info {
            border-left: 4px solid @accent_bg_color;
        }

        .toast-success {
            border-left: 4px solid @success_color;
            background-color: alpha(@success_color, 0.1);
        }

        .toast-warning {
            border-left: 4px solid @warning_color;
            background-color: alpha(@warning_color, 0.1);
        }

        .toast-error {
            border-left: 4px solid @error_color;
            background-color: alpha(@error_color, 0.1);
        }

        /* Validation styles */
        entry.error {
            border-color: @error_color;
            box-shadow: 0 0 0 1px @error_color;
        }

        entry.warning {
            border-color: @warning_color;
            box-shadow: 0 0 0 1px @warning_color;
        }

        entry.success {
            border-color: @success_color;
        }

        label.error {
            color: @error_color;
            font-size: 0.9em;
        }

        label.warning {
            color: @warning_color;
            font-size: 0.9em;
        }

        /* Monospace text for technical details */
        .monospace {
            font-family: monospace;
            font-size: 0.9em;
        }

        /* Keyboard shortcuts dialog styles */
        .keycap {
            background-color: alpha(@theme_fg_color, 0.1);
            border: 1px solid alpha(@borders, 0.5);
            border-radius: 4px;
            padding: 2px 8px;
            font-family: monospace;
            font-size: 0.9em;
            min-width: 24px;
        }

        /* Empty state styles */
        .empty-state {
            padding: 48px;
        }

        .empty-state-icon {
            opacity: 0.3;
        }

        .empty-state-title {
            font-size: 1.4em;
            font-weight: bold;
            margin-top: 12px;
        }

        .empty-state-description {
            opacity: 0.7;
            margin-top: 6px;
        }

        /* Loading spinner styles */
        .loading-spinner {
            min-width: 32px;
            min-height: 32px;
        }

        /* Connection status animations */
        /* Note: GTK4 CSS doesn't support @keyframes, using opacity for visual feedback */
        .status-connecting {
            opacity: 0.6;
        }

        /* Enhanced drag-drop visual feedback */
        .drag-source-active {
            opacity: 0.6;
            transform: scale(0.98);
        }

        .drop-zone-active {
            background-color: alpha(@accent_bg_color, 0.1);
            border: 2px dashed @accent_bg_color;
            border-radius: 6px;
        }

        /* Form field hint styles */
        .field-hint {
            font-size: 0.85em;
            opacity: 0.7;
            margin-top: 2px;
        }

        /* Theme toggle button group */
        .theme-toggle-group button {
            min-width: 70px;
        }

        .theme-toggle-group button:checked {
            background-color: @accent_bg_color;
            color: @accent_fg_color;
        }

        /* Status indicator styles for settings dialog */
        .success {
            color: @success_color;
        }

        .warning {
            color: @warning_color;
        }

        .error {
            color: @error_color;
        }

        /* Status icons with better visibility */
        label.success {
            color: @success_color;
            font-weight: 600;
        }

        label.warning {
            color: @warning_color;
            font-weight: 600;
        }

        label.error {
            color: @error_color;
            font-weight: 600;
        }

        /* Heading styles for settings sections */
        .heading {
            font-weight: 600;
            font-size: 0.95em;
        }

        /* Context menu destructive button - ensure text is visible */
        .context-menu-destructive {
            color: @error_color;
        }

        .context-menu-destructive:hover {
            background-color: alpha(@error_color, 0.1);
            color: @error_color;
        }
        ",
    );

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

    // Set up keyboard shortcuts
    // Application shortcuts
    app.set_accels_for_action("app.quit", &["<Control>q"]);
    app.set_accels_for_action("app.shortcuts", &["<Control>question", "F1"]);

    // Connection management shortcuts
    app.set_accels_for_action("win.new-connection", &["<Control>n"]);
    app.set_accels_for_action("win.new-group", &["<Control><Shift>n"]);
    app.set_accels_for_action("win.import", &["<Control>i"]);
    // Note: Enter key is NOT bound globally to avoid intercepting terminal input
    // Use double-click on sidebar items to connect instead
    app.set_accels_for_action("win.edit-connection", &["<Control>e"]);
    app.set_accels_for_action("win.delete-connection", &["Delete"]);
    app.set_accels_for_action("win.duplicate-connection", &["<Control>d"]);

    // Navigation shortcuts
    app.set_accels_for_action("win.search", &["<Control>f", "<Control>k"]);
    app.set_accels_for_action("win.focus-sidebar", &["<Control>1", "<Alt>1"]);
    app.set_accels_for_action("win.focus-terminal", &["<Control>2", "<Alt>2"]);

    // Terminal shortcuts
    app.set_accels_for_action("win.copy", &["<Control><Shift>c"]);
    app.set_accels_for_action("win.paste", &["<Control><Shift>v"]);
    app.set_accels_for_action("win.terminal-search", &["<Control><Shift>f"]);
    app.set_accels_for_action("win.close-tab", &["<Control>w"]);
    app.set_accels_for_action("win.next-tab", &["<Control>Tab", "<Control>Page_Down"]);
    app.set_accels_for_action("win.prev-tab", &["<Control><Shift>Tab", "<Control>Page_Up"]);

    // Settings
    app.set_accels_for_action("win.settings", &["<Control>comma"]);

    // New actions
    app.set_accels_for_action("win.local-shell", &["<Control><Shift>t"]);
    app.set_accels_for_action("win.quick-connect", &["<Control><Shift>q"]);
    app.set_accels_for_action("win.export", &["<Control><Shift>e"]);

    // Split view shortcuts
    app.set_accels_for_action("win.split-horizontal", &["<Control><Shift>h"]);
    app.set_accels_for_action("win.split-vertical", &["<Control><Shift>s"]);
    app.set_accels_for_action("win.close-pane", &["<Control><Shift>w"]);
    app.set_accels_for_action("win.focus-next-pane", &["<Control>grave"]); // Ctrl+`

    // View shortcuts
    app.set_accels_for_action("win.toggle-fullscreen", &["F11"]);
}

/// Shows the about dialog
fn show_about_dialog(parent: &adw::ApplicationWindow) {
    let description = "Modern GTK4/libadwaita connection manager for Linux, \
designed with Wayland-first approach.\n\n\
Developed by Anton Isaiev, 2024-2026";

    let about = adw::AboutDialog::builder()
        .application_name("RustConn")
        .developer_name("Modern connection manager for Linux")
        .version(env!("CARGO_PKG_VERSION"))
        .comments(description)
        .website("https://github.com/totoshko88/RustConn")
        .issue_url("https://github.com/totoshko88/rustconn/issues")
        .license_type(gtk4::License::Gpl30)
        .developers(vec!["Anton Isaiev <totoshko88@gmail.com>"])
        .copyright("© 2024-2026 Anton Isaiev")
        .application_icon("io.github.totoshko88.RustConn")
        .build();

    // Add documentation links
    about.add_link(
        "📖 User Guide",
        "https://github.com/totoshko88/RustConn/blob/main/docs/USER_GUIDE.md",
    );
    about.add_link(
        "📦 Installation",
        "https://github.com/totoshko88/RustConn/blob/main/docs/INSTALL.md",
    );
    about.add_link(
        "🚀 Releases",
        "https://github.com/totoshko88/RustConn/releases",
    );
    about.add_link(
        "📜 License (GPL v3.0)",
        "https://www.gnu.org/licenses/gpl-3.0.html",
    );

    // Add support/sponsorship information
    about.add_credit_section(
        Some("Support the Project"),
        &[
            "☕ Ko-Fi: https://ko-fi.com/totoshko88",
            "💳 PayPal: https://www.paypal.com/qrcodes/p2pqrc/JJLUXRZSQ5V3A",
            "🇺🇦 Monobank: https://send.monobank.ua/jar/2UgaGcQ3JC",
        ],
    );

    about.add_credit_section(
        Some("Acknowledgments"),
        &[
            "GTK4 and the GNOME project",
            "The Rust community",
            "IronRDP project",
            "FreeRDP project",
            "All contributors and supporters",
            "Made with ❤️ in Ukraine 🇺🇦",
        ],
    );

    // Add legal sections for key dependencies
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
        eprintln!("Failed to initialize libadwaita: {e}");
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
