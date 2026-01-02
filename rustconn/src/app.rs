//! GTK4 Application setup and initialization
//!
//! This module provides the main application entry point and configuration
//! for the `RustConn` GTK4 application, including state management and
//! action setup.

use gtk4::prelude::*;
use gtk4::{gio, glib, Application};
use std::cell::RefCell;
use std::rc::Rc;

use crate::state::{create_shared_state, SharedAppState};
use crate::tray::{TrayManager, TrayMessage};
use crate::window::MainWindow;

/// Application ID for `RustConn`
pub const APP_ID: &str = "io.github.totoshko88.RustConn";

/// Shared tray manager type
type SharedTrayManager = Rc<RefCell<Option<TrayManager>>>;

/// Creates and configures the GTK4 Application
///
/// Sets up the application with Wayland-native configuration and
/// connects the activate signal to create the main window.
#[must_use]
pub fn create_application() -> Application {
    let app = Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::default())
        .build();

    // Create shared tray manager (will be initialized in build_ui)
    let tray_manager: SharedTrayManager = Rc::new(RefCell::new(None));

    let tray_manager_clone = tray_manager.clone();
    app.connect_activate(move |app| {
        build_ui(app, tray_manager_clone.clone());
    });

    // Keep the application running even when all windows are closed (for tray icon)
    app.set_accels_for_action("app.quit", &["<Control>q"]);

    app
}

/// Builds the main UI when the application is activated
fn build_ui(app: &Application, tray_manager: SharedTrayManager) {
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

    // Create main window with state
    let window = MainWindow::new(app, state.clone());

    // Initialize tray icon if enabled in settings
    let enable_tray = state.borrow().settings().ui.enable_tray_icon;
    if enable_tray {
        if let Some(tray) = TrayManager::new() {
            // Update tray with initial state
            update_tray_state(&tray, &state);
            *tray_manager.borrow_mut() = Some(tray);
        }
    }

    // Set up application actions
    setup_app_actions(app, &window, state.clone(), tray_manager.clone());

    // Set up tray message polling
    setup_tray_polling(app, &window, state, tray_manager);

    window.present();
}

/// Updates the tray icon state from the application state
fn update_tray_state(tray: &TrayManager, state: &SharedAppState) {
    let state_ref = state.borrow();

    // Update active session count
    let session_count = state_ref.active_sessions().len();
    #[allow(clippy::cast_possible_truncation)]
    tray.set_active_sessions(session_count as u32);

    // Update recent connections (get last 10 connections sorted by last_connected)
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
}

/// Sets up polling for tray messages
fn setup_tray_polling(
    app: &Application,
    window: &MainWindow,
    state: SharedAppState,
    tray_manager: SharedTrayManager,
) {
    let app_weak = app.downgrade();
    let window_weak = window.gtk_window().downgrade();
    let state_clone = state;
    let tray_manager_clone = tray_manager;

    // Poll for tray messages every 100ms
    glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
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

        // Update tray state periodically
        update_tray_state(tray, &state_clone);

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
        ",
    );

    gtk4::style_context_add_provider_for_display(
        &gtk4::gdk::Display::default().expect("Could not get default display"),
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

/// Sets up application-level actions
fn setup_app_actions(
    app: &Application,
    window: &MainWindow,
    state: SharedAppState,
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

    // Set up keyboard shortcuts
    // Application shortcuts
    app.set_accels_for_action("app.quit", &["<Control>q"]);
    app.set_accels_for_action("app.about", &["F1"]);

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
}

/// Shows the about dialog
fn show_about_dialog(parent: &gtk4::ApplicationWindow) {
    let about = gtk4::AboutDialog::builder()
        .transient_for(parent)
        .modal(true)
        .program_name("RustConn")
        .version(env!("CARGO_PKG_VERSION"))
        .comments(
            "A modern connection manager for Linux\n\n\
            Wayland-native GTK4 application for managing SSH, RDP, VNC, and SPICE connections.\n\n\
            Features:\n\
            • Embedded RDP with clipboard, shared folders, RemoteFX\n\
            • Zero Trust providers (AWS SSM, GCP IAP, Azure Bastion)\n\
            • Import/Export: Remmina, Asbru-CM, Royal TS, SSH config\n\
            • Password Generator with strength analysis\n\
            • Session Restore on startup\n\
            • Expect automation for SSH\n\n\
            Made with ❤️ in Ukraine 🇺🇦",
        )
        .website("https://github.com/totoshko88/rustconn")
        .website_label("GitHub Repository")
        .license_type(gtk4::License::Gpl30)
        .authors(vec!["Anton Isaiev <totoshko88@gmail.com>"])
        .build();

    // Add support/sponsorship information
    // Note: & must be escaped as &amp; for Pango markup in GTK
    about.add_credit_section(
        "Support the Project",
        &[
            "☕ Ko-Fi", "one-time/monthly https://ko-fi.com/totoshko88",
            "💳 PayPal", "international https://www.paypal.com/qrcodes/p2pqrc/JJLUXRZSQ5V3A",
            "💸 Payoneer", "international https://link.payoneer.com/Token?t=135B68D8EB1E4860B4B632ECD755182F&amp;src=pl",
            "🇺🇦 Monobank", "UAH hryvnia https://send.monobank.ua/jar/2UgaGcQ3JC",
        ],
    );

    about.add_credit_section(
        "Acknowledgments",
        &[
            "GTK4 and the GNOME project",
            "The Rust community",
            "IronRDP project",
            "FreeRDP project",
            "All contributors and supporters",
        ],
    );

    // Load and set the application logo
    // Try multiple locations: system paths first, then development path
    let icon_paths = [
        // System installation paths
        "/usr/share/icons/hicolor/scalable/apps/io.github.totoshko88.RustConn.svg",
        "/usr/local/share/icons/hicolor/scalable/apps/io.github.totoshko88.RustConn.svg",
        "/app/share/icons/hicolor/scalable/apps/io.github.totoshko88.RustConn.svg", // Flatpak
        // Development path (cargo run)
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/icons/hicolor/scalable/apps/io.github.totoshko88.RustConn.svg"
        ),
    ];

    for path in &icon_paths {
        if let Ok(logo) = gtk4::gdk::Texture::from_filename(path) {
            about.set_logo(Some(&logo));
            break;
        }
    }

    about.present();
}

/// Shows an error dialog
fn show_error_dialog(app: &Application, title: &str, message: &str) {
    let dialog = gtk4::AlertDialog::builder()
        .message(title)
        .detail(message)
        .modal(true)
        .build();

    // Create a temporary window to show the dialog
    let window = gtk4::ApplicationWindow::builder().application(app).build();

    dialog.show(Some(&window));
}

/// Runs the GTK4 application
///
/// This is the main entry point that initializes GTK and runs the event loop.
pub fn run() -> glib::ExitCode {
    let app = create_application();
    app.run()
}
