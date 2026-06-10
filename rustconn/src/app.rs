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
use std::sync::atomic::{AtomicBool, Ordering};

use crate::state::{
    SharedAppState, create_shared_state, try_with_state, with_state, with_state_mut,
};
use crate::tray::{TrayManager, TrayMessage};
use crate::window::MainWindow;
use gettextrs::gettext;
use rustconn_core::config::ColorScheme;

/// Global flag indicating the application is shutting down.
/// When set, session exit callbacks should suppress error logging
/// and reconnect overlays — the exits are expected because
/// `close_all_control_sockets()` kills SSH connections during shutdown.
static APP_SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

/// Returns `true` if the application is in the process of shutting down.
pub fn is_shutting_down() -> bool {
    APP_SHUTTING_DOWN.load(Ordering::Relaxed)
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

/// Applies the "Compact interface" setting to all open application windows.
///
/// Adds or removes the `compact` CSS class on every window. The CSS rules in
/// `assets/style.css` (`window.compact ...`) reduce header bar `min-height`,
/// tab bar height, and button padding to give more vertical space to content.
///
/// Designed to run live: changes take effect without restart, and re-running
/// with the same value is a no-op (`add_css_class` / `remove_css_class` are
/// idempotent).
pub fn apply_compact_ui(compact: bool) {
    use gtk4::prelude::*;

    let Some(app) = gtk4::gio::Application::default() else {
        return;
    };
    let Some(gtk_app) = app.downcast_ref::<gtk4::Application>() else {
        return;
    };

    for window in gtk_app.windows() {
        if compact {
            window.add_css_class("compact");
        } else {
            window.remove_css_class("compact");
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
    // Guard against repeated activation (e.g. second instance, D-Bus
    // activation).  If a window already exists just present it.
    if let Some(window) = app.active_window() {
        window.present();
        return;
    }

    // Force Adwaita icon theme and suppress deprecated dark-theme property
    // BEFORE loading CSS to prevent libadwaita warnings during theme parsing.
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

        // Safety net: clear the deprecated property again in case it was
        // re-set between run() and activate (e.g. by a settings daemon).
        if settings.is_gtk_application_prefer_dark_theme() {
            settings.set_gtk_application_prefer_dark_theme(false);
            tracing::debug!(
                "Cleared deprecated gtk-application-prefer-dark-theme (using AdwStyleManager)"
            );
        }
    }

    // Load CSS styles for split view panes (after dark-theme suppression)
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

    // Apply saved color scheme from settings
    apply_saved_color_scheme(&state);

    // Apply saved language from settings
    apply_saved_language(&state);

    // Create main window with state
    let window = MainWindow::new(app, state.clone());

    // Apply saved compact-UI preference now that the window exists.
    // Adds the `.compact` CSS class to the window if enabled in settings.
    apply_compact_ui(state.borrow().settings().ui.compact_ui);

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

    // Set up tray message polling
    let tray_shutdown = tray_manager.clone();
    setup_tray_polling(app, &window, state.clone(), tray_manager);

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
                let total: usize = reports
                    .iter()
                    .map(|r| r.connections_added + r.connections_updated)
                    .sum();
                if total > 0 {
                    tracing::info!(
                        groups = reports.len(),
                        total_changes = total,
                        "Startup sync completed"
                    );
                    MainWindow::reload_sidebar_preserving_state(&state_for_sync, &sidebar_for_sync);
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

        // Show toast if any credentials were migrated from legacy XOR encryption
        let migrated = rustconn_core::config::legacy_migration_count();
        if migrated > 0 {
            rustconn_core::config::reset_legacy_migration_count();
            if let Some(win) = window_for_secrets.upgrade() {
                let msg = crate::i18n::i18n_f(
                    "{} credentials migrated from legacy encryption to AES-256-GCM",
                    &[&migrated.to_string()],
                );
                crate::toast::show_toast_on_window(&win, &msg, crate::toast::ToastType::Info);
            }
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

        // Phase 2: Bitwarden auto-unlock (only when Bitwarden is the preferred backend)
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

        // Stop polling if the window has been finalized to avoid
        // interacting with stale GTK objects.
        if window_for_msgs.upgrade().is_none() {
            return glib::ControlFlow::Break;
        }

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
    let secrets = &state_ref.settings().secrets;
    let backend = secrets.preferred_backend;

    // Only warn for non-default backends (LibSecret is always the fallback)
    if matches!(backend, rustconn_core::config::SecretBackendType::LibSecret) {
        return;
    }

    // KeePassXc/KdbxFile use direct KDBX file access, not SecretManager backends.
    // Check KDBX-specific availability instead of probing LibSecretBackend
    // (which is what SecretManager contains for these backend types).
    let available = if matches!(
        backend,
        rustconn_core::config::SecretBackendType::KeePassXc
            | rustconn_core::config::SecretBackendType::KdbxFile
    ) {
        secrets.kdbx_enabled && secrets.kdbx_path.as_ref().is_some_and(|p| p.exists())
    } else {
        state_ref.has_secret_backend()
    };
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
    let notebook_for_quit = window.notebook_rc();
    let window_for_quit = window.gtk_window().downgrade();
    quit_action.connect_activate(move |_, _| {
        let app_weak = app_weak.clone();
        let state_clone = state_clone.clone();
        let sidebar_rc = sidebar_rc.clone();

        let do_quit = move || {
            // Save expanded groups state
            let expanded = sidebar_rc.get_expanded_groups();
            if let Ok(mut state_ref) = state_clone.try_borrow_mut() {
                let _ = state_ref.update_expanded_groups(expanded);
            }
            if let Some(app) = app_weak.upgrade() {
                app.quit();
            }
        };

        // Confirm before quitting with open session tabs (GNOME HIG) —
        // Ctrl+Q goes through app.quit() and bypasses close_request, so
        // the same confirmation dialog is shown here.
        let open_sessions = notebook_for_quit.session_count();
        if open_sessions > 0
            && let Some(win) = window_for_quit.upgrade()
        {
            let dialog = crate::window::MainWindow::close_confirmation_dialog(open_sessions);
            dialog.connect_response(Some("close"), move |_, _| do_quit());
            dialog.present(Some(&win));
            return;
        }
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
    shortcuts_action.connect_activate(move |_, _| {
        if let Some(window) = window_weak.upgrade() {
            let dialog = crate::dialogs::ShortcutsDialog::new(Some(&window));
            dialog.show(Some(&window));
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

    // Suppress the libadwaita warning about gtk-application-prefer-dark-theme.
    // KDE/XFCE set this property globally via xsettings. We clear it before
    // adw::init() so AdwStyleManager never sees it as true.
    // We also connect a notify handler to catch the xsettings daemon re-setting
    // the property after we clear it (race condition on KDE).
    if let Some(display) = gtk4::gdk::Display::default() {
        let settings = gtk4::Settings::for_display(&display);
        if settings.is_gtk_application_prefer_dark_theme() {
            settings.set_gtk_application_prefer_dark_theme(false);
            tracing::debug!(
                "Cleared deprecated gtk-application-prefer-dark-theme before adw::init()"
            );
        }

        // Permanently suppress: if xsettings daemon re-sets the property,
        // clear it again immediately before libadwaita can warn about it.
        settings.connect_gtk_application_prefer_dark_theme_notify(|s| {
            if s.is_gtk_application_prefer_dark_theme() {
                s.set_gtk_application_prefer_dark_theme(false);
                tracing::debug!(
                    "Re-cleared deprecated gtk-application-prefer-dark-theme (xsettings race)"
                );
            }
        });
    }

    // Now initialize libadwaita (gtk_init() is idempotent, safe to call again)
    if let Err(e) = adw::init() {
        tracing::error!(%e, "Failed to initialize libadwaita");
        return glib::ExitCode::FAILURE;
    }

    let app = create_application();
    app.run()
}

/// Applies the saved color scheme from settings to GTK
fn apply_saved_color_scheme(state: &SharedAppState) {
    let color_scheme = with_state(state, |s| s.settings().ui.color_scheme);

    apply_color_scheme(color_scheme);
}

/// Applies the saved language from settings to gettext
fn apply_saved_language(state: &SharedAppState) {
    let language = with_state(state, |s| s.settings().ui.language.clone());

    crate::i18n::apply_language(&language);
}
