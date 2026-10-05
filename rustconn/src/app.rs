//! GTK4 Application setup and initialization
//!
//! This module provides the main application entry point and configuration
//! for the `RustConn` GTK4 application, including state management and
//! action setup.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

use adw::prelude::*;
use gettextrs::gettext;
use gtk4::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;
use rustconn_core::config::ColorScheme;

use crate::state::{
    SharedAppState, create_shared_state, try_with_state, with_state, with_state_mut,
};
use crate::tray::{TrayManager, TrayMessage};
use crate::window::MainWindow;

/// Global flag indicating the application is shutting down.
///
/// When set, session exit callbacks should suppress error logging and reconnect
/// overlays: the exits are expected, because
/// [`crate::window::shutdown_sessions_for_exit`] signalled the children itself.
/// (`close_all_control_sockets()` in `connect_shutdown` closes the SSH
/// ControlMaster sockets, but that runs later — it is not what ends the
/// sessions, and naming it here sent two comments looking at the wrong cause.)
static APP_SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

/// Returns `true` if the application is in the process of shutting down.
pub fn is_shutting_down() -> bool {
    APP_SHUTTING_DOWN.load(Ordering::Relaxed)
}

/// Records that the application has begun shutting down.
///
/// Called at the top of [`crate::window::shutdown_sessions_for_exit`], which is
/// the point the flag has to be true from. It used to be set only in
/// `connect_shutdown`, and `connect_shutdown` runs from `app.quit()` — which
/// `do_quit` calls *after* the teardown has already signalled every session
/// child. So the flag that exists to tell exit callbacks "we caused this" was
/// set after the exits it was meant to explain.
///
/// Idempotent, like the teardown it belongs to; `connect_shutdown` still sets it
/// as well, for an exit that never reaches the helper.
pub fn mark_shutting_down() {
    APP_SHUTTING_DOWN.store(true, Ordering::Relaxed);
}

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

/// Reports whether the application is currently rendering dark.
///
/// This is the *resolved* state, not the user's preference: with
/// [`ColorScheme::System`] it follows the desktop (via the settings portal on
/// Wayland/Flatpak), and with `Light`/`Dark` it follows the forced choice. That is
/// the value `rustconn-core` needs to resolve
/// [`FOLLOW_SYSTEM_THEME`][rustconn_core::terminal_themes::FOLLOW_SYSTEM_THEME],
/// which is why this wrapper exists rather than each call site reaching for
/// `StyleManager` — a headless crate cannot, and the GUI should ask in one place.
#[must_use]
pub fn system_is_dark() -> bool {
    adw::StyleManager::default().is_dark()
}

thread_local! {
    /// Global compact preferences shared by all windows: `(manual, auto)`.
    ///
    /// `manual` forces the compact chrome on always; `auto` engages it only
    /// while a window is small (see [`window_is_small`]).
    static COMPACT_PREFS: Cell<(bool, bool)> = const { Cell::new((false, false)) };

    /// Whether the floating session toolbar opens on pointer hover.
    ///
    /// Kept here, next to the compact preferences, for the same reason: an
    /// embedded RDP/VNC view is built by a constructor that takes no settings,
    /// and threading `AppSettings` through every one of them to carry a single
    /// boolean would be worse than reading it where it is needed. Mirrors
    /// `ui.reveal_session_toolbar_on_hover`.
    static REVEAL_TOOLBAR_ON_HOVER: Cell<bool> = const { Cell::new(true) };
}

/// Auto-compact engages when the window is no taller than this (logical px).
// ponytail: fixed height/width heuristic, fine for desktop/laptop screens;
// promote to a user setting if people on unusual DPIs find the trigger wrong.
const COMPACT_AUTO_MAX_HEIGHT: i32 = 800;
/// ...or no wider than this (logical px). Either dimension being tight engages it.
const COMPACT_AUTO_MAX_WIDTH: i32 = 900;

/// Returns `true` when `window` is small enough for auto-compact to engage.
fn window_is_small(window: &gtk4::Window) -> bool {
    // Prefer the allocated size; before the first allocation fall back to the
    // requested default size so startup already reflects the right density.
    let mut w = window.width();
    let mut h = window.height();
    if w == 0 {
        w = window.default_width();
    }
    if h == 0 {
        h = window.default_height();
    }
    (h > 0 && h <= COMPACT_AUTO_MAX_HEIGHT) || (w > 0 && w <= COMPACT_AUTO_MAX_WIDTH)
}

/// Adds or removes the `.compact` CSS class on `window` from the given inputs.
///
/// Effective compact = `manual || (auto && window_is_small)`. The CSS rules in
/// `assets/style.css` (`window.compact ...`) reduce header bar, tab bar,
/// monitoring bar, split panel, playback toolbar, and button dimensions to give
/// more vertical space to content. Idempotent (`add`/`remove_css_class` are).
fn set_window_compact(window: &gtk4::Window, manual: bool, auto: bool) {
    if manual || (auto && window_is_small(window)) {
        window.add_css_class("compact");
    } else {
        window.remove_css_class("compact");
    }
}

/// Recomputes the `.compact` class for one window from the stored preferences.
///
/// Used as the resize handler so auto-compact reacts live to window size, and
/// once per window at creation to apply the current preferences immediately.
pub fn recompute_window_compact(window: &gtk4::Window) {
    let (manual, auto) = COMPACT_PREFS.with(Cell::get);
    set_window_compact(window, manual, auto);
}

/// Stores the compact preferences and re-applies `.compact` to every window.
///
/// Call after the manual or automatic toggle changes (settings dialog, menu,
/// or keyboard shortcut). Runs live — no restart needed.
pub fn set_compact_prefs(manual: bool, auto: bool) {
    COMPACT_PREFS.with(|p| p.set((manual, auto)));

    let Some(app) = gtk4::gio::Application::default() else {
        return;
    };
    let Some(gtk_app) = app.downcast_ref::<gtk4::Application>() else {
        return;
    };
    for window in gtk_app.windows() {
        set_window_compact(&window, manual, auto);
    }
}

/// Stores whether hovering the session-toolbar handle reveals the toolbar.
///
/// Call after the settings toggle changes. Existing embedded views pick up
/// the new value immediately — the preference is read per pointer event, not
/// captured at construction time.
pub fn set_reveal_toolbar_on_hover(enabled: bool) {
    REVEAL_TOOLBAR_ON_HOVER.with(|pref| pref.set(enabled));
}

/// Whether hovering the session-toolbar handle should reveal the toolbar.
///
/// Read when an embedded RDP/VNC view builds its toolbar controller.
#[must_use]
pub fn reveal_toolbar_on_hover() -> bool {
    REVEAL_TOOLBAR_ON_HOVER.with(Cell::get)
}

/// Attaches a size watcher so auto-compact reacts live to window resizing.
///
/// Call once per window at creation. GTK4 keeps `default-width`/`default-height`
/// in sync with the current size and notifies on resize, which is independent of
/// the adaptive `AdwBreakpoint`s (those drive property setters, not CSS classes),
/// so the two never fight over the same state.
pub fn watch_window_for_compact(window: &gtk4::Window) {
    window.connect_default_width_notify(recompute_window_compact);
    window.connect_default_height_notify(recompute_window_compact);
}

thread_local! {
    /// The application's main window, tracked so re-activation presents it
    /// specifically (issue #236).
    ///
    /// `app.active_window()` returns whichever window has focus, which since
    /// detached session windows exist may not be the main one. Held weakly, so
    /// this never keeps the window alive past its own close.
    static MAIN_WINDOW: RefCell<Option<glib::WeakRef<adw::ApplicationWindow>>> =
        const { RefCell::new(None) };
}

/// Returns the application's main window, if it has been built.
fn main_window() -> Option<adw::ApplicationWindow> {
    MAIN_WINDOW.with(|cell| cell.borrow().as_ref().and_then(glib::WeakRef::upgrade))
}

/// Returns the focused visible application window, for parenting a dialog.
///
/// `GtkApplication::active_window` is deliberately not used: it returns any
/// registered window, which includes a modal `adw::Window` put up by a dialog and
/// the main window while it sits hidden in the tray. Only a visible
/// `adw::ApplicationWindow` — the main window or a detached session window — is a
/// sensible parent, and the focused one is where the user just pressed the
/// shortcut (issue #236). The caller falls back to the main window.
fn focused_application_window(app: &adw::Application) -> Option<gtk4::Window> {
    app.windows()
        .into_iter()
        .filter(|window| window.is_visible() && window.is::<adw::ApplicationWindow>())
        .find(gtk4::prelude::GtkWindowExt::is_active)
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
    // Guard against repeated activation (e.g. second instance, D-Bus
    // activation). If the main window already exists just present it. Tracked
    // explicitly rather than via `app.active_window()`, so a focused detached
    // session window is never presented in its place (issue #236).
    if let Some(window) = main_window() {
        window.present();
        return;
    }

    // Force the Adwaita icon theme before loading CSS, so every icon name the UI
    // asks for resolves the same way regardless of the desktop's icon theme.
    //
    // `gtk-application-prefer-dark-theme` is deliberately NOT touched here. A
    // "safety net" clear used to live in this block, on the theory that a settings
    // daemon might have re-set the property between `run()` and activate. By this
    // point `adw::init()` has run, so the thing that set it is `AdwStyleManager`
    // reporting the resolved colour scheme — and clearing it forced the window
    // light on a dark desktop. See the longer note in `run()`.
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
    }

    // What libadwaita actually resolved, which is the only way a
    // window-is-light-on-a-dark-desktop report can be diagnosed from a log: the
    // preference alone does not say what came out of it.
    tracing::debug!(dark = system_is_dark(), "resolved color scheme at startup");

    // Load CSS styles for split view panes
    load_css_styles();

    // Create shared application state (fast — secret backends deferred)
    let state = match create_shared_state() {
        Ok(state) => state,
        Err(e) => {
            tracing::error!(%e, "Failed to initialize application state");
            show_error_dialog(app, &gettext("Initialization Error"), &e);
            return;
        }
    };

    // Install the debounced connection-history flusher (writes happen on a
    // background thread instead of inline in the connect/disconnect paths)
    setup_history_flush(&state);

    // Prune stale agent key files left behind by a previous crash or kill.
    rustconn_core::ssh_agent::prune_stale_agent_keys();

    // Apply saved color scheme from settings
    apply_saved_color_scheme(&state);

    // NOTE: the saved language is deliberately NOT applied here. `setlocale` is
    // unsound once the process is multi-threaded (RUSTSEC-2026-0244), and by the
    // time `activate` runs the GIO worker thread is up. `i18n::init()` and
    // `i18n::apply_language_from_config()` both run in `main()` before any
    // thread exists and already cover every case this call used to.

    // Create main window with state
    let window = MainWindow::new(app, state.clone());

    // Track it for the re-activation guard above.
    MAIN_WINDOW.with(|cell| {
        *cell.borrow_mut() = Some(window.gtk_window().downgrade());
    });

    // Make application accelerators work under non-Latin keyboard layouts
    // (Ukrainian, Russian, Greek, …) where GTK's keyval-based matching fails.
    install_layout_independent_accels(window.gtk_window(), app);

    // Apply saved compact-UI preferences now that the window exists. The size
    // watcher lets auto-compact react to later resizing; set_compact_prefs adds
    // the `.compact` class immediately for the current size and manual setting.
    watch_window_for_compact(window.gtk_window().upcast_ref());
    let (compact_manual, compact_auto) = {
        let state_ref = state.borrow();
        let ui = &state_ref.settings().ui;
        (ui.compact_ui, ui.compact_auto)
    };
    set_compact_prefs(compact_manual, compact_auto);
    set_reveal_toolbar_on_hover(state.borrow().settings().ui.reveal_session_toolbar_on_hover);

    // Initialize tray icon if enabled in settings
    let enable_tray = state.borrow().settings().ui.enable_tray_icon;
    if enable_tray {
        // macOS: TrayManager uses NSStatusItem which MUST be created on the
        // main thread AFTER the NSApplication run loop is fully active.
        // On macOS Sequoia 15.5+, LaunchServices requires the app's scene
        // to be fully registered with FrontBoardServices before NSStatusItem
        // can acquire a scene from ControlCenter. This takes longer than
        // GTK's initial activation — retry up to 3 times with increasing
        // delays to give WindowServer time to complete scene registration.
        #[cfg(feature = "tray-macos")]
        {
            let state_for_tray = state.clone();
            let tray_mgr_for_init = tray_manager.clone();
            // Initial delay: 2 seconds (Sequoia needs more time than Ventura)
            glib::timeout_add_local_once(std::time::Duration::from_secs(2), move || {
                try_create_macos_tray(state_for_tray, tray_mgr_for_init, 0);
            });
        }

        // Linux: Spawn TrayManager on a background thread so that the blocking
        // D-Bus registration (`tray.spawn()` → `compat::block_on`) does
        // not stall the GTK main loop.  The result is polled back via a
        // lightweight channel.
        #[cfg(feature = "tray")]
        {
            let (tray_tx, tray_rx) = std::sync::mpsc::channel::<TrayManager>();
            std::thread::Builder::new()
                .name("tray-init".into())
                .spawn(move || {
                    if let Some(tray) = TrayManager::new() {
                        let _ = tray_tx.send(tray);
                    }
                })
                .ok();

            let state_for_tray = state.clone();
            let tray_mgr_for_init = tray_manager.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(50), move || match tray_rx
                .try_recv()
            {
                Ok(tray) => {
                    let mut initial_cache = TrayStateCache::default();
                    update_tray_state(&tray, &state_for_tray, &mut initial_cache);
                    tray.force_refresh();
                    *tray_mgr_for_init.borrow_mut() = Some(tray);
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    tracing::warn!("Tray initialization thread exited without creating tray");
                    glib::ControlFlow::Break
                }
            });
        }
    }

    // Schedule delayed force-refreshes so the D-Bus host caches our menu.
    // The host may not be ready immediately after spawn(), so we retry
    // at 500ms, 2s, and 5s to cover both fast and slow host registration.
    {
        let tray_500ms = tray_manager.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(500), move || {
            if let Some(tray) = tray_500ms.borrow().as_ref() {
                tray.force_refresh();
            }
        });
        let tray_2s = tray_manager.clone();
        glib::timeout_add_local_once(std::time::Duration::from_secs(2), move || {
            if let Some(tray) = tray_2s.borrow().as_ref() {
                tray.force_refresh();
            }
        });
        let tray_5s = tray_manager.clone();
        glib::timeout_add_local_once(std::time::Duration::from_secs(5), move || {
            if let Some(tray) = tray_5s.borrow().as_ref() {
                tray.force_refresh();
            }
        });
    }

    // Set up application actions
    setup_app_actions(app, &window, &state, tray_manager.clone());

    // Set up tray message handling and state sync (skipped entirely when
    // the tray is disabled — no tray will ever be created in that case)
    let tray_shutdown = tray_manager.clone();
    if enable_tray {
        setup_tray_handling(app, &window, state.clone(), tray_manager);
    }

    // Connect shutdown signal to flush persistence and drop tray manager.
    // Dropping the tray manager before GTK tears down widgets prevents
    // D-Bus callbacks from referencing already-finalized GObjects (SIGSEGV).
    let state_shutdown = state.clone();
    app.connect_shutdown(move |_| {
        // Signal that the app is shutting down — session exit callbacks
        // should suppress error logging and reconnect overlays.
        APP_SHUTTING_DOWN.store(true, Ordering::Relaxed);

        // Drop tray manager first — stops D-Bus service loop and releases
        // any widget references held by tray state callbacks.
        tray_shutdown.borrow_mut().take();

        // Close SSH ControlMaster sockets to prevent stale sockets lingering
        // after app exit. Uses filesystem scan instead of session state because
        // GTK destroys widgets (and terminates sessions) before shutdown fires.
        let rt = tokio::runtime::Runtime::new().ok();
        if let Some(rt) = rt {
            rt.block_on(rustconn_core::close_all_control_sockets());
        }

        if let Some(Err(e)) = try_with_state(&state_shutdown, |s| s.flush_persistence()) {
            tracing::error!(%e, "Failed to flush persistence on shutdown");
        }
    });

    // Present window immediately — no waiting for secret backends
    window.present();

    // Debug helper: RUSTCONN_OPEN_SETTINGS=1 auto-opens the Settings dialog
    // shortly after startup, so dialog timing instrumentation can be captured
    // hands-free (also works inside flatpak/snap where input automation is
    // unavailable). No effect unless the variable is set.
    if std::env::var("RUSTCONN_OPEN_SETTINGS").is_ok_and(|v| v == "1") {
        let win_for_settings = window.gtk_window().clone();
        gtk4::glib::timeout_add_local_once(std::time::Duration::from_secs(2), move || {
            let _ =
                gtk4::prelude::WidgetExt::activate_action(&win_for_settings, "win.settings", None);
        });
    }

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

        // Reopen the previous session set (issue #243). After the startup
        // action, so an explicit startup connection lands first. A no-op when
        // "Restore sessions on startup" is off or no usable snapshot exists.
        window.restore_previous_sessions();
    }

    // Run Cloud Sync startup import for all Import groups
    {
        let state_for_sync = state.clone();
        let sidebar_for_sync = window.sidebar_rc();
        glib::idle_add_local_once(move || {
            let reports = {
                let Ok(mut state_mut) = state_for_sync.try_borrow_mut() else {
                    return;
                };
                state_mut.run_startup_sync()
            };
            if !reports.is_empty() {
                // Removals and group renames/moves change the sidebar too;
                // counting only additions and updates left it stale.
                let changed = reports.iter().filter(|r| r.has_changes()).count();
                if changed > 0 {
                    tracing::info!(
                        groups = reports.len(),
                        changed_groups = changed,
                        "Startup sync completed"
                    );
                    MainWindow::reload_sidebar_preserving_state(&state_for_sync, &sidebar_for_sync);
                }
            }

            // Simple Sync: import any changes another device wrote to
            // full-sync.rcn, applying creates/updates/deletes (tombstones).
            {
                let outcome = if let Ok(mut state_mut) = state_for_sync.try_borrow_mut() {
                    let device_id = state_mut.settings().sync.device_id;
                    if state_mut.simple_sync_enabled()
                        && state_mut
                            .sync_manager()
                            .should_import_simple_sync(device_id)
                    {
                        Some(state_mut.simple_sync_import_and_apply())
                    } else {
                        None
                    }
                } else {
                    None
                };
                match outcome {
                    Some(Ok(report)) if !report.is_empty() => {
                        tracing::info!(
                            created = report.created,
                            updated = report.updated,
                            deleted = report.deleted,
                            "Simple Sync startup import applied"
                        );
                        MainWindow::reload_sidebar_preserving_state(
                            &state_for_sync,
                            &sidebar_for_sync,
                        );
                    }
                    Some(Err(e)) => tracing::warn!(%e, "Simple Sync startup import failed"),
                    _ => {}
                }
            }
        });
    }

    // Wire up Cloud Sync auto-export: ConnectionManager notifies SyncManager
    // when Master group connections change, debounced via a glib timer.
    {
        let state_for_export = state.clone();
        if let Ok(mut state_mut) = state_for_export.try_borrow_mut() {
            let tx = state_mut.sync_manager_mut().setup_export_channel();
            let debounce_secs = state_mut.sync_manager().export_debounce_secs();
            state_mut.connection_manager().set_export_sender(tx);
            drop(state_mut);

            // Poll the export channel periodically and trigger debounced exports
            let state_poll = state_for_export.clone();
            let sync_banner = window.sync_banner().clone();
            let debounce_ms = u64::from(debounce_secs.max(1)) * 1000;
            glib::timeout_add_local(std::time::Duration::from_millis(debounce_ms), move || {
                // Drain all pending group IDs from the channel
                let mut pending = std::collections::HashSet::new();
                if let Ok(mut state_mut) = state_poll.try_borrow_mut() {
                    while let Some(group_id) = state_mut.sync_manager_mut().try_recv_export() {
                        pending.insert(group_id);
                    }
                    for group_id in &pending {
                        match state_mut.sync_now_group(*group_id) {
                            Ok(report) => {
                                tracing::info!(
                                    group = %report.group_name,
                                    connections = report.connections_added,
                                    "Auto-exported Master group"
                                );
                                // Successful sync clears the failure banner
                                sync_banner.set_revealed(false);
                            }
                            Err(e) => {
                                tracing::warn!(%e, "Auto-export failed");
                                // Background failure would otherwise be
                                // invisible — surface it in the persistent
                                // sync banner (GNOME HIG: banner for state
                                // that needs attention)
                                let group_name = state_mut
                                    .list_groups()
                                    .iter()
                                    .find(|g| g.id == *group_id)
                                    .map_or_else(|| group_id.to_string(), |g| g.name.clone());
                                crate::window::MainWindow::show_sync_error_banner(
                                    &sync_banner,
                                    &group_name,
                                    &e,
                                );
                                // In Flatpak, show a one-time toast with actionable hint
                                if rustconn_core::flatpak::is_flatpak()
                                    && e.contains("not configured")
                                {
                                    tracing::info!(
                                        "Flatpak: grant filesystem access with: \
                                         flatpak override --user --filesystem=/path/to/sync \
                                         io.github.totoshko88.RustConn"
                                    );
                                }
                            }
                        }
                    }

                    // Simple Sync: debounced whole-store export when local data
                    // changed since the last tick.
                    if state_mut.simple_sync_enabled() && state_mut.take_simple_sync_dirty() {
                        match state_mut.simple_sync_export() {
                            Ok(()) => {
                                tracing::debug!("Simple Sync auto-exported");
                                sync_banner.set_revealed(false);
                            }
                            Err(e) => {
                                tracing::warn!(%e, "Simple Sync auto-export failed");
                                crate::window::MainWindow::show_sync_error_banner(
                                    &sync_banner,
                                    "Simple Sync",
                                    &e,
                                );
                            }
                        }
                    }
                }
                glib::ControlFlow::Continue
            });
        }
    }

    // Initialize secret backends in a background thread after the window is visible.
    // Decryption is fast and runs on the main thread; the slow Bitwarden vault
    // unlock runs in a background thread to avoid blocking the GTK main loop.
    let state_for_secrets = state.clone();
    let sidebar_for_secrets = window.sidebar_rc();
    let window_for_secrets = window.gtk_window().downgrade();
    let secret_banner_for_secrets = window.secret_banner().clone();
    glib::idle_add_local_once(move || {
        // Phase 1: Decrypt stored credentials only for the active backend (lazy)
        // Bitwarden credentials are decrypted only when Bitwarden is the preferred
        // backend — avoids holding unused secrets in memory.
        let needs_bitwarden = with_state(&state_for_secrets, |s| {
            matches!(
                s.settings().secrets.preferred_backend,
                rustconn_core::config::SecretBackendType::Bitwarden
            )
        });

        if needs_bitwarden {
            with_state_mut(&state_for_secrets, |s| {
                let settings = &mut s.settings_mut().secrets;

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
            });
        }

        // Show toast if KeePass keyring load failed at startup
        if let Ok(mut state_mut) = state_for_secrets.try_borrow_mut()
            && state_mut.take_kdbx_keyring_failed()
            && let Some(win) = window_for_secrets.upgrade()
        {
            let msg = crate::i18n::i18n(
                "KeePass password not loaded from keyring — re-enter it in Settings",
            );
            crate::toast::show_toast_with_action_on_window(
                &win,
                &msg,
                &crate::i18n::i18n("Settings"),
                "win.settings",
                crate::toast::ToastType::Warning,
            );
        }

        // The backend verdict has to be reachable again after startup, because
        // changing the backend in Settings is exactly when it stops being true.
        // Until now `check_secret_backend_available` ran only here, so selecting
        // a backend that cannot work on this system was accepted in silence and
        // the banner explaining why did not appear until the next launch.
        if let Some(win) = window_for_secrets.upgrade() {
            register_secret_backend_recheck(
                &win,
                &state_for_secrets,
                &sidebar_for_secrets,
                &secret_banner_for_secrets,
            );
        }

        // Phase 2: Bitwarden auto-unlock (only when Bitwarden is the preferred backend)
        if needs_bitwarden {
            // Clone settings for the background thread (Send + 'static)
            let secret_settings = state_for_secrets.borrow().settings().secrets.clone();

            // Channel to receive the result on the GTK main thread
            let (tx, rx) = std::sync::mpsc::channel::<bool>();

            // Run slow Bitwarden unlock in a background thread
            std::thread::spawn(move || {
                // Resolve bw CLI path first (probes Flatpak dirs and PATH by
                // spawning `bw --version` — a Node.js cold start that takes
                // seconds inside flatpak; it used to run on the GTK main
                // thread and froze the UI right after startup). The result is
                // cached process-wide, so later callers get it instantly.
                let _ = rustconn_core::secret::resolve_bw_cmd();
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
            let secret_banner_for_poll = secret_banner_for_secrets.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                match rx.try_recv() {
                    Ok(_) => {
                        state_for_poll.borrow_mut().refresh_secret_backend_cache();
                        refresh_sidebar_secret_status(&state_for_poll, &sidebar_for_poll);
                        check_secret_backend_available(
                            &state_for_poll,
                            &window_for_poll,
                            &secret_banner_for_poll,
                        );
                        tracing::info!("Secret backends initialized after window presentation");
                        glib::ControlFlow::Break
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        state_for_poll.borrow_mut().refresh_secret_backend_cache();
                        refresh_sidebar_secret_status(&state_for_poll, &sidebar_for_poll);
                        check_secret_backend_available(
                            &state_for_poll,
                            &window_for_poll,
                            &secret_banner_for_poll,
                        );
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
            check_secret_backend_available(
                &state_for_secrets,
                &window_for_secrets,
                &secret_banner_for_secrets,
            );
            tracing::info!("Secret backends initialized after window presentation");
        }
    });
}

/// Attempts to create the macOS tray icon with retry logic.
///
/// On macOS Sequoia 15.5+, GTK4 apps launched via LaunchServices (`open`,
/// Finder, Dock) cannot acquire a FrontBoardServices scene for NSStatusItem.
/// This is a known limitation of GTK4's GDK macOS backend which does not
/// perform the proper scene handshake with ControlCenter's statusitems
/// service. The icon is "created" (API returns Ok) but not displayed.
///
/// When launched directly from terminal (bypassing LaunchServices), the
/// status item uses the legacy codepath and works correctly.
///
/// This function retries creation in case a future macOS update or GTK4
/// fix resolves the scene registration issue.
#[cfg(feature = "tray-macos")]
fn try_create_macos_tray(state: SharedAppState, tray_manager: SharedTrayManager, attempt: u32) {
    const MAX_ATTEMPTS: u32 = 3;
    // Retry delays: 3s, 5s after initial 2s delay
    const RETRY_DELAYS_MS: [u64; 2] = [3000, 5000];

    if let Some(tray) = TrayManager::new() {
        let mut initial_cache = TrayStateCache::default();
        update_tray_state(&tray, &state, &mut initial_cache);
        *tray_manager.borrow_mut() = Some(tray);
        if attempt > 0 {
            tracing::info!(attempt, "macOS tray icon created (after retry)");
        } else {
            tracing::info!("macOS tray icon created successfully");
        }
    } else if attempt + 1 < MAX_ATTEMPTS {
        let next_delay = RETRY_DELAYS_MS[attempt as usize];
        tracing::debug!(
            attempt,
            next_delay_ms = next_delay,
            "macOS tray icon creation failed — retrying"
        );
        let next_attempt = attempt + 1;
        glib::timeout_add_local_once(std::time::Duration::from_millis(next_delay), move || {
            try_create_macos_tray(state, tray_manager, next_attempt);
        });
    } else {
        tracing::warn!(
            attempts = MAX_ATTEMPTS,
            "macOS tray icon unavailable when launched via Finder/Dock \
             (known GTK4 limitation on macOS Sequoia 15.5+). \
             Tray icon works when launched from terminal."
        );
    }
}

/// Updates the tray icon state from the application state
///
/// Only updates if state has actually changed to avoid unnecessary work.
fn update_tray_state(tray: &TrayManager, state: &SharedAppState, last_state: &mut TrayStateCache) {
    let state_ref = state.borrow();

    // Update active session count only if changed
    let session_count = state_ref.active_sessions().len();
    #[expect(
        clippy::cast_possible_truncation,
        reason = "value range fits the target type by construction in this code path"
    )]
    let session_count_u32 = session_count as u32;

    if last_state.session_count != session_count_u32 {
        tray.set_active_sessions(session_count_u32);
        last_state.session_count = session_count_u32;
    }

    // Update recent connections only if connection list has changed
    // Use DefaultHasher for a proper dirty check instead of a simple sum
    let connections_hash = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        for c in state_ref
            .list_connections()
            .iter()
            .filter(|c| c.last_connected.is_some())
        {
            c.id.hash(&mut hasher);
            c.last_connected.map(|t| t.timestamp()).hash(&mut hasher);
        }
        hasher.finish() as i64
    };

    if last_state.connections_hash != connections_hash {
        let mut connections: Vec<_> = state_ref
            .list_connections()
            .iter()
            .filter(|c| c.last_connected.is_some())
            .map(|c| (c.id, c.name.clone(), c.last_connected))
            .collect();
        connections.sort_by_key(|b| std::cmp::Reverse(b.2));
        let recent: Vec<_> = connections
            .into_iter()
            .take(10)
            .map(|(id, name, _)| (id, name))
            .collect();
        tray.set_recent_connections(recent);
        last_state.connections_hash = connections_hash;
    }

    // Update favorites (pinned connections) only when the pinned set or its
    // order changed. The hash covers id + pin_order so a re-pin that only
    // reorders still refreshes the tray, but an unrelated edit does not.
    let favorites_hash = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        for c in state_ref.list_connections().iter().filter(|c| c.is_pinned) {
            c.id.hash(&mut hasher);
            c.pin_order.hash(&mut hasher);
            c.name.hash(&mut hasher);
        }
        hasher.finish() as i64
    };

    if last_state.favorites_hash != favorites_hash {
        let mut favorites: Vec<_> = state_ref
            .list_connections()
            .iter()
            .filter(|c| c.is_pinned)
            .map(|c| (c.id, c.name.clone(), c.pin_order))
            .collect();
        // Match the sidebar's Favorites order: pin_order, then name.
        favorites.sort_by(|a, b| match a.2.cmp(&b.2) {
            std::cmp::Ordering::Equal => a.1.to_lowercase().cmp(&b.1.to_lowercase()),
            other => other,
        });
        let favorites: Vec<_> = favorites
            .into_iter()
            .map(|(id, name, _)| (id, name))
            .collect();
        tray.set_favorites(favorites);
        last_state.favorites_hash = favorites_hash;
    }
}

/// Cache for tray state to avoid unnecessary updates
#[derive(Default)]
struct TrayStateCache {
    session_count: u32,
    connections_hash: i64,
    favorites_hash: i64,
}

/// Window actions the tray activates, named as the window's own action map
/// spells them — **without** a `win.` prefix.
///
/// The distinction is what broke every session-opening tray item. There are two
/// ways to activate a window action from code and they take different names:
///
/// - `ActionGroupExt::activate_action(&window, ..)` — the window *is* the
///   `GActionGroup`, so the name is looked up verbatim in its action map and a
///   prefix must not be present. This is what the rest of the codebase does
///   (`window/mod.rs` for `connect-to` and the command palette).
/// - `WidgetExt::activate_action(&widget, ..)` — goes through the widget action
///   muxer, which splits the name on the first `.` to pick a group, so the
///   prefix is *required* (`app.rs` uses `"win.settings"` this way).
///
/// The tray used the second function with the first function's names. Without a
/// dot the muxer finds no group, returns `FALSE`, and the `let _ =` discarded it —
/// so Local Shell, Quick Connect and every Recent Connections entry highlighted
/// on click and then did nothing, on both the ksni and the macOS tray, which
/// share this dispatch.
const TRAY_QUICK_CONNECT_ACTION: &str = "quick-connect";
/// See [`TRAY_QUICK_CONNECT_ACTION`] for why this carries no `win.` prefix.
const TRAY_LOCAL_SHELL_ACTION: &str = "local-shell";
/// The parameterised connect action, taking a connection id.
///
/// Not `connect`: that one is `SimpleAction::new("connect", None)` and acts on
/// the *sidebar selection*, ignoring any parameter. So the tray's chosen
/// connection had nowhere to arrive even once the prefix was right, and GTK
/// would reject the activation anyway for passing a parameter to an action
/// declared without one. See [`TRAY_QUICK_CONNECT_ACTION`] for the prefix.
const TRAY_CONNECT_ACTION: &str = "connect-to";

/// Sets up event-driven tray message handling and periodic state sync.
///
/// Tray messages (user clicks) arrive over an `async_channel` and are
/// awaited on the main context, so the main loop only wakes on real events
/// instead of polling. Tray state (session count, recent connections) is
/// synced every 2 seconds with dirty-flag tracking to minimize D-Bus calls.
fn setup_tray_handling(
    app: &adw::Application,
    window: &MainWindow,
    state: SharedAppState,
    tray_manager: SharedTrayManager,
) {
    let app_weak = app.downgrade();
    let window_weak = window.gtk_window().downgrade();

    // --- Event-driven message handling ---
    // The tray is created asynchronously (background D-Bus registration on
    // Linux, delayed retries on macOS), so first wait for it to appear,
    // then await messages with no further wakeups.
    let tray_for_msgs = tray_manager.clone();
    let app_for_msgs = app_weak;
    let window_for_msgs = window_weak;
    // Needed by the Quit message to decide whether the confirmation dialog will
    // appear, which decides whether the tray-hidden window has to be presented.
    let notebook_for_tray = window.notebook_rc();
    glib::spawn_future_local(async move {
        // Tray init normally completes within ~5s (macOS retries take up
        // to ~16s). Give up after 30s (120 × 250ms) — creation failed.
        const STARTUP_WAIT_TICKS: u32 = 120;
        let mut ticks = 0u32;
        let receiver = loop {
            let maybe_rx = tray_for_msgs
                .borrow()
                .as_ref()
                .map(TrayManager::message_receiver);
            if let Some(rx) = maybe_rx {
                break rx;
            }
            ticks += 1;
            if ticks > STARTUP_WAIT_TICKS {
                return;
            }
            glib::timeout_future(std::time::Duration::from_millis(250)).await;
        };

        while let Ok(msg) = receiver.recv().await {
            let Some(app) = app_for_msgs.upgrade() else {
                break;
            };

            // Stop handling if the window has been finalized to avoid
            // interacting with stale GTK objects.
            if window_for_msgs.upgrade().is_none() {
                break;
            }

            let tray_ref = tray_for_msgs.borrow();
            let Some(tray) = tray_ref.as_ref() else {
                break;
            };

            // This path had no logging at all, which is why a tray item that did
            // nothing was indistinguishable from a click that never arrived: the
            // absence of output proved neither. One line here answers that,
            // because the message is logged before anything can silently fail.
            tracing::debug!(?msg, "handling tray message");

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
                        gio::prelude::ActionGroupExt::activate_action(
                            &win,
                            TRAY_CONNECT_ACTION,
                            Some(&conn_id.to_string().to_variant()),
                        );
                    }
                }
                TrayMessage::QuickConnect => {
                    if let Some(win) = window_for_msgs.upgrade() {
                        win.present();
                        tray.set_window_visible(true);
                        gio::prelude::ActionGroupExt::activate_action(
                            &win,
                            TRAY_QUICK_CONNECT_ACTION,
                            None,
                        );
                    }
                }
                TrayMessage::LocalShell => {
                    if let Some(win) = window_for_msgs.upgrade() {
                        win.present();
                        tray.set_window_visible(true);
                        gio::prelude::ActionGroupExt::activate_action(
                            &win,
                            TRAY_LOCAL_SHELL_ACTION,
                            None,
                        );
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
                    // Route through the `quit` action rather than calling
                    // `app.quit()`, which skipped the session teardown entirely:
                    // quitting from the tray left external viewers (#209),
                    // detached windows (#236) and every session child (#304)
                    // behind. This is the exit path a tray user actually takes,
                    // so fixing #304 only in the window handler would have missed
                    // exactly the people who hit it.
                    //
                    // The action may raise the close-confirmation dialog, and an
                    // AdwDialog parented to a tray-hidden window is never drawn —
                    // the quit would look like it did nothing. So the window is
                    // presented first whenever there is something to confirm.
                    //
                    // `open > 0` is the whole guard, and deliberately so. It used
                    // to also require `!win.is_visible()`, which answers a
                    // different question than the one that matters: a window can
                    // be visible and still not be the window in front of the user
                    // — behind others, on another workspace, or merely unfocused.
                    // In each of those the present was skipped, the dialog was
                    // drawn on a surface nobody was watching, and quitting from
                    // the tray meant going to look for the confirmation by hand.
                    // On an already-visible window `present` raises and focuses
                    // it, which is what asking to quit should do, so there is
                    // nothing here to protect against. The flash that guard was
                    // written to avoid is prevented by `open > 0` by itself: with
                    // nothing to confirm the window is never touched at all.
                    let external =
                        crate::window::external_session_registry().map_or(0, |r| r.active_count());
                    let open = crate::window::open_session_count(
                        notebook_for_tray.session_count(),
                        notebook_for_tray.detached_count(),
                        external,
                    );
                    if open > 0
                        && let Some(win) = window_for_msgs.upgrade()
                    {
                        win.present();
                        tray.set_window_visible(true);
                    }
                    gio::prelude::ActionGroupExt::activate_action(&app, "quit", None);
                }
            }
        }
    });

    // --- Slow state sync (2 seconds) ---
    // Updates session count, recent connections, and window visibility
    // with dirty-flag tracking.
    let state_clone = state;
    let tray_for_state = tray_manager;
    let state_cache = std::rc::Rc::new(std::cell::RefCell::new(TrayStateCache::default()));
    let window_for_state = window.gtk_window().downgrade();

    glib::timeout_add_local(std::time::Duration::from_secs(2), move || {
        let tray_ref = tray_for_state.borrow();
        let Some(tray) = tray_ref.as_ref() else {
            return glib::ControlFlow::Continue;
        };
        let Some(win) = window_for_state.upgrade() else {
            // Window has been finalized — stop polling to avoid
            // touching stale GTK objects.
            return glib::ControlFlow::Break;
        };
        // Sync window visibility so tray menu shows correct Show/Hide label
        tray.set_window_visible(win.is_visible());
        update_tray_state(tray, &state_clone, &mut state_cache.borrow_mut());
        glib::ControlFlow::Continue
    });
}

/// Installs the debounced connection-history flusher.
///
/// `record_connection_*` used to serialize and write `history.toml` inline
/// on the GTK main thread (twice per session). Instead they now mark the
/// history dirty and wake this task, which coalesces a burst of changes
/// into a single write performed on a background thread.
fn setup_history_flush(state: &SharedAppState) {
    /// Changes arriving within this window are written together; small
    /// enough that history survives crashes, large enough to coalesce a
    /// session's start/end pair.
    const DEBOUNCE: std::time::Duration = std::time::Duration::from_secs(2);

    let (tx, rx) = async_channel::unbounded::<()>();
    state.borrow_mut().set_history_dirty_sender(tx.clone());
    let state_weak = std::rc::Rc::downgrade(state);

    glib::spawn_future_local(async move {
        while rx.recv().await.is_ok() {
            glib::timeout_future(DEBOUNCE).await;
            // Drain wake-ups that accumulated during the debounce window.
            while rx.try_recv().is_ok() {}

            let Some(state) = state_weak.upgrade() else {
                break;
            };
            let Ok(state_ref) = state.try_borrow() else {
                // State is borrowed right now — retry on the next wake-up.
                let _ = tx.try_send(());
                continue;
            };
            let snapshot = state_ref.take_history_snapshot_if_dirty();
            drop(state_ref);
            if let Some((config_manager, entries)) = snapshot {
                std::thread::spawn(move || {
                    if let Err(e) = config_manager.save_history(&entries) {
                        tracing::error!(%e, "Failed to save connection history");
                    }
                });
            }
        }
    });
}

/// Registers `win.recheck-secret-backend`, which re-probes the selected backend
/// and updates the sidebar indicator and the banner.
///
/// An action rather than a function the settings callback calls directly: the
/// probe needs the application state, the sidebar and the banner, and
/// `MainWindow::show_settings_dialog` holds none of the three — it has a plain
/// `adw::ApplicationWindow`. Threading all three through the dialog's parameter
/// list to reach one call site would be the larger change, and an action is the
/// mechanism the rest of this window already uses for exactly that reason
/// (`win.settings` is activated the same way from a toast).
///
/// Activated after a settings save so the verdict follows the selection instead
/// of describing whatever was selected at launch.
fn register_secret_backend_recheck(
    window: &adw::ApplicationWindow,
    state: &SharedAppState,
    sidebar: &std::rc::Rc<crate::sidebar::ConnectionSidebar>,
    secret_banner: &adw::Banner,
) {
    let action = gio::SimpleAction::new("recheck-secret-backend", None);
    let state = state.clone();
    let sidebar = sidebar.clone();
    let banner = secret_banner.clone();
    let window_weak = window.downgrade();
    action.connect_activate(move |_, _| {
        // `try_borrow_mut` rather than `borrow_mut`: this runs from a settings
        // save, and the caller there is inside its own `try_borrow_mut` scope on
        // the same state. A missed refresh costs a stale banner until the next
        // launch; a panic costs the session.
        if let Ok(mut state_mut) = state.try_borrow_mut() {
            state_mut.refresh_secret_backend_cache();
        } else {
            tracing::debug!(
                "secret backend re-check skipped: application state is borrowed elsewhere"
            );
            return;
        }
        refresh_sidebar_secret_status(&state, &sidebar);
        check_secret_backend_available(&state, &window_weak, &banner);
    });
    window.add_action(&action);
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
        | rustconn_core::config::SecretBackendType::MacOsKeychain
        | rustconn_core::config::SecretBackendType::Bitwarden
        | rustconn_core::config::SecretBackendType::OnePassword
        | rustconn_core::config::SecretBackendType::Passbolt
        | rustconn_core::config::SecretBackendType::Pass
        | rustconn_core::config::SecretBackendType::EncryptedFile => (true, true),
        // The portable store is configured but unusable while locked, and a
        // locked store is the normal state at startup when the passphrase is not
        // remembered. Reporting it as ready would put a green indicator next to a
        // backend that cannot answer a single lookup; the second flag is the
        // "store is reachable" one, so the unlock state belongs there.
        rustconn_core::config::SecretBackendType::PortableEncryptedFile => {
            (true, state_ref.portable_store_unlocked())
        }
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

/// Surfaces a startup warning when the preferred secret backend cannot store
/// passwords.
///
/// Unlike the previous version, the default system-keyring backend
/// (LibSecret/Keychain) is *also* checked (R4.1): on a desktop without a
/// responding Secret Service the keyring silently failed before, leaving the
/// user with no signal until a save failed (#201). The fine-grained
/// [`BackendAvailability`] lets the warning distinguish a missing client from
/// an unresponsive service (R4.2), and it is shown in a persistent banner
/// (GNOME HIG: a state needing attention) whose button opens Settings →
/// Secrets. KeePassXC/KDBX, which use a database file rather than the keyring,
/// keep their existing file-presence toast.
///
/// [`BackendAvailability`]: rustconn_core::secret::BackendAvailability
fn check_secret_backend_available(
    state: &SharedAppState,
    window_weak: &glib::WeakRef<adw::ApplicationWindow>,
    secret_banner: &adw::Banner,
) {
    use crate::dialogs::settings::secrets_tab::detection::{
        LocalBackendState, backend_needs_probe, backend_readiness, detect_secret_backends,
    };

    let secrets = state.borrow().settings().secrets.clone();
    let backend = secrets.preferred_backend;

    // The verdict is the same one the Secrets page shows in its Status row, from
    // the same function. It used to be derived here from `BackendAvailability`,
    // which has three variants and all of them are about a keyring — so a
    // Bitwarden vault that was merely not logged in came out as `ClientMissing`
    // and the banner read "its keyring client is not installed", about a product
    // that has no keyring client and was in fact installed. That is the shape of
    // issue #312 stated by the application itself.
    //
    // The two file backends answer from local state alone, so nothing is probed
    // for them. That is not a micro-optimisation: this runs on every launch, and
    // the probe is seven child processes with a 5-second ceiling, so without this
    // branch the encrypted file — the store the fallback offer writes to, and a
    // reasonable primary choice — would pay for seven `--version` calls to be told
    // something that is a constant.
    if !backend_needs_probe(backend) {
        let local = LocalBackendState::from_settings(&secrets);
        render_secret_backend_banner(
            backend,
            &backend_readiness(None, backend, &local),
            secret_banner,
            window_weak,
        );
        return;
    }

    // `detect_secret_backends` spawns short-lived probes and is cached for 30s,
    // so it runs on a thread: a cold pass can cost up to the 5s probe budget, and
    // this is called on the GTK main thread both at startup and after a save.
    let banner = secret_banner.clone();
    let window_weak = window_weak.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let detection = detect_secret_backends(
            secrets
                .pass_store_dir
                .as_ref()
                .map(|dir| dir.display().to_string()),
        );
        let local = LocalBackendState::from_settings(&secrets);
        let _ = tx.send(backend_readiness(Some(&detection), backend, &local));
    });

    glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        let readiness = match rx.try_recv() {
            Ok(readiness) => readiness,
            Err(std::sync::mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                tracing::warn!("secret backend probe thread disconnected");
                return glib::ControlFlow::Break;
            }
        };

        render_secret_backend_banner(backend, &readiness, &banner, &window_weak);
        glib::ControlFlow::Break
    });
}

/// Reveals or clears the startup banner for a readiness verdict.
///
/// Split out of [`check_secret_backend_available`] so the two routes into it — the
/// probe-free verdict computed inline, and the one that arrives from the probe
/// thread — cannot render it differently.
fn render_secret_backend_banner(
    backend: rustconn_core::config::SecretBackendType,
    readiness: &crate::dialogs::settings::secrets_tab::detection::BackendReadiness,
    banner: &adw::Banner,
    window_weak: &glib::WeakRef<adw::ApplicationWindow>,
) {
    if readiness.is_usable() {
        // Works — clear any warning left from a previous configuration.
        banner.set_revealed(false);
        return;
    }

    let backend_name = crate::vault_ops::backend_display_name(backend);
    let detail = readiness.label();
    // In a snap an unresponsive keyring is usually the interface not being
    // connected rather than a missing service (#249), which is worth saying
    // because the fix is one command.
    let message = if rustconn_core::snap::is_snap()
        && matches!(
            backend,
            rustconn_core::config::SecretBackendType::LibSecret
                | rustconn_core::config::SecretBackendType::MacOsKeychain
        ) {
        crate::i18n::i18n_f(
            "{} is selected, but the system keyring is not accessible. Run: sudo snap connect rustconn:password-manager-service — or open Settings, then Secrets, to choose another backend.",
            &[&backend_name],
        )
    } else {
        crate::i18n::i18n_f(
            "{} is selected, but it cannot store passwords yet: {}. Open Settings, then Secrets.",
            &[&backend_name, &detail],
        )
    };

    tracing::warn!(
        backend = %backend_name,
        readiness = ?readiness,
        "Selected secret backend cannot store passwords"
    );
    banner.set_title(&message);
    banner.set_revealed(true);
    // The banner lives on the window; if that is gone there is nothing left
    // to warn on, and the reveal above was a no-op on a dead widget.
    if window_weak.upgrade().is_none() {
        tracing::debug!("secret backend warning had no window to appear on");
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
    // Second handle: the confirmation dialog is parented to whichever window is
    // focused, and `app_weak` is moved into the quit closure.
    let app_for_dialog = app.downgrade();
    let state_clone = state.clone();
    let sidebar_rc = window.sidebar_rc();
    let notebook_for_quit = window.notebook_rc();
    let window_for_quit = window.gtk_window().downgrade();
    quit_action.connect_activate(move |_, _| {
        let app_weak = app_weak.clone();
        let state_clone = state_clone.clone();
        let sidebar_rc = sidebar_rc.clone();
        let notebook_for_do_quit = std::rc::Rc::clone(&notebook_for_quit);

        let do_quit = move || {
            // Save expanded groups state
            let expanded = sidebar_rc.get_expanded_groups();
            if let Ok(mut state_ref) = state_clone.try_borrow_mut() {
                let _ = state_ref.update_expanded_groups(expanded);
            }
            // Ctrl+Q bypasses the window's close_request entirely, so the whole
            // session teardown has to run here too: external viewers (#209),
            // session children (#304) and detached windows (#236). Shared with
            // close_request and idempotent, so both firing for one quit is fine.
            crate::window::shutdown_sessions_for_exit(&notebook_for_do_quit);
            if let Some(app) = app_weak.upgrade() {
                app.quit();
            }
        };

        // Confirm before quitting with open sessions (GNOME HIG) — Ctrl+Q goes
        // through app.quit() and bypasses close_request, so the same
        // confirmation dialog is shown here. Count tabless external-viewer
        // sessions too (issue #209).
        // Detached sessions have no tab either (issue #236), so they are
        // counted separately or quitting with only detached sessions open
        // would skip the confirmation.
        let external_open =
            crate::window::external_session_registry().map_or(0, |reg| reg.active_count());
        let open_sessions = crate::window::open_session_count(
            notebook_for_quit.session_count(),
            notebook_for_quit.detached_count(),
            external_open,
        );
        if open_sessions > 0 {
            // Ctrl+Q can be pressed in a detached window, which is a real
            // application window: parent the dialog there so it is not raised
            // behind (or on a tray-hidden) main window (issue #236).
            let parent = app_for_dialog
                .upgrade()
                .and_then(|app| focused_application_window(&app))
                .or_else(|| window_for_quit.upgrade().map(Cast::upcast));
            if let Some(parent) = parent {
                // `None` means the question is already on screen and has just
                // been raised — a second Quit must not stack another copy of it,
                // and must not quit either. Returning without a dialog is
                // therefore correct: the pending one owns the decision.
                if let Some(dialog) =
                    crate::window::present_close_confirmation(&parent, open_sessions)
                {
                    dialog.connect_response(Some("close"), move |_, _| do_quit());
                }
                return;
            }
            tracing::warn!(
                open_sessions,
                "quitting with open sessions but no window to confirm in"
            );
        }
        tracing::debug!(open_sessions, "quitting without confirmation");
        do_quit();
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
    let state_for_shortcuts = state.clone();
    shortcuts_action.connect_activate(move |_, _| {
        if let Some(window) = window_weak.upgrade() {
            // Read the bindings from settings, not from the application: while a
            // terminal has focus the single-modifier accelerators are suspended
            // (issue #216), so the live registration is not what the user has
            // configured.
            let keybindings =
                with_state(&state_for_shortcuts, |s| s.settings().keybindings.clone());
            let dialog = crate::dialogs::ShortcutsDialog::new(Some(&window), Some(&keybindings));
            dialog.show(Some(&window));
        }
    });
    app.add_action(&shortcuts_action);

    // Set up keyboard shortcuts dynamically from settings
    apply_keybindings(app, state);
}

/// Application actions whose single-`<Control>` accelerators collide with
/// common terminal/readline chords and are suspended while a terminal or
/// embedded viewer has focus (issue #197).
const TERMINAL_CONFLICTING_ACTIONS: &[&str] = &[
    "win.search",          // Ctrl+F
    "win.command-palette", // Ctrl+P
    "win.new-connection",  // Ctrl+N
    "win.close-tab",       // Ctrl+W
    "win.show-history",    // Ctrl+H
    "win.move-to-group",   // Ctrl+M
    "win.import",          // Ctrl+I
];

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
    let keybinding_settings = with_state(state, |s| s.settings().keybindings.clone());
    let defaults = rustconn_core::default_keybindings();

    for def in &defaults {
        let accel_str = keybinding_settings.get_accel(def);
        let accels: Vec<&str> = accel_str.split('|').collect();
        app.set_accels_for_action(&def.action, &accels);
    }
}

/// Enables or disables keyboard passthrough mode.
///
/// When passthrough is enabled, all keybindings are removed except those in
/// `passthrough_exceptions` (toggle itself, quit, fullscreen by default).
/// This allows all key combinations to reach the VTE terminal or embedded
/// viewer without being intercepted by the application.
///
/// When passthrough is disabled, all keybindings are restored from settings.
///
/// Per-connection command-macro accels are not in that table; the
/// `win.toggle-passthrough` handler asks `window::macro_dispatch` to drop and
/// later restore them.
///
/// Note: the F10 primary-menu key and `AdwTabView`'s built-in tab shortcuts
/// are GTK-internal bindings, not application accelerators, so they are
/// suspended separately — the header-bar menu button's `primary` property and
/// `TerminalNotebook::set_keyboard_passthrough` (see the
/// `win.toggle-passthrough` action handler). Compositor shortcuts (Alt+Tab,
/// Super) never reach the application at all and are not affected.
pub fn set_passthrough(app: &adw::Application, state: &SharedAppState, enable: bool) {
    if enable {
        let exceptions = with_state(state, |s| {
            s.settings().keybindings.passthrough_exceptions.clone()
        });
        let defaults = rustconn_core::default_keybindings();
        let keybinding_settings = with_state(state, |s| s.settings().keybindings.clone());

        for def in &defaults {
            if exceptions.contains(&def.action) {
                // Keep exception bindings active
                let accel_str = keybinding_settings.get_accel(def);
                let accels: Vec<&str> = accel_str.split('|').collect();
                app.set_accels_for_action(&def.action, &accels);
            } else {
                // Remove all other bindings
                app.set_accels_for_action(&def.action, &[]);
            }
        }
    } else {
        apply_keybindings(app, state);
    }
}

/// Suspends the single-`<Control>` accelerators that collide with the terminal.
///
/// For every action in [`TERMINAL_CONFLICTING_ACTIONS`], removes only the
/// single-modifier accelerators (e.g. `<Control>w`) while keeping
/// multi-modifier variants (e.g. `<Control><Shift>w`) active. This lets
/// readline chords (Ctrl+F/P/N/W…) reach the focused VTE terminal while
/// still allowing `Ctrl+Shift+W` to close a tab (issue #216).
///
/// This is stateless: restore the full set of accelerators with
/// [`restore_terminal_accels`] when focus leaves the terminal.
pub fn suspend_terminal_accels(app: &adw::Application, state: &SharedAppState) {
    let keybinding_settings = with_state(state, |s| s.settings().keybindings.clone());
    let defaults = rustconn_core::default_keybindings();

    for def in &defaults {
        if !TERMINAL_CONFLICTING_ACTIONS.contains(&def.action.as_str()) {
            continue;
        }
        let accel_str = keybinding_settings.get_accel(def);
        // Keep only multi-modifier accelerators (those with more than one
        // modifier flag set, e.g. Ctrl+Shift). Single-Control chords are
        // dropped so they pass through to VTE/readline.
        let kept: Vec<&str> = accel_str
            .split('|')
            .filter(|accel| is_multi_modifier(accel))
            .collect();
        app.set_accels_for_action(&def.action, &kept);
    }
}

/// Restores the terminal-conflicting accelerators from settings.
///
/// Re-applies the accelerators for only the actions in
/// [`TERMINAL_CONFLICTING_ACTIONS`], resolving each from the live keybinding
/// settings (mirroring [`apply_keybindings`]). Non-conflicting actions are left
/// untouched. Called when focus leaves the terminal/viewer (issue #197).
pub fn restore_terminal_accels(app: &adw::Application, state: &SharedAppState) {
    let keybinding_settings = with_state(state, |s| s.settings().keybindings.clone());
    let defaults = rustconn_core::default_keybindings();

    for def in &defaults {
        if TERMINAL_CONFLICTING_ACTIONS.contains(&def.action.as_str()) {
            let accel_str = keybinding_settings.get_accel(def);
            let accels: Vec<&str> = accel_str.split('|').collect();
            app.set_accels_for_action(&def.action, &accels);
        }
    }
}

/// Returns `true` if a GTK accelerator string uses more than one modifier key.
///
/// Single-modifier accelerators like `<Control>w` return `false`; multi-modifier
/// ones like `<Control><Shift>w` or `<Control><Alt>x` return `true`. Also
/// considers function keys (F1–F35) and Page_Up/Page_Down/Tab as non-conflicting
/// with readline, so they are always kept.
///
/// This is used by [`suspend_terminal_accels`] to decide which accelerators to
/// keep active while the terminal has focus (issue #216).
fn is_multi_modifier(accel: &str) -> bool {
    // Function keys and navigation keys never conflict with readline.
    let key_part = accel.rsplit('>').next().unwrap_or(accel);
    if key_part.starts_with('F')
        && key_part.len() > 1
        && key_part[1..].chars().all(|c| c.is_ascii_digit())
    {
        return true;
    }
    if matches!(key_part, "Page_Up" | "Page_Down" | "Tab" | "ISO_Left_Tab") {
        return true;
    }

    // Count distinct modifier tokens (<Control>, <Shift>, <Alt>, <Super>, <Meta>).
    let modifiers = ["<Control>", "<Ctrl>", "<Primary>"];
    let extra_modifiers = ["<Shift>", "<Alt>", "<Super>", "<Meta>"];

    let has_ctrl = modifiers
        .iter()
        .any(|m| accel.to_ascii_lowercase().contains(&m.to_ascii_lowercase()));
    let has_extra = extra_modifiers
        .iter()
        .any(|m| accel.to_ascii_lowercase().contains(&m.to_ascii_lowercase()));

    // Multi-modifier = Ctrl + at least one more modifier.
    has_ctrl && has_extra
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
        .support_url("https://donatello.to/totoshko88")
        .license_type(gtk4::License::Gpl30)
        .developers(vec!["Anton Isaiev <totoshko88@gmail.com>"])
        .copyright("© 2024-2026 Anton Isaiev")
        .application_icon("io.github.totoshko88.RustConn")
        // Translators: Replace this with your name and language, e.g. "John Doe (German)"
        .translator_credits(gettext("translator-credits"))
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
    about.add_link("Donatello", "https://donatello.to/totoshko88");
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
    dialog.add_response("ok", &crate::i18n::i18n("OK"));
    dialog.set_default_response(Some("ok"));

    // Present without a parent window — avoids creating an orphaned
    // ApplicationWindow that lingers after the dialog is dismissed.
    let parent = app.active_window();
    dialog.present(parent.as_ref());
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
    // macOS: Load GSettings schemas programmatically from the .app bundle
    // or Homebrew. Must happen BEFORE gtk4::init() which reads schemas.
    #[cfg(target_os = "macos")]
    crate::configure_gsettings_schemas();

    // Initialize GTK first (creates the display and loads GtkSettings from
    // the desktop environment). This must happen BEFORE adw::init() so we
    // can clear the deprecated property before libadwaita sees it.
    if let Err(e) = gtk4::init() {
        tracing::error!(%e, "Failed to initialize GTK4");
        return glib::ExitCode::FAILURE;
    }

    // Clear `gtk-application-prefer-dark-theme` once, before `adw::init()`.
    //
    // KDE/XFCE set this legacy property globally via xsettings, and older
    // libadwaita warned when it found it already true. Clearing it here is safe
    // because libadwaita has no opinion yet: `adw::init()` runs immediately after
    // and sets the property itself to whatever it resolves the colour scheme to.
    //
    // What used to follow this, and must not come back: a
    // `notify::gtk-application-prefer-dark-theme` handler that cleared the property
    // *again* every time it changed, plus a second clear in `build_ui()`. Both were
    // written for an "xsettings race", but the thing re-setting the property is
    // `AdwStyleManager` — that is how it tells GTK the resolved scheme is dark. So
    // on any dark desktop the handler fought libadwaita and won, and the app
    // rendered light with the system set to dark. `is_dark()` still reported dark,
    // so "Follow System" terminal colours disagreed with the window around them.
    //
    // macOS never had either: there the property is driven by the GTK Quartz
    // backend to mirror the system NSAppearance, so touching it fights macOS'
    // own "follow system" dark mode. The cfg stays for the same reason.
    //
    // The `if` below is the whole guard, and it is the condition the workaround was
    // written for rather than a proxy for it. Only a desktop that set the legacy
    // property *itself* — KDE and XFCE, through xsettings or `gtk-4.0/settings.ini`
    // — has it true this early; GNOME expresses the preference through
    // `org.gnome.desktop.interface color-scheme` and leaves this one alone, so the
    // branch is a no-op there and the clear cannot affect what libadwaita resolves.
    //
    // This used to also carry `not(feature = "gtk-4-20")`, on the reasoning that a
    // build against GTK 4.20 is a build against libadwaita 1.8+, which no longer
    // warns about the legacy property — so the version that deprecates the accessors
    // is the version that makes the clearing pointless. The reasoning holds for the
    // *warning* and not for the *behaviour*: what the clear actually buys on KDE is
    // that libadwaita starts from a property it did not set, and that is true of
    // every libadwaita. Tying it to a GTK feature meant the Flatpak silently lost
    // the workaround the moment it was built against 4.22, on a desktop nobody in
    // this project tests. So the deprecation is suppressed instead of the block being
    // removed, and only in the colour where the lint actually fires — a bare
    // `expect` would trip `unfulfilled_lint_expectations` on a baseline build.
    #[cfg(not(target_os = "macos"))]
    #[cfg_attr(
        feature = "gtk-4-20",
        expect(
            deprecated,
            reason = "clearing a legacy property a legacy settings daemon set is only \
                      expressible through the legacy accessors"
        )
    )]
    if let Some(display) = gtk4::gdk::Display::default() {
        let settings = gtk4::Settings::for_display(&display);
        if settings.is_gtk_application_prefer_dark_theme() {
            settings.set_gtk_application_prefer_dark_theme(false);
            tracing::debug!(
                "Cleared deprecated gtk-application-prefer-dark-theme before adw::init() \
                 (desktop had set it; GNOME does not)"
            );
        }
    }

    // Now initialize libadwaita (gtk_init() is idempotent, safe to call again)
    if let Err(e) = adw::init() {
        tracing::error!(%e, "Failed to initialize libadwaita");
        return glib::ExitCode::FAILURE;
    }

    // macOS: put RustConn's icon on the Dock tile when no bundle can supply one.
    // After gtk4::init() on purpose — the NSApplication this needs is created by
    // the GDK macOS backend when the display is opened, and this is still the
    // thread that ran main(), which AppKit requires.
    #[cfg(target_os = "macos")]
    crate::apply_macos_dock_icon();

    let app = create_application();
    app.run()
}

/// Applies the saved color scheme from settings to GTK
fn apply_saved_color_scheme(state: &SharedAppState) {
    let color_scheme = with_state(state, |s| s.settings().ui.color_scheme);

    apply_color_scheme(color_scheme);
}

/// Installs a capture-phase key controller that makes application accelerators
/// work under non-Latin keyboard layouts (e.g. Ukrainian, Russian, Greek).
///
/// GTK matches the accelerators registered via `set_accels_for_action` against
/// the keyval produced by the *active* layout. Under a Cyrillic layout the "N"
/// key yields `Cyrillic_en`, so `<Control>n` never matches and the shortcut
/// silently does nothing. This controller detects a non-ASCII keyval, maps the
/// hardware keycode back to its Latin keyval, and — if the resulting accelerator
/// is currently registered for an action — activates that action directly.
///
/// Querying `accels_for_action` at press time means user overrides and
/// passthrough mode (which clears accelerators) are honored automatically: when
/// an action has no active accelerator, the key falls through unchanged.
pub fn install_layout_independent_accels(window: &adw::ApplicationWindow, app: &adw::Application) {
    // Action names are stable for the lifetime of the process.
    let actions: Vec<String> = rustconn_core::default_keybindings()
        .into_iter()
        .map(|def| def.action)
        .collect();

    let controller = gtk4::EventControllerKey::new();
    controller.set_propagation_phase(gtk4::PropagationPhase::Capture);

    let app = app.clone();
    let window_weak = window.downgrade();
    controller.connect_key_pressed(move |_ctrl, keyval, keycode, state| {
        // Latin layout (ASCII keyval): GTK already matches it. Proceed and,
        // crucially, do NOT double-dispatch the action.
        if keyval.to_unicode().is_some_and(|c| c.is_ascii()) {
            return glib::Propagation::Proceed;
        }

        // Only accelerator-like combos (with a real modifier) are eligible —
        // never hijack plain text entry under a non-Latin layout.
        let mods = state & gtk4::accelerator_get_default_mod_mask();
        let trigger = gtk4::gdk::ModifierType::CONTROL_MASK
            | gtk4::gdk::ModifierType::ALT_MASK
            | gtk4::gdk::ModifierType::SUPER_MASK
            | gtk4::gdk::ModifierType::META_MASK;
        if !mods.intersects(trigger) {
            return glib::Propagation::Proceed;
        }

        // Map the hardware key to its Latin keyval; bail out if unchanged.
        let latin = crate::utils::latin_keyval(keyval, keycode);
        tracing::debug!(
            keyval = ?keyval.name(),
            keycode,
            latin = ?latin.name(),
            ?mods,
            "layout-independent accel: non-Latin modified key"
        );
        if latin == keyval {
            return glib::Propagation::Proceed;
        }

        let Some(window) = window_weak.upgrade() else {
            return glib::Propagation::Proceed;
        };

        for action in &actions {
            for accel in app.accels_for_action(action) {
                if let Some((accel_key, accel_mods)) = gtk4::accelerator_parse(accel.as_str())
                    && accel_key == latin
                    && accel_mods == mods
                {
                    tracing::debug!(%action, %accel, "layout-independent accel: matched, activating");
                    if gtk4::prelude::WidgetExt::activate_action(&window, action, None).is_ok() {
                        return glib::Propagation::Stop;
                    }
                }
            }
        }
        glib::Propagation::Proceed
    });

    window.add_controller(controller);
}

#[cfg(test)]
mod tests {
    use super::{TRAY_CONNECT_ACTION, TRAY_LOCAL_SHELL_ACTION, TRAY_QUICK_CONNECT_ACTION};

    /// The tray activates these on the window's own `GActionGroup`, which resolves
    /// a name verbatim against its action map and knows nothing about groups. A
    /// `win.` prefix here matches nothing and fails silently, which is the bug
    /// this guards: Local Shell, Quick Connect and Recent Connections all did
    /// nothing when the tray passed muxer-style names to a plain action group.
    ///
    /// If a future change moves these back to `WidgetExt::activate_action`, the
    /// prefix becomes mandatory and this test has to change with it — that is the
    /// point, since the two functions cannot share one spelling.
    #[test]
    fn tray_window_actions_carry_no_group_prefix() {
        for name in [
            TRAY_CONNECT_ACTION,
            TRAY_QUICK_CONNECT_ACTION,
            TRAY_LOCAL_SHELL_ACTION,
        ] {
            assert!(
                !name.contains('.'),
                "{name}: activated on the window's action group, which does not \
                 resolve a group prefix"
            );
            assert!(
                !name.is_empty(),
                "an empty action name activates nothing, silently"
            );
        }
    }

    /// Recent Connections was broken twice over, and the prefix was only half of
    /// it. `connect` is declared with no parameter and connects whatever the
    /// sidebar has selected, so the id the tray sends is discarded and GTK
    /// rejects the activation for supplying a parameter at all. `connect-to`
    /// is the action that takes a connection id.
    #[test]
    fn tray_connect_targets_the_parameterised_action() {
        assert_eq!(
            TRAY_CONNECT_ACTION, "connect-to",
            "the tray sends a connection id, so it needs the action that accepts one"
        );
        assert_ne!(
            TRAY_CONNECT_ACTION, "connect",
            "`connect` acts on the sidebar selection and ignores the id"
        );
    }
}
