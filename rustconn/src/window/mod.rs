//! Main application window
//!
//! This module provides the main window implementation for `RustConn`,
//! including the header bar, sidebar, terminal area, and action handling.

mod batch_edit;
mod clusters;
mod connection_actions;
mod connection_dialogs;
mod credentials;
mod document_actions;
mod edit_actions;
mod edit_dialogs;
mod edit_group;
mod groups;
mod history_actions;
mod navigation_actions;
mod network_monitor;
mod operations;
mod protocols;
mod protocols_ssh;
mod rdp_vnc;
mod session_lifecycle;
mod sessions;
mod smart_folders;
mod snippet_actions;
mod snippets;
mod sorting;
mod split_view_actions;
mod templates;
mod terminal_actions;
pub mod types;
mod ui;
mod workspaces;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Orientation, gio, glib};
use libadwaita as adw;
use rustconn_core::automation::TaskExecutor;
use rustconn_core::split::ColorPool;
use rustconn_core::variables::{VariableManager, VariableScope};
use uuid::Uuid;
use vte4::prelude::*;

use self::document_actions as doc_actions;
use self::types::{
    SessionSplitBridges, SharedExternalWindowManager, SharedNotebook, SharedSidebar,
    SharedSplitView, get_protocol_string,
};
use crate::activity_coordinator::ActivityCoordinator;
use crate::dialogs::{ExportDialog, SettingsDialog};
use crate::external_window::ExternalWindowManager;
use crate::monitoring::MonitoringCoordinator;
use crate::sidebar::{ConnectionItem, ConnectionSidebar};
use crate::split_view::{SplitDirection, SplitViewBridge};
use crate::state::{SharedAppState, try_with_state_mut, with_state};
use crate::terminal::TerminalNotebook;
use crate::toast::ToastOverlay;

/// Shared color pool type for global color allocation across all split containers
type SharedColorPool = Rc<RefCell<ColorPool>>;

/// Shared toast overlay reference
pub type SharedToastOverlay = Rc<ToastOverlay>;

/// Shared tunnel manager for standalone SSH tunnels
pub type SharedTunnelManager = Rc<RefCell<rustconn_core::tunnel_manager::TunnelManager>>;

// Thread-local busy stack for connection operations.
//
// Stored in a thread-local because GTK is single-threaded and the
// static `start_connection_*` methods need access without threading
// the stack through every call site.
thread_local! {
    static BUSY_STACK: RefCell<Option<rustconn_core::BusyStack>> = const { RefCell::new(None) };
}

// External viewer session registry (issue #209), published to a thread-local
// for the same reason as `BUSY_STACK`: the static `start_*_connection` launch
// paths route external VNC/RDP/SPICE viewers into it without threading the
// registry through every call site.
thread_local! {
    static EXTERNAL_SESSIONS: RefCell<Option<Rc<crate::external_session::ExternalSessionRegistry>>> =
        const { RefCell::new(None) };
}

/// Acquires a busy guard from the thread-local [`BusyStack`].
///
/// Returns `None` if the stack has not been initialised yet (before
/// `MainWindow::new` runs). The returned [`BusyGuard`] keeps the
/// header-bar spinner visible until dropped.
fn acquire_busy_guard() -> Option<rustconn_core::BusyGuard> {
    BUSY_STACK.with(|cell| cell.borrow().as_ref().map(rustconn_core::BusyStack::busy))
}

/// Returns the thread-local external viewer session registry.
///
/// Returns `None` before `MainWindow::new` has published it. Callers on the
/// launch path use it to register a spawned external viewer so its child
/// process is watched by the shared poll timer (issue #209).
pub fn external_session_registry() -> Option<Rc<crate::external_session::ExternalSessionRegistry>> {
    EXTERNAL_SESSIONS.with(|cell| cell.borrow().as_ref().map(Rc::clone))
}

/// Main application window wrapper
///
/// Provides access to the main window and its components.
#[expect(
    dead_code,
    reason = "Fields kept for GTK widget lifecycle and future use"
)]
pub struct MainWindow {
    window: adw::ApplicationWindow,
    sidebar: SharedSidebar,
    terminal_notebook: SharedNotebook,
    split_view: SharedSplitView,
    /// Per-session split bridges - each session that has been split gets its own bridge
    /// Each tab maintains its own independent split layout
    session_split_bridges: SessionSplitBridges,
    /// Global color pool shared across all split containers
    /// Ensures different split containers get different colors
    global_color_pool: SharedColorPool,
    /// Container for split views - we swap which bridge is visible based on active session
    split_container: gtk4::Box,
    state: SharedAppState,
    overlay_split_view: adw::OverlaySplitView,
    external_window_manager: SharedExternalWindowManager,
    /// Registry of external viewer sessions (VNC/RDP/SPICE delegated to a
    /// separate viewer process, issue #209). Tracks child processes and drives
    /// sidebar session-count + history via callbacks; watched by a shared timer.
    external_sessions: Rc<crate::external_session::ExternalSessionRegistry>,
    toast_overlay: SharedToastOverlay,
    monitoring: Rc<MonitoringCoordinator>,
    activity_coordinator: types::SharedActivityCoordinator,
    tunnel_manager: SharedTunnelManager,
    /// Busy-state tracker — shows/hides header bar spinner on 0→1 / 1→0 transitions
    busy_stack: rustconn_core::BusyStack,
    /// Runtime-only quick connect history (max 15 entries, LIFO, not persisted)
    quick_connect_history: types::SharedQuickConnectHistory,
    /// Passthrough mode indicator button in header bar (visible when active)
    passthrough_indicator: gtk4::Button,
    /// Primary menu button in the header bar. Its `primary` property is
    /// toggled off in passthrough mode to suspend the GTK-internal F10
    /// binding, which is not covered by `set_accels_for_action`.
    menu_button: gtk4::MenuButton,
    /// Split-view broadcast toggle button in the header bar. Only visible
    /// when the active tab has a split layout with two or more panels;
    /// see `update_broadcast_toggle_state`.
    broadcast_toggle: gtk4::ToggleButton,
    /// One-shot flag for the "broadcast available" discoverability toast.
    /// Set to true after the user makes the first split that produces ≥2
    /// active panels in the current application session, so the hint is
    /// shown at most once. Not persisted across restarts.
    broadcast_hint_shown: Rc<std::cell::Cell<bool>>,
    /// Persistent banner below the header bar for cloud sync failures.
    /// Shown by `show_sync_error_banner`, hidden on the next successful
    /// sync or via its Dismiss button.
    sync_banner: adw::Banner,
    /// Persistent banner below the header bar warning that the preferred
    /// secret backend cannot store passwords (e.g. no Secret Service
    /// responding). Shown at startup by `check_secret_backend_available`;
    /// its action button opens Settings → Secrets (#201).
    secret_banner: adw::Banner,
}

impl MainWindow {
    /// Creates a new main window for the application
    #[must_use]
    pub fn new(app: &adw::Application, state: SharedAppState) -> Self {
        // Register custom icon from assets before creating window
        Self::register_app_icon();

        // Create the main window
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("RustConn")
            .default_width(1200)
            .default_height(800)
            // Initial minimum-width floor; refined to the exact narrow-tier
            // header width (incl. window controls) by the runtime measurement in
            // the connect_map handler below (#204). With breakpoints present,
            // AdwApplicationWindow derives its minimum from width-request.
            .width_request(400)
            .height_request(400)
            .icon_name("io.github.totoshko88.RustConn")
            .build();

        // Apply saved window geometry if available
        with_state(&state, |state_ref| {
            let settings = state_ref.settings();
            if settings.ui.remember_window_geometry {
                if let (Some(width), Some(height)) =
                    (settings.ui.window_width, settings.ui.window_height)
                    && width > 0
                    && height > 0
                {
                    window.set_default_size(width, height);
                }
                // Restore maximized state (#202). GTK keeps the unmaximized
                // default size above, so unmaximizing later restores it.
                if settings.ui.window_maximized {
                    window.maximize();
                }
            }
        });

        // Create header bar with busy spinner
        let (
            header_bar,
            busy_spinner,
            passthrough_indicator,
            broadcast_toggle,
            menu_button,
            header_title,
        ) = ui::create_header_bar();

        // Mirror the window title into the header bar's centre label so the
        // active connection name (issue #211) is visible there too, not only in
        // the WM title read by time-tracking tools. One-way binding: whenever
        // `update_window_title` sets the window title, the label follows.
        window
            .bind_property("title", &header_title, "label")
            .sync_create()
            .build();

        // Create BusyStack that shows/hides the header bar spinner.
        // GTK widgets are !Send, so we bridge via std::sync::mpsc channel.
        // The BusyStack callback (Send+Sync) sends a bool, and a
        // glib::idle_add_local receiver dispatches it on the main thread.
        let (busy_tx, busy_rx) = std::sync::mpsc::channel::<bool>();
        let busy_stack = rustconn_core::BusyStack::new(move |busy| {
            let _ = busy_tx.send(busy);
            // Wake the main loop so it processes the message promptly
            let ctx = glib::MainContext::ref_thread_default();
            ctx.wakeup();
        });
        {
            let spinner = busy_spinner;
            let rx = std::sync::Mutex::new(busy_rx);
            glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
                if let Ok(guard) = rx.lock() {
                    // Drain all pending messages, apply the latest state
                    let mut latest = None;
                    while let Ok(busy) = guard.try_recv() {
                        latest = Some(busy);
                    }
                    if let Some(busy) = latest {
                        spinner.set_spinning(busy);
                        spinner.set_visible(busy);
                    }
                }
                glib::ControlFlow::Continue
            });
        }

        // Create the main layout with OverlaySplitView (GNOME HIG)
        let overlay_split_view = adw::OverlaySplitView::new();

        // Apply saved sidebar width as max-sidebar-width
        // Migration: if saved width > 400, it was set with the old 360px minimum —
        // reset to default to avoid an overly wide sidebar on HiDPI displays.
        let saved_width = with_state(&state, |s| s.settings().ui.sidebar_width);
        // Migration: reset sidebar width if it was set by an older version.
        // - Values > 500 came from the old 360px minimum (too wide on HiDPI)
        // - Values < 180 are below the minimum
        // Only keep values in the 180..=500 range that the user intentionally set.
        let sidebar_width = match saved_width {
            Some(w) if (180..=500).contains(&w) => w,
            _ => 320,
        };
        overlay_split_view.set_max_sidebar_width(f64::from(sidebar_width.clamp(180, 500)));
        overlay_split_view.set_min_sidebar_width(180.0);
        // High fraction so the configured sidebar width (max-sidebar-width) is the
        // effective width whenever the sidebar is shown, instead of being capped
        // below the setting by a small proportion (#204: "sidebar width not
        // adjustable"). The sidebar is only shown at ≥ 820 sp, where
        // 0.7 × width already exceeds the 500 px maximum, so max wins.
        overlay_split_view.set_sidebar_width_fraction(0.7);
        overlay_split_view.set_enable_show_gesture(true);
        overlay_split_view.set_enable_hide_gesture(true);
        overlay_split_view.set_pin_sidebar(true);

        // Create sidebar
        let sidebar = Rc::new(ConnectionSidebar::new());
        overlay_split_view.set_sidebar(Some(sidebar.widget()));

        // Load persisted search history
        with_state(&state, |s| {
            sidebar.load_search_history(&s.settings().ui.search_history);
        });

        // Create global color pool shared across all split containers
        // This ensures different split containers get different colors
        let global_color_pool: SharedColorPool = Rc::new(RefCell::new(ColorPool::new()));

        // Create split terminal view as the main terminal container
        // Uses the global color pool for consistent color allocation
        let mut split_bridge = SplitViewBridge::with_color_pool(Rc::clone(&global_color_pool));
        with_state(&state, |s| {
            split_bridge.set_show_scrollbar(s.settings().terminal.show_scrollbar);
        });
        let split_view = Rc::new(split_bridge);

        // Create per-session split bridges map
        // Each tab maintains its own independent split layout
        let session_split_bridges: SessionSplitBridges =
            Rc::new(RefCell::new(std::collections::HashMap::new()));

        // Create container for split views - we swap which bridge is visible based on active session
        let split_container = gtk4::Box::new(Orientation::Vertical, 0);
        split_container.set_vexpand(true);
        split_container.set_hexpand(true);

        // Create terminal notebook for tab management (using adw::TabView)
        let terminal_notebook = Rc::new(TerminalNotebook::new());

        // Refresh VTE font state when fontconfig changes (#171).
        // VTE reads `gtk-fontconfig-timestamp` only when it creates its
        // cached FontInfo and never subscribes to changes, so after a
        // fontconfig update (font install, fc-cache, KDE pushing
        // Fontconfig/Timestamp via XSettings on screen unlock) terminals
        // keep stale Pango font references and crash with SIGSEGV in
        // pango_itemize during the next snapshot.
        if let Some(gtk_settings) = gtk4::Settings::default() {
            let notebook_weak = Rc::downgrade(&terminal_notebook);
            gtk_settings.connect_notify_local(Some("gtk-fontconfig-timestamp"), move |_, _| {
                if let Some(notebook) = notebook_weak.upgrade() {
                    tracing::info!(
                        "fontconfig timestamp changed; refreshing fonts on all terminals"
                    );
                    notebook.refresh_fonts_after_fontconfig_change();
                }
            });
        }

        // Apply initial protocol tab coloring setting
        if let Ok(state_ref) = state.try_borrow() {
            terminal_notebook
                .set_color_tabs_by_protocol(state_ref.settings().ui.color_tabs_by_protocol);
            sidebar.set_filter_visible(state_ref.settings().ui.show_protocol_filters);
            sidebar.set_smart_folders_visible(state_ref.settings().ui.show_smart_folders);
        }

        // Set up callback for when SSH tabs are closed via TabView
        // This ensures sidebar status is cleared when tabs are closed
        // Note: Split view cleanup is handled in connect_signals() where we have access to session_bridges
        let sidebar_for_close = sidebar.clone();
        let monitoring = Rc::new(MonitoringCoordinator::new());
        let monitoring_for_close = monitoring.clone();
        let activity_coordinator = Rc::new(ActivityCoordinator::new());
        let activity_for_close = activity_coordinator.clone();
        terminal_notebook.set_activity_coordinator(activity_coordinator.clone());
        terminal_notebook.set_on_page_closed(move |session_id, connection_id| {
            monitoring_for_close.stop_monitoring(session_id);
            activity_for_close.stop(session_id);
            sidebar_for_close.decrement_session_count(&connection_id.to_string(), false);
        });

        // Wire activity monitoring from the single session-creation choke point.
        // This covers every terminal protocol and both synchronous and async
        // (port-checked) connection paths, regardless of which connect action
        // was used (sidebar Connect, command palette, cluster, double-click).
        {
            let state_for_activity = state.clone();
            let activity_for_setup = activity_coordinator.clone();
            let notebook_weak = Rc::downgrade(&terminal_notebook);
            terminal_notebook.set_on_session_created(move |session_id, connection_id| {
                let Some(notebook) = notebook_weak.upgrade() else {
                    return;
                };
                Self::setup_activity_monitoring(
                    &state_for_activity,
                    &notebook,
                    &activity_for_setup,
                    session_id,
                    connection_id,
                );
            });
        }

        // Focus-based accelerator suspend (#197): when the VTE gains focus,
        // suspend the single-Ctrl accelerators that collide with readline
        // chords; restore them from settings when focus leaves. Honors the
        // `terminal_passthrough_ctrl` setting live (off → leave accels as-is,
        // composing with the global passthrough). A weak `app` ref avoids a
        // reference cycle (the application owns the window).
        {
            let state_for_focus = state.clone();
            let app_weak = app.downgrade();
            terminal_notebook.set_on_terminal_focus(move |focused| {
                let passthrough = with_state(&state_for_focus, |s| {
                    s.settings().ui.terminal_passthrough_ctrl
                });
                if !passthrough {
                    return;
                }
                let Some(app) = app_weak.upgrade() else {
                    return;
                };
                if focused {
                    crate::app::suspend_terminal_accels(&app, &state_for_focus);
                } else {
                    crate::app::restore_terminal_accels(&app, &state_for_focus);
                }
            });
        }

        // Set up reconnect callback for VTE sessions
        // When user clicks "Reconnect" in a disconnected tab, reuse the
        // existing terminal tab instead of closing and creating a new one.
        // This preserves tab position, avoids visual flicker, and keeps
        // the user's tab arrangement intact (#89).
        {
            let state_for_reconnect = state.clone();
            let notebook_for_reconnect = terminal_notebook.clone();
            let split_view_for_reconnect = split_view.clone();
            let sidebar_for_reconnect = sidebar.clone();
            let monitoring_for_reconnect = monitoring.clone();
            let activity_for_reconnect = activity_coordinator.clone();
            terminal_notebook.set_on_reconnect(move |session_id, connection_id| {
                tracing::info!(
                    %session_id,
                    %connection_id,
                    "Reconnecting session in-place"
                );

                // Determine the protocol of the disconnected session
                let protocol = notebook_for_reconnect
                    .get_session_info(session_id)
                    .map(|info| info.protocol.clone());

                // For VTE-based sessions, reconnect in-place (reuse existing tab)
                let is_vte_protocol = protocol.as_deref().is_some_and(|p| {
                    p == "ssh"
                        || p == "telnet"
                        || p == "serial"
                        || p == "kubernetes"
                        || p == "mosh"
                        || p.starts_with("zerotrust")
                });
                if is_vte_protocol {
                    let success = if protocol.as_deref() == Some("ssh") {
                        protocols::reconnect_ssh_in_place(
                            &state_for_reconnect,
                            &notebook_for_reconnect,
                            &sidebar_for_reconnect,
                            &monitoring_for_reconnect,
                            session_id,
                            connection_id,
                        )
                    } else {
                        protocols::reconnect_generic_vte_in_place(
                            &state_for_reconnect,
                            &notebook_for_reconnect,
                            &sidebar_for_reconnect,
                            session_id,
                            connection_id,
                        )
                    };
                    if success {
                        // In-place reconnect reuses the terminal, so the VTE
                        // signal handlers persist; only the coordinator session
                        // entry must be recreated so monitoring/menu resume.
                        Self::reactivate_activity_monitoring(
                            &state_for_reconnect,
                            &activity_for_reconnect,
                            connection_id,
                            session_id,
                        );
                        return;
                    }
                    tracing::warn!(
                        %session_id,
                        "In-place reconnect failed, falling back to close+create"
                    );
                }

                // Fallback for non-SSH protocols or if in-place failed:
                // close old tab, create new one, reorder to original position
                let tab_position = {
                    let sessions = notebook_for_reconnect.sessions_map();
                    let sessions_ref = sessions.borrow();
                    sessions_ref
                        .get(&session_id)
                        .map(|page| notebook_for_reconnect.tab_view().page_position(page))
                };

                notebook_for_reconnect.close_tab(session_id);

                let tabs_before = notebook_for_reconnect.tab_view().n_pages();

                Self::start_connection_with_credential_resolution(
                    state_for_reconnect.clone(),
                    notebook_for_reconnect.clone(),
                    split_view_for_reconnect.clone(),
                    sidebar_for_reconnect.clone(),
                    monitoring_for_reconnect.clone(),
                    connection_id,
                    Some(activity_for_reconnect.clone()),
                );

                if let Some(original_pos) = tab_position {
                    let tabs_after = notebook_for_reconnect.tab_view().n_pages();
                    if tabs_after > tabs_before {
                        let new_page = notebook_for_reconnect.tab_view().nth_page(tabs_after - 1);
                        notebook_for_reconnect
                            .tab_view()
                            .reorder_page(&new_page, original_pos);
                    }
                }
            });
        }

        // TabView/TabBar configuration is handled internally
        // TabView is always visible — content lives inside TabPages
        terminal_notebook.widget().set_vexpand(true);
        // Ensure notebook is visible
        terminal_notebook.widget().set_visible(true);
        terminal_notebook.show_tab_view_content();

        // Create a container for the terminal area
        let terminal_container = gtk4::Box::new(Orientation::Vertical, 0);
        terminal_container.set_vexpand(true);
        terminal_container.set_hexpand(true);

        // Add notebook tabs at top for session switching (tabs only, content hidden by size)
        terminal_container.append(terminal_notebook.widget());

        // Add split view as the main content area - takes full space
        // With per-tab split architecture, this is hidden by default
        // (content lives inside TabPages, not in a global split view)
        split_view.widget().set_vexpand(false);
        split_view.widget().set_hexpand(true);
        split_view.widget().set_visible(false);
        terminal_container.append(split_view.widget());

        // Add split_container for per-session split views (initially hidden)
        split_container.set_visible(false);
        terminal_container.append(&split_container);

        // Note: drag-and-drop is set up in connect_signals after we have access to notebook

        overlay_split_view.set_content(Some(&terminal_container));

        // Create toast overlay and wrap the split view
        let toast_overlay = Rc::new(ToastOverlay::new());
        toast_overlay.set_child(Some(&overlay_split_view));

        // Create main layout using adw::ToolbarView for proper libadwaita integration
        // This provides better responsive behavior and follows GNOME HIG
        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header_bar);

        // Persistent banner for cloud sync failures (GNOME HIG: a state
        // that needs attention belongs in a banner, not a transient toast).
        // Hidden by default; shown via show_sync_error_banner(), hidden on
        // the next successful sync.
        let sync_banner = adw::Banner::new("");
        sync_banner.set_button_label(Some(&crate::i18n::i18n("Dismiss")));
        sync_banner.connect_button_clicked(|banner| {
            banner.set_revealed(false);
        });
        toolbar_view.add_top_bar(&sync_banner);

        // Persistent banner warning that the preferred secret backend cannot
        // store passwords (GNOME HIG: keyring-unavailable is a state that
        // needs attention, so it belongs in a banner, not a transient toast).
        // Hidden by default; shown via check_secret_backend_available(). Its
        // action button opens Settings, where the user picks a working backend
        // in the Secrets section (#201).
        let secret_banner = adw::Banner::new("");
        secret_banner.set_button_label(Some(&crate::i18n::i18n("Open Settings")));
        secret_banner.connect_button_clicked(|banner| {
            // The banner lives inside the window that owns the `win.settings`
            // action; activating it from the banner opens Settings → Secrets.
            let _ = gtk4::prelude::WidgetExt::activate_action(banner, "win.settings", None);
            banner.set_revealed(false);
        });
        toolbar_view.add_top_bar(&secret_banner);

        toolbar_view.set_content(Some(toast_overlay.widget()));

        // Wrap everything with TabOverview — must be the outermost widget
        // so it can overlay the entire window content (GNOME Web pattern)
        let tab_overview = terminal_notebook.tab_overview();
        tab_overview.set_child(Some(&toolbar_view));
        // Clip overflow to prevent the TabOverview from requesting more space
        // than the window provides when embedded RDP sessions have large framebuffers
        tab_overview.set_overflow(gtk4::Overflow::Hidden);

        window.set_content(Some(tab_overview));

        // Adaptive layout breakpoints (#204).
        //
        // CRITICAL: AdwApplicationWindow applies only ONE breakpoint at a time —
        // the last-added whose condition currently holds — NOT a cumulative
        // stack. So each tier must repeat every setter of the wider tiers, and
        // tiers must be added widest→narrowest so the narrowest matching one wins
        // (e.g. at 500 sp both tiers match, the narrow tier added last applies).
        //
        // Why these two things move together:
        // - The header packs many fixed 44×44 icon buttons whose combined minimum
        //   otherwise squeezes the GtkWindowControls (min/max/close) off the end.
        // - The sidebar's own minimum (~368 px — its filter/bulk revealers demand
        //   width even while hidden) pins the window wide unless it is overlaid
        //   first, so auto-hide can never engage.
        //
        // Thresholds are chosen so each tier's resulting minimum width is BELOW
        // the next (narrower) tier's threshold — otherwise the window's minimum
        // plateaus at a tier boundary and a single drag "sticks" there, needing
        // a second drag to continue (the reported jank). With the full header ≈
        // 794 px:
        // - medium ≤ 820sp: collapse + hide the sidebar (F9-style) and hide the
        //   split-view buttons, Delete and New Group → header ≈ 578 px (< 600).
        // - narrow ≤ 600sp: everything above, plus hide Quick Connect, Settings
        //   and the Shell pill → header ≈ 390 px, leaving only Sidebar toggle,
        //   New Connection and the menu beside the window controls.
        // The sidebar is hidden (show-sidebar = false), not shown as an overlay,
        // when collapsed; F9 / the edge gesture still reveals it as an overlay.
        // Growing the window past a threshold restores the hidden setters.
        let collapsed_val = true.to_value();
        let hide_flag = false.to_value();
        // The centre title ("RustConn") is hidden in both tiers so the freed
        // space guarantees room for the window controls at the minimum width.
        let title_widget = header_bar.title_widget();

        let bp_medium = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
            adw::BreakpointConditionLengthType::MaxWidth,
            820.0,
            adw::LengthUnit::Sp,
        ));
        bp_medium.add_setter(&overlay_split_view, "collapsed", Some(&collapsed_val));
        bp_medium.add_setter(&overlay_split_view, "pin-sidebar", Some(&hide_flag));
        bp_medium.add_setter(&overlay_split_view, "show-sidebar", Some(&hide_flag));
        if let Some(title) = title_widget.as_ref() {
            bp_medium.add_setter(title, "visible", Some(&hide_flag));
        }
        for action in [
            "win.split-vertical",
            "win.split-horizontal",
            "win.delete-connection",
            "win.new-group",
        ] {
            if let Some(btn) = Self::header_button(&header_bar, action) {
                bp_medium.add_setter(&btn, "visible", Some(&hide_flag));
            }
        }
        window.add_breakpoint(bp_medium);

        let bp_narrow = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
            adw::BreakpointConditionLengthType::MaxWidth,
            600.0,
            adw::LengthUnit::Sp,
        ));
        bp_narrow.add_setter(&overlay_split_view, "collapsed", Some(&collapsed_val));
        bp_narrow.add_setter(&overlay_split_view, "pin-sidebar", Some(&hide_flag));
        bp_narrow.add_setter(&overlay_split_view, "show-sidebar", Some(&hide_flag));
        if let Some(title) = title_widget.as_ref() {
            bp_narrow.add_setter(title, "visible", Some(&hide_flag));
        }
        for action in [
            "win.split-vertical",
            "win.split-horizontal",
            "win.delete-connection",
            "win.new-group",
            "win.quick-connect",
            "win.settings",
            "win.local-shell",
        ] {
            if let Some(btn) = Self::header_button(&header_bar, action) {
                bp_narrow.add_setter(&btn, "visible", Some(&hide_flag));
            }
        }
        window.add_breakpoint(bp_narrow);

        // Pin the window's minimum width to the *measured* narrow-tier layout so
        // the window controls (min/max/close) are never clipped, regardless of
        // theme, font, locale or the compositor's decoration layout (#204).
        // Guessing this from button counts proved unreliable, so once the window
        // is mapped (and the window controls are realized) we temporarily hide
        // exactly what the narrow tier hides, measure the real minimum width of
        // the content, restore visibility, and set width-request to it. Runs
        // once. `AdwApplicationWindow` derives its minimum from width-request
        // when breakpoints are present, so this becomes the true floor.
        let narrow_hidden: Vec<gtk4::Widget> = {
            let mut widgets = Vec::new();
            if let Some(title) = header_bar.title_widget() {
                widgets.push(title);
            }
            for action in [
                "win.split-vertical",
                "win.split-horizontal",
                "win.delete-connection",
                "win.new-group",
                "win.quick-connect",
                "win.settings",
                "win.local-shell",
            ] {
                if let Some(btn) = Self::header_button(&header_bar, action) {
                    widgets.push(btn.upcast());
                }
            }
            widgets
        };
        let overview_for_measure = tab_overview.clone();
        let split_for_measure = overlay_split_view.clone();
        let measured = std::cell::Cell::new(false);
        window.connect_map(move |win| {
            if measured.replace(true) {
                return;
            }
            // Mirror the narrow tier *exactly*: besides hiding the shed header
            // buttons, collapse the sidebar. When the split view is not
            // collapsed its minimum is sidebar-width + content-width, and the
            // sidebar's filter/search/bulk-action labels are locale-dependent —
            // wider in German than in English — which pinned the window far too
            // wide for tiling (#204 follow-up). Collapsing here (as the narrow
            // breakpoint does) makes the sidebar overlay instead, so only the
            // content contributes and the measured floor is locale-independent.
            let was_collapsed = split_for_measure.is_collapsed();
            let was_showing = split_for_measure.shows_sidebar();
            split_for_measure.set_collapsed(true);
            split_for_measure.set_show_sidebar(false);
            for widget in &narrow_hidden {
                widget.set_visible(false);
            }
            let (min_width, _, _, _) =
                overview_for_measure.measure(gtk4::Orientation::Horizontal, -1);
            for widget in &narrow_hidden {
                widget.set_visible(true);
            }
            split_for_measure.set_collapsed(was_collapsed);
            split_for_measure.set_show_sidebar(was_showing);
            // Small margin absorbs sub-pixel rounding; floor guards a degenerate
            // zero measurement before first layout.
            win.set_width_request((min_width + 4).max(360));
        });

        // Create external window manager
        let external_window_manager = Rc::new(ExternalWindowManager::new());

        // External viewer session registry (issue #209): tracks VNC/RDP/SPICE
        // sessions delegated to a separate viewer process, surfaced in the
        // sidebar without a notebook tab. The callbacks bridge the registry into
        // the sidebar session count and the connection history; a single shared
        // 2 s poll timer (inside the registry) reaps closed viewers.
        let external_sessions = {
            let sidebar_registered = sidebar.clone();
            let on_registered: Box<dyn Fn(Uuid)> = Box::new(move |connection_id| {
                // R2.1: show the green "connected" status via the session count.
                sidebar_registered.increment_session_count(&connection_id.to_string());
                // R2.2: show the `window-new-symbolic` emblem alongside the
                // connected icon. `record_connection_start` (R3.1) is done at the
                // launch site (task 6); its entry id is passed into `register`.
                sidebar_registered.set_external_session(&connection_id.to_string(), true);
            });

            let sidebar_ended = sidebar.clone();
            let state_ended = state.clone();
            let on_ended: Box<dyn Fn(Uuid, Option<Uuid>)> = Box::new(
                move |connection_id, history_entry_id| {
                    // R4.3/4.5: clear the connected state when the viewer exits.
                    sidebar_ended.decrement_session_count(&connection_id.to_string(), false);
                    // R3.2/4.4: record the end exactly once; skip if there is no
                    // start entry and log a warning (R3.5). A failed borrow is
                    // logged and does not interrupt the session (R3.4).
                    if let Some(entry_id) = history_entry_id {
                        if let Ok(mut state_mut) = state_ended.try_borrow_mut() {
                            state_mut.record_connection_end(entry_id);
                        } else {
                            tracing::warn!(
                                %connection_id,
                                "Could not borrow AppState to record external session end"
                            );
                        }
                    } else {
                        tracing::warn!(
                            %connection_id,
                            "External session ended without a history start entry; skipping record_connection_end"
                        );
                    }
                    // R2.5/2.6: the emblem is visible iff the connection still
                    // has at least one active external session. `on_ended` fires
                    // after the ending session is removed from the registry, so
                    // querying it here reflects the remaining sessions — the
                    // emblem is cleared only once the count reaches zero (a
                    // connection may have more than one external viewer).
                    let still_external = external_session_registry()
                        .is_some_and(|reg| reg.has_active_session(connection_id));
                    sidebar_ended.set_external_session(&connection_id.to_string(), still_external);
                },
            );

            crate::external_session::ExternalSessionRegistry::new(
                crate::external_session::ExternalSessionCallbacks {
                    on_registered,
                    on_ended,
                },
            )
        };

        // Create tunnel manager for standalone SSH tunnels
        let tunnel_manager: SharedTunnelManager = Rc::new(RefCell::new(
            rustconn_core::tunnel_manager::TunnelManager::new(),
        ));

        let main_window = Self {
            window,
            sidebar,
            terminal_notebook,
            split_view,
            session_split_bridges,
            global_color_pool,
            split_container,
            state: state.clone(),
            overlay_split_view,
            external_window_manager,
            external_sessions,
            toast_overlay,
            monitoring,
            activity_coordinator,
            tunnel_manager,
            busy_stack,
            quick_connect_history: types::load_quick_connect_history(&state),
            passthrough_indicator,
            menu_button,
            broadcast_toggle,
            broadcast_hint_shown: Rc::new(std::cell::Cell::new(false)),
            sync_banner,
            secret_banner,
        };

        // Set up window actions
        main_window.setup_actions();

        // Publish BusyStack to thread-local so static methods can acquire guards
        BUSY_STACK.with(|cell| {
            *cell.borrow_mut() = Some(main_window.busy_stack.clone());
        });

        // Publish the external viewer session registry to a thread-local so the
        // static `start_*_connection` launch paths can register external viewers
        // (issue #209) without threading the registry through every call site.
        EXTERNAL_SESSIONS.with(|cell| {
            *cell.borrow_mut() = Some(Rc::clone(&main_window.external_sessions));
        });

        // Set up recording checker for sidebar context menu
        {
            let notebook = main_window.terminal_notebook.clone();
            main_window
                .sidebar
                .set_recording_checker(move |conn_id_str| {
                    if let Ok(conn_id) = Uuid::parse_str(conn_id_str) {
                        notebook
                            .get_all_sessions()
                            .iter()
                            .any(|s| s.connection_id == conn_id && notebook.is_recording(s.id))
                    } else {
                        false
                    }
                });
        }

        // Drive the sidebar recording indicator from recording start/stop.
        // Use a weak notebook reference — the notebook owns this callback,
        // so a strong clone would create an Rc cycle.
        {
            let sidebar = main_window.sidebar.clone();
            let notebook_weak = Rc::downgrade(&main_window.terminal_notebook);
            main_window.terminal_notebook.set_on_recording_changed(
                move |connection_id, recording| {
                    // A connection may have several sessions; keep the dot
                    // while ANY of them is still being recorded.
                    let still_recording = recording
                        || notebook_weak.upgrade().is_some_and(|nb| {
                            nb.get_all_sessions()
                                .iter()
                                .any(|s| s.connection_id == connection_id && nb.is_recording(s.id))
                        });
                    sidebar
                        .update_connection_recording(&connection_id.to_string(), still_recording);
                },
            );
        }

        // Drive the sidebar split-membership marker (R6.2) from the notebook's
        // split-color map. A single re-sync handler recomputes the full desired
        // per-connection state on every split mutation (join / leave / close),
        // which is simpler and more robust than tracking individual deltas.
        {
            let sidebar = main_window.sidebar.clone();
            let notebook_weak = Rc::downgrade(&main_window.terminal_notebook);
            // Connections we last marked as in-a-split, so we can clear the
            // marker when a connection's last split session leaves.
            let marked: Rc<RefCell<std::collections::HashSet<Uuid>>> =
                Rc::new(RefCell::new(std::collections::HashSet::new()));
            main_window
                .terminal_notebook
                .set_on_split_colors_changed(move || {
                    let Some(notebook) = notebook_weak.upgrade() else {
                        return;
                    };
                    // Snapshot the split map first, then drop the borrow before
                    // touching the sidebar to avoid BorrowMutError / nested borrows.
                    let pairs: Vec<(Uuid, usize)> = notebook
                        .split_colors()
                        .borrow()
                        .iter()
                        .map(|(session_id, color)| (*session_id, *color))
                        .collect();

                    // Resolve session_id → connection_id. A connection may host
                    // several split sessions; the last one wins for the single
                    // per-connection marker.
                    // ponytail: last-writer-wins per connection is fine for a
                    // one-square summary; index by session if per-session markers
                    // are ever needed.
                    let mut desired: std::collections::HashMap<Uuid, usize> =
                        std::collections::HashMap::new();
                    for (session_id, color) in pairs {
                        if let Some(info) = notebook.get_session_info(session_id) {
                            desired.insert(info.connection_id, color);
                        }
                    }

                    // Show / refresh markers for connections currently in a split.
                    for (connection_id, color) in &desired {
                        sidebar.set_split_color(&connection_id.to_string(), Some(*color));
                    }
                    // Clear markers for connections that left every split.
                    let mut marked_ref = marked.borrow_mut();
                    for connection_id in marked_ref.iter() {
                        if !desired.contains_key(connection_id) {
                            sidebar.set_split_color(&connection_id.to_string(), None);
                        }
                    }
                    *marked_ref = desired.keys().copied().collect();
                });
        }

        // Load initial data
        main_window.load_connections();

        // Initialize KeePass button status
        main_window.update_keepass_button_status();

        // Connect signals
        main_window.connect_signals();

        // Monitor network interface changes (VPN, WiFi/Ethernet switch) to
        // clean up stale ControlMaster sockets and trigger auto-reconnect (#217)
        network_monitor::setup_network_monitor(
            &main_window.state,
            &main_window.terminal_notebook,
            &main_window.sidebar,
            &main_window.toast_overlay,
        );

        main_window
    }

    /// Sets up window actions
    fn setup_actions(&self) {
        let window = &self.window;
        let state = self.state.clone();
        let sidebar = self.sidebar.clone();
        let terminal_notebook = self.terminal_notebook.clone();

        // Set up action groups
        self.setup_connection_actions(window, &state, &sidebar, &terminal_notebook);
        self.setup_edit_actions(window, &state, &sidebar);
        self.setup_terminal_actions(window, &terminal_notebook, &sidebar, &state);
        self.setup_navigation_actions(
            window,
            &terminal_notebook,
            &sidebar,
            &state,
            &self.session_split_bridges,
        );
        self.setup_group_operations_actions(window, &state, &terminal_notebook, &sidebar);
        self.setup_snippet_actions(window, &state, &terminal_notebook, &sidebar);
        self.setup_cluster_actions(window, &state, &terminal_notebook, &sidebar);
        self.setup_template_actions(window, &state, &sidebar);
        self.setup_workspace_actions(window, &state, &terminal_notebook, &sidebar);
        self.setup_split_view_actions(window);
        self.setup_document_actions(window, &state, &sidebar);
        self.setup_variables_actions(window, &state);
        self.setup_history_actions(window, &state);
        self.setup_misc_actions(window, &state, &sidebar, &terminal_notebook);
        Self::setup_smart_folder_actions(window, &state, &sidebar);
    }
    fn setup_document_actions(
        &self,
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) {
        // adw::ApplicationWindow extends gtk4::ApplicationWindow, so we can use upcast_ref
        let gtk_app_window: &gtk4::ApplicationWindow = window.upcast_ref();
        doc_actions::setup_document_actions(gtk_app_window, state, sidebar);
    }

    /// Sets up miscellaneous actions (drag-drop)
    fn setup_misc_actions(
        &self,
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
        _terminal_notebook: &SharedNotebook,
    ) {
        // Drag-drop item action for reordering connections
        let drag_drop_action =
            gio::SimpleAction::new("drag-drop-item", Some(glib::VariantTy::STRING));
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        drag_drop_action.connect_activate(move |_, param| {
            if let Some(data) = param.and_then(gtk4::glib::Variant::get::<String>) {
                Self::handle_drag_drop(&state_clone, &sidebar_clone, &data);
            }
        });
        window.add_action(&drag_drop_action);

        // Hide drop indicator action - called when drag ends or drop completes
        let hide_drop_indicator_action = gio::SimpleAction::new("hide-drop-indicator", None);
        let sidebar_clone = sidebar.clone();
        hide_drop_indicator_action.connect_activate(move |_, _| {
            sidebar_clone.hide_drop_indicator();
        });
        window.add_action(&hide_drop_indicator_action);

        // Toggle sidebar visibility
        let toggle_sidebar_action = gio::SimpleAction::new("toggle-sidebar", None);
        let split_view_clone = self.overlay_split_view.clone();
        toggle_sidebar_action.connect_activate(move |_, _| {
            let visible = split_view_clone.shows_sidebar();
            split_view_clone.set_show_sidebar(!visible);
        });
        window.add_action(&toggle_sidebar_action);

        // Toggle protocol filters visibility
        let toggle_filters_action = gio::SimpleAction::new("toggle-protocol-filters", None);
        let sidebar_clone = sidebar.clone();
        let state_clone = state.clone();
        toggle_filters_action.connect_activate(move |_, _| {
            let new_visible = !sidebar_clone.is_filter_visible();
            sidebar_clone.set_filter_visible(new_visible);
            // Persist the setting
            if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                let mut settings = state_mut.settings().clone();
                settings.ui.show_protocol_filters = new_visible;
                let _ = state_mut.update_settings(settings);
            }
        });
        window.add_action(&toggle_filters_action);
    }

    /// Connects UI signals
    #[expect(
        clippy::too_many_lines,
        reason = "long match/dispatch over many enum variants; splitting per variant only relocates the boilerplate"
    )]
    fn connect_signals(&self) {
        let state = self.state.clone();
        let sidebar = self.sidebar.clone();
        let terminal_notebook = self.terminal_notebook.clone();
        let split_view = self.split_view.clone();
        let split_view_for_close = self.overlay_split_view.clone();
        let window = self.window.clone();

        // Set up split view cleanup callback for when tabs are closed via TabView
        // This ensures panels show "Empty Panel" placeholder when their session is closed
        {
            let session_bridges_for_cleanup = self.session_split_bridges.clone();
            let split_view_for_cleanup = split_view.clone();
            // Weak ref: the closure is stored *inside* terminal_notebook, so a
            // strong clone here would form an Rc cycle (notebook → closure →
            // notebook) and leak the whole notebook.
            let notebook_for_cleanup = Rc::downgrade(&terminal_notebook);
            let monitoring_for_cleanup = self.monitoring.clone();
            terminal_notebook.set_on_split_cleanup(move |session_id| {
                // If the closing tab OWNS a split bridge (key == owner session_id),
                // its guest (Select-Tabbed) sessions live inside this tab's split
                // widget and their home tabs only show the "Displayed in Split View"
                // placeholder. Return every guest to its home tab before the owner
                // tab — and its bridge widget — is torn down, otherwise the guest
                // is stranded on that placeholder.
                if let Some(notebook) = notebook_for_cleanup.upgrade() {
                    let guests: Vec<Uuid> = {
                        let bridges = session_bridges_for_cleanup.borrow();
                        bridges.get(&session_id).map_or_else(Vec::new, |bridge| {
                            bridge
                                .pane_ids()
                                .iter()
                                .filter_map(|&pane_id| bridge.get_pane_session(pane_id))
                                .filter(|&sid| sid != session_id)
                                .collect()
                        })
                    };
                    for guest_id in guests {
                        tracing::debug!(
                            "on_split_cleanup: returning guest session {} to its home tab \
                             (split owner {} closing)",
                            guest_id,
                            session_id
                        );
                        notebook.clear_tab_split_color(guest_id);
                        notebook.reparent_terminal_to_tab(guest_id);
                        // Resume monitoring suspended when the guest entered the split.
                        if monitoring_for_cleanup.is_suspended(guest_id)
                            && let Some(container) = notebook.get_session_container(guest_id)
                        {
                            monitoring_for_cleanup.resume_monitoring(guest_id, &container);
                        }
                    }
                }

                // Clear session from ALL per-session split bridges
                {
                    let bridges = session_bridges_for_cleanup.borrow();
                    for bridge in bridges.values() {
                        if bridge.is_session_displayed(session_id) {
                            tracing::debug!(
                                "on_split_cleanup: clearing session {} from per-session bridge",
                                session_id
                            );
                            bridge.clear_session_from_panes(session_id);
                        }
                    }
                }
                // Drop the owner's now-emptied bridge so its stale entry does not
                // linger in the map keyed by a closed session.
                session_bridges_for_cleanup.borrow_mut().remove(&session_id);
                // Clear from global split view
                split_view_for_cleanup.clear_session_from_panes(session_id);
            });
        }

        // Set up "Select Tab" callback for empty panel placeholders
        // This provides an alternative to drag-and-drop for moving sessions to split panels
        {
            let split_view_for_select = split_view.clone();
            let notebook_for_select = terminal_notebook.clone();
            let notebook_for_provider = terminal_notebook.clone();
            let notebook_for_terminal = terminal_notebook.clone();
            split_view.setup_select_tab_callback_with_provider(
                move || {
                    // Get all sessions from the notebook
                    // Only show VTE-based sessions (SSH, ZeroTrust, Local Shell)
                    // RDP/VNC/SPICE not supported in split view
                    notebook_for_provider
                        .get_all_sessions()
                        .into_iter()
                        .filter(|s| {
                            s.protocol == "ssh"
                                || s.protocol == "local"
                                || s.protocol.starts_with("zerotrust")
                        })
                        .map(|s| (s.id, s.name, s.protocol))
                        .collect()
                },
                move |panel_uuid, session_id| {
                    tracing::debug!(
                        "Select Tab callback: moving session {} to panel {}",
                        session_id,
                        panel_uuid
                    );

                    // Get terminal from notebook (not from bridge's internal map)
                    let Some(terminal) = notebook_for_terminal.get_terminal(session_id) else {
                        tracing::warn!(
                            "Select Tab callback (global): no terminal found for session {}",
                            session_id
                        );
                        return;
                    };

                    // Move the session to the panel with the terminal
                    if let Err(e) = split_view_for_select
                        .move_session_to_panel_with_terminal(panel_uuid, session_id, &terminal)
                    {
                        tracing::warn!("Failed to move session to panel: {}", e);
                        return;
                    }

                    // Get color for this pane using the new method
                    let color_index = split_view_for_select.get_pane_color(panel_uuid);

                    tracing::debug!(
                        "Select Tab callback (global): panel {} has color {:?}",
                        panel_uuid,
                        color_index
                    );

                    // Set tab color indicator
                    if let Some(color) = color_index {
                        notebook_for_select.set_tab_split_color(session_id, color);
                        split_view_for_select.set_session_color(session_id, color);
                        tracing::debug!(
                            "Select Tab callback (global): applied color {} to session {}",
                            color,
                            session_id
                        );
                    } else {
                        tracing::warn!(
                            "Select Tab callback (global): no color found for panel {}",
                            panel_uuid
                        );
                    }

                    // Note: Do NOT call switch_to_tab() here - the terminal should be
                    // displayed in the split panel, not switched to as the active tab
                },
                Rc::clone(terminal_notebook.split_colors()),
            );

            // Setup close panel callback for empty panel close buttons
            let split_view_for_close = split_view.clone();
            split_view.setup_close_panel_callback(move |pane_uuid| {
                // Focus the pane first so close_pane() closes the correct one
                split_view_for_close.set_focused_pane(Some(pane_uuid));

                // Update focus styling via the adapter
                if let Some(panel_id) = split_view_for_close.get_panel_id_for_uuid(pane_uuid)
                    && let Err(e) = split_view_for_close.adapter_set_focus(panel_id)
                {
                    tracing::warn!("Failed to set focus on panel: {}", e);
                }
            });
        }

        // Set up drag-and-drop for initial pane with notebook lookup
        if let Some(initial_pane_id) = split_view.pane_ids().first().copied() {
            let notebook_for_drop = terminal_notebook.clone();
            let notebook_for_color = terminal_notebook.clone();
            split_view.setup_pane_drop_target_with_callbacks(
                initial_pane_id,
                move |session_id| {
                    let info = notebook_for_drop.get_session_info(session_id)?;
                    let terminal = notebook_for_drop.get_terminal(session_id);
                    Some((info, terminal))
                },
                move |session_id, color_index| {
                    // Set tab color indicator when session is dropped into pane
                    notebook_for_color.set_tab_split_color(session_id, color_index);
                },
            );
        }

        // Set up click handlers for focus management on global split view
        // Note: This is for the global split view; per-session bridges set up their own handlers
        {
            let split_view_for_click = split_view.clone();
            let notebook_for_click = terminal_notebook.clone();
            let sv_for_focus = split_view_for_click.clone();
            let sv_for_session = split_view_for_click.clone();
            let notebook_clone = notebook_for_click.clone();
            let sv_for_terminal = split_view_for_click.clone();

            split_view_for_click.setup_all_panel_click_handlers(move |clicked_pane_uuid| {
                // Update the bridge's focused pane state (handles all focus styling)
                sv_for_focus.set_focused_pane(Some(clicked_pane_uuid));
                // Get session_id from the clicked pane via adapter
                let session_to_switch = sv_for_session.get_pane_session(clicked_pane_uuid);
                // Switch to the tab if there's a session in this pane
                if let Some(session_id) = session_to_switch {
                    notebook_clone.switch_to_tab(session_id);
                    // Grab focus on the terminal (click event is claimed, so we must do this)
                    if let Some(terminal) = sv_for_terminal.get_terminal(session_id) {
                        terminal.grab_focus();
                    }
                }
            });
        }

        // Connect sidebar search with debouncing
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        sidebar.search_entry().connect_search_changed(move |entry| {
            let query = entry.text().to_string();

            // Save pre-search state on first keystroke
            if !query.is_empty() {
                sidebar_clone.save_pre_search_state();
            }

            // Check if we should debounce
            let debouncer = sidebar_clone.search_debouncer();
            if debouncer.should_proceed() {
                // Immediate search - hide spinner and filter
                sidebar_clone.hide_search_pending();
                Self::filter_connections(&state_clone, &sidebar_clone, &query);

                // Restore state if search cleared
                if query.is_empty() {
                    sidebar_clone.restore_pre_search_state();
                }
            } else {
                // Debounced - show spinner and schedule search
                sidebar_clone.show_search_pending();
                sidebar_clone.set_pending_search_query(Some(query.clone()));

                // Schedule delayed search using glib timeout
                let state_for_timeout = state_clone.clone();
                let sidebar_for_timeout = sidebar_clone.clone();
                let delay_ms = debouncer.delay().as_millis() as u32;

                glib::timeout_add_local_once(
                    std::time::Duration::from_millis(u64::from(delay_ms)),
                    move || {
                        // Only proceed if this is still the pending query
                        if let Some(pending) = sidebar_for_timeout.pending_search_query()
                            && pending == query
                        {
                            sidebar_for_timeout.hide_search_pending();
                            sidebar_for_timeout.set_pending_search_query(None);
                            Self::filter_connections(
                                &state_for_timeout,
                                &sidebar_for_timeout,
                                &pending,
                            );

                            // Restore state if search cleared
                            if pending.is_empty() {
                                sidebar_for_timeout.restore_pre_search_state();
                            }
                        }
                    },
                );
            }
        });

        // Add to search history when user presses Enter or stops searching
        let sidebar_for_history = sidebar.clone();
        let state_for_history = state.clone();
        sidebar.search_entry().connect_activate(move |entry| {
            let query = entry.text().to_string();
            if !query.is_empty() {
                sidebar_for_history.add_to_search_history(&query);
                // Persist to settings
                if let Ok(mut state_mut) = state_for_history.try_borrow_mut() {
                    state_mut.settings_mut().ui.add_search_history(&query);
                    if let Err(e) = state_mut.save_settings() {
                        tracing::warn!(?e, "Failed to save settings");
                    }
                }
            }
        });

        // Also add to history when search entry loses focus with non-empty query
        let sidebar_for_focus = sidebar.clone();
        let state_for_focus = state.clone();
        sidebar
            .search_entry()
            .connect_has_focus_notify(move |entry| {
                if !entry.has_focus() {
                    let query = entry.text().to_string();
                    if !query.is_empty() {
                        sidebar_for_focus.add_to_search_history(&query);
                        // Persist to settings
                        if let Ok(mut state_mut) = state_for_focus.try_borrow_mut() {
                            state_mut.settings_mut().ui.add_search_history(&query);
                            if let Err(e) = state_mut.save_settings() {
                                tracing::warn!(?e, "Failed to save settings");
                            }
                        }
                    }
                }
            });

        // Connect sidebar double-click to connect
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let notebook_clone = terminal_notebook.clone();
        let split_view_clone = split_view.clone();
        let monitoring_clone = self.monitoring.clone();
        let activity_clone_sidebar = self.activity_coordinator.clone();
        let session_bridges_for_activate = self.session_split_bridges.clone();

        // R7.5: `connect_activate` does not carry keyboard-modifier state, so a
        // capture-phase primary-button gesture records whether Shift/Ctrl was
        // held on the press that precedes activation. Capture phase without
        // claiming the sequence means it never interferes with the ListView's
        // own selection/activation or the secondary-button context-menu gesture
        // (mirrors the existing primary-button dismiss gesture in `sidebar/mod`).
        let force_new_modifier = Rc::new(std::cell::Cell::new(false));
        let modifier_gesture = gtk4::GestureClick::new();
        modifier_gesture.set_button(gtk4::gdk::BUTTON_PRIMARY);
        modifier_gesture.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let force_new_for_gesture = force_new_modifier.clone();
        modifier_gesture.connect_pressed(move |gesture, _n_press, _x, _y| {
            let modifiers = gesture.current_event_state();
            force_new_for_gesture.set(
                modifiers.contains(gtk4::gdk::ModifierType::SHIFT_MASK)
                    || modifiers.contains(gtk4::gdk::ModifierType::CONTROL_MASK),
            );
            // Observe-only: deny the sequence so the gesture never claims the
            // press and the ListView's own selection/activation handling (and
            // the TreeExpander's primary-button expand/collapse) proceed intact.
            gesture.set_state(gtk4::EventSequenceState::Denied);
        });
        sidebar.list_view().add_controller(modifier_gesture);

        sidebar
            .list_view()
            .connect_activate(move |list_view, position| {
                // Get the item at position from the tree model
                let tree_model = sidebar_clone.tree_model();
                if let Some(item) = tree_model.item(position)
                    && let Some(row) = item.downcast_ref::<gtk4::TreeListRow>()
                    && let Some(conn_item) = row
                        .item()
                        .and_then(|i| i.downcast::<crate::sidebar::ConnectionItem>().ok())
                    && conn_item.is_group()
                {
                    // Toggle expand/collapse for groups on double-click
                    row.set_expanded(!row.is_expanded());
                    // Re-select the row after toggle so it stays highlighted
                    if let Some(model) = list_view.model()
                        && let Some(sel) = model.downcast_ref::<gtk4::SingleSelection>()
                    {
                        sel.set_selected(position);
                    }
                    return;
                }
                // Read (and reset) the modifier captured on the preceding press.
                // Reset guards against a stale value on a later keyboard
                // (Enter) activation, which fires no primary-button press.
                let force_new = force_new_modifier.get();
                force_new_modifier.set(false);

                Self::connect_at_position_with_split(
                    &state_clone,
                    &sidebar_clone,
                    &notebook_clone,
                    &split_view_clone,
                    &monitoring_clone,
                    &session_bridges_for_activate,
                    position,
                    force_new,
                    Some(&activity_clone_sidebar),
                );
            });

        // Connect TabView page selection
        // With the new per-tab split architecture, GTK handles content switching
        // automatically — split views live inside TabPages, not in a global container.
        let session_bridges_for_tab = self.session_split_bridges.clone();
        let global_split_view = split_view.clone();
        let split_container_for_tab = self.split_container.clone();
        let notebook_clone = terminal_notebook.clone();
        let activity_for_tab = self.activity_coordinator.clone();
        let sessions_for_tab = terminal_notebook.sessions_map();
        terminal_notebook.tab_view().connect_notify_local(
            Some("selected-page"),
            move |tab_view, _| {
                let Some(selected_page) = tab_view.selected_page() else {
                    return;
                };
                let page_num = tab_view.page_position(&selected_page) as u32;

                // Hide legacy global containers (no longer used for content)
                global_split_view.widget().set_visible(false);
                split_container_for_tab.set_visible(false);

                // Get session ID for this page
                if let Some(session_id) = notebook_clone.get_session_id_for_page(page_num) {
                    // Clear activity monitor indicator and reset notification state
                    // but preserve split color indicators
                    activity_for_tab.on_tab_switched(session_id);
                    if !notebook_clone
                        .split_colors()
                        .borrow()
                        .contains_key(&session_id)
                        && let Some(page) = sessions_for_tab.borrow().get(&session_id)
                    {
                        page.set_indicator_icon(gio::Icon::NONE);
                    }

                    // If session has a split bridge, focus the correct pane
                    let bridges = session_bridges_for_tab.borrow();
                    if let Some(bridge) = bridges.get(&session_id) {
                        // Focus the pane containing the selected session
                        for pane_id in bridge.pane_ids() {
                            if bridge.get_pane_session(pane_id) == Some(session_id) {
                                let _ = bridge.focus_pane(pane_id);
                                if let Some(terminal) = bridge.get_terminal(session_id) {
                                    terminal.grab_focus();
                                }
                                break;
                            }
                        }
                    } else {
                        // Regular tab — focus the terminal directly
                        if let Some(terminal) = notebook_clone.get_terminal(session_id) {
                            terminal.grab_focus();
                        }
                    }
                }
                // Welcome tab — nothing extra to do, GTK shows the content
            },
        );

        // Save window state on close and handle minimize to tray
        let state_clone = state.clone();
        let split_view_clone = split_view_for_close;
        let sidebar_clone = sidebar.clone();
        let notebook_for_close = terminal_notebook.clone();
        let tunnel_manager_for_close = self.tunnel_manager.clone();
        // One-shot flag: set after the user confirms closing with open
        // sessions, so the second close() pass proceeds without re-asking.
        let force_close = Rc::new(std::cell::Cell::new(false));
        window.connect_close_request(move |win| {
            // When minimize-to-tray is enabled the window only hides and the
            // app keeps running — no confirmation needed.
            let minimize_to_tray = state_clone.try_borrow().is_ok_and(|s| {
                s.settings().ui.minimize_to_tray && s.settings().ui.enable_tray_icon
            });

            // Confirm before quitting with open sessions (GNOME HIG: protect
            // against accidental loss of active connections). Count tabless
            // external-viewer sessions too (issue #209), so a window holding
            // only external VNC/RDP/SPICE sessions still warns before quitting.
            let external_open = external_session_registry().map_or(0, |reg| reg.active_count());
            let open_sessions = notebook_for_close.session_count() + external_open;
            if !minimize_to_tray && !force_close.get() && open_sessions > 0 {
                let dialog = Self::close_confirmation_dialog(open_sessions);
                let force_close_confirm = force_close.clone();
                let win_weak = win.downgrade();
                dialog.connect_response(Some("close"), move |_, _| {
                    force_close_confirm.set(true);
                    if let Some(w) = win_weak.upgrade() {
                        w.close();
                    }
                });
                dialog.present(Some(win));
                return glib::Propagation::Stop;
            }

            // Flush all active session recordings before shutdown
            notebook_for_close.flush_active_recordings();

            // Terminate tracked external viewers (issue #209): kill owned
            // children so they do not outlive RustConn as orphans, and close
            // their open history entries. Detaching viewers keep running.
            if let Some(registry) = external_session_registry() {
                registry.shutdown();
            }

            // Stop all standalone SSH tunnels
            tunnel_manager_for_close.borrow_mut().stop_all();

            // Save window geometry and expanded groups state
            let (width, height) = win.default_size();
            let is_maximized = win.is_maximized();
            let sidebar_width = (split_view_clone.max_sidebar_width() as i32).max(180);

            // Save expanded groups state
            let expanded = sidebar_clone.get_expanded_groups();

            if let Ok(mut state) = state_clone.try_borrow_mut() {
                // Update expanded groups
                if let Err(e) = state.update_expanded_groups(expanded) {
                    tracing::warn!(?e, "Failed to update expanded groups");
                }

                let mut settings = state.settings().clone();
                if settings.ui.remember_window_geometry {
                    settings.ui.window_width = Some(width);
                    settings.ui.window_height = Some(height);
                    settings.ui.window_maximized = is_maximized;
                    settings.ui.sidebar_width = Some(sidebar_width);
                    if let Err(e) = state.update_settings(settings.clone()) {
                        tracing::warn!(?e, "Failed to update settings");
                    }
                }

                // Check if we should minimize to tray instead of closing
                if settings.ui.minimize_to_tray && settings.ui.enable_tray_icon {
                    // Hide the window instead of closing
                    win.set_visible(false);
                    return glib::Propagation::Stop;
                }
            }

            glib::Propagation::Proceed
        });
    }

    /// Loads connections into the sidebar
    fn load_connections(&self) {
        let expanded_groups = self.state.borrow().expanded_groups().clone();

        // Use sorted rebuild to ensure alphabetical order by default
        sorting::rebuild_sidebar_sorted(&self.state, &self.sidebar);

        // Apply expanded state after populating
        self.sidebar.apply_expanded_groups(&expanded_groups);
    }

    /// Updates the password vault button status in the sidebar based on current settings
    fn update_keepass_button_status(&self) {
        let state_ref = self.state.borrow();
        let settings = state_ref.settings();
        let backend = settings.secrets.preferred_backend;

        // For libsecret, Bitwarden, 1Password, Pass, and Passbolt, always enabled (no database file needed)
        // For KeePassXC/KdbxFile, check if enabled and database exists
        let (enabled, database_exists) = match backend {
            rustconn_core::config::SecretBackendType::LibSecret
            | rustconn_core::config::SecretBackendType::MacOsKeychain
            | rustconn_core::config::SecretBackendType::Bitwarden
            | rustconn_core::config::SecretBackendType::OnePassword
            | rustconn_core::config::SecretBackendType::Passbolt
            | rustconn_core::config::SecretBackendType::Pass
            | rustconn_core::config::SecretBackendType::EncryptedFile => (true, true),
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
        drop(state_ref);

        self.sidebar.update_keepass_status(enabled, database_exists);
    }

    /// Public method to refresh KeePass button status (called after settings change)
    #[expect(
        dead_code,
        reason = "Part of KeePass integration API, called from settings dialog"
    )]
    pub fn refresh_keepass_status(&self) {
        self.update_keepass_button_status();
    }

    /// Filters connections based on search query
    fn filter_connections(state: &SharedAppState, sidebar: &SharedSidebar, query: &str) {
        use rustconn_core::search::SearchEngine;

        if query.is_empty() {
            // Restore full hierarchy when search is cleared
            Self::reload_sidebar(state, sidebar);
            // Restore the tree state that was saved before search started
            sidebar.restore_pre_search_state();
            return;
        }

        // Save tree state before first search keystroke
        sidebar.save_pre_search_state();

        let store = sidebar.store();
        store.remove_all();

        let state_ref = state.borrow();

        // Get connections and groups for search. `list_connections()` already
        // returns borrowed `&Connection`s, so the search runs on references —
        // no per-keystroke deep clone of every connection.
        let connections: Vec<&rustconn_core::models::Connection> = state_ref.list_connections();
        let groups: Vec<_> = state_ref.list_groups().iter().cloned().cloned().collect();

        // Check for single protocol filter syntax (protocol:rdp, proto:ssh, p:vnc)
        let single_protocol = query
            .strip_prefix("protocol:")
            .or_else(|| query.strip_prefix("proto:"))
            .or_else(|| query.strip_prefix("p:"));

        if let Some(protocol_name) = single_protocol {
            // Handle single protocol filter — direct filtering without scoring
            let protocol_names: Vec<&str> = vec![protocol_name.trim()];
            let mut filtered_connections = Vec::new();

            for conn in &connections {
                let protocol = get_protocol_string(&conn.protocol_config);
                let protocol_lower = protocol.to_lowercase();

                if protocol_names
                    .iter()
                    .any(|p| p.to_lowercase() == protocol_lower)
                {
                    filtered_connections.push(conn);
                }
            }

            for conn in filtered_connections {
                let protocol = get_protocol_string(&conn.protocol_config);
                let item = ConnectionItem::new_connection(
                    &conn.id.to_string(),
                    &conn.name,
                    &protocol,
                    &conn.host,
                );
                item.set_description(conn.description.as_deref().unwrap_or(""));
                store.append(&item);
            }
        } else if let Some(protocols_str) = query.strip_prefix("protocols:") {
            // Handle multiple protocol filters with OR logic
            let protocol_names: Vec<&str> = protocols_str.split(',').collect();
            let mut filtered_connections = Vec::new();

            for conn in &connections {
                let protocol = get_protocol_string(&conn.protocol_config);
                let protocol_lower = protocol.to_lowercase();

                if protocol_names
                    .iter()
                    .any(|p| p.to_lowercase() == protocol_lower)
                {
                    filtered_connections.push(conn);
                }
            }

            for conn in filtered_connections {
                let protocol = get_protocol_string(&conn.protocol_config);
                let item = ConnectionItem::new_connection(
                    &conn.id.to_string(),
                    &conn.name,
                    &protocol,
                    &conn.host,
                );
                item.set_description(conn.description.as_deref().unwrap_or(""));
                store.append(&item);
            }
        } else {
            // Use standard search engine for other queries
            let search_engine = SearchEngine::new();
            let parsed_query = match SearchEngine::parse_query(query) {
                Ok(q) => q,
                Err(_) => {
                    // Fall back to simple text search on parse error
                    rustconn_core::search::SearchQuery::with_text(query)
                }
            };

            // Perform search with ranking
            let results = search_engine.search(&parsed_query, &connections, &groups);

            // Index by id once so result lookup is O(1) instead of O(n) per hit.
            let conn_by_id: std::collections::HashMap<_, _> =
                connections.iter().map(|c| (c.id, *c)).collect();

            // Display results sorted by relevance
            for result in results {
                if let Some(conn) = conn_by_id.get(&result.connection_id) {
                    let protocol = get_protocol_string(&conn.protocol_config);

                    // Create display name with relevance indicator
                    let display_name = if result.score >= 0.9 {
                        format!("★★★ {}", conn.name) // High relevance
                    } else if result.score >= 0.7 {
                        format!("★★ {}", conn.name) // Medium relevance
                    } else if result.score >= 0.5 {
                        format!("★ {}", conn.name) // Low relevance
                    } else {
                        conn.name.clone() // Very low relevance
                    };

                    let item = ConnectionItem::new_connection(
                        &conn.id.to_string(),
                        &display_name,
                        &protocol,
                        &conn.host,
                    );
                    item.set_description(conn.description.as_deref().unwrap_or(""));
                    store.append(&item);
                }
            }
        }
    }

    /// Connects to the selected connection
    fn connect_selected(
        state: &SharedAppState,
        sidebar: &SharedSidebar,
        notebook: &SharedNotebook,
        monitoring: &types::SharedMonitoring,
    ) {
        // Get selected item from sidebar using the sidebar's method
        let Some(conn_item) = sidebar.get_selected_item() else {
            return;
        };

        // Only connect if it's not a group
        if conn_item.is_group() {
            return;
        }

        let id_str = conn_item.id();
        if let Ok(conn_id) = Uuid::parse_str(&id_str) {
            Self::start_connection(state, notebook, sidebar, monitoring, conn_id);
        }
    }

    /// Connects to a connection at a specific position with split view support.
    ///
    /// Smart double-click (R7): unless `force_new` is set (a Shift/Ctrl modifier
    /// was held or the "Open new session" menu item was used), an existing live
    /// session for the connection is focused instead of spawning a duplicate.
    /// An embedded session is focused in place (R7.1/7.2/7.4); an external-only
    /// session shows an informational toast (a foreign OS window cannot be
    /// raised reliably from RustConn); zero sessions launch a new one (R7.3).
    #[expect(
        clippy::too_many_arguments,
        reason = "smart double-click dispatch (R7) needs the full window context: state, sidebar, notebook, split view, monitoring, split bridges plus position/force_new/activity"
    )]
    fn connect_at_position_with_split(
        state: &SharedAppState,
        sidebar: &SharedSidebar,
        notebook: &SharedNotebook,
        split_view: &SharedSplitView,
        monitoring: &types::SharedMonitoring,
        session_split_bridges: &SessionSplitBridges,
        position: u32,
        force_new: bool,
        activity: Option<&types::SharedActivityCoordinator>,
    ) {
        // Get the item at position from the tree model (not the flat store)
        let tree_model = sidebar.tree_model();
        if let Some(item) = tree_model.item(position) {
            // TreeListModel returns TreeListRow, need to get the actual item
            if let Some(row) = item.downcast_ref::<gtk4::TreeListRow>()
                && let Some(conn_item) =
                    row.item().and_then(|i| i.downcast::<ConnectionItem>().ok())
                && !conn_item.is_group()
            {
                let id_str = conn_item.id();
                if let Ok(conn_id) = Uuid::parse_str(&id_str) {
                    if !force_new {
                        // R7.1/7.4: focus the most recently created embedded
                        // session. Collect first (an owned `Uuid`) so no notebook
                        // borrow is held across the focus call below.
                        let target = notebook
                            .get_all_sessions()
                            .into_iter()
                            .filter(|s| s.connection_id == conn_id)
                            .max_by_key(|s| s.connected_at)
                            .map(|s| s.id);

                        if let Some(target_id) = target {
                            if Self::focus_embedded_session(
                                notebook,
                                session_split_bridges,
                                target_id,
                            ) {
                                return;
                            }
                            // R7.6: the session vanished between resolution and
                            // focus — fall through and launch a new one.
                        } else if external_session_registry()
                            .is_some_and(|reg| reg.has_active_session(conn_id))
                        {
                            // R7 external-only: do not duplicate a session that
                            // lives in a foreign viewer window; inform the user.
                            crate::toast::show_info_toast_on_active_window(&crate::i18n::i18n(
                                "Already running in an external window",
                            ));
                            return;
                        }
                    }

                    // force_new, zero live sessions, or the R7.6 race: launch new.
                    sidebar.update_connection_status(&conn_id.to_string(), "connecting");
                    Self::start_connection_with_credential_resolution(
                        state.clone(),
                        notebook.clone(),
                        split_view.clone(),
                        sidebar.clone(),
                        monitoring.clone(),
                        conn_id,
                        activity.cloned(),
                    );
                }
            }
        }
    }

    /// Focuses an existing embedded session for the smart double-click.
    ///
    /// Selects the session's owner tab and, when the session lives in a split,
    /// focuses its pane and grabs input focus (R7.1, R7.2, R7.4). Returns
    /// `false` if the session disappeared before it could be focused (R7.6),
    /// signalling the caller to launch a new session instead.
    fn focus_embedded_session(
        notebook: &SharedNotebook,
        session_split_bridges: &SessionSplitBridges,
        target_id: Uuid,
    ) -> bool {
        // Resolve the owner tab and (if split) the pane holding the session up
        // front, then release the bridges borrow before any tab switch / focus
        // call — those re-enter the notebook/bridge and would risk a
        // BorrowMutError if a borrow were still held.
        let mut owner_tab = target_id;
        let mut bridge_pane: Option<(Rc<SplitViewBridge>, Uuid)> = None;
        {
            let bridges = session_split_bridges.borrow();
            for (owner_id, bridge) in bridges.iter() {
                if bridge.active_sessions().contains(&target_id) {
                    owner_tab = *owner_id;
                    bridge_pane = bridge
                        .pane_ids()
                        .into_iter()
                        .find(|pane| bridge.get_pane_session(*pane) == Some(target_id))
                        .map(|pane| (Rc::clone(bridge), pane));
                    break;
                }
            }
        }

        // R7.6: bail if the session vanished between resolution and focus.
        if notebook.get_session_info(target_id).is_none() {
            return false;
        }

        notebook.switch_to_tab(owner_tab);
        notebook.show_tab_view_content();

        if let Some((bridge, pane_uuid)) = bridge_pane
            && let Err(e) = bridge.focus_pane(pane_uuid)
        {
            tracing::debug!(error = %e, %target_id, "focus_pane failed for smart double-click");
        }

        // Set input focus on the session widget where one exists (terminals);
        // embedded viewers have no VTE widget, so selecting the tab is enough.
        if let Some(terminal) = notebook.get_terminal(target_id) {
            terminal.grab_focus();
        }
        true
    }

    /// Starts a connection with split view integration
    pub fn start_connection_with_split(
        state: &SharedAppState,
        notebook: &SharedNotebook,
        split_view: &SharedSplitView,
        sidebar: &SharedSidebar,
        monitoring: &types::SharedMonitoring,
        connection_id: Uuid,
        activity: Option<&types::SharedActivityCoordinator>,
    ) -> Option<Uuid> {
        // Update status to connecting
        sidebar.update_connection_status(&connection_id.to_string(), "connecting");

        let session_id =
            match Self::start_connection(state, notebook, sidebar, monitoring, connection_id) {
                types::ConnectionStartResult::Started(id) => id,
                types::ConnectionStartResult::Pending => {
                    // Async port check in progress — keep "connecting" status.
                    // The protocol callback will set "connected" or "failed".
                    return None;
                }
                types::ConnectionStartResult::Failed => {
                    sidebar.update_connection_status(&connection_id.to_string(), "failed");
                    // Show connection failure toast with connection name
                    if let Ok(state_ref) = state.try_borrow()
                        && let Some(conn) = state_ref.get_connection(connection_id)
                    {
                        let name = conn.name.clone();
                        drop(state_ref);
                        crate::toast::show_error_toast_on_active_window(&crate::i18n::i18n_f(
                            "Connection to ‘{}’ failed",
                            &[&name],
                        ));
                    }
                    return None;
                }
            };

        // Get session info to check protocol
        if let Some(info) = notebook.get_session_info(session_id) {
            // VNC, RDP, and SPICE sessions are displayed directly in notebook tab
            if info.protocol == "vnc" || info.protocol == "rdp" || info.protocol == "spice" {
                // Hide split view and expand notebook for VNC/RDP/SPICE
                split_view.widget().set_visible(false);
                split_view.widget().set_vexpand(false);
                notebook.widget().set_vexpand(true);
                notebook.show_tab_view_content();
                return Some(session_id);
            }

            // For SSH: register session info for potential drag-and-drop
            // Per spec: new connections ALWAYS open in a new tab, never in split pane
            // The display widget is resolved on demand via the content provider.
            split_view.add_session(info.clone());

            // Per spec: new connections always show in TabView (as a new tab)
            // Hide split view, show TabView content
            split_view.widget().set_visible(false);
            split_view.widget().set_vexpand(false);
            notebook.widget().set_vexpand(true);
            notebook.show_tab_view_content();

            // For Zero Trust, detect connection via terminal content changes
            // (SSH status detection is handled inside start_ssh_connection_internal)
            if info.protocol.starts_with("zerotrust") {
                // Set status to connecting initially (only if not already connected)
                if sidebar
                    .get_connection_status(&connection_id.to_string())
                    .is_none()
                {
                    sidebar.update_connection_status(&connection_id.to_string(), "connecting");
                }

                let sidebar_clone = sidebar.clone();
                let notebook_clone = notebook.clone();
                let connection_id_str = connection_id.to_string();
                let session_connected = std::rc::Rc::new(std::cell::Cell::new(false));
                let session_connected_clone = session_connected.clone();

                notebook.connect_contents_changed(session_id, move || {
                    if !session_connected_clone.get() {
                        // Zero Trust: any output indicates success (threshold 0)
                        if let Some(row) = notebook_clone.get_terminal_cursor_row(session_id) {
                            tracing::debug!(
                                protocol = "zerotrust",
                                cursor_row = row,
                                threshold = 0,
                                "Zero Trust status detection: checking cursor row"
                            );
                            if row > 0 {
                                sidebar_clone.increment_session_count(&connection_id_str);
                                session_connected_clone.set(true);
                                tracing::info!(
                                    protocol = "zerotrust",
                                    cursor_row = row,
                                    "Terminal connection detected as established"
                                );
                            }
                        }
                    }
                });
            }
        }

        // Activity monitoring is wired centrally via the notebook's
        // `on_session_created` hook (see `MainWindow::new`), so it is NOT set
        // up here. The old per-path call only covered synchronous connects and
        // missed the async port-check path; the central hook covers both.
        let _ = activity;

        Some(session_id)
    }

    /// Starts a connection and returns the `session_id`
    pub fn start_connection(
        state: &SharedAppState,
        notebook: &SharedNotebook,
        sidebar: &SharedSidebar,
        monitoring: &types::SharedMonitoring,
        connection_id: Uuid,
    ) -> types::ConnectionStartResult {
        let state_ref = state.borrow();

        let Some(conn) = state_ref.get_connection(connection_id) else {
            return types::ConnectionStartResult::Failed;
        };

        // Auto-WoL: send magic packet before connecting if configured
        // Fire-and-forget on background thread to avoid blocking GTK
        if let Some(wol_config) = conn.get_wol_config() {
            let wol_config = wol_config.clone();
            let conn_name = conn.name.clone();
            tracing::info!(
                mac = %wol_config.mac_address,
                "Sending auto-WoL before connecting to {}",
                conn_name,
            );
            std::thread::spawn(move || {
                if let Err(e) = rustconn_core::wol::send_wol_with_retry(&wol_config, 3, 500) {
                    tracing::warn!(?e, "Auto-WoL failed for {}", conn_name,);
                }
            });
        }

        let protocol = get_protocol_string(&conn.protocol_config);
        let logging_enabled = state_ref.settings().logging.enabled;

        // Clone connection data before dropping borrow
        let conn_clone = conn.clone();
        drop(state_ref);

        // Execute pre-connect task if configured
        if let Some(ref task) = conn_clone.pre_connect_task {
            tracing::info!(
                connection = %conn_clone.name,
                command = %task.command,
                "Executing pre-connect task"
            );

            // Build variable manager for substitution
            let global_variables = state
                .try_borrow()
                .ok()
                .map(|s| crate::state::resolve_global_variables(s.settings()))
                .unwrap_or_default();
            let mut var_manager = VariableManager::new();
            for var in &global_variables {
                var_manager.set_global(var.clone());
            }
            // Add connection-scoped synthetic variables (host, port, username, name)
            let conn_id = conn_clone.id;
            var_manager.set_connection(
                conn_id,
                rustconn_core::Variable::new("host", &conn_clone.host),
            );
            var_manager.set_connection(
                conn_id,
                rustconn_core::Variable::new("port", conn_clone.port.to_string()),
            );
            if let Some(ref user) = conn_clone.username {
                var_manager.set_connection(conn_id, rustconn_core::Variable::new("username", user));
            }
            var_manager.set_connection(
                conn_id,
                rustconn_core::Variable::new("name", &conn_clone.name),
            );

            let folder_tracker = state
                .try_borrow()
                .ok()
                .map(|s| Arc::clone(s.folder_tracker()))
                .unwrap_or_default();
            let executor = TaskExecutor::with_tracker(Arc::new(var_manager), folder_tracker);
            let folder_id = conn_clone.group_id;

            let result = crate::async_utils::with_runtime(|rt| {
                rt.block_on(async {
                    // ponytail: 60s ceiling protects the GTK main thread from
                    // hanging forever when the user omits a timeout on the task.
                    // The task's own timeout (if set) fires first; this is a safety net.
                    let ceiling = std::time::Duration::from_mins(1);
                    tokio::time::timeout(
                        ceiling,
                        executor.execute_pre_connect(
                            task,
                            VariableScope::Connection(conn_id),
                            folder_id,
                        ),
                    )
                    .await
                    .unwrap_or(Err(rustconn_core::automation::TaskError::Timeout(60_000)))
                })
            });

            match result {
                Ok(Ok(_)) => {
                    tracing::info!(
                        connection = %conn_clone.name,
                        "Pre-connect task completed successfully"
                    );
                }
                Ok(Err(e)) => {
                    tracing::error!(
                        connection = %conn_clone.name,
                        command = %task.command,
                        error = %e,
                        "Pre-connect task failed"
                    );
                    if task.abort_on_failure {
                        crate::toast::show_error_toast_on_active_window(&crate::i18n::i18n_f(
                            "Pre-connect task failed: {}",
                            &[&e.to_string()],
                        ));
                        return types::ConnectionStartResult::Failed;
                    }
                }
                Err(runtime_err) => {
                    tracing::error!(
                        connection = %conn_clone.name,
                        error = %runtime_err,
                        "Failed to create async runtime for pre-connect task"
                    );
                    if task.abort_on_failure {
                        crate::toast::show_error_toast_on_active_window(&crate::i18n::i18n_f(
                            "Pre-connect task failed: {}",
                            &[&runtime_err],
                        ));
                        return types::ConnectionStartResult::Failed;
                    }
                }
            }
        }

        // Execute port knock sequence if configured (after pre-connect task, before protocol connect)
        if let Some(ref knock_seq) = conn_clone.knock_sequence {
            tracing::info!(
                connection = %conn_clone.name,
                knocks = knock_seq.knocks.len(),
                "Executing port knock sequence"
            );
            match rustconn_core::connection::knock::execute_knock_sequence(
                &conn_clone.host,
                knock_seq,
            ) {
                Ok(result) => {
                    tracing::info!(
                        connection = %conn_clone.name,
                        total_ms = result.total_ms,
                        "Port knock sequence completed"
                    );
                }
                Err(e) => {
                    tracing::error!(
                        connection = %conn_clone.name,
                        error = %e,
                        "Port knock sequence failed"
                    );
                    crate::toast::show_error_toast_on_active_window(&crate::i18n::i18n_f(
                        "Port knock failed: {}",
                        &[&e.to_string()],
                    ));
                    return types::ConnectionStartResult::Failed;
                }
            }
        }

        // Send fwknop SPA packet if configured (after knock, before port check)
        if let Some(ref spa_cfg) = conn_clone.spa_config
            && let (Some(rij_key_val), Some(hmac_key_val)) =
                (&spa_cfg.rijndael_key_ref, &spa_cfg.hmac_key_ref)
            && !rij_key_val.is_empty()
            && !hmac_key_val.is_empty()
        {
            let rij_secret = secrecy::SecretString::new(rij_key_val.clone().into());
            let hmac_secret = secrecy::SecretString::new(hmac_key_val.clone().into());
            let username = conn_clone.username.as_deref().unwrap_or("root");

            tracing::info!(
                connection = %conn_clone.name,
                dest_port = spa_cfg.dest_port,
                access = %spa_cfg.access,
                "Sending fwknop SPA packet"
            );

            match rustconn_core::connection::spa::send_spa(
                &conn_clone.host,
                spa_cfg.dest_port,
                &rij_secret,
                &hmac_secret,
                &spa_cfg.access,
                username,
            ) {
                Ok(result) => {
                    tracing::info!(
                        connection = %conn_clone.name,
                        packet_size = result.packet_size,
                        elapsed_ms = result.elapsed_ms,
                        "SPA packet sent successfully"
                    );
                }
                Err(e) => {
                    tracing::error!(
                        connection = %conn_clone.name,
                        error = %e,
                        "SPA packet send failed"
                    );
                    crate::toast::show_error_toast_on_active_window(&crate::i18n::i18n_f(
                        "SPA failed: {}",
                        &[&e.to_string()],
                    ));
                    return types::ConnectionStartResult::Failed;
                }
            }
        }

        // Protocols that use async port check return None when the check is
        // in progress — this is NOT a failure.  We track whether the protocol
        // *may* be pending so we can distinguish Pending from Failed below.
        let may_be_pending = matches!(
            protocol.as_str(),
            "ssh" | "vnc" | "spice" | "telnet" | "mosh"
        );

        let session_id = match protocol.as_str() {
            "ssh" => protocols::start_ssh_connection(
                state,
                notebook,
                sidebar,
                monitoring,
                connection_id,
                &conn_clone,
                logging_enabled,
            ),
            "vnc" => protocols::start_vnc_connection(
                state,
                notebook,
                sidebar,
                connection_id,
                &conn_clone,
            ),
            "rdp" => {
                // RDP connections are handled by start_rdp_session_with_credentials
                // which is called from start_connection_with_credential_resolution
                tracing::warn!(
                    "RDP connection reached start_connection without credentials. \
                     Use start_connection_with_credential_resolution instead."
                );
                None
            }
            "spice" => protocols::start_spice_connection(
                state,
                notebook,
                sidebar,
                connection_id,
                &conn_clone,
            ),
            "telnet" => protocols::start_telnet_connection(
                state,
                notebook,
                sidebar,
                connection_id,
                &conn_clone,
                logging_enabled,
            ),
            "serial" => protocols::start_serial_connection(
                state,
                notebook,
                sidebar,
                connection_id,
                &conn_clone,
                logging_enabled,
            ),
            "kubernetes" => protocols::start_kubernetes_connection(
                state,
                notebook,
                sidebar,
                connection_id,
                &conn_clone,
                logging_enabled,
            ),
            "mosh" => protocols::start_mosh_connection(
                state,
                notebook,
                sidebar,
                connection_id,
                &conn_clone,
                logging_enabled,
            ),
            p if p == "zerotrust" || p.starts_with("zerotrust:") => {
                protocols::start_zerotrust_connection(
                    state,
                    notebook,
                    sidebar,
                    connection_id,
                    &conn_clone,
                    logging_enabled,
                )
            }
            "sftp" => {
                // SFTP opens file manager — no terminal session
                Self::handle_sftp_connect(state, notebook, Some(sidebar), None, connection_id);
                None
            }
            "web" => {
                // Web: Embedded mode opens in-tab, System/Custom open externally
                Self::handle_web_connect(state, notebook, sidebar, connection_id);
                None
            }
            _ => {
                // Unknown protocol
                None
            }
        };

        // Execute key sequence after connection is established (terminal protocols only)
        if let Some(sid) = session_id
            && let Some(ref seq) = conn_clone.key_sequence
            && !seq.is_empty()
        {
            tracing::info!(
                connection = %conn_clone.name,
                elements = seq.len(),
                "Scheduling key sequence after connection"
            );
            // Delay key sequence to allow terminal to initialize
            let notebook_clone = notebook.clone();
            let seq_clone = seq.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(500), move || {
                notebook_clone.execute_key_sequence(sid, &seq_clone);
            });
        }

        match session_id {
            Some(sid) => types::ConnectionStartResult::Started(sid),
            None if may_be_pending => types::ConnectionStartResult::Pending,
            None => types::ConnectionStartResult::Failed,
        }
    }

    /// Shows the new connection dialog with optional template selection
    fn show_new_connection_dialog(
        window: &adw::ApplicationWindow,
        state: SharedAppState,
        sidebar: SharedSidebar,
    ) {
        connection_dialogs::show_new_connection_dialog(window.upcast_ref(), state, sidebar);
    }

    /// Shows the Connection Wizard (simplified step-by-step flow)
    fn show_connection_wizard(
        window: &adw::ApplicationWindow,
        state: SharedAppState,
        sidebar: SharedSidebar,
        toast_overlay: SharedToastOverlay,
    ) {
        use crate::dialogs::connection_wizard::{ConnectionWizard, WizardResult};

        let wizard = ConnectionWizard::new(state.clone());

        let state_for_cb = state.clone();
        let sidebar_for_cb = sidebar.clone();
        let window_weak = window.downgrade();
        let toast_for_cb = toast_overlay;
        wizard.connect_complete(move |result| {
            match result {
                WizardResult::Save(conn, password) => {
                    let conn_name = conn.name.clone();
                    let conn_host = conn.host.clone();
                    let conn_username = conn.username.clone();
                    let password_source = conn.password_source.clone();
                    let protocol = conn.protocol;
                    if let Ok(mut state_mut) = state_for_cb.try_borrow_mut()
                        && let Ok(conn_id) = state_mut.create_connection(conn)
                    {
                        Self::persist_wizard_password(
                            &state_mut,
                            conn_id,
                            &password_source,
                            password.as_ref(),
                            &conn_name,
                            &conn_host,
                            conn_username.as_deref().unwrap_or(""),
                            protocol,
                        );
                        drop(state_mut);
                        let state_c = state_for_cb.clone();
                        let sidebar_c = sidebar_for_cb.clone();
                        let toast_c = toast_for_cb.clone();
                        let name = conn_name;
                        glib::idle_add_local_once(move || {
                            Self::reload_sidebar_preserving_state(&state_c, &sidebar_c);
                            toast_c.show_success(&crate::i18n::i18n_f(
                                "Connection “{}” created",
                                &[&name],
                            ));
                        });
                    }
                }
                WizardResult::SaveAndConnect(conn, password) => {
                    let conn_id_outer = conn.id;
                    let conn_name = conn.name.clone();
                    let conn_host = conn.host.clone();
                    let conn_username = conn.username.clone();
                    let password_source = conn.password_source.clone();
                    let protocol = conn.protocol;
                    if let Ok(mut state_mut) = state_for_cb.try_borrow_mut()
                        && let Ok(conn_id) = state_mut.create_connection(conn)
                    {
                        Self::persist_wizard_password(
                            &state_mut,
                            conn_id,
                            &password_source,
                            password.as_ref(),
                            &conn_name,
                            &conn_host,
                            conn_username.as_deref().unwrap_or(""),
                            protocol,
                        );
                        drop(state_mut);
                        let state_c = state_for_cb.clone();
                        let sidebar_c = sidebar_for_cb.clone();
                        let window_w = window_weak.clone();
                        glib::idle_add_local_once(move || {
                            Self::reload_sidebar_preserving_state(&state_c, &sidebar_c);
                            // Connect after sidebar refresh
                            if let Some(win) = window_w.upgrade() {
                                let variant = conn_id_outer.to_string().to_variant();
                                gio::prelude::ActionGroupExt::activate_action(
                                    &win,
                                    "connect-to",
                                    Some(&variant),
                                );
                            }
                        });
                    }
                }
                WizardResult::OpenAdvanced(partial) => {
                    // Open full dialog pre-filled with wizard data
                    if let Some(win) = window_weak.upgrade() {
                        let password = partial.storable_password();
                        let conn = partial.to_connection();
                        connection_dialogs::show_new_connection_dialog_prefilled(
                            win.upcast_ref(),
                            state_for_cb.clone(),
                            sidebar_for_cb.clone(),
                            conn,
                            password,
                        );
                    }
                }
            }
        });

        wizard.present(window);
    }

    /// Persists a wizard-entered password to the configured vault.
    ///
    /// No-op unless `password_source` is `Vault` and a password is present.
    /// Mirrors the full connection dialog so connections created via the
    /// wizard resolve their credentials correctly (issue #188).
    #[expect(
        clippy::too_many_arguments,
        reason = "mirrors save_password_to_vault parameters 1:1; a struct would only restate them"
    )]
    fn persist_wizard_password(
        state_mut: &crate::state::AppState,
        conn_id: uuid::Uuid,
        password_source: &rustconn_core::models::PasswordSource,
        password: Option<&secrecy::SecretString>,
        conn_name: &str,
        conn_host: &str,
        username: &str,
        protocol: rustconn_core::models::ProtocolType,
    ) {
        if *password_source != rustconn_core::models::PasswordSource::Vault {
            return;
        }
        let Some(pwd) = password else {
            return;
        };
        let settings = state_mut.settings().clone();
        let groups: Vec<_> = state_mut.list_groups().into_iter().cloned().collect();
        let conn_for_path = state_mut.get_connection(conn_id).cloned();
        crate::state::save_password_to_vault(
            &settings,
            &groups,
            conn_for_path.as_ref(),
            conn_name,
            conn_host,
            protocol,
            username,
            pwd,
            conn_id,
        );
    }

    /// Opens the Connection Wizard pre-filled with data from the selected connection.
    ///
    /// Used for "Duplicate via Wizard…" — allows modifying fields before saving as new.
    fn duplicate_via_wizard(
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
        toast_overlay: &SharedToastOverlay,
    ) {
        use crate::dialogs::connection_wizard::{
            ConnectionWizard, PartialConnection, WizardResult,
        };

        let Some(selected) = sidebar.get_selected_item() else {
            return;
        };
        if selected.is_group() {
            return;
        }

        let id_str = selected.id();
        let Ok(id) = uuid::Uuid::parse_str(&id_str) else {
            return;
        };

        let conn = {
            let Ok(state_ref) = state.try_borrow() else {
                return;
            };
            let Some(c) = state_ref.get_connection(id).cloned() else {
                return;
            };
            c
        };

        let partial = PartialConnection::from_connection(&conn);
        let wizard = ConnectionWizard::new(state.clone());
        wizard.set_partial(&partial);

        let state_for_cb = state.clone();
        let sidebar_for_cb = sidebar.clone();
        let window_weak = window.downgrade();
        let toast_for_cb = toast_overlay.clone();
        wizard.connect_complete(move |result| match result {
            WizardResult::Save(new_conn, password) => {
                let conn_name = new_conn.name.clone();
                let conn_host = new_conn.host.clone();
                let conn_username = new_conn.username.clone();
                let password_source = new_conn.password_source.clone();
                let protocol = new_conn.protocol;
                if let Ok(mut state_mut) = state_for_cb.try_borrow_mut()
                    && let Ok(conn_id) = state_mut.create_connection(new_conn)
                {
                    Self::persist_wizard_password(
                        &state_mut,
                        conn_id,
                        &password_source,
                        password.as_ref(),
                        &conn_name,
                        &conn_host,
                        conn_username.as_deref().unwrap_or(""),
                        protocol,
                    );
                    drop(state_mut);
                    let state_c = state_for_cb.clone();
                    let sidebar_c = sidebar_for_cb.clone();
                    let toast_c = toast_for_cb.clone();
                    glib::idle_add_local_once(move || {
                        Self::reload_sidebar_preserving_state(&state_c, &sidebar_c);
                        toast_c.show_success(&crate::i18n::i18n_f(
                            "Connection “{}” created",
                            &[&conn_name],
                        ));
                    });
                }
            }
            WizardResult::SaveAndConnect(new_conn, password) => {
                let conn_id_outer = new_conn.id;
                let conn_name = new_conn.name.clone();
                let conn_host = new_conn.host.clone();
                let conn_username = new_conn.username.clone();
                let password_source = new_conn.password_source.clone();
                let protocol = new_conn.protocol;
                if let Ok(mut state_mut) = state_for_cb.try_borrow_mut()
                    && let Ok(conn_id) = state_mut.create_connection(new_conn)
                {
                    Self::persist_wizard_password(
                        &state_mut,
                        conn_id,
                        &password_source,
                        password.as_ref(),
                        &conn_name,
                        &conn_host,
                        conn_username.as_deref().unwrap_or(""),
                        protocol,
                    );
                    drop(state_mut);
                    let state_c = state_for_cb.clone();
                    let sidebar_c = sidebar_for_cb.clone();
                    let window_w = window_weak.clone();
                    glib::idle_add_local_once(move || {
                        Self::reload_sidebar_preserving_state(&state_c, &sidebar_c);
                        if let Some(win) = window_w.upgrade() {
                            let variant = conn_id_outer.to_string().to_variant();
                            gio::prelude::ActionGroupExt::activate_action(
                                &win,
                                "connect-to",
                                Some(&variant),
                            );
                        }
                    });
                }
            }
            WizardResult::OpenAdvanced(new_partial) => {
                if let Some(win) = window_weak.upgrade() {
                    let password = new_partial.storable_password();
                    let new_conn = new_partial.to_connection();
                    connection_dialogs::show_new_connection_dialog_prefilled(
                        win.upcast_ref(),
                        state_for_cb.clone(),
                        sidebar_for_cb.clone(),
                        new_conn,
                        password,
                    );
                }
            }
        });

        wizard.present(window);
        let _ = partial; // Will be used when wizard supports pre-fill from partial
    }

    /// Shows the new group dialog with optional parent selection
    fn show_new_group_dialog(
        window: &adw::ApplicationWindow,
        state: SharedAppState,
        sidebar: SharedSidebar,
    ) {
        connection_dialogs::show_new_group_dialog(window.upcast_ref(), state, sidebar);
    }

    /// Shows the command palette dialog
    fn show_command_palette(
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
        notebook: &SharedNotebook,
        monitoring: &types::SharedMonitoring,
        prefix: &str,
    ) {
        let gtk_window: &gtk4::Window = window.upcast_ref();
        let palette = crate::dialogs::CommandPaletteDialog::new(Some(gtk_window));

        // Populate with current connections and groups
        {
            let state_ref = state.borrow();
            let connections: Vec<_> = state_ref.list_connections().into_iter().cloned().collect();
            let groups: Vec<_> = state_ref.get_root_groups().into_iter().cloned().collect();
            palette.set_connections(connections);
            palette.set_groups(groups);
        }

        // Populate open tabs for % mode
        {
            let open_tabs: Vec<crate::dialogs::OpenTabInfo> = notebook
                .get_all_sessions()
                .into_iter()
                .map(|s| crate::dialogs::OpenTabInfo {
                    session_id: s.id,
                    title: s.name,
                    protocol: s.protocol,
                    group: s.tab_group,
                })
                .collect();
            palette.set_open_tabs(open_tabs);
        }

        // Wire action callback
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let notebook_clone = notebook.clone();
        let monitoring_clone = monitoring.clone();
        let window_weak = window.downgrade();
        palette.connect_on_action(move |action| match action {
            rustconn_core::search::command_palette::CommandPaletteAction::Connect(uuid) => {
                Self::start_connection(
                    &state_clone,
                    &notebook_clone,
                    &sidebar_clone,
                    &monitoring_clone,
                    uuid,
                );
            }
            rustconn_core::search::command_palette::CommandPaletteAction::SwitchTab(session_id) => {
                notebook_clone.switch_to_tab(session_id);
            }
            rustconn_core::search::command_palette::CommandPaletteAction::GtkAction(name) => {
                if let Some(win) = window_weak.upgrade() {
                    gio::ActionGroup::activate_action(
                        win.upcast_ref::<gio::ActionGroup>(),
                        &name,
                        None,
                    );
                }
            }
            other => {
                let action_name = match other {
                    rustconn_core::search::command_palette::CommandPaletteAction::OpenSettings => {
                        "settings"
                    }
                    rustconn_core::search::command_palette::CommandPaletteAction::NewConnection => {
                        "new-connection"
                    }
                    rustconn_core::search::command_palette::CommandPaletteAction::NewGroup => {
                        "new-group"
                    }
                    rustconn_core::search::command_palette::CommandPaletteAction::Import => {
                        "import"
                    }
                    rustconn_core::search::command_palette::CommandPaletteAction::Export => {
                        "export"
                    }
                    rustconn_core::search::command_palette::CommandPaletteAction::LocalShell => {
                        "local-shell"
                    }
                    rustconn_core::search::command_palette::CommandPaletteAction::QuickConnect => {
                        "quick-connect"
                    }
                    _ => return,
                };
                if let Some(win) = window_weak.upgrade() {
                    gio::ActionGroup::activate_action(
                        win.upcast_ref::<gio::ActionGroup>(),
                        action_name,
                        None,
                    );
                }
            }
        });

        palette.present_with_prefix(prefix);
    }

    /// Shows the import dialog
    fn show_import_dialog(
        window: &adw::ApplicationWindow,
        state: SharedAppState,
        sidebar: SharedSidebar,
    ) {
        connection_dialogs::show_import_dialog(window.upcast_ref(), state, sidebar);
    }

    /// Shows the settings dialog
    /// Updates the window title to reflect the active connection (issue #211).
    ///
    /// When `enabled` and a session tab is active, sets the title to
    /// `"RustConn - <active tab>"`; otherwise resets it to `"RustConn"`. Driven
    /// by the `ui.window_title_shows_connection` setting so time-tracking tools
    /// can attribute usage per connection by reading the window title.
    pub(crate) fn update_window_title(
        window: &adw::ApplicationWindow,
        notebook: &SharedNotebook,
        enabled: bool,
    ) {
        let name = if enabled {
            notebook
                .tab_view()
                .selected_page()
                .map(|page| page.title().to_string())
                .filter(|title| !title.is_empty() && *title != crate::i18n::i18n("Welcome"))
        } else {
            None
        };
        match name {
            Some(name) => window.set_title(Some(&crate::i18n::i18n_f("RustConn - {}", &[&name]))),
            None => window.set_title(Some("RustConn")),
        }
    }

    fn show_settings_dialog(
        window: &adw::ApplicationWindow,
        state: SharedAppState,
        notebook: SharedNotebook,
        monitoring: Rc<crate::monitoring::MonitoringCoordinator>,
        sidebar: SharedSidebar,
        overlay_split_view: adw::OverlaySplitView,
    ) {
        let opened_at = std::time::Instant::now();
        tracing::debug!("settings action activated");
        let mut dialog = SettingsDialog::new(None);

        // Load current settings and connections
        {
            let state_ref = state.borrow();
            dialog.set_settings(state_ref.settings().clone());
            let connections: Vec<_> = state_ref.list_connections().into_iter().cloned().collect();
            dialog.set_connections(connections);

            // Populate Cloud Sync sections
            let groups: Vec<_> = state_ref.list_groups().into_iter().cloned().collect();
            dialog.populate_cloud_sync(&groups, state_ref.sync_manager(), &state);
        }
        tracing::debug!(
            elapsed_ms = opened_at.elapsed().as_millis() as u64,
            "settings dialog constructed and populated"
        );

        let window_clone = window.clone();
        dialog.run(Some(window), move |result| {
            if let Some(settings) = result {
                // Capture backend and KeePass state for action update
                let backend = settings.secrets.preferred_backend;
                let keepass_enabled = settings.secrets.kdbx_enabled;
                let kdbx_path_exists = settings
                    .secrets
                    .kdbx_path
                    .as_ref()
                    .is_some_and(|p: &std::path::PathBuf| p.exists());

                // Apply terminal settings to existing terminals
                notebook.apply_settings(&settings.terminal);

                // Re-apply per-connection theme overrides that were wiped
                // by the global theme application above (fixes #99)
                {
                    let state_ref = state.borrow();
                    notebook.reapply_theme_overrides(
                        &settings.terminal.color_theme,
                        |connection_id| {
                            state_ref
                                .get_connection(connection_id)
                                .and_then(|c| c.theme_override.clone())
                        },
                    );
                }

                // Apply protocol tab coloring setting
                notebook.set_color_tabs_by_protocol(settings.ui.color_tabs_by_protocol);

                // Apply protocol filter visibility setting
                sidebar.set_filter_visible(settings.ui.show_protocol_filters);

                // Apply smart folders visibility setting
                sidebar.set_smart_folders_visible(settings.ui.show_smart_folders);

                // Apply sidebar width setting
                if let Some(w) = settings.ui.sidebar_width {
                    let width = f64::from(w.clamp(180, 500));
                    overlay_split_view.set_max_sidebar_width(width);
                }

                // Apply monitoring settings to active bars
                monitoring.apply_settings_to_all(&settings.monitoring);

                // Apply window-title-shows-connection setting live (issue #211)
                Self::update_window_title(
                    &window_clone,
                    &notebook,
                    settings.ui.window_title_shows_connection,
                );

                if let Ok(mut state_mut) = state.try_borrow_mut() {
                    let simple_sync_was = state_mut.simple_sync_enabled();
                    let simple_sync_now = settings.sync.simple_sync_enabled;
                    if let Err(e) = state_mut.update_settings(settings) {
                        tracing::error!(%e, "Failed to save settings");
                    } else {
                        // Simple Sync just turned on: publish the current store,
                        // then pull any existing remote data on idle so the
                        // device converges immediately.
                        if simple_sync_now && !simple_sync_was {
                            if let Err(e) = state_mut.simple_sync_export() {
                                tracing::warn!(%e, "Initial Simple Sync export failed");
                            }
                            let state_idle = state.clone();
                            let sidebar_idle = sidebar.clone();
                            glib::idle_add_local_once(move || {
                                let outcome = state_idle.try_borrow_mut().ok().and_then(|mut s| {
                                    let device_id = s.settings().sync.device_id;
                                    s.sync_manager()
                                        .should_import_simple_sync(device_id)
                                        .then(|| s.simple_sync_import_and_apply())
                                });
                                if let Some(Ok(report)) = outcome
                                    && !report.is_empty()
                                {
                                    Self::reload_sidebar_preserving_state(
                                        &state_idle,
                                        &sidebar_idle,
                                    );
                                }
                            });
                        }
                        // Update open-keepass action enabled state based on backend
                        if let Some(action) = window_clone.lookup_action("open-keepass")
                            && let Some(simple_action) = action.downcast_ref::<gio::SimpleAction>()
                        {
                            let action_enabled = match backend {
                                rustconn_core::config::SecretBackendType::LibSecret
                                | rustconn_core::config::SecretBackendType::MacOsKeychain
                                | rustconn_core::config::SecretBackendType::Bitwarden
                                | rustconn_core::config::SecretBackendType::OnePassword
                                | rustconn_core::config::SecretBackendType::Passbolt
                                | rustconn_core::config::SecretBackendType::Pass
                                | rustconn_core::config::SecretBackendType::EncryptedFile => true,
                                rustconn_core::config::SecretBackendType::KeePassXc
                                | rustconn_core::config::SecretBackendType::KdbxFile => {
                                    keepass_enabled && kdbx_path_exists
                                }
                            };
                            simple_action.set_enabled(action_enabled);
                        }
                    }
                } else {
                    tracing::error!("Failed to borrow state for settings update");
                }
            }
        });
    }

    /// Edits the selected connection or group
    fn edit_selected_connection(
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) {
        edit_dialogs::edit_selected_connection(window.upcast_ref(), state, sidebar);
    }

    /// Renames the selected connection or group
    fn rename_selected_item(
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) {
        edit_dialogs::rename_selected_item(window.upcast_ref(), state, sidebar);
    }

    /// Deletes the selected connection or group
    fn delete_selected_connection(
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) {
        operations::delete_selected_connection(window.upcast_ref(), state, sidebar);
    }

    /// Deletes all selected connections (bulk delete for group operations mode)
    fn delete_selected_connections(
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) {
        operations::delete_selected_connections(window.upcast_ref(), state, sidebar);
    }

    /// Shows dialog to move selected items to a group
    fn show_move_selected_to_group_dialog(
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) {
        operations::show_move_selected_to_group_dialog(window.upcast_ref(), state, sidebar);
    }

    /// Duplicates the selected connection
    fn duplicate_selected_connection(
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) {
        operations::duplicate_selected_connection(window.upcast_ref(), state, sidebar);
    }

    /// Toggles pin state of the selected connection
    fn toggle_pin_selected(state: &SharedAppState, sidebar: &SharedSidebar) {
        operations::toggle_pin_selected(state, sidebar);
    }

    /// Copies the selected connection to the internal clipboard
    fn copy_selected_connection(
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) {
        operations::copy_selected_connection(window.upcast_ref(), state, sidebar);
    }

    /// Pastes a connection from the internal clipboard
    fn paste_connection(
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) {
        operations::paste_connection(window.upcast_ref(), state, sidebar);
    }

    /// Reloads the sidebar with current data (preserving hierarchy)
    fn reload_sidebar(state: &SharedAppState, sidebar: &SharedSidebar) {
        sorting::rebuild_sidebar_sorted(state, sidebar);
    }

    /// Reloads the sidebar while preserving tree state
    ///
    /// This method saves the current expanded groups, scroll position, and selection,
    /// reloads the sidebar data, and then restores the state. Use this when editing
    /// connections to maintain the user's view.
    pub fn reload_sidebar_preserving_state(state: &SharedAppState, sidebar: &SharedSidebar) {
        // Save current tree state
        let tree_state = sidebar.save_state();

        // Perform the reload
        Self::reload_sidebar(state, sidebar);

        // Restore tree state
        sidebar.restore_state(&tree_state);
    }

    /// Presents the window to the user
    pub fn present(&self) {
        self.window.present();
    }

    /// Returns a reference to the underlying GTK window
    /// Builds the "close with open sessions" confirmation dialog.
    ///
    /// Shared by the window close button (`close_request`) and the
    /// `app.quit` action so both paths protect active connections from
    /// an accidental Ctrl+Q / window close (GNOME HIG).
    #[must_use]
    pub fn close_confirmation_dialog(open_sessions: usize) -> adw::AlertDialog {
        let dialog = adw::AlertDialog::new(
            Some(&crate::i18n::i18n("Close RustConn?")),
            Some(&crate::i18n::i18n_f(
                "{} session tab(s) are open. Active connections will be disconnected.",
                &[&open_sessions.to_string()],
            )),
        );
        dialog.add_response("cancel", &crate::i18n::i18n("Cancel"));
        dialog.add_response("close", &crate::i18n::i18n("Close"));
        dialog.set_response_appearance("close", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        dialog
    }

    /// Returns a shared handle to the terminal notebook.
    #[must_use]
    pub fn notebook_rc(&self) -> SharedNotebook {
        self.terminal_notebook.clone()
    }

    /// Returns the cloud-sync error banner shown below the header bar.
    #[must_use]
    pub fn sync_banner(&self) -> &adw::Banner {
        &self.sync_banner
    }

    /// Returns the secret-backend availability banner shown below the header
    /// bar. Revealed by `check_secret_backend_available` when the preferred
    /// backend cannot store passwords (#201).
    #[must_use]
    pub fn secret_banner(&self) -> &adw::Banner {
        &self.secret_banner
    }

    /// Shows the persistent cloud-sync failure banner.
    pub fn show_sync_error_banner(banner: &adw::Banner, group_name: &str, error: &str) {
        banner.set_title(&crate::i18n::i18n_f(
            "Cloud sync failed for '{}': {}",
            &[group_name, error],
        ));
        banner.set_revealed(true);
    }

    #[must_use]
    pub const fn gtk_window(&self) -> &adw::ApplicationWindow {
        &self.window
    }

    /// Finds a header-bar button by its `win.*` action name, if present.
    ///
    /// Used by the adaptive breakpoints (#204) to shed non-essential buttons on
    /// narrow widths. `AdwHeaderBar` nests packed widgets inside internal boxes,
    /// so `observe_children()` does not expose them — this walks the whole
    /// descendant tree instead. Window controls and the title carry no matching
    /// `win.*` action, so they are skipped naturally.
    fn header_button(header_bar: &adw::HeaderBar, action: &str) -> Option<gtk4::Button> {
        let mut child = header_bar.first_child();
        while let Some(widget) = child {
            if let Some(found) = Self::button_with_action(&widget, action) {
                return Some(found);
            }
            child = widget.next_sibling();
        }
        None
    }

    /// Recursively searches `widget` and its descendants for a `gtk4::Button`
    /// whose action name equals `action`.
    fn button_with_action(widget: &gtk4::Widget, action: &str) -> Option<gtk4::Button> {
        if let Some(btn) = widget.downcast_ref::<gtk4::Button>()
            && btn.action_name().is_some_and(|a| a == action)
        {
            return Some(btn.clone());
        }
        let mut child = widget.first_child();
        while let Some(descendant) = child {
            if let Some(found) = Self::button_with_action(&descendant, action) {
                return Some(found);
            }
            child = descendant.next_sibling();
        }
        None
    }

    /// Registers the application icon in the icon theme
    fn register_app_icon() {
        if let Some(display) = gtk4::gdk::Display::default() {
            let icon_theme = gtk4::IconTheme::for_display(&display);

            // Add multiple icon search paths for different installation scenarios
            // 1. Development path (cargo run)
            let dev_icons_path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons");
            icon_theme.add_search_path(dev_icons_path);

            // 2. System installation paths
            let system_paths = [
                "/usr/share/icons",
                "/usr/local/share/icons",
                "/app/share/icons", // Flatpak
            ];
            for path in &system_paths {
                if std::path::Path::new(path).exists() {
                    icon_theme.add_search_path(path);
                }
            }

            // 3. User local installation path
            if let Some(data_dir) = dirs::data_dir() {
                let user_icons = data_dir.join("icons");
                if user_icons.exists() {
                    icon_theme.add_search_path(user_icons.to_string_lossy().as_ref());
                }
            }

            // 4. macOS .app bundle: icons bundled inside Resources/share/icons
            #[cfg(target_os = "macos")]
            {
                if let Ok(exe_path) = std::env::current_exe() {
                    // exe is at RustConn.app/Contents/MacOS/rustconn
                    // icons are at RustConn.app/Contents/Resources/share/icons
                    if let Some(macos_dir) = exe_path.parent() {
                        let bundle_icons = macos_dir
                            .parent() // Contents/
                            .map(|p| p.join("Resources/share/icons"));
                        if let Some(ref icons_path) = bundle_icons
                            && icons_path.exists()
                        {
                            icon_theme.add_search_path(icons_path.to_string_lossy().as_ref());
                        }
                    }
                }

                // Also add Homebrew icon paths (for non-bundled runs)
                let homebrew_icons = ["/opt/homebrew/share/icons", "/usr/local/share/icons"];
                for path in &homebrew_icons {
                    if std::path::Path::new(path).exists() {
                        icon_theme.add_search_path(path);
                    }
                }
            }
        }
    }

    /// Returns a reference to the connection sidebar
    ///
    /// Note: Part of public API for accessing sidebar from external code.
    #[must_use]
    #[expect(
        dead_code,
        reason = "kept alive for GTK widget lifecycle / future API exposure"
    )]
    pub fn sidebar(&self) -> &ConnectionSidebar {
        &self.sidebar
    }

    /// Returns a clone of the shared sidebar Rc
    #[must_use]
    pub fn sidebar_rc(&self) -> Rc<ConnectionSidebar> {
        self.sidebar.clone()
    }

    /// Executes a startup action (open local shell or connect to a saved connection)
    ///
    /// Called from `build_ui` after the window is presented. CLI args override
    /// the persisted setting.
    pub fn execute_startup_action(&self, action: &rustconn_core::config::StartupAction) {
        use rustconn_core::config::StartupAction;
        match action {
            StartupAction::None => {}
            StartupAction::LocalShell => {
                tracing::info!("Startup action: opening local shell");
                Self::open_local_shell_with_split(
                    &self.terminal_notebook,
                    &self.split_view,
                    Some(&self.state),
                );
            }
            StartupAction::Connection(id) => {
                // Verify the connection exists before attempting to connect
                let exists = self
                    .state
                    .try_borrow()
                    .ok()
                    .and_then(|s| s.get_connection(*id).cloned())
                    .is_some();
                if exists {
                    tracing::info!(%id, "Startup action: connecting to saved connection");
                    Self::start_connection_with_split(
                        &self.state,
                        &self.terminal_notebook,
                        &self.split_view,
                        &self.sidebar,
                        &self.monitoring,
                        *id,
                        Some(&self.activity_coordinator),
                    );
                } else {
                    tracing::warn!(%id, "Startup action: connection not found, skipping");
                    self.toast_overlay
                        .show_warning(&crate::i18n::i18n("Startup connection not found"));
                }
            }
            StartupAction::RdpFile(path) => {
                tracing::info!(path = %path.display(), "Startup action: opening .rdp file");
                match rustconn_core::import::RdpFileImporter::parse_rdp_file(path) {
                    Ok(connection) => {
                        // Add the imported connection to state and connect
                        let conn_id = connection.id;
                        if let Ok(mut state_mut) = self.state.try_borrow_mut()
                            && let Err(e) = state_mut.create_connection(connection)
                        {
                            tracing::error!(%e, "Failed to add imported .rdp connection");
                        }
                        Self::start_connection_with_split(
                            &self.state,
                            &self.terminal_notebook,
                            &self.split_view,
                            &self.sidebar,
                            &self.monitoring,
                            conn_id,
                            Some(&self.activity_coordinator),
                        );
                        let state_clone = self.state.clone();
                        let sidebar_clone = Rc::clone(&self.sidebar);
                        glib::idle_add_local_once(move || {
                            Self::reload_sidebar_preserving_state(&state_clone, &sidebar_clone);
                        });
                    }
                    Err(e) => {
                        tracing::error!(
                            ?e,
                            path = %path.display(),
                            "Failed to parse .rdp file"
                        );
                        self.toast_overlay
                            .show_warning(&crate::i18n::i18n("Failed to open .rdp file"));
                    }
                }
            }
            StartupAction::VvFile(path) => {
                tracing::info!(path = %path.display(), "Startup action: opening .vv file");
                match rustconn_core::import::VirtViewerImporter::parse_vv_file(path) {
                    Ok(connection) => {
                        let conn_id = connection.id;
                        if let Ok(mut state_mut) = self.state.try_borrow_mut()
                            && let Err(e) = state_mut.create_connection(connection)
                        {
                            tracing::error!(%e, "Failed to add imported .vv connection");
                        }
                        Self::start_connection_with_split(
                            &self.state,
                            &self.terminal_notebook,
                            &self.split_view,
                            &self.sidebar,
                            &self.monitoring,
                            conn_id,
                            Some(&self.activity_coordinator),
                        );
                        let state_clone = self.state.clone();
                        let sidebar_clone = Rc::clone(&self.sidebar);
                        glib::idle_add_local_once(move || {
                            Self::reload_sidebar_preserving_state(&state_clone, &sidebar_clone);
                        });
                    }
                    Err(e) => {
                        tracing::error!(
                            ?e,
                            path = %path.display(),
                            "Failed to parse .vv file"
                        );
                        self.toast_overlay
                            .show_warning(&crate::i18n::i18n("Failed to open .vv file"));
                    }
                }
            }
        }

        // Auto-start standalone tunnels (runs regardless of startup action)
        Self::auto_start_tunnels(&self.state, &self.tunnel_manager);

        // Health check polling for standalone tunnels (every 5 seconds)
        {
            let tm = self.tunnel_manager.clone();
            let state_c = self.state.clone();
            glib::timeout_add_local(std::time::Duration::from_secs(5), move || {
                let failed = tm.borrow_mut().health_check();
                if !failed.is_empty() {
                    // Auto-reconnect failed tunnels
                    let tunnels = state_c.borrow().settings().standalone_tunnels.clone();
                    let connections: Vec<_> = state_c
                        .borrow()
                        .list_connections()
                        .into_iter()
                        .cloned()
                        .collect();
                    for id in &failed {
                        if let Some(tunnel) = tunnels.iter().find(|t| t.id == *id)
                            && tunnel.auto_reconnect
                            && tunnel.enabled
                        {
                            // Check if tunnel exceeded max reconnect attempts
                            if tm.borrow().exceeded_max_reconnects(*id) {
                                tracing::warn!(
                                    tunnel = %tunnel.name,
                                    tunnel_id = %id,
                                    "Tunnel exceeded max reconnect attempts, giving up"
                                );
                                continue;
                            }

                            if let Some(conn) =
                                connections.iter().find(|c| c.id == tunnel.connection_id)
                            {
                                tracing::info!(tunnel = %tunnel.name, "Auto-reconnecting failed tunnel");
                                // Resolve cached password for reconnection
                                let cached_pw: Option<secrecy::SecretString> = state_c
                                    .try_borrow()
                                    .ok()
                                    .and_then(|s| {
                                        s.get_cached_credentials(tunnel.connection_id).cloned()
                                    })
                                    .and_then(|c| {
                                        use secrecy::ExposeSecret;
                                        if c.password.expose_secret().is_empty() {
                                            None
                                        } else {
                                            Some(c.password.clone())
                                        }
                                    });
                                let _ =
                                    tm.borrow_mut().start(tunnel, conn, cached_pw.as_ref(), &[]);
                            }
                        }
                    }
                }
                glib::ControlFlow::Continue
            });
        }
    }

    /// Returns a reference to the terminal notebook
    ///
    /// Note: Part of public API for accessing notebook from external code.
    #[must_use]
    #[expect(
        dead_code,
        reason = "kept alive for GTK widget lifecycle / future API exposure"
    )]
    pub fn terminal_notebook(&self) -> &TerminalNotebook {
        &self.terminal_notebook
    }

    /// Auto-starts standalone tunnels that have `auto_start` and `enabled` set
    fn auto_start_tunnels(state: &SharedAppState, tunnel_manager: &SharedTunnelManager) {
        let tunnels = state.borrow().settings().standalone_tunnels.clone();
        let auto_tunnels: Vec<_> = tunnels
            .iter()
            .filter(|t| t.auto_start && t.enabled)
            .collect();

        if auto_tunnels.is_empty() {
            return;
        }

        tracing::info!(
            count = auto_tunnels.len(),
            "Auto-starting standalone tunnels"
        );

        let connections: Vec<_> = state
            .borrow()
            .list_connections()
            .into_iter()
            .cloned()
            .collect();

        for tunnel in auto_tunnels {
            let conn = connections.iter().find(|c| c.id == tunnel.connection_id);
            if let Some(conn) = conn {
                // Resolve cached password for the connection
                let cached_pw: Option<secrecy::SecretString> = state
                    .try_borrow()
                    .ok()
                    .and_then(|s| s.get_cached_credentials(tunnel.connection_id).cloned())
                    .and_then(|c| {
                        use secrecy::ExposeSecret;
                        if c.password.expose_secret().is_empty() {
                            None
                        } else {
                            Some(c.password.clone())
                        }
                    });
                if let Err(e) =
                    tunnel_manager
                        .borrow_mut()
                        .start(tunnel, conn, cached_pw.as_ref(), &[])
                {
                    tracing::warn!(tunnel = %tunnel.name, %e, "Failed to auto-start tunnel");
                }
            } else {
                tracing::warn!(
                    tunnel = %tunnel.name,
                    connection_id = %tunnel.connection_id,
                    "SSH connection not found for auto-start tunnel"
                );
            }
        }
    }

    /// Saves the current expanded groups state to settings
    ///
    /// Note: Part of tree state persistence API.
    #[expect(
        dead_code,
        reason = "kept alive for GTK widget lifecycle / future API exposure"
    )]
    pub fn save_expanded_groups(&self) {
        let expanded = self.sidebar.get_expanded_groups();
        if let Some(Err(e)) =
            try_with_state_mut(&self.state, |state| state.update_expanded_groups(expanded))
        {
            tracing::warn!(?e, "Failed to update expanded groups");
        }
    }

    /// Opens a local shell terminal with split view integration
    fn open_local_shell_with_split(
        notebook: &SharedNotebook,
        split_view: &SharedSplitView,
        state: Option<&SharedAppState>,
    ) {
        // Get terminal settings from state if available
        let terminal_settings = state
            .and_then(|s| s.try_borrow().ok())
            .map(|s| s.settings().terminal.clone())
            .unwrap_or_default();

        let session_id = notebook.create_terminal_tab_with_settings(
            Uuid::nil(),
            "Local Shell",
            "local",
            None,
            &terminal_settings,
            None,
            &[],
        );

        // Get user's default shell
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());

        // Check if a custom local shell command is configured
        let custom_command = if terminal_settings.local_shell_command.is_empty() {
            None
        } else {
            Some(terminal_settings.local_shell_command.clone())
        };

        // In Flatpak, spawn the shell on the host via flatpak-spawn so the
        // user gets their full system shell with all tools and dotfiles (#122).
        //
        // VTE allocates a PTY for the child process. We need the host shell
        // to inherit this PTY. `flatpak-spawn --host` with the Development
        // interface forwards stdin/stdout/stderr (including the PTY fd) to
        // the host process, but the shell must be told it's a login shell.
        //
        // $SHELL inside the sandbox is /bin/sh, not the user's host shell.
        // Query the host $SHELL first, then exec into it.
        if rustconn_core::flatpak::is_flatpak() {
            let host_shell = std::process::Command::new("flatpak-spawn")
                .args(["--host", "sh", "-c", "echo $SHELL"])
                .output()
                .ok()
                .and_then(|out| {
                    if out.status.success() {
                        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                        if s.is_empty() { None } else { Some(s) }
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| "/bin/bash".to_string());

            // If a custom command is set, run it via the host shell.
            // Otherwise use the default login shell behavior.
            let spawn_cmd = if let Some(ref cmd) = custom_command {
                let escaped_cmd = cmd.replace('\'', "'\\''");
                format!(
                    "flatpak-spawn --host --env=TERM=xterm-256color -- script -qfc '{host_shell} -c '\"'\"'{escaped_cmd}'\"'\"'' /dev/null"
                )
            } else {
                format!(
                    "flatpak-spawn --host --env=TERM=xterm-256color -- script -qfc '{host_shell} --login' /dev/null"
                )
            };
            notebook.spawn_command(session_id, &["/bin/sh", "-c", &spawn_cmd], None, None, None);

            // Wire up PTY resize propagation for Flatpak host shell (#122).
            //
            // VTE automatically resizes its own PTY (sandbox-side), but the
            // host-side PTY created by `script` never receives TIOCSWINSZ.
            // On each VTE char-size-changed, forward the new dimensions to the
            // host via `flatpak-spawn --host -- stty rows R cols C`.
            //
            // Debounced: only the last resize in a 200ms window is sent to
            // avoid spawning dozens of threads during rapid window dragging.
            if let Some(terminal) = notebook.get_terminal(session_id) {
                use vte4::prelude::*;
                let last_resize = std::sync::Arc::new(std::sync::Mutex::new(
                    std::time::Instant::now()
                        .checked_sub(std::time::Duration::from_secs(1))
                        .unwrap_or_else(std::time::Instant::now),
                ));
                terminal.connect_char_size_changed(move |term, _width, _height| {
                    let rows = term.row_count();
                    let cols = term.column_count();
                    let last = last_resize.clone();
                    // Debounce: skip if last resize was less than 200ms ago
                    // (the spawned thread will use the latest values).
                    let mut guard = last.lock().unwrap_or_else(|e| e.into_inner());
                    if guard.elapsed() < std::time::Duration::from_millis(200) {
                        return;
                    }
                    *guard = std::time::Instant::now();
                    drop(guard);

                    // Spawn a background process to resize the host PTY.
                    // `stty` on the host sets the PTY dimensions and the kernel
                    // delivers SIGWINCH to the foreground process group.
                    let cmd = format!("flatpak-spawn --host -- stty rows {rows} cols {cols}");
                    std::thread::spawn(move || {
                        let _ = std::process::Command::new("sh").args(["-c", &cmd]).output();
                    });
                });
            }
        } else if let Some(ref cmd) = custom_command {
            // Custom command: run via user's shell with -c
            notebook.spawn_command(session_id, &[&shell, "-c", cmd], None, None, None);
        } else {
            // On macOS, launch as login shell so .zprofile/.zshrc are sourced
            // and the shell gets a proper controlling terminal.
            #[cfg(target_os = "macos")]
            notebook.spawn_command(session_id, &[&shell, "--login"], None, None, None);
            #[cfg(not(target_os = "macos"))]
            notebook.spawn_command(session_id, &[&shell], None, None, None);
        }

        // Per spec: New connections ALWAYS create independent Root_Tabs
        // Register session for potential drag-and-drop, but don't show in split pane
        if let Some(info) = notebook.get_session_info(session_id) {
            // The display widget is resolved on demand via the content provider.
            split_view.add_session(info);
        }

        // Hide split view, show TabView content for the new tab
        split_view.widget().set_visible(false);
        split_view.widget().set_vexpand(false);
        notebook.widget().set_vexpand(true);
        notebook.show_tab_view_content();

        // Note: The switch_page signal handler will handle visibility
        // based on whether the session has a split_color assigned
    }

    /// Shows the quick connect dialog with protocol selection
    fn show_quick_connect_dialog(
        window: &adw::ApplicationWindow,
        notebook: SharedNotebook,
        split_view: SharedSplitView,
        sidebar: SharedSidebar,
        state: &SharedAppState,
        history: types::SharedQuickConnectHistory,
    ) {
        edit_dialogs::show_quick_connect_dialog(
            window.upcast_ref(),
            notebook,
            split_view,
            sidebar,
            state,
            history,
        );
    }

    /// Toggles group operations mode for multi-select
    fn toggle_group_operations_mode(sidebar: &SharedSidebar, enabled: bool) {
        sorting::toggle_group_operations_mode(sidebar, enabled);
    }

    /// Sorts connections alphabetically and updates `sort_order`
    fn sort_connections(state: &SharedAppState, sidebar: &SharedSidebar) {
        sorting::sort_connections(state, sidebar);
    }

    /// Sorts connections by recent usage (most recently used first)
    fn sort_recent(state: &SharedAppState, sidebar: &SharedSidebar) {
        sorting::sort_recent(state, sidebar);
    }

    /// Handles drag-drop operations for reordering connections
    fn handle_drag_drop(state: &SharedAppState, sidebar: &SharedSidebar, data: &str) {
        sorting::handle_drag_drop(state, sidebar, data);
    }

    /// Shows the export dialog
    ///
    /// Displays a dialog for exporting connections to various formats:
    /// - Ansible Inventory (INI/YAML)
    /// - SSH Config
    /// - Remmina (.remmina files)
    /// - Asbru-CM (YAML)
    fn show_export_dialog(window: &adw::ApplicationWindow, state: SharedAppState) {
        let dialog = ExportDialog::new(Some(&window.clone().upcast()));

        // Get connections and groups from state
        let state_ref = state.borrow();
        let connections: Vec<_> = state_ref
            .list_connections()
            .iter()
            .map(|c| (*c).clone())
            .collect();
        let groups: Vec<_> = state_ref
            .list_groups()
            .iter()
            .map(|g| (*g).clone())
            .collect();
        let snippets: Vec<_> = state_ref.list_snippets().into_iter().cloned().collect();
        let smart_folders = state_ref.settings().smart_folders.clone();
        let templates = state_ref.get_all_templates();
        let clusters: Vec<_> = state_ref.get_all_clusters().into_iter().cloned().collect();
        let variables = state_ref.settings().global_variables.clone();
        drop(state_ref);

        // Set data for export
        dialog.set_connections(connections);
        dialog.set_groups(groups);
        dialog.set_snippets(snippets);
        dialog.set_smart_folders(smart_folders);
        dialog.set_templates(templates);
        dialog.set_clusters(clusters);
        dialog.set_variables(variables);

        let window_clone = window.clone();
        dialog.run(move |result| {
            if let Some(export_result) = result {
                // Optionally open the output location on success
                if !export_result.output_files.is_empty()
                    && let Some(first_file) = export_result.output_files.first()
                {
                    ExportDialog::open_output_location(first_file);
                }

                // Non-blocking success feedback (GNOME HIG: toast, not dialog).
                crate::toast::show_toast_on_window(
                    &window_clone,
                    &crate::i18n::i18n_f(
                        "Successfully exported {} connection(s). {} skipped.",
                        &[
                            &export_result.exported_count.to_string(),
                            &export_result.skipped_count.to_string(),
                        ],
                    ),
                    crate::toast::ToastType::Success,
                );
            }
        });
    }

    /// Shows the terminal search dialog
    fn show_terminal_search_dialog(window: &adw::ApplicationWindow, notebook: &SharedNotebook) {
        if let Some(terminal) = notebook.get_active_terminal() {
            let dialog =
                crate::dialogs::TerminalSearchDialog::new(Some(&window.clone().upcast()), terminal);
            dialog.show();
        }
    }
}
