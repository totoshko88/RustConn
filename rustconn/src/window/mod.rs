//! Main application window
//!
//! This module provides the main window implementation for `RustConn`,
//! including the header bar, sidebar, terminal area, and action handling.

mod actions_setup;
mod batch_edit;
mod clusters;
mod command_env;
mod connection_actions;
mod connection_dialogs;
mod construct;
pub(crate) mod copy_field_actions;
mod credentials;
mod detach_actions;
mod edit_actions;
mod edit_dialogs;
mod edit_group;
mod entry_dialogs;
mod fullscreen_header;
mod group_broadcast;
mod groups;
mod history_actions;
mod key_setup;
mod key_unlock;
mod local_shell;
mod macro_dispatch;
mod mc_launch;
mod navigation_actions;
mod network_monitor;
mod operations;
mod prompt_autofill;
pub(crate) mod protocols;
mod protocols_ssh;
mod rdp_vnc;
mod remote_script;
mod resume_monitor;
mod session_lifecycle;
pub mod session_restore;
mod session_start;
mod sessions;
mod settings_glue;
mod sidebar_resize;
mod sidebar_sync;
mod smart_folders;
mod snippet_actions;
// `pub(crate)` for `truncate_command`, which the embedded RDP script gate needs:
// its confirmation shows the same command text and has to cap it the same way.
#[cfg(test)]
mod action_wiring_tests;
mod network_tools;
mod script_actions;
pub(crate) mod snippets;
mod sorting;
mod split_view_actions;
mod startup;
mod templates;
mod terminal_actions;
pub mod types;
mod ui;
mod wizard;
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

use self::types::{
    SessionSplitBridges, SharedNotebook, SharedSidebar, SharedSplitView, get_protocol_string,
};
use crate::activity_coordinator::ActivityCoordinator;
use crate::dialogs::{ExportDialog, SettingsDialog};
use crate::monitoring::MonitoringCoordinator;
use crate::sidebar::{ConnectionItem, ConnectionSidebar};
use crate::split_view::{SplitDirection, SplitViewBridge};
use crate::state::{SharedAppState, try_with_state_mut, with_state};
use crate::terminal::{ChildExitHook, LOCAL_SHELL_PROTOCOL, TerminalNotebook, local_shell_label};
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
// Detached session windows (issue #236), published for the same reason as
// `EXTERNAL_SESSIONS`: `MainWindow` itself is a short-lived value in `build_ui`,
// so the registry needs one owner that lives as long as the process while every
// closure holds only a `Weak` handle to it.
thread_local! {
    static DETACHED_WINDOWS: RefCell<Option<Rc<crate::detached_window::DetachedWindowRegistry>>> =
        const { RefCell::new(None) };
}

/// Acquires a busy guard from the thread-local [`rustconn_core::BusyStack`].
///
/// Returns `None` if the stack has not been initialised yet (before
/// `MainWindow::new` runs). The returned [`rustconn_core::BusyGuard`] keeps the
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

/// Returns the thread-local registry of detached session windows.
///
/// Returns `None` before `MainWindow::new` has published it (issue #236).
///
/// The shutdown paths use it to close every detached window: the main window's
/// close handler and the `app.quit` action, which bypasses that handler.
pub fn detached_window_registry() -> Option<Rc<crate::detached_window::DetachedWindowRegistry>> {
    DETACHED_WINDOWS.with(|cell| cell.borrow().as_ref().map(Rc::clone))
}

/// Returns how many sessions a shutdown would end (pure).
///
/// `TerminalNotebook::session_count` only counts sessions that have a tab, so
/// the two tabless kinds are added here: sessions in a detached window (issue
/// #236) and sessions delegated to an external viewer (issue #209). Shared by
/// the main window's close handler and the `app.quit` action so both ask the
/// same question before showing the confirmation dialog.
#[must_use]
pub const fn open_session_count(tabbed: usize, detached: usize, external: usize) -> usize {
    tabbed + detached + external
}

// The close confirmation currently on screen, if any.
//
// Two independent call sites raise this dialog — the main window's
// `close_request` and the `app.quit` action that Ctrl+Q, the primary menu and
// the tray's Quit item all go through — and each built a fresh one on every
// activation. Asking to quit twice therefore produced two stacked copies of the
// same question, which is what a tray Quit that "did nothing" looks like from
// the outside: the log records two `Quit` messages ten seconds apart with the
// teardown only after the second was answered.
//
// Held weakly on purpose. A strong handle has to be released by the `closed`
// signal, and a guard that can outlive the thing it guards is how a control ends
// up permanently dead — the failure mode that took the click guard back off Add
// Key. A destroyed dialog upgrades to `None`, so the worst case here is asking
// again, never being unable to ask.
thread_local! {
    static OPEN_CLOSE_CONFIRMATION: RefCell<Option<glib::WeakRef<adw::AlertDialog>>> =
        const { RefCell::new(None) };
}

/// Raises the close confirmation, or the one already on screen.
///
/// Returns the new dialog for the caller to wire a `close` response onto, or
/// `None` when a confirmation was already up — in which case the window carrying
/// it has been presented and the caller must not build a second one, nor treat
/// the absence of a dialog as consent to proceed.
///
/// Shared by the main window's `close_request` and the `app.quit` action so all
/// four ways of asking to quit ask once.
#[must_use]
pub fn present_close_confirmation(
    parent: &impl IsA<gtk4::Widget>,
    open_sessions: usize,
) -> Option<adw::AlertDialog> {
    let already_open = OPEN_CLOSE_CONFIRMATION
        .with(|cell| cell.borrow().as_ref().and_then(glib::WeakRef::upgrade));

    if let Some(existing) = already_open {
        tracing::debug!(
            open_sessions,
            "close confirmation already on screen — raising it instead of asking twice"
        );
        // An AdwDialog is drawn inside its host window, so raising that window
        // is what puts the question back in front of the user. This is the case
        // where the first ask went to a window that was behind, on another
        // workspace, or hidden to the tray.
        if let Some(window) = existing.root().and_downcast::<gtk4::Window>() {
            window.present();
        }
        return None;
    }

    let dialog = MainWindow::close_confirmation_dialog(open_sessions);
    OPEN_CLOSE_CONFIRMATION.with(|cell| *cell.borrow_mut() = Some(dialog.downgrade()));
    dialog.connect_closed(|_| {
        OPEN_CLOSE_CONFIRMATION.with(|cell| cell.borrow_mut().take());
    });

    tracing::debug!(open_sessions, "close confirmation presented");
    dialog.present(Some(parent));
    Some(dialog)
}

/// Ends every live session so none of them outlives the process.
///
/// The single teardown for every way RustConn can exit, and it is one function
/// because being three copies is what caused issue
/// [#304](https://github.com/totoshko88/RustConn/issues/304): the window's
/// `close_request`, the `app.quit` action and the tray's Quit item each grew
/// their own list, and no list was complete. External viewers were in two of
/// them, detached windows in two, and the children of the sessions actually in
/// tabs in none. Add a new exit path and it calls this; add a new kind of session
/// and it goes here.
///
/// Order matters. Owned external viewers go first because nothing else refers to
/// them. Session children come next and are signalled *synchronously* — every
/// pid, tabbed or detached, lives in the one map, so this is what makes the
/// escalation reliable on the way out, where a deferred one would never run.
/// Detached windows close last, by which point their teardown finds nothing left
/// to kill and only has widgets to drop.
///
/// Idempotent in every part, so two exit paths firing for one quit is harmless.
/// Must be called *after* `flush_active_recordings` — see
/// [`crate::terminal::TerminalNotebook::shutdown_sessions`].
pub fn shutdown_sessions_for_exit(notebook: &SharedNotebook) {
    // First, before anything is signalled. Every session child is about to exit
    // because this function is about to kill it, and the exit callbacks decide
    // whether that is a fault by asking `is_shutting_down()`. The flag was only
    // set in `connect_shutdown`, which GTK runs from `app.quit()` — and `do_quit`
    // calls `app.quit()` *after* this, so the answer was still `false` for
    // exactly the exits it exists to explain. A tray-minimize never reaches here,
    // which is what makes this the right place: the flag cannot be set by a close
    // that leaves the application running.
    crate::app::mark_shutting_down();

    // Issue #209: kill owned viewer children so they do not become orphans, and
    // close their open history entries. Detaching viewers keep running.
    if let Some(registry) = external_session_registry() {
        registry.shutdown();
    }

    // Tunnels still being raised on a worker thread are known to nobody else:
    // end them, and refuse new ones, before the process goes.
    rustconn_core::ssh_tunnel::stop_all_tunnels();

    // Issue #304.
    notebook.shutdown_sessions();

    // Issue #236: no detached window outlives the main window.
    if let Some(registry) = detached_window_registry() {
        registry.close_all();
    }
}

/// Bridges hardware-key touch notifications from the worker thread that runs
/// `keepassxc-cli` to the main-thread banner (issue #350).
///
/// GTK widgets are `!Send`, so this cannot touch the banner directly. It sends
/// a `true` when a touch-waiting run starts and a `false` when it ends over an
/// `mpsc` channel; a `glib::timeout_add_local` drain on the main thread nets
/// the deltas into an in-flight count and reveals the banner while it is above
/// zero. `mpsc::Sender<bool>` is `Send + Sync` (Rust ≥ 1.72), which the
/// [`TouchObserver`](rustconn_core::secret::TouchObserver) bound requires.
struct BannerTouchObserver {
    tx: std::sync::mpsc::Sender<bool>,
}

impl rustconn_core::secret::TouchObserver for BannerTouchObserver {
    fn touch_started(&self) {
        let _ = self.tx.send(true);
        // Wake the main loop so the drain reveals the banner promptly rather
        // than on the next 50 ms tick.
        glib::MainContext::ref_thread_default().wakeup();
    }

    fn touch_finished(&self) {
        let _ = self.tx.send(false);
        glib::MainContext::ref_thread_default().wakeup();
    }
}

/// Main application window wrapper
///
/// Provides access to the main window and its components.
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
    /// Content-header title widget. Held so settings-apply can refresh the
    /// Nautilus-style hierarchy-path subtitle live when the toggle changes,
    /// not only on the next tab switch.
    header_title: adw::WindowTitle,
    /// Registry of external viewer sessions (VNC/RDP/SPICE delegated to a
    /// separate viewer process, issue #209). Tracks child processes and drives
    /// sidebar session-count + history via callbacks; watched by a shared timer.
    external_sessions: Rc<crate::external_session::ExternalSessionRegistry>,
    /// Windows hosting detached sessions (issue #236), keyed by session. Owns
    /// the window values; the notebook keeps owning all session state. Also
    /// published to a thread-local so the lifecycle paths can reach it.
    detached_windows: Rc<crate::detached_window::DetachedWindowRegistry>,
    toast_overlay: SharedToastOverlay,
    monitoring: Rc<MonitoringCoordinator>,
    activity_coordinator: types::SharedActivityCoordinator,
    tunnel_manager: SharedTunnelManager,
    /// Busy-state tracker — shows/hides header bar spinner on 0→1 / 1→0 transitions
    busy_stack: rustconn_core::BusyStack,
    /// Quick Connect history (max 15 entries, LIFO, persisted in settings)
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
    /// Cross-tab keystroke broadcast state (issue #329). Independent of the
    /// per-tab split broadcast: this mirrors keystrokes across sessions that
    /// each live on their own tab. See `window::group_broadcast`.
    group_broadcast: group_broadcast::GroupBroadcast,
    /// Header-bar toggle for cross-tab group broadcast. Visible only when the
    /// broadcast set has two or more members; drives `win.toggle-group-broadcast`.
    group_broadcast_toggle: gtk4::ToggleButton,
    /// Persistent banner shown below the header bar while group broadcast is
    /// active, naming how many sessions receive the mirrored input. A group
    /// broadcast is not all-visible at once, so its active state must be
    /// impossible to miss (issue #329).
    group_broadcast_banner: adw::Banner,
    /// Persistent banner below the header bar shown at startup when a config
    /// file could not be read and was kept aside, or was written by a newer
    /// RustConn. Filled once by `show_config_file_notice`; Dismiss hides it.
    config_banner: adw::Banner,
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

/// Content-header buttons the medium (≤ 820sp) breakpoint hides, by action name.
const MEDIUM_HIDDEN: [&str; 2] = ["win.split-vertical", "win.split-horizontal"];

/// Content-header buttons the narrow (≤ 600sp) breakpoint hides, by action name.
/// Repeats the medium tier: only one breakpoint applies at a time.
///
/// `win.local-shell` is deliberately NOT here: on a narrow window the Shell
/// button stays, shrinking to icon-only (its "Shell" label is collapsed by the
/// `.compact` CSS, see `.shell-button-label` in style.css). There is room for
/// the icon even at the minimum width, and launching a local shell is a primary
/// action that should not vanish just because the window is narrow (user
/// request, 0.23.5 UI/UX audit).
const NARROW_HIDDEN: [&str; 3] = ["win.split-vertical", "win.split-horizontal", "win.settings"];

/// Builds the body of the "jump host unavailable" question (issue #345).
///
/// Three short paragraphs, each a whole translatable sentence: what is wrong
/// with which connection's Jump Host setting, where an inherited setting comes
/// from, and what connecting now actually does — derived from
/// [`rustconn_core::connection::jump_chain::bastion_fallback`], so it never
/// says "directly" while a ProxyJump or the live part of the chain still applies.
pub(crate) fn skipped_bastion_body(
    state: &crate::state::AppState,
    conn: &rustconn_core::Connection,
    skipped: &crate::state::SkippedBastion,
) -> String {
    use crate::i18n::{i18n, i18n_f};
    use rustconn_core::connection::jump_chain::{
        BastionFallback, BastionRefOrigin, DanglingReason,
    };

    let dangling = &skipped.dangling;
    let source_name = state
        .get_connection(dangling.source_id)
        .map_or_else(|| conn.name.clone(), |c| c.name.clone());

    let mut paragraphs = vec![match dangling.reason {
        DanglingReason::Missing => i18n_f(
            "The jump host set for “{}” no longer exists.",
            &[&source_name],
        ),
        DanglingReason::SelfReference => i18n_f(
            "“{}” is set to use itself as its jump host.",
            &[&source_name],
        ),
    }];
    match dangling.origin {
        BastionRefOrigin::Connection => {}
        BastionRefOrigin::Group(group_id) => {
            let group_name = state
                .get_group(group_id)
                .map(|g| g.name.clone())
                .unwrap_or_default();
            paragraphs.push(i18n_f(
                "The setting is inherited from the group “{}”.",
                &[&group_name],
            ));
        }
        BastionRefOrigin::Network => {
            paragraphs.push(i18n("The setting is the Global Jump Host in Settings."));
        }
    }
    paragraphs.push(match skipped.fallback {
        BastionFallback::Direct => i18n_f(
            "Connecting to “{}” now reaches the server directly, without a jump host.",
            &[&conn.name],
        ),
        BastionFallback::RemainingRoute => i18n_f(
            "Connecting to “{}” now skips that jump host and uses only the rest of its configured route.",
            &[&conn.name],
        ),
    });
    paragraphs.join("\n\n")
}

impl MainWindow {
    /// Applies a sidebar auto-hide verdict to the split view (issue #401).
    ///
    /// Idempotent: setting the sidebar to the state it is already in is a no-op,
    /// so this never fights the adaptive breakpoint or a manual toggle.
    fn apply_sidebar_auto_action(
        split: &adw::OverlaySplitView,
        action: rustconn_core::config::SidebarAutoAction,
    ) {
        use rustconn_core::config::SidebarAutoAction;
        match action {
            SidebarAutoAction::Hide => {
                if split.shows_sidebar() {
                    tracing::debug!("auto-hide: collapsing sidebar on connect");
                    split.set_show_sidebar(false);
                }
            }
            SidebarAutoAction::Show => {
                if !split.shows_sidebar() {
                    tracing::debug!("auto-hide: revealing sidebar, last session closed");
                    split.set_show_sidebar(true);
                }
            }
        }
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
        // Only a page that belongs to a session counts — the Welcome
        // placeholder has none, and identifying it by its (translated) title
        // would also hide a real session that happens to share the word.
        let name = if enabled && notebook.get_active_session_id().is_some() {
            notebook
                .tab_view()
                .selected_page()
                .map(|page| page.title().to_string())
                .filter(|title| !title.is_empty())
        } else {
            None
        };
        match name {
            Some(name) => window.set_title(Some(&crate::i18n::i18n_f("RustConn - {}", &[&name]))),
            None => window.set_title(Some("RustConn")),
        }
    }

    /// Sets the content header's subtitle to the active connection's group
    /// path (e.g. "AWS Test Lab / Prod"), gated on `window_title_shows_path`.
    ///
    /// The connection comes from the session metadata of the session the
    /// selected tab is named after (for a split tab, the focused pane's —
    /// issue #421), not from its title: a title carries a "\[group\] " prefix
    /// once the tab is grouped, and two connections may share a name. The
    /// subtitle is cleared when the
    /// setting is off, the page has no session (Welcome), the session has no
    /// saved connection (local shell), or the connection is ungrouped. The WM
    /// window title (issue #211) is untouched. Shared by the tab-switch hook
    /// and the live settings-apply path. Leaves the subtitle as it is when the
    /// state is borrowed elsewhere — the next tab switch catches up.
    pub(crate) fn refresh_header_subtitle(
        header_title: &adw::WindowTitle,
        notebook: &SharedNotebook,
        state: &SharedAppState,
    ) {
        // The session the tab is named after: for a split tab, the focused
        // pane's, so the subtitle and the title describe one connection.
        let connection_id = notebook
            .active_named_session_id()
            .and_then(|session_id| notebook.get_session_info(session_id))
            .map(|info| info.connection_id);
        let Ok(state_ref) = state.try_borrow() else {
            return;
        };
        let subtitle = if state_ref.settings().ui.window_title_shows_path {
            connection_id
                .and_then(|id| state_ref.get_connection(id))
                .and_then(|conn| conn.group_id)
                .and_then(|group_id| state_ref.get_group_path(group_id))
                .unwrap_or_default()
        } else {
            String::new()
        };
        drop(state_ref);
        header_title.set_subtitle(&subtitle);
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

    /// Reveals the config-file banner when startup kept an unreadable file aside
    /// or found one written by a newer RustConn.
    fn show_config_file_notice(&self) {
        // Taken in a block of its own, so no borrow of the state is held while
        // the banner's notify handlers run.
        let notice = {
            let Ok(mut state) = self.state.try_borrow_mut() else {
                tracing::warn!("State busy at startup; config file notice not shown");
                return;
            };
            state.take_config_file_notice()
        };
        let Some(notice) = notice else {
            return;
        };
        let title = match notice {
            crate::state::ConfigFileNotice::KeptUnreadable(kept) => {
                let kept = kept.display().to_string();
                crate::i18n::i18n_f(
                    "Settings could not be read, so defaults are in use. The unreadable file was kept as {}.",
                    &[&kept],
                )
            }
            crate::state::ConfigFileNotice::WrittenByNewer(version) => crate::i18n::i18n_f(
                "These settings were saved by a newer RustConn ({}). A backup is kept before they are changed.",
                &[&version],
            ),
        };
        self.config_banner.set_title(&title);
        self.config_banner.set_revealed(true);
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
}

/// Where a request to open a connection goes when it may already be running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenRoute {
    /// Focus this live embedded session.
    Focus(Uuid),
    /// The connection runs only in an external viewer window.
    AlreadyExternal,
    /// Nothing runs: start a new session.
    Launch,
}

/// Picks the route for opening a connection that may already be running.
///
/// A live embedded session wins over an external viewer, which can only be
/// reported, not raised. `external_running` is asked only when no embedded
/// session is live.
fn open_route(newest_live: Option<Uuid>, external_running: impl FnOnce() -> bool) -> OpenRoute {
    match newest_live {
        Some(session_id) => OpenRoute::Focus(session_id),
        None if external_running() => OpenRoute::AlreadyExternal,
        None => OpenRoute::Launch,
    }
}

#[cfg(test)]
mod tests {
    use super::startup::rdp_file_requested_devices;
    use super::{OpenRoute, open_route, open_session_count};

    #[test]
    fn a_running_connection_is_shown_not_duplicated() {
        let live = uuid::Uuid::new_v4();
        assert_eq!(open_route(Some(live), || true), OpenRoute::Focus(live));
        assert_eq!(open_route(Some(live), || false), OpenRoute::Focus(live));
        assert_eq!(open_route(None, || true), OpenRoute::AlreadyExternal);
        assert_eq!(open_route(None, || false), OpenRoute::Launch);
    }

    #[test]
    fn an_rdp_file_without_device_redirection_needs_no_prompt() {
        let mut connection = rustconn_core::models::Connection::new_rdp(
            "host".to_string(),
            "host.example".to_string(),
            3389,
        );
        assert!(rdp_file_requested_devices(&connection).is_empty());

        if let rustconn_core::ProtocolConfig::Rdp(rdp) = &mut connection.protocol_config {
            rdp.microphone_enabled = true;
            rdp.smartcard_enabled = true;
            rdp.fido2_enabled = true;
        }
        assert_eq!(rdp_file_requested_devices(&connection).len(), 3);
    }

    #[test]
    fn tabless_sessions_are_counted_for_the_close_confirmation() {
        // One tab, one detached window, one external viewer: three sessions
        // would end, so the confirmation has to appear.
        assert_eq!(open_session_count(1, 1, 1), 3);
    }

    #[test]
    fn only_detached_sessions_still_count_as_open() {
        // The main window can hold nothing but a Welcome tab while a session
        // runs in its own window — quitting must still ask.
        assert_eq!(open_session_count(0, 2, 0), 2);
    }

    #[test]
    fn nothing_open_needs_no_confirmation() {
        assert_eq!(open_session_count(0, 0, 0), 0);
    }
}
