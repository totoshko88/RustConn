//! Terminal notebook area using adw::TabView
//!
//! This module provides the tabbed terminal interface using VTE4
//! for SSH sessions and native GTK widgets for VNC/RDP/SPICE connections.
//!
//! # Module Structure
//!
//! - `types` - Data structures for sessions
//! - `config` - Terminal appearance and behavior configuration

mod config;
pub mod file_drop;
pub mod highlight_overlay;
pub mod playback;
mod recording;
pub mod tab_container;
mod tab_menu;
mod types;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Orientation, Widget, gio, glib};
use libadwaita as adw;
use libadwaita::prelude::*;
use rustconn_core::models::AutomationConfig;
use rustconn_core::terminal_themes::TerminalTheme;
pub use types::{SessionWidgetStorage, TerminalSession};
use uuid::Uuid;
#[cfg(not(target_os = "macos"))]
use vte4::PtyFlags;
use vte4::Terminal;
use vte4::prelude::*;

/// PCRE2 multiline compile flag — required by VTE's `match_add_regex()`.
///
/// Without this flag VTE emits a runtime warning:
/// `_vte_regex_has_multiline_compile_flag(regex)` check failed.
const PCRE2_MULTILINE: u32 = 0x0000_0400;

use rustconn_core::automation::{KeyElement, KeySequence};
use rustconn_core::highlight::CompiledHighlightRules;
use rustconn_core::models::HighlightRule;
use rustconn_core::session::recording::{RecordingMetadata, metadata_path, write_metadata};
use rustconn_core::split::tab_groups::TabGroupManager;

use crate::activity_coordinator::ActivityCoordinator;
use crate::automation::{AutomationSession, prepare_rules_from_config};
use crate::embedded_rdp::EmbeddedRdpWidget;
use crate::i18n::{i18n, i18n_f};
use crate::session::{SessionState, SessionWidget, VncSessionWidget};
use crate::terminal::highlight_overlay::HighlightOverlay;
use crate::terminal::tab_container::TabPageContainer;

/// SSH connection parameters needed for remote recording file retrieval.
#[derive(Debug, Clone)]
pub struct SshRecordingParams {
    /// Remote host address
    pub host: String,
    /// Remote port
    pub port: u16,
    /// Username for SSH
    pub username: Option<String>,
    /// Path to SSH identity file
    pub identity_file: Option<String>,
}

/// Tracks a remote recording session (script running on a remote host).
struct RemoteRecordingInfo {
    /// Remote path to the data file (on the SSH host)
    remote_data: String,
    /// Remote path to the timing file (on the SSH host)
    remote_timing: String,
    /// Local destination for the data file
    local_data: PathBuf,
    /// Local destination for the timing file
    local_timing: PathBuf,
    /// SSH connection params for SCP retrieval
    ssh_params: SshRecordingParams,
}

/// Whether a session can be hosted in a split panel, and how.
///
/// Keyed on the stored widget kind rather than a protocol string, so an
/// external-process viewer is declined even when its protocol is rdp/vnc/spice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitEligibility {
    /// VTE terminal or an in-process embedded viewer — can be split.
    Embeddable,
    /// rdp/vnc/spice running via an external process/viewer — cannot be embedded.
    ExternalViewer,
    /// No live session/widget for this id.
    None,
}

/// Maps a session's stored state to its split eligibility (pure, GTK-free).
///
/// `session_widgets` storage wins over `has_terminal`: an embedded viewer or
/// external process is classified by its variant; otherwise a live VTE terminal
/// is `Embeddable`; anything unknown is `None`.
#[must_use]
fn eligibility_from(
    has_terminal: bool,
    storage: Option<&SessionWidgetStorage>,
) -> SplitEligibility {
    match storage {
        Some(SessionWidgetStorage::Vnc(_) | SessionWidgetStorage::EmbeddedRdp(_)) => {
            SplitEligibility::Embeddable
        }
        #[cfg(feature = "web-embedded")]
        Some(SessionWidgetStorage::EmbeddedWeb(_)) => SplitEligibility::Embeddable,
        Some(SessionWidgetStorage::ExternalProcess(_)) => SplitEligibility::ExternalViewer,
        None if has_terminal => SplitEligibility::Embeddable,
        None => SplitEligibility::None,
    }
}

/// Terminal notebook widget for managing multiple terminal sessions
/// Now using adw::TabView for modern GNOME HIG compliance
pub struct TerminalNotebook {
    /// Main container with TabView and TabBar
    container: GtkBox,
    /// The adw::TabView for managing tabs
    tab_view: adw::TabView,
    /// The adw::TabBar for displaying tabs
    tab_bar: adw::TabBar,
    /// The adw::TabOverview for grid view of all tabs
    tab_overview: adw::TabOverview,
    /// Map of session IDs to their TabPage
    sessions: Rc<RefCell<HashMap<Uuid, adw::TabPage>>>,
    /// Callback for when a page is closed (session_id, connection_id)
    on_page_closed: Rc<RefCell<Option<Box<dyn Fn(Uuid, Uuid)>>>>,
    /// Callback fired when a new terminal session tab is created
    /// (session_id, connection_id). The single choke point for per-session
    /// setup such as activity monitoring — covers every terminal protocol
    /// and both synchronous and async (port-checked) connection paths.
    on_session_created: Rc<RefCell<Option<Box<dyn Fn(Uuid, Uuid)>>>>,
    /// One-shot callback fired when ANY tab is added (terminal, VNC, SPICE,
    /// RDP, external). Used by workspace restore to detect when an
    /// asynchronously-connected session finally appears so it can be placed
    /// in the split panel. Receives (session_id, connection_id).
    on_tab_added: Rc<RefCell<Option<Box<dyn Fn(Uuid, Uuid)>>>>,
    /// Callback for recording start/stop (`connection_id`, recording) —
    /// drives the sidebar recording indicator
    on_recording_changed: Rc<RefCell<Option<Box<dyn Fn(Uuid, bool)>>>>,
    /// Callback fired after the split-color map changes (a session joins or
    /// leaves a split, or a split tab closes) — drives the sidebar
    /// split-membership marker. Takes no args; the handler re-syncs the whole
    /// sidebar from `split_colors()`, which is robust and side-steps tracking
    /// individual join/leave deltas.
    on_split_colors_changed: Rc<RefCell<Option<Box<dyn Fn()>>>>,
    /// Callback for split view cleanup when a page is about to close (session_id)
    on_split_cleanup: Rc<RefCell<Option<Box<dyn Fn(Uuid)>>>>,
    /// Map of session IDs to terminal widgets (for SSH sessions)
    terminals: Rc<RefCell<HashMap<Uuid, Terminal>>>,
    /// Map of session IDs to session widgets (for VNC/RDP/SPICE sessions)
    session_widgets: Rc<RefCell<HashMap<Uuid, SessionWidgetStorage>>>,
    /// Map of session IDs to automation sessions
    automation_sessions: Rc<RefCell<HashMap<Uuid, AutomationSession>>>,
    /// Session metadata
    session_info: Rc<RefCell<HashMap<Uuid, TerminalSession>>>,
    /// Whether to color tab indicators by protocol type
    color_tabs_by_protocol: Rc<RefCell<bool>>,
    /// Direct tracking of split view colors per session (session_id → color_index).
    /// Used to prevent protocol/clear operations from overwriting split indicators.
    split_session_colors: Rc<RefCell<HashMap<Uuid, usize>>>,
    /// Tab group manager for assigning colors to named groups
    tab_group_manager: Rc<RefCell<TabGroupManager>>,
    /// Callback for reconnect button clicks (session_id, connection_id)
    on_reconnect: Rc<RefCell<Option<Box<dyn Fn(Uuid, Uuid)>>>>,
    /// Callback fired when terminal focus changes (`true` = focus entered the
    /// VTE, `false` = focus left). Drives focus-based accelerator suspend (#197).
    on_terminal_focus: Rc<RefCell<Option<Box<dyn Fn(bool)>>>>,
    /// Sessions that already have a reconnect banner (prevents duplicates)
    reconnect_shown: Rc<RefCell<HashSet<Uuid>>>,
    /// Cluster terminal tracking: cluster_id → Vec<session_id>
    cluster_sessions: Rc<RefCell<HashMap<Uuid, Vec<Uuid>>>>,
    /// Reverse lookup: session_id → cluster_id
    session_to_cluster: Rc<RefCell<HashMap<Uuid, Uuid>>>,
    /// Pending cluster registrations awaiting their session_id.
    ///
    /// When a connection is launched as part of a cluster but its terminal is
    /// created asynchronously (e.g. after a TCP port check), we cannot register
    /// the session_id at launch time. Instead we record the (connection_id →
    /// cluster_id) pair here and resolve it the moment a tab is created.
    cluster_pending: Rc<RefCell<HashMap<Uuid, Uuid>>>,
    /// Active recording sessions (tracked by session_id)
    active_recordings: Rc<RefCell<HashSet<Uuid>>>,
    /// Recording paths and start times: session_id → (data_path, timing_path, connection_name, start_time)
    recording_paths: RefCell<HashMap<Uuid, (PathBuf, PathBuf, String, Instant)>>,
    /// Remote recording info for SSH sessions: session_id → RemoteRecordingInfo
    remote_recordings: RefCell<HashMap<Uuid, RemoteRecordingInfo>>,
    /// Compiled highlight rules per session: session_id → CompiledHighlightRules
    session_highlight_rules: Rc<RefCell<HashMap<Uuid, CompiledHighlightRules>>>,
    /// Highlight overlay widgets per session: session_id → HighlightOverlay
    highlight_overlays: Rc<RefCell<HashMap<Uuid, HighlightOverlay>>>,
    /// GTK Overlay widgets per session for layering highlight DrawingArea
    terminal_overlays: Rc<RefCell<HashMap<Uuid, gtk4::Overlay>>>,
    /// Cancel tokens for background polling tasks (host check, auto-reconnect, WoL)
    /// Keyed by session_id or connection_id depending on context
    poll_cancel_tokens: Rc<RefCell<HashMap<Uuid, std::sync::Arc<std::sync::atomic::AtomicBool>>>>,
    /// SSH tunnels for jump-host connections (RDP, VNC, SPICE, Telnet).
    /// Killed automatically when the tab is closed.
    ssh_tunnels: Rc<RefCell<HashMap<Uuid, rustconn_core::ssh_tunnel::SshTunnel>>>,
    /// Activity coordinator for terminal activity/silence monitoring (set after construction)
    activity_coordinator: Rc<RefCell<Option<Rc<ActivityCoordinator>>>>,
    /// Per-session tab page containers (session_id → TabPageContainer).
    /// Guarantees every TabPage.child() has non-zero allocation for TabOverview.
    tab_containers: Rc<RefCell<HashMap<Uuid, TabPageContainer>>>,
    /// Sessions whose standalone tab was removed while they live in another
    /// tab's split (issue: split guests should not clutter the tab bar or
    /// Tab Overview). Their session data (widget, terminal, info) stays alive;
    /// `restore_session_tab` recreates the tab when the session leaves the split.
    parked_in_split: Rc<RefCell<HashSet<Uuid>>>,
    /// Shared snippet menu section for terminal context menus.
    /// Updated when snippets are created/edited/deleted; all terminals
    /// share the same live `gio::Menu` model so changes propagate automatically.
    snippet_menu_section: Rc<gio::Menu>,
    /// VTE child process PIDs per session.
    /// Used to send SIGTERM/SIGKILL to the process group on tab close.
    /// Some terminal clients (e.g. telnet) do not exit on PTY close (SIGHUP),
    /// so an explicit kill is needed (#172).
    vte_child_pids: Rc<RefCell<HashMap<Uuid, i32>>>,
}

impl TerminalNotebook {
    /// Creates a new terminal notebook using adw::TabView
    #[must_use]
    pub fn new() -> Self {
        let container = GtkBox::new(Orientation::Vertical, 0);

        // Create TabView - content visibility controlled dynamically
        // For SSH: TabView hidden, content in split_view
        // For RDP/VNC/SPICE: TabView visible, content in TabView pages
        let tab_view = adw::TabView::new();
        tab_view.set_hexpand(true);
        tab_view.set_vexpand(true); // Will expand when visible for RDP/VNC/SPICE

        // Create TabBar - this is what we show
        let tab_bar = adw::TabBar::new();
        tab_bar.set_view(Some(&tab_view));
        tab_bar.set_autohide(false);
        tab_bar.set_expand_tabs(false);
        tab_bar.set_inverted(false);

        // Enable drag-and-drop for reordering tabs within the bar
        // but NOT to external targets (we handle that separately)
        tab_bar.set_extra_drag_preload(false);

        // Create TabOverview for grid view of all tabs (GNOME Web-style)
        let tab_overview = adw::TabOverview::new();
        tab_overview.set_view(Some(&tab_view));
        tab_overview.set_enable_new_tab(false);

        // Add overview button to the end of the TabBar
        let overview_button = gtk4::Button::from_icon_name("view-grid-symbolic");
        overview_button.set_tooltip_text(Some(&i18n("Tab Overview (Ctrl+Shift+O)")));
        overview_button.add_css_class("flat");
        overview_button.set_action_name(Some("win.tab-overview"));
        overview_button
            .update_property(&[gtk4::accessible::Property::Label(&i18n("Tab Overview"))]);
        tab_bar.set_end_action_widget(Some(&overview_button));

        // Only add TabBar to container - TabView is hidden but still manages tabs
        container.append(&tab_bar);
        // TabView must be in widget tree for TabBar to work, but hidden
        container.append(&tab_view);

        // Add a welcome page
        let welcome = Self::create_welcome_tab();
        let welcome_container = TabPageContainer::welcome(&welcome.upcast::<gtk4::Widget>());
        let welcome_page = tab_view.append(welcome_container.widget());
        welcome_page.set_title(&i18n("Welcome"));
        welcome_page.set_icon(Some(&gio::ThemedIcon::new("go-home-symbolic")));

        let term_notebook = Self {
            container,
            tab_view,
            tab_bar,
            tab_overview,
            sessions: Rc::new(RefCell::new(HashMap::new())),
            on_page_closed: Rc::new(RefCell::new(None)),
            on_session_created: Rc::new(RefCell::new(None)),
            on_tab_added: Rc::new(RefCell::new(None)),
            on_recording_changed: Rc::new(RefCell::new(None)),
            on_split_colors_changed: Rc::new(RefCell::new(None)),
            on_split_cleanup: Rc::new(RefCell::new(None)),
            terminals: Rc::new(RefCell::new(HashMap::new())),
            session_widgets: Rc::new(RefCell::new(HashMap::new())),
            automation_sessions: Rc::new(RefCell::new(HashMap::new())),
            session_info: Rc::new(RefCell::new(HashMap::new())),
            color_tabs_by_protocol: Rc::new(RefCell::new(false)),
            split_session_colors: Rc::new(RefCell::new(HashMap::new())),
            tab_group_manager: Rc::new(RefCell::new(TabGroupManager::new())),
            on_reconnect: Rc::new(RefCell::new(None)),
            on_terminal_focus: Rc::new(RefCell::new(None)),
            reconnect_shown: Rc::new(RefCell::new(HashSet::new())),
            cluster_sessions: Rc::new(RefCell::new(HashMap::new())),
            session_to_cluster: Rc::new(RefCell::new(HashMap::new())),
            cluster_pending: Rc::new(RefCell::new(HashMap::new())),
            recording_paths: RefCell::new(HashMap::new()),
            session_highlight_rules: Rc::new(RefCell::new(HashMap::new())),
            highlight_overlays: Rc::new(RefCell::new(HashMap::new())),
            terminal_overlays: Rc::new(RefCell::new(HashMap::new())),
            active_recordings: Rc::new(RefCell::new(HashSet::new())),
            remote_recordings: RefCell::new(HashMap::new()),
            poll_cancel_tokens: Rc::new(RefCell::new(HashMap::new())),
            ssh_tunnels: Rc::new(RefCell::new(HashMap::new())),
            activity_coordinator: Rc::new(RefCell::new(None)),
            tab_containers: Rc::new(RefCell::new(HashMap::new())),
            parked_in_split: Rc::new(RefCell::new(HashSet::new())),
            snippet_menu_section: Rc::new(gio::Menu::new()),
            vte_child_pids: Rc::new(RefCell::new(HashMap::new())),
        };

        term_notebook.setup_tab_view_signals();
        term_notebook.setup_tab_context_menu();
        term_notebook.setup_tab_overview_cleanup();
        term_notebook
    }

    /// Sets up TabView signals for close requests
    fn setup_tab_view_signals(&self) {
        let sessions = self.sessions.clone();
        let terminals = self.terminals.clone();
        let session_widgets = self.session_widgets.clone();
        let session_info = self.session_info.clone();
        let tab_view = self.tab_view.clone();
        let split_session_colors_close = self.split_session_colors.clone();
        let on_split_colors_changed_close = self.on_split_colors_changed.clone();
        let on_page_closed = self.on_page_closed.clone();
        let on_split_cleanup = self.on_split_cleanup.clone();
        let active_recordings = self.active_recordings.clone();
        let session_highlight_rules = self.session_highlight_rules.clone();
        let highlight_overlays = self.highlight_overlays.clone();
        let terminal_overlays = self.terminal_overlays.clone();
        let ssh_tunnels = self.ssh_tunnels.clone();
        let tab_containers = self.tab_containers.clone();
        let parked_in_split = self.parked_in_split.clone();
        let vte_child_pids = self.vte_child_pids.clone();

        // Handle create-window signal - we must connect this to prevent the default
        // behavior which causes CRITICAL warnings. Returning None cancels the tearoff.
        // Note: libadwaita will still show a CRITICAL warning, but this is unavoidable
        // without implementing multi-window support.
        self.tab_view.connect_create_window(|_| {
            // Log instead of letting libadwaita complain
            tracing::debug!("Tab tearoff attempted but not supported - cancelling");
            // Return None to cancel the operation
            // The CRITICAL warning from libadwaita is unavoidable
            None
        });

        // Handle close-page signal
        self.tab_view.connect_close_page(move |view, page| {
            // Find session ID for this page
            let (session_id, connection_id) = {
                let sessions_ref = sessions.borrow();
                let info_ref = session_info.borrow();
                sessions_ref
                    .iter()
                    .find(|(_, p)| *p == page)
                    .map(|(id, _)| {
                        let conn_id = info_ref.get(id).map(|i| i.connection_id);
                        (*id, conn_id)
                    })
                    .unwrap_or((Uuid::nil(), None))
            };

            // Parked into a split (Option B): the tab is being removed because the
            // session moved into another tab's split, NOT closed. Its live widget
            // lives in the split and its session data must survive, so drop only
            // the tab page and its (now-stale) container mapping — skip all
            // teardown. `restore_session_tab` recreates the tab on split-leave.
            if !session_id.is_nil() && parked_in_split.borrow().contains(&session_id) {
                sessions.borrow_mut().remove(&session_id);
                tab_containers.borrow_mut().remove(&session_id);
                view.close_page_finish(page, true);
                return glib::Propagation::Stop;
            }

            if !session_id.is_nil() {
                // Call the on_split_cleanup callback FIRST to clear split view panels
                // This must happen before on_page_closed to ensure proper cleanup
                if let Some(ref callback) = *on_split_cleanup.borrow() {
                    callback(session_id);
                }

                // Call the on_page_closed callback to update sidebar status
                if let Some(conn_id) = connection_id
                    && let Some(ref callback) = *on_page_closed.borrow()
                {
                    callback(session_id, conn_id);
                }

                let was_in_split = split_session_colors_close
                    .borrow_mut()
                    .remove(&session_id)
                    .is_some();
                // Re-sync the sidebar split marker only when this tab actually
                // held a split color; the borrow above is already dropped so the
                // handler can freely re-read the map.
                if was_in_split && let Some(ref callback) = *on_split_colors_changed_close.borrow()
                {
                    callback();
                }

                // Clean up session data
                sessions.borrow_mut().remove(&session_id);
                terminals.borrow_mut().remove(&session_id);

                // Remove active recording flag if present
                active_recordings.borrow_mut().remove(&session_id);

                // Remove compiled highlight rules for this session
                session_highlight_rules.borrow_mut().remove(&session_id);

                // Remove highlight overlay for this session
                highlight_overlays.borrow_mut().remove(&session_id);

                // Remove terminal overlay widget for this session
                terminal_overlays.borrow_mut().remove(&session_id);

                // Disconnect embedded widgets before removing
                if let Some(widget_storage) = session_widgets.borrow_mut().remove(&session_id) {
                    match widget_storage {
                        SessionWidgetStorage::EmbeddedRdp(widget) => widget.disconnect(),
                        SessionWidgetStorage::Vnc(widget) => widget.disconnect(),
                        #[cfg(feature = "web-embedded")]
                        SessionWidgetStorage::EmbeddedWeb(widget) => {
                            let _ = widget.disconnect();
                        }
                        SessionWidgetStorage::ExternalProcess(process) => {
                            if let Some(mut child) = process.borrow_mut().take() {
                                let _ = child.kill();
                                let _ = child.wait();
                                tracing::debug!(
                                    session = %session_id,
                                    "Killed external process on tab close"
                                );
                            }
                        }
                    }
                }

                session_info.borrow_mut().remove(&session_id);

                // Kill VTE child process group explicitly (#172).
                // Some CLI clients (notably telnet) do not exit on SIGHUP
                // when the PTY master fd is closed. Sending SIGTERM to the
                // process group ensures all children terminate.
                if let Some(pid) = vte_child_pids.borrow_mut().remove(&session_id) {
                    // kill(-pid) sends the signal to the entire process group
                    let pgid = nix::unistd::Pid::from_raw(-pid);
                    if nix::sys::signal::kill(pgid, nix::sys::signal::Signal::SIGTERM).is_err() {
                        // Process (group) may have already exited — try direct PID
                        let direct = nix::unistd::Pid::from_raw(pid);
                        let _ = nix::sys::signal::kill(direct, nix::sys::signal::Signal::SIGKILL);
                    } else {
                        // SIGTERM delivered successfully, but the process may
                        // ignore it. Schedule a SIGKILL fallback after 500ms.
                        let pgid_raw = pid;
                        glib::timeout_add_local_once(
                            std::time::Duration::from_millis(500),
                            move || {
                                // Check if process still exists AND belongs to our
                                // process group (guards against PID reuse by verifying
                                // the process group leader is still `pid`).
                                let probe = nix::unistd::Pid::from_raw(pid);
                                if nix::sys::signal::kill(probe, None).is_ok() {
                                    // Verify process group hasn't changed (PID reuse guard):
                                    // if the PID was recycled, getpgid will return a different
                                    // group or fail.
                                    let still_ours = nix::unistd::getpgid(Some(probe))
                                        .is_ok_and(|pgid| pgid.as_raw() == pgid_raw);
                                    if still_ours {
                                        let _ = nix::sys::signal::kill(
                                            probe,
                                            nix::sys::signal::Signal::SIGKILL,
                                        );
                                        tracing::debug!(
                                            %pid,
                                            "VTE child ignored SIGTERM, sent SIGKILL"
                                        );
                                    } else {
                                        tracing::debug!(
                                            %pid,
                                            "PID recycled (pgid mismatch), skipping SIGKILL"
                                        );
                                    }
                                }
                            },
                        );
                    }
                    tracing::debug!(
                        session = %session_id,
                        %pid,
                        "Killed VTE child process group on tab close"
                    );
                }

                // Drop SSH tunnel — the SshTunnel::drop impl kills the SSH process
                ssh_tunnels.borrow_mut().remove(&session_id);

                // Remove tab page container
                tab_containers.borrow_mut().remove(&session_id);
            }

            // Confirm close
            view.close_page_finish(page, true);

            // If no more sessions, show welcome page
            if sessions.borrow().is_empty() && tab_view.n_pages() == 0 {
                let welcome = Self::create_welcome_tab();
                let welcome_wrap = TabPageContainer::welcome(&welcome.upcast::<gtk4::Widget>());
                let welcome_page = tab_view.append(welcome_wrap.widget());
                welcome_page.set_title(&i18n("Welcome"));
                welcome_page.set_icon(Some(&gio::ThemedIcon::new("go-home-symbolic")));
            }

            glib::Propagation::Stop
        });
    }

    /// Creates the welcome tab content - uses the full welcome screen with features
    fn create_welcome_tab() -> GtkBox {
        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);

        // Use the full welcome content from SplitViewBridge for consistency
        let status_page = crate::split_view::SplitViewBridge::create_welcome_content_static();
        container.append(&status_page);
        container
    }

    /// Gets the icon name for a protocol
    fn get_protocol_icon(protocol: &str) -> &'static str {
        rustconn_core::get_protocol_icon_by_name(protocol)
    }

    /// Removes the welcome page if it exists
    fn remove_welcome_page(&self) {
        if self.sessions.borrow().is_empty() && self.tab_view.n_pages() > 0 {
            // Find and remove welcome page
            for i in 0..self.tab_view.n_pages() {
                let page = self.tab_view.nth_page(i);
                if page.title() == i18n("Welcome") {
                    self.tab_view.close_page(&page);
                    break;
                }
            }
        }
    }

    /// Creates a new terminal tab for an SSH session with default settings
    pub fn create_terminal_tab(
        &self,
        connection_id: Uuid,
        title: &str,
        protocol: &str,
        automation: Option<&AutomationConfig>,
    ) -> Uuid {
        self.create_terminal_tab_with_settings(
            connection_id,
            title,
            protocol,
            automation,
            &rustconn_core::config::TerminalSettings::default(),
            None,
            &[], // no variables for default tab
        )
    }

    /// Creates a new terminal tab with specific settings
    ///
    /// When `theme_override` is `Some`, the per-connection colors are applied
    /// on top of the global theme. When `None`, the global theme is used as-is.
    ///
    /// `global_variables` are used to substitute `${VAR}` references in
    /// Expect-rule responses before the automation session is created.
    #[expect(
        clippy::too_many_arguments,
        reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
    )]
    pub fn create_terminal_tab_with_settings(
        &self,
        connection_id: Uuid,
        title: &str,
        protocol: &str,
        automation: Option<&AutomationConfig>,
        settings: &rustconn_core::config::TerminalSettings,
        theme_override: Option<&rustconn_core::models::ConnectionThemeOverride>,
        global_variables: &[rustconn_core::Variable],
    ) -> Uuid {
        let session_id = Uuid::new_v4();
        self.remove_welcome_page();

        let terminal = Terminal::new();
        terminal.set_hexpand(true);
        terminal.set_vexpand(true);

        // Focus-based accelerator suspend (#197): when the VTE gains focus,
        // single-Ctrl chords (Ctrl+F/P/N…) must reach the shell instead of the
        // app accelerators; restore them when focus leaves. The actual
        // suspend/restore (and the `terminal_passthrough_ctrl` setting) is
        // decided by the listener wired via `set_on_terminal_focus`.
        self.attach_focus_passthrough(&terminal);

        // Build a VariableManager for substituting ${VAR} in Expect responses
        let var_manager = {
            let mut mgr = rustconn_core::variables::VariableManager::new();
            for var in global_variables {
                mgr.set_global(var.clone());
            }
            mgr
        };

        // Setup automation if configured
        if let Some(cfg) = automation
            && !cfg.expect_rules.is_empty()
        {
            let rules = prepare_rules_from_config(&cfg.expect_rules, &var_manager);

            if !rules.is_empty() {
                let session = AutomationSession::new(terminal.clone(), rules);
                self.automation_sessions
                    .borrow_mut()
                    .insert(session_id, session);
            }
        }

        // Apply user settings
        config::configure_terminal_with_settings(&terminal, settings);

        // Apply per-connection theme override (if present) on top of the global theme
        if let Some(override_colors) = theme_override {
            let base_theme = TerminalTheme::by_name(&settings.color_theme)
                .unwrap_or_else(TerminalTheme::dark_theme);
            config::apply_theme_override_with_base(&terminal, override_colors, &base_theme);
        }

        // VTE implements GtkScrollable natively — no ScrolledWindow needed.
        // Wrapping in ScrolledWindow intercepts mouse events and breaks
        // ncurses apps (mc, htop) that rely on VTE's internal mouse handling.
        // Instead, pair VTE with a standalone GtkScrollbar connected to its
        // vadjustment — the same approach used by GNOME Terminal.
        let terminal_row = GtkBox::new(Orientation::Horizontal, 0);
        terminal_row.set_hexpand(true);
        terminal_row.set_vexpand(true);
        terminal_row.append(&terminal);

        if settings.show_scrollbar {
            let scrollbar =
                gtk4::Scrollbar::new(Orientation::Vertical, terminal.vadjustment().as_ref());
            terminal_row.append(&scrollbar);
        }

        // Wrap terminal_row in an Overlay so the highlight DrawingArea can
        // be layered on top without interfering with VTE input.
        let terminal_overlay = gtk4::Overlay::new();
        terminal_overlay.set_child(Some(&terminal_row));
        terminal_overlay.set_hexpand(true);
        terminal_overlay.set_vexpand(true);

        // Outer vertical container: terminal row on top, monitoring bar below.
        // get_session_container() returns this box so monitoring can append to it.
        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);
        container.append(&terminal_overlay);

        // Right-click context menu actions installed on the terminal widget
        // so they follow it when reparented between TabView and split view.
        config::setup_context_menu(&terminal, &self.snippet_menu_section);

        // Drag-and-drop: insert shell-escaped file paths when files are
        // dragged from a file manager onto the terminal (GNOME Terminal behavior).
        file_drop::setup_file_drop_target(&terminal);

        // Wrap in TabPageContainer to guarantee non-zero allocation for TabOverview
        let tab_container = TabPageContainer::single(&container);

        // Add page to TabView — child is the TabPageContainer outer box
        let page = self.tab_view.append(tab_container.widget());
        page.set_title(title);
        page.set_icon(Some(&gio::ThemedIcon::new(Self::get_protocol_icon(
            protocol,
        ))));
        page.set_tooltip(title);

        // Store session data
        self.sessions.borrow_mut().insert(session_id, page.clone());
        let terminal_for_focus = terminal.clone();
        self.terminals.borrow_mut().insert(session_id, terminal);
        self.terminal_overlays
            .borrow_mut()
            .insert(session_id, terminal_overlay);
        self.tab_containers
            .borrow_mut()
            .insert(session_id, tab_container);

        self.session_info.borrow_mut().insert(
            session_id,
            TerminalSession {
                id: session_id,
                connection_id,
                name: title.to_string(),
                protocol: protocol.to_string(),
                is_embedded: true,
                log_file: None,
                history_entry_id: None,
                tab_group: None,
                tab_color_index: None,
                connected_at: chrono::Utc::now(),
            },
        );

        // Select the new page
        self.tab_view.set_selected_page(&page);

        // Auto-focus the terminal so the user can type immediately (#79).
        // Use idle_add_local_once so the focus request runs after the page
        // is fully mapped, and only if this page is still selected (avoids
        // focus-stealing when multiple tabs open in quick succession).
        let tab_view_focus = self.tab_view.clone();
        let page_focus = page.clone();
        let terminal_focus = terminal_for_focus;
        glib::idle_add_local_once(move || {
            if tab_view_focus.selected_page().as_ref() == Some(&page_focus) {
                terminal_focus.grab_focus();
            }
        });

        // Apply protocol color indicator if enabled
        if *self.color_tabs_by_protocol.borrow() {
            self.apply_protocol_color(session_id, protocol);
        }

        // Resolve any pending cluster registration for this connection
        self.resolve_cluster_pending(connection_id, session_id);

        // Notify listeners that a new terminal session was created.
        // Single choke point for per-session wiring (activity monitoring):
        // fires for every terminal protocol and for both synchronous and
        // async (port-checked) connection paths, regardless of which connect
        // action started the session.
        if let Some(ref callback) = *self.on_session_created.borrow() {
            callback(session_id, connection_id);
        }

        self.notify_tab_added(session_id, connection_id);

        session_id
    }

    /// Creates a new VNC session tab
    pub fn create_vnc_session_tab(&self, connection_id: Uuid, title: &str) -> Uuid {
        self.create_vnc_session_tab_with_host(connection_id, title, "")
    }

    /// Creates a new VNC session tab with host information
    pub fn create_vnc_session_tab_with_host(
        &self,
        connection_id: Uuid,
        title: &str,
        host: &str,
    ) -> Uuid {
        let session_id = Uuid::new_v4();
        self.remove_welcome_page();

        let vnc_widget = Rc::new(VncSessionWidget::new());

        // #197: suspend single-Ctrl accelerators while the viewer has focus.
        self.attach_focus_passthrough(vnc_widget.widget());

        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);
        container.append(vnc_widget.widget());

        let tab_container = TabPageContainer::single(&container);
        let page = self.tab_view.append(tab_container.widget());
        page.set_title(title);
        page.set_icon(Some(&gio::ThemedIcon::new(
            "video-joined-displays-symbolic",
        )));
        let tooltip = if host.is_empty() {
            title.to_string()
        } else {
            format!("{title}\n{host}")
        };
        page.set_tooltip(&tooltip);

        self.sessions.borrow_mut().insert(session_id, page.clone());
        // Register the container so split (switch_tab_to_split) and unsplit /
        // close-pane (reparent_terminal_to_tab) can swap this tab's content.
        self.tab_containers
            .borrow_mut()
            .insert(session_id, tab_container);
        self.session_widgets
            .borrow_mut()
            .insert(session_id, SessionWidgetStorage::Vnc(vnc_widget));

        self.session_info.borrow_mut().insert(
            session_id,
            TerminalSession {
                id: session_id,
                connection_id,
                name: title.to_string(),
                protocol: "vnc".to_string(),
                is_embedded: true,
                log_file: None,
                history_entry_id: None,
                tab_group: None,
                tab_color_index: None,
                connected_at: chrono::Utc::now(),
            },
        );

        self.tab_view.set_selected_page(&page);
        // Apply protocol color indicator if enabled
        if *self.color_tabs_by_protocol.borrow() {
            self.apply_protocol_color(session_id, "vnc");
        }
        self.notify_tab_added(session_id, connection_id);
        session_id
    }

    /// Adds an embedded RDP tab with the EmbeddedRdpWidget
    pub fn add_embedded_rdp_tab(
        &self,
        session_id: Uuid,
        connection_id: Uuid,
        title: &str,
        widget: Rc<EmbeddedRdpWidget>,
    ) {
        self.remove_welcome_page();

        // #197: suspend single-Ctrl accelerators while the viewer has focus.
        self.attach_focus_passthrough(widget.widget());

        // Wrap in ToastOverlay for file DnD notifications
        let toast_overlay = libadwaita::ToastOverlay::new();
        toast_overlay.set_child(Some(widget.widget()));
        toast_overlay.set_hexpand(true);
        toast_overlay.set_vexpand(true);
        widget.set_toast_overlay(toast_overlay.clone());

        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);
        container.append(&toast_overlay);

        let tab_container = TabPageContainer::single(&container);
        let page = self.tab_view.append(tab_container.widget());
        page.set_title(title);
        page.set_icon(Some(&gio::ThemedIcon::new("computer-symbolic")));
        page.set_tooltip(title);

        self.sessions.borrow_mut().insert(session_id, page.clone());
        // Register the container so split (switch_tab_to_split) and unsplit /
        // close-pane (reparent_terminal_to_tab) can swap this tab's content.
        self.tab_containers
            .borrow_mut()
            .insert(session_id, tab_container);
        self.session_widgets
            .borrow_mut()
            .insert(session_id, SessionWidgetStorage::EmbeddedRdp(widget));

        self.session_info.borrow_mut().insert(
            session_id,
            TerminalSession {
                id: session_id,
                connection_id,
                name: title.to_string(),
                protocol: "rdp".to_string(),
                is_embedded: true,
                log_file: None,
                history_entry_id: None,
                tab_group: None,
                tab_color_index: None,
                connected_at: chrono::Utc::now(),
            },
        );

        self.tab_view.set_selected_page(&page);
        // Apply protocol color indicator if enabled
        if *self.color_tabs_by_protocol.borrow() {
            self.apply_protocol_color(session_id, "rdp");
        }
        self.notify_tab_added(session_id, connection_id);
    }

    /// Adds an embedded Web browser tab with the `EmbeddedWebWidget`.
    ///
    /// Creates a new tab page, stores the widget as
    /// `SessionWidgetStorage::EmbeddedWeb`, and selects the page.
    #[cfg(feature = "web-embedded")]
    pub fn add_embedded_web_tab(
        &self,
        session_id: Uuid,
        connection_id: Uuid,
        title: &str,
        widget: Rc<crate::embedded_web::EmbeddedWebWidget>,
    ) {
        self.remove_welcome_page();

        // #197: suspend single-Ctrl accelerators while the viewer has focus.
        self.attach_focus_passthrough(widget.widget());

        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);
        container.append(widget.widget());

        let tab_container = TabPageContainer::single(&container);
        let page = self.tab_view.append(tab_container.widget());
        page.set_title(title);
        page.set_icon(Some(&gio::ThemedIcon::new("web-browser-symbolic")));
        page.set_tooltip(title);

        self.sessions.borrow_mut().insert(session_id, page.clone());
        self.tab_containers
            .borrow_mut()
            .insert(session_id, tab_container);
        self.session_widgets
            .borrow_mut()
            .insert(session_id, SessionWidgetStorage::EmbeddedWeb(widget));

        self.session_info.borrow_mut().insert(
            session_id,
            TerminalSession {
                id: session_id,
                connection_id,
                name: title.to_string(),
                protocol: "web".to_string(),
                is_embedded: true,
                log_file: None,
                history_entry_id: None,
                tab_group: None,
                tab_color_index: None,
                connected_at: chrono::Utc::now(),
            },
        );

        self.tab_view.set_selected_page(&page);
        // Apply protocol color indicator if enabled
        if *self.color_tabs_by_protocol.borrow() {
            self.apply_protocol_color(session_id, "web");
        }
        self.notify_tab_added(session_id, connection_id);
    }

    /// Adds an embedded session tab (for RDP/VNC external processes)
    pub fn add_embedded_session_tab(
        &self,
        session_id: Uuid,
        connection_id: Uuid,
        title: &str,
        protocol: &str,
        widget: &GtkBox,
        process: Option<Rc<RefCell<Option<std::process::Child>>>>,
    ) {
        self.remove_welcome_page();

        let tab_container = TabPageContainer::single(widget);
        let page = self.tab_view.append(tab_container.widget());
        page.set_title(title);
        page.set_icon(Some(&gio::ThemedIcon::new(Self::get_protocol_icon(
            protocol,
        ))));
        page.set_tooltip(title);

        self.sessions.borrow_mut().insert(session_id, page.clone());

        // Store external process for cleanup on tab close
        if let Some(proc) = process {
            self.session_widgets
                .borrow_mut()
                .insert(session_id, SessionWidgetStorage::ExternalProcess(proc));
        }

        self.session_info.borrow_mut().insert(
            session_id,
            TerminalSession {
                id: session_id,
                connection_id,
                name: title.to_string(),
                protocol: protocol.to_string(),
                is_embedded: false,
                log_file: None,
                history_entry_id: None,
                tab_group: None,
                tab_color_index: None,
                connected_at: chrono::Utc::now(),
            },
        );

        self.tab_view.set_selected_page(&page);
        // Apply protocol color indicator if enabled
        if *self.color_tabs_by_protocol.borrow() {
            self.apply_protocol_color(session_id, protocol);
        }
        self.notify_tab_added(session_id, connection_id);
    }

    /// Gets the VNC session widget for a session
    #[must_use]
    pub fn get_vnc_widget(&self, session_id: Uuid) -> Option<Rc<VncSessionWidget>> {
        let widgets = self.session_widgets.borrow();
        match widgets.get(&session_id) {
            Some(SessionWidgetStorage::Vnc(widget)) => Some(widget.clone()),
            _ => None,
        }
    }

    /// Gets the RDP session widget for a session
    #[must_use]
    pub fn get_rdp_widget(&self, session_id: Uuid) -> Option<Rc<EmbeddedRdpWidget>> {
        let widgets = self.session_widgets.borrow();
        match widgets.get(&session_id) {
            Some(SessionWidgetStorage::EmbeddedRdp(widget)) => Some(widget.clone()),
            _ => None,
        }
    }

    /// Queues a redraw for an RDP widget
    pub fn queue_rdp_redraw(&self, session_id: Uuid) {
        if let Some(widget) = self.get_rdp_widget(session_id) {
            widget.queue_draw();
        }
    }

    /// Gets the session widget (VNC) for a session
    #[must_use]
    pub fn get_session_widget(&self, session_id: Uuid) -> Option<SessionWidget> {
        let widgets = self.session_widgets.borrow();
        if let Some(SessionWidgetStorage::Vnc(_)) = widgets.get(&session_id) {
            Some(SessionWidget::Vnc(VncSessionWidget::new()))
        } else {
            drop(widgets);
            self.terminals
                .borrow()
                .get(&session_id)
                .map(|terminal| SessionWidget::Ssh(terminal.clone()))
        }
    }

    /// Gets the GTK widget for a session (for display in split view)
    #[must_use]
    pub fn get_session_display_widget(&self, session_id: Uuid) -> Option<Widget> {
        let widgets = self.session_widgets.borrow();
        if let Some(storage) = widgets.get(&session_id) {
            return match storage {
                SessionWidgetStorage::Vnc(widget) => Some(widget.widget().clone()),
                SessionWidgetStorage::EmbeddedRdp(widget) => Some(widget.widget().clone().upcast()),
                #[cfg(feature = "web-embedded")]
                SessionWidgetStorage::EmbeddedWeb(widget) => Some(widget.widget().clone().upcast()),
                SessionWidgetStorage::ExternalProcess(_) => None,
            };
        }
        drop(widgets);

        self.terminals
            .borrow()
            .get(&session_id)
            .map(|t| t.clone().upcast())
    }

    /// Reports whether a session can be split, keyed on its stored widget kind.
    #[must_use]
    pub fn split_eligibility(&self, session_id: Uuid) -> SplitEligibility {
        // Scope each borrow so we never hold two RefCell borrows at once.
        let from_widget = {
            let widgets = self.session_widgets.borrow();
            widgets
                .get(&session_id)
                .map(|storage| eligibility_from(false, Some(storage)))
        };
        if let Some(eligibility) = from_widget {
            return eligibility;
        }

        let has_terminal = self.terminals.borrow().contains_key(&session_id);
        eligibility_from(has_terminal, None)
    }

    /// Gets the session state for a VNC session
    #[must_use]
    pub fn get_session_state(&self, session_id: Uuid) -> Option<SessionState> {
        let widgets = self.session_widgets.borrow();
        match widgets.get(&session_id) {
            Some(SessionWidgetStorage::Vnc(widget)) => Some(widget.state()),
            _ => None,
        }
    }

    /// Spawns a command in the terminal
    pub fn spawn_command(
        &self,
        session_id: Uuid,
        argv: &[&str],
        envv: Option<&[&str]>,
        working_directory: Option<&str>,
        ssh_agent_socket: Option<&str>,
    ) -> bool {
        let terminals = self.terminals.borrow();
        let Some(terminal) = terminals.get(&session_id) else {
            return false;
        };

        let argv_gstr: Vec<glib::GString> = argv.iter().map(|s| glib::GString::from(*s)).collect();
        let argv_refs: Vec<&str> = argv_gstr.iter().map(gtk4::glib::GString::as_str).collect();

        // Inherit the current process environment so that child
        // processes see SSH_AUTH_SOCK, HOME, TERM, DISPLAY, etc.
        // Then override PATH with our extended version (Flatpak CLI
        // tools) and layer any caller-provided variables on top.
        let extended_path = rustconn_core::cli_download::get_extended_path();

        let mut env_vec: Vec<glib::GString> = Vec::new();

        // Start with the full parent environment
        for (key, value) in std::env::vars() {
            if key == "PATH" {
                // Replace PATH with our extended version
                env_vec.push(glib::GString::from(format!("PATH={extended_path}")));
            } else {
                env_vec.push(glib::GString::from(format!("{key}={value}")));
            }
        }

        // If PATH wasn't in the parent env, add it explicitly
        if std::env::var("PATH").is_err() {
            env_vec.push(glib::GString::from(format!("PATH={extended_path}")));
        }

        // Inject SSH agent env: custom socket override takes priority,
        // then OnceLock agent info, then inherited environment.
        if let Some(custom_socket) = ssh_agent_socket {
            env_vec.retain(|e| !e.starts_with("SSH_AUTH_SOCK="));
            env_vec.push(glib::GString::from(format!(
                "SSH_AUTH_SOCK={custom_socket}"
            )));
        } else if let Some(agent_info) = rustconn_core::sftp::get_agent_info() {
            env_vec.retain(|e| !e.starts_with("SSH_AUTH_SOCK="));
            env_vec.push(glib::GString::from(format!(
                "SSH_AUTH_SOCK={}",
                agent_info.socket_path
            )));
            if let Some(ref pid) = agent_info.pid {
                env_vec.retain(|e| !e.starts_with("SSH_AGENT_PID="));
                env_vec.push(glib::GString::from(format!("SSH_AGENT_PID={pid}")));
            }
        }

        // Strip host SSH_ASKPASS — RustConn handles password input via
        // VTE feed_child() injection, so the host askpass program (e.g.
        // ksshaskpass on KDE) is never needed and may not exist inside
        // sandboxed environments like Flatpak (#48).
        env_vec.retain(|e| !e.starts_with("SSH_ASKPASS="));

        // On macOS, SSH may still try the compiled-in default askpass path
        // (e.g. /usr/X11R6/bin/ssh-askpass from XQuartz) even when SSH_ASKPASS
        // is unset. Setting SSH_ASKPASS_REQUIRE=never tells OpenSSH ≥8.4 to
        // never invoke an external askpass program. (#161)
        #[cfg(target_os = "macos")]
        {
            env_vec.retain(|e| !e.starts_with("SSH_ASKPASS_REQUIRE="));
            env_vec.push(glib::GString::from("SSH_ASKPASS_REQUIRE=never"));
        }

        // In Flatpak, redirect CLI config directories to writable sandbox
        // locations. Host directories are either mounted read-only (gcloud,
        // Azure, kubectl) or not mounted at all (Teleport, Boundary, etc.).
        if rustconn_core::flatpak::is_flatpak() {
            // gcloud: ~/.config/gcloud/ mounted :ro
            if !env_vec.iter().any(|e| e.starts_with("CLOUDSDK_CONFIG="))
                && let Some(dir) = rustconn_core::flatpak::get_flatpak_gcloud_config_dir()
            {
                env_vec.push(glib::GString::from(format!(
                    "CLOUDSDK_CONFIG={}",
                    dir.display()
                )));
            }
            // Azure CLI: ~/.azure/ mounted :ro
            if !env_vec.iter().any(|e| e.starts_with("AZURE_CONFIG_DIR="))
                && let Some(dir) = rustconn_core::flatpak::get_flatpak_azure_config_dir()
            {
                env_vec.push(glib::GString::from(format!(
                    "AZURE_CONFIG_DIR={}",
                    dir.display()
                )));
            }
            // Teleport: ~/.tsh/ not mounted — TELEPORT_HOME redirects
            // tsh config/data directory (default ~/.tsh)
            if !env_vec.iter().any(|e| e.starts_with("TELEPORT_HOME="))
                && let Some(dir) = rustconn_core::flatpak::get_flatpak_teleport_config_dir()
            {
                env_vec.push(glib::GString::from(format!(
                    "TELEPORT_HOME={}",
                    dir.display()
                )));
            }
            // Boundary: uses system keyring via D-Bus (org.freedesktop.secrets)
            // which works in Flatpak — no env var redirection needed.
            //
            // Cloudflare Tunnel: `cloudflared access ssh` uses browser-based
            // auth with short-lived tokens — no persistent config dir needed
            // for the SSH proxy use case.
            // OCI CLI: ~/.oci/ not mounted
            if !env_vec
                .iter()
                .any(|e| e.starts_with("OCI_CLI_CONFIG_FILE="))
                && let Some(dir) = rustconn_core::flatpak::get_flatpak_oci_config_dir()
            {
                env_vec.push(glib::GString::from(format!(
                    "OCI_CLI_CONFIG_FILE={}",
                    dir.join("config").display()
                )));
            }
        }
        // In a snap, mirror the Flatpak redirection above using the snap's
        // writable user-data CLI config dirs. The personal-files plugs expose
        // host credentials read-only, so writable config must live under
        // $SNAP_USER_DATA (see rustconn_core::snap::get_snap_cli_config_dir).
        else if rustconn_core::is_snap() {
            if !env_vec.iter().any(|e| e.starts_with("CLOUDSDK_CONFIG="))
                && let Some(dir) = rustconn_core::snap::get_snap_gcloud_config_dir()
            {
                env_vec.push(glib::GString::from(format!(
                    "CLOUDSDK_CONFIG={}",
                    dir.display()
                )));
            }
            if !env_vec.iter().any(|e| e.starts_with("AZURE_CONFIG_DIR="))
                && let Some(dir) = rustconn_core::snap::get_snap_azure_config_dir()
            {
                env_vec.push(glib::GString::from(format!(
                    "AZURE_CONFIG_DIR={}",
                    dir.display()
                )));
            }
            if !env_vec.iter().any(|e| e.starts_with("TELEPORT_HOME="))
                && let Some(dir) = rustconn_core::snap::get_snap_teleport_config_dir()
            {
                env_vec.push(glib::GString::from(format!(
                    "TELEPORT_HOME={}",
                    dir.display()
                )));
            }
            if !env_vec
                .iter()
                .any(|e| e.starts_with("OCI_CLI_CONFIG_FILE="))
                && let Some(dir) = rustconn_core::snap::get_snap_oci_config_dir()
            {
                env_vec.push(glib::GString::from(format!(
                    "OCI_CLI_CONFIG_FILE={}",
                    dir.join("config").display()
                )));
            }
        }

        // Ensure TERM is set. GUI applications (like RustConn) typically
        // don't have TERM in their environment. Without it, ncurses-based
        // programs (mc, htop, etc.) can't detect terminal capabilities
        // including mouse support, causing raw escape sequences to appear
        // as text artifacts. VTE doesn't auto-add TERM when envv is provided.
        //
        // Always use xterm-256color for VTE child processes.
        // In Flatpak the sandbox may inherit TERM=dumb; outside Flatpak
        // GUI apps typically don't have TERM set at all. xterm-256color
        // is universally available and provides full color + mouse support.
        // MC is launched with `-g` (--oldmouse) to force X10 mouse mode
        // regardless of the XM terminfo capability.
        if !env_vec.iter().any(|e| e.starts_with("TERM=")) {
            env_vec.push(glib::GString::from("TERM=xterm-256color"));
        } else if rustconn_core::flatpak::is_flatpak() || env_vec.iter().any(|e| e == "TERM=dumb") {
            env_vec.retain(|e| !e.starts_with("TERM="));
            env_vec.push(glib::GString::from("TERM=xterm-256color"));
        }

        // Layer caller-provided variables (override parent values)
        if let Some(user_env) = envv {
            for e in user_env {
                // Remove any existing entry with the same key
                if let Some(eq_pos) = e.find('=') {
                    let key_prefix = &e[..=eq_pos];
                    env_vec.retain(|existing| !existing.starts_with(key_prefix));
                }
                env_vec.push(glib::GString::from(*e));
            }
        }

        let env_refs: Vec<&str> = env_vec.iter().map(gtk4::glib::GString::as_str).collect();

        // Capture command name for error reporting
        let command_name = argv.first().unwrap_or(&"").to_string();

        // Capture Rc references for the spawn error callback
        let sessions_rc = self.sessions.clone();
        let session_info_rc = self.session_info.clone();
        let on_reconnect_rc = self.on_reconnect.clone();
        let vte_child_pids_rc = self.vte_child_pids.clone();

        tracing::debug!(
            command = %command_name,
            %session_id,
            argv = ?argv_refs,
            working_directory = ?working_directory,
            env_count = env_refs.len(),
            "Spawning command via VTE spawn_async"
        );

        // On macOS, VTE's built-in spawn_async doesn't connect PTY to child
        // process output (known Homebrew VTE issue). Use native PTY instead.
        #[cfg(target_os = "macos")]
        {
            match crate::macos_pty::spawn_native_pty(
                terminal,
                &argv_refs,
                &env_refs,
                working_directory,
            ) {
                Ok(pid) => {
                    vte_child_pids_rc
                        .borrow_mut()
                        .insert(session_id, pid as i32);
                    tracing::info!(
                        command = %command_name,
                        %session_id,
                        %pid,
                        "Command spawned successfully (macOS native PTY)"
                    );
                }
                Err(e) => {
                    tracing::error!(
                        command = %command_name,
                        %session_id,
                        %e,
                        "Failed to spawn command (macOS native PTY)"
                    );

                    // A missing executable surfaces as a NotFound spawn error
                    // ("No such file or directory" / os error 2). Anything else
                    // (PTY allocation, fd dup, controlling-terminal setup) is a
                    // genuine failure the user must see verbatim instead of a
                    // misleading "not installed" message.
                    let not_found =
                        e.contains("No such file or directory") || e.contains("os error 2");
                    let banner_msg = if not_found {
                        i18n_f("Command not found: {}", &[&command_name])
                    } else {
                        i18n_f("Failed to start '{}'", &[&command_name])
                    };

                    // Mark tab as disconnected and show reconnect overlay
                    if let Some(page) = sessions_rc.borrow().get(&session_id) {
                        page.set_indicator_icon(Some(&gio::ThemedIcon::new(
                            "network-offline-symbolic",
                        )));
                        page.set_indicator_activatable(false);

                        // Build reconnect banner inside the tab container
                        if let Ok(outer) = page.child().downcast::<GtkBox>()
                            && let Some(inner) = outer.first_child()
                            && let Ok(container) = inner.downcast::<GtkBox>()
                        {
                            let info = session_info_rc.borrow();
                            let connection_id = info
                                .get(&session_id)
                                .map(|i| i.connection_id)
                                .unwrap_or(Uuid::nil());
                            drop(info);

                            let banner = GtkBox::new(Orientation::Horizontal, 6);
                            banner.set_margin_start(12);
                            banner.set_margin_end(12);
                            banner.set_margin_top(6);
                            banner.set_margin_bottom(6);
                            banner.set_halign(gtk4::Align::Center);
                            banner.set_widget_name("reconnect-banner");

                            let msg = banner_msg.clone();
                            let label = gtk4::Label::new(Some(&msg));
                            label.add_css_class("dim-label");

                            let button = gtk4::Button::with_label(&i18n("Reconnect"));
                            button.add_css_class("suggested-action");
                            button.set_tooltip_text(Some(&i18n("Reconnect to this session")));

                            banner.append(&label);
                            banner.append(&button);
                            container.append(&banner);

                            let on_reconnect = on_reconnect_rc.clone();
                            button.connect_clicked(move |_| {
                                if let Some(ref cb) = *on_reconnect.borrow() {
                                    cb(session_id, connection_id);
                                }
                            });
                        }
                    }

                    // Show toast on the nearest window
                    let msg = if not_found {
                        i18n_f("'{}' is not installed", &[&command_name])
                    } else {
                        i18n_f("Failed to start '{}': {}", &[&command_name, &e])
                    };
                    crate::toast::show_error_toast_on_active_window(&msg);
                }
            }
            true
        }

        #[cfg(not(target_os = "macos"))]
        {
            terminal.spawn_async(
                PtyFlags::DEFAULT,
                working_directory,
                &argv_refs,
                &env_refs,
                glib::SpawnFlags::SEARCH_PATH_FROM_ENVP,
                || {},
                -1,
                gio::Cancellable::NONE,
                move |result| {
                    if let Ok(pid) = &result {
                        vte_child_pids_rc.borrow_mut().insert(session_id, pid.0);
                        tracing::info!(
                            command = %command_name,
                            %session_id,
                            pid = pid.0,
                            "Command spawned successfully"
                        );
                    }
                    if let Err(e) = result {
                        tracing::error!(
                            command = %command_name,
                            %session_id,
                            %e,
                            "Failed to spawn command"
                        );

                        // Mark tab as disconnected and show reconnect overlay
                        if let Some(page) = sessions_rc.borrow().get(&session_id) {
                            page.set_indicator_icon(Some(&gio::ThemedIcon::new(
                                "network-offline-symbolic",
                            )));
                            page.set_indicator_activatable(false);

                            // Build reconnect banner inside the tab container
                            if let Ok(outer) = page.child().downcast::<GtkBox>()
                                && let Some(inner) = outer.first_child()
                                && let Ok(container) = inner.downcast::<GtkBox>()
                            {
                                let info = session_info_rc.borrow();
                                let connection_id = info
                                    .get(&session_id)
                                    .map(|i| i.connection_id)
                                    .unwrap_or(Uuid::nil());
                                drop(info);

                                let banner = GtkBox::new(Orientation::Horizontal, 6);
                                banner.set_margin_start(12);
                                banner.set_margin_end(12);
                                banner.set_margin_top(6);
                                banner.set_margin_bottom(6);
                                banner.set_halign(gtk4::Align::Center);
                                banner.set_widget_name("reconnect-banner");

                                let msg = i18n_f("Command not found: {}", &[&command_name]);
                                let label = gtk4::Label::new(Some(&msg));
                                label.add_css_class("dim-label");

                                let button = gtk4::Button::with_label(&i18n("Reconnect"));
                                button.add_css_class("suggested-action");
                                button.set_tooltip_text(Some(&i18n("Reconnect to this session")));

                                banner.append(&label);
                                banner.append(&button);
                                container.append(&banner);

                                let on_reconnect = on_reconnect_rc.clone();
                                button.connect_clicked(move |_| {
                                    if let Some(ref cb) = *on_reconnect.borrow() {
                                        cb(session_id, connection_id);
                                    }
                                });
                            }
                        }

                        // Show toast on the nearest window
                        let msg = i18n_f("'{}' is not installed", &[&command_name]);
                        crate::toast::show_error_toast_on_active_window(&msg);
                    }
                },
            );

            true
        }
    }

    /// Spawns an SSH command in the terminal
    #[expect(
        clippy::too_many_arguments,
        reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
    )]
    pub fn spawn_ssh(
        &self,
        session_id: Uuid,
        host: &str,
        port: u16,
        username: Option<&str>,
        identity_file: Option<&str>,
        extra_args: &[&str],
        use_waypipe: bool,
        ssh_agent_socket: Option<&str>,
        startup_command: Option<&str>,
        extra_env: Option<&[&str]>,
    ) -> bool {
        let mut argv = if use_waypipe {
            vec!["waypipe", "ssh"]
        } else {
            vec!["ssh"]
        };

        let port_str;
        if port != 22 {
            port_str = port.to_string();
            argv.push("-p");
            argv.push(&port_str);
        }

        if let Some(key) = identity_file {
            argv.push("-i");
            argv.push(key);
        }

        // Always enable ControlMaster so monitoring can multiplex over the
        // same authenticated connection without a second key/passphrase prompt.
        // If the user already set ControlMaster via extra_args (build_command_args),
        // skip to avoid duplicates. But always ensure ControlPath is set to the
        // shared path so monitoring can find the socket.

        let has_control_master = extra_args.iter().any(|a| a.contains("ControlMaster"));
        let has_control_path = extra_args.iter().any(|a| a.contains("ControlPath"));
        let control_path_opt = format!(
            "ControlPath={}",
            rustconn_core::ssh_control_path(host, port)
        );
        if !has_control_master {
            argv.push("-o");
            argv.push("ControlMaster=auto");
            argv.push("-o");
            argv.push(&control_path_opt);
            argv.push("-o");
            // ponytail: 60s persist keeps the master alive briefly for monitoring
            // multiplex, but dies fast after network changes (#217). Was 10m.
            argv.push("ControlPersist=60");
        } else if !has_control_path {
            // User enabled ControlMaster manually but no ControlPath —
            // add our shared path so monitoring can reuse the socket.
            argv.push("-o");
            argv.push(&control_path_opt);
        }

        // In Flatpak, ~/.ssh is read-only — use a writable known_hosts path
        // unless the caller already set UserKnownHostsFile via extra_args
        let kh_option;
        let has_known_hosts_opt = extra_args.iter().any(|a| a.contains("UserKnownHostsFile"));
        if !has_known_hosts_opt && let Some(kh_path) = rustconn_core::get_flatpak_known_hosts_path()
        {
            kh_option = format!("UserKnownHostsFile={}", kh_path.display());
            argv.push("-o");
            argv.push(&kh_option);
        }

        // Default keep-alive: detect dead connections within ~45s (15s × 3)
        // so auto-reconnect triggers promptly after network changes (#217).
        // Skip if user already configured via SshConfig.keep_alive_interval
        // (which lands in extra_args from build_command_args).
        // NOTE: This overrides any ServerAliveInterval set in ~/.ssh/config
        // because CLI -o takes precedence. Users who want to respect their
        // ssh_config value should set the keep-alive in the connection editor
        // (even to the same value) so it appears in extra_args and skips this.
        let has_server_alive = extra_args.iter().any(|a| a.contains("ServerAliveInterval"));
        let has_alive_count = extra_args.iter().any(|a| a.contains("ServerAliveCountMax"));
        if !has_server_alive {
            argv.push("-o");
            argv.push("ServerAliveInterval=15");
        }
        if !has_alive_count {
            argv.push("-o");
            argv.push("ServerAliveCountMax=3");
        }

        argv.extend(extra_args);

        let destination = if let Some(user) = username {
            format!("{user}@{host}")
        } else {
            host.to_string()
        };
        argv.push(&destination);

        // Append startup command after destination — runs the command and then
        // drops into an interactive login shell so the session stays open.
        // Uses `-t` to force PTY allocation (required for interactive shell after command).
        let startup_wrapped;
        if let Some(cmd) = startup_command {
            // Insert -t before destination to force PTY allocation
            // (skip if already present in extra_args to avoid duplicates)
            if !extra_args.contains(&"-t") {
                let dest_idx = argv.len() - 1;
                argv.insert(dest_idx, "-t");
            }
            // Wrap: run the command, then exec the user's login shell
            startup_wrapped = format!("{cmd}; exec $SHELL -l");
            argv.push(&startup_wrapped);
        }

        self.spawn_command(session_id, &argv, extra_env, None, ssh_agent_socket)
    }

    /// Spawns a Telnet command in the terminal
    ///
    /// Supports configurable backspace/delete key behavior via VTE
    /// `EraseBinding`. Settings are applied directly on the terminal
    /// widget before spawning the telnet process.
    pub fn spawn_telnet(
        &self,
        session_id: Uuid,
        host: &str,
        port: u16,
        extra_args: &[&str],
        backspace_sends: rustconn_core::models::TelnetBackspaceSends,
        delete_sends: rustconn_core::models::TelnetDeleteSends,
    ) -> bool {
        use rustconn_core::models::{TelnetBackspaceSends, TelnetDeleteSends};
        use vte4::EraseBinding;

        // Apply keyboard bindings directly on the VTE terminal
        if let Some(terminal) = self.terminals.borrow().get(&session_id) {
            match backspace_sends {
                TelnetBackspaceSends::Automatic => {
                    terminal.set_backspace_binding(EraseBinding::Auto);
                }
                TelnetBackspaceSends::Backspace => {
                    terminal.set_backspace_binding(EraseBinding::AsciiBackspace);
                }
                TelnetBackspaceSends::Delete => {
                    terminal.set_backspace_binding(EraseBinding::AsciiDelete);
                }
            }
            match delete_sends {
                TelnetDeleteSends::Automatic => {
                    terminal.set_delete_binding(EraseBinding::Auto);
                }
                TelnetDeleteSends::Backspace => {
                    terminal.set_delete_binding(EraseBinding::AsciiBackspace);
                }
                TelnetDeleteSends::Delete => {
                    terminal.set_delete_binding(EraseBinding::AsciiDelete);
                }
            }
        }

        // Spawn telnet directly — no shell wrapper needed
        let mut argv = vec!["telnet"];
        argv.extend(extra_args);
        argv.push(host);
        let port_str = port.to_string();
        argv.push(&port_str);
        self.spawn_command(session_id, &argv, None, None, None)
    }

    /// Spawns a serial connection using picocom in the terminal tab.
    ///
    /// Builds the picocom command from the `SerialConfig` and spawns it
    /// directly in the VTE terminal (no shell wrapper).
    pub fn spawn_serial(&self, session_id: Uuid, command: &[String]) -> bool {
        let argv: Vec<&str> = command.iter().map(String::as_str).collect();
        self.spawn_command(session_id, &argv, None, None, None)
    }

    /// Closes a terminal tab by session ID
    pub fn close_tab(&self, session_id: Uuid) {
        self.reconnect_shown.borrow_mut().remove(&session_id);
        // Cancel any background polling (auto-reconnect, host check) for this session
        self.cancel_poll(session_id);
        let page = self.sessions.borrow().get(&session_id).cloned();
        if let Some(page) = page {
            self.tab_view.close_page(&page);
        }
    }

    /// Prepares an existing disconnected tab for in-place reconnect.
    ///
    /// Instead of closing the old tab and creating a new one (which loses
    /// tab position, scrollback, and causes visual flicker), this method:
    /// 1. Removes the reconnect banner from the tab container
    /// 2. Resets the VTE terminal (clears screen, resets state)
    /// 3. Clears the disconnected indicator
    /// 4. Removes stale automation sessions
    /// 5. Cancels any background polling
    ///
    /// After calling this, the caller can re-use the same `session_id` to
    /// spawn a new process in the existing terminal via `spawn_ssh()` etc.
    ///
    /// Returns `true` if the tab was successfully prepared, `false` if the
    /// session no longer exists (tab was closed by user).
    pub fn prepare_for_reconnect(&self, session_id: Uuid) -> bool {
        // Check that the session still exists
        let page = self.sessions.borrow().get(&session_id).cloned();
        let Some(page) = page else {
            return false;
        };

        // Cancel any background polling (auto-reconnect)
        self.cancel_poll(session_id);

        // Remove reconnect banner from the tab container
        if let Ok(outer) = page.child().downcast::<GtkBox>()
            && let Some(inner) = outer.first_child()
            && let Ok(container) = inner.downcast::<GtkBox>()
        {
            // Find and remove the reconnect-banner widget
            let mut child = container.first_child();
            while let Some(widget) = child {
                let next = widget.next_sibling();
                if widget.widget_name() == "reconnect-banner" {
                    container.remove(&widget);
                }
                child = next;
            }
        }

        // Reset the VTE terminal (clear screen, reset state machine)
        if let Some(terminal) = self.terminals.borrow().get(&session_id) {
            terminal.reset(true, true);
        }

        // Clear disconnected indicator
        page.set_indicator_icon(gio::Icon::NONE);

        // Allow a new reconnect banner to be shown if this reconnect also fails
        self.reconnect_shown.borrow_mut().remove(&session_id);

        // Remove stale automation session (will be re-created by the caller)
        self.automation_sessions.borrow_mut().remove(&session_id);

        // Remove stale highlight rules (will be re-applied by the caller)
        self.session_highlight_rules
            .borrow_mut()
            .remove(&session_id);

        // Remove stale highlight overlay (will be re-created by set_highlight_rules)
        self.highlight_overlays.borrow_mut().remove(&session_id);

        // Remove stale VTE child PID entry — the process should have already
        // exited (child-exited removes it), but if reconnect is triggered
        // before child-exited fires (e.g. timeout disconnect), we must clean
        // it to avoid killing a recycled PID later.
        self.vte_child_pids.borrow_mut().remove(&session_id);

        true
    }

    /// Registers a cancel token for a background polling task
    pub fn register_poll_cancel(
        &self,
        key: Uuid,
        cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) {
        self.poll_cancel_tokens.borrow_mut().insert(key, cancel);
    }

    /// Cancels and removes a background polling task by key
    pub fn cancel_poll(&self, key: Uuid) {
        if let Some(cancel) = self.poll_cancel_tokens.borrow_mut().remove(&key) {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            tracing::debug!(%key, "Cancelled background poll");
        }
    }

    /// Marks a tab as disconnected (changes indicator)
    pub fn mark_tab_disconnected(&self, session_id: Uuid) {
        if let Some(page) = self.sessions.borrow().get(&session_id) {
            page.set_indicator_icon(Some(&gio::ThemedIcon::new("network-offline-symbolic")));
            page.set_indicator_activatable(false);
        }
        // Reset VTE internal state to prevent use-after-free in libvte/pango
        // during the next GTK snapshot cycle. After the child process exits,
        // VTE may hold stale references to Pango font resources that get
        // invalidated (e.g. on screen lock/unlock or GPU context loss).
        // Calling reset(true, false) forces VTE to release internal state
        // (including Pango layout caches) while preserving scrollback history
        // for reconnect (#171).
        if let Some(terminal) = self.terminals.borrow().get(&session_id) {
            terminal.reset(true, false);
        }
    }

    /// Marks a tab as connected (removes the disconnected indicator).
    ///
    /// A split owner's tab uses the same single `indicator-icon` slot to show
    /// its split color, so preserve that here instead of clearing it — otherwise
    /// connection-state events (RDP fires "connected" on every resolution change)
    /// would wipe the split-color indicator.
    pub fn mark_tab_connected(&self, session_id: Uuid) {
        if let Some(&color_index) = self.split_session_colors.borrow().get(&session_id) {
            if let Some(page) = self.sessions.borrow().get(&session_id)
                && let Some(icon) = crate::split_view::create_colored_circle_icon(color_index, 16)
            {
                page.set_indicator_icon(Some(&icon));
                page.set_indicator_activatable(false);
            }
            return;
        }
        if let Some(page) = self.sessions.borrow().get(&session_id) {
            page.set_indicator_icon(gio::Icon::NONE);
        }
    }

    /// Forces every VTE terminal to drop and rebuild its cached font state.
    ///
    /// VTE reads `gtk-fontconfig-timestamp` only when it creates its cached
    /// `FontInfo` (the timestamp is part of the font-cache key) and never
    /// subscribes to changes. After a fontconfig update (font installation,
    /// `fc-cache`, or KDE pushing `Fontconfig/Timestamp` via XSettings on
    /// screen unlock) terminals keep Pango objects that may reference freed
    /// fonts, which crashes with SIGSEGV inside `pango_itemize` during the
    /// next GTK snapshot (#171). Re-applying the current font description
    /// goes through `vte_terminal_set_font`, which deliberately recreates
    /// the font even when the description is unchanged, picking up the new
    /// timestamp and releasing the stale Pango state.
    pub fn refresh_fonts_after_fontconfig_change(&self) {
        for (session_id, terminal) in self.terminals.borrow().iter() {
            let desc = terminal.font_desc();
            terminal.set_font(desc.as_ref());
            tracing::debug!(%session_id, "Refreshed VTE font after fontconfig change");
        }
    }

    /// Shows a reconnect overlay banner at the bottom of a disconnected VTE tab
    ///
    /// Appends a horizontal bar with a "Session disconnected" label and a
    /// "Reconnect" button to the tab's container. The button triggers the
    /// `on_reconnect` callback with the session's connection ID.
    ///
    /// If `auto_reconnect_active` is true, an additional label is shown
    /// indicating that automatic reconnection is in progress.
    pub fn show_reconnect_overlay(&self, session_id: Uuid) {
        self.show_reconnect_overlay_with_status(session_id, false);
    }

    /// Shows a reconnect overlay with optional auto-reconnect status indicator
    pub fn show_reconnect_overlay_with_status(
        &self,
        session_id: Uuid,
        auto_reconnect_active: bool,
    ) {
        // Guard: child-exited can fire twice for the same session; show only one banner
        if !self.reconnect_shown.borrow_mut().insert(session_id) {
            // If banner already shown but auto-reconnect just started, update it
            if auto_reconnect_active {
                self.update_reconnect_banner_status(session_id, true);
            }
            return;
        }

        let Some(page) = self.sessions.borrow().get(&session_id).cloned() else {
            return;
        };
        let Some(info) = self.session_info.borrow().get(&session_id).cloned() else {
            return;
        };

        // Only for VTE-based protocols (SSH, Telnet, Serial, Kubernetes)
        if matches!(info.protocol.as_str(), "rdp" | "vnc" | "spice") {
            return;
        }

        let outer = page.child().downcast::<GtkBox>().ok();
        let Some(outer) = outer else {
            return;
        };
        // Navigate through TabPageContainer outer box to inner content container
        let Some(inner_widget) = outer.first_child() else {
            return;
        };
        let Some(container) = inner_widget.downcast::<GtkBox>().ok() else {
            return;
        };

        // Build the reconnect banner
        let banner = GtkBox::new(Orientation::Horizontal, 6);
        banner.set_margin_start(12);
        banner.set_margin_end(12);
        banner.set_margin_top(6);
        banner.set_margin_bottom(6);
        banner.set_halign(gtk4::Align::Center);
        banner.set_widget_name("reconnect-banner");

        let label = gtk4::Label::new(Some(&i18n("Session disconnected")));
        label.add_css_class("dim-label");

        banner.append(&label);

        // Auto-reconnect status indicator
        if auto_reconnect_active {
            let status_label = gtk4::Label::new(Some(&i18n("Auto-reconnecting…")));
            status_label.add_css_class("dim-label");
            status_label.set_widget_name("reconnect-status");
            banner.append(&status_label);
        }

        let button = gtk4::Button::with_label(&i18n("Reconnect"));
        button.add_css_class("suggested-action");
        button.set_tooltip_text(Some(&i18n("Reconnect to this session")));

        banner.append(&button);
        container.append(&banner);

        // Wire up the reconnect button
        let on_reconnect = self.on_reconnect.clone();
        let connection_id = info.connection_id;
        button.connect_clicked(move |_| {
            if let Some(ref callback) = *on_reconnect.borrow() {
                callback(session_id, connection_id);
            }
        });

        tracing::info!(
            %session_id,
            protocol = %info.protocol,
            "Reconnect overlay shown for disconnected session"
        );
    }

    /// Updates the auto-reconnect status label in an existing reconnect banner
    pub fn update_reconnect_banner_status(&self, session_id: Uuid, active: bool) {
        let Some(page) = self.sessions.borrow().get(&session_id).cloned() else {
            return;
        };
        let outer = page.child().downcast::<GtkBox>().ok();
        let Some(outer) = outer else {
            return;
        };
        let Some(inner_widget) = outer.first_child() else {
            return;
        };
        let Some(container) = inner_widget.downcast::<GtkBox>().ok() else {
            return;
        };

        // Find the reconnect-banner widget
        let mut child = container.first_child();
        while let Some(widget) = child {
            if widget.widget_name() == "reconnect-banner" {
                if let Ok(banner) = widget.downcast::<GtkBox>() {
                    // Check if status label already exists
                    let mut has_status = false;
                    let mut banner_child = banner.first_child();
                    while let Some(bc) = banner_child {
                        if bc.widget_name() == "reconnect-status" {
                            has_status = true;
                            if !active {
                                banner.remove(&bc);
                            }
                            break;
                        }
                        banner_child = bc.next_sibling();
                    }
                    // Add status label if needed and not already present
                    if active && !has_status {
                        let status_label = gtk4::Label::new(Some(&i18n("Auto-reconnecting…")));
                        status_label.add_css_class("dim-label");
                        status_label.set_widget_name("reconnect-status");
                        // Insert before the button (last child)
                        if let Some(button) = banner.last_child() {
                            banner
                                .insert_child_after(&status_label, button.prev_sibling().as_ref());
                        } else {
                            banner.append(&status_label);
                        }
                    }
                }
                break;
            }
            child = widget.next_sibling();
        }
    }

    /// Updates the auto-reconnect status label with attempt progress (N/M)
    pub fn update_reconnect_banner_attempt(
        &self,
        session_id: Uuid,
        attempt: u32,
        max_attempts: u32,
    ) {
        let Some(page) = self.sessions.borrow().get(&session_id).cloned() else {
            return;
        };
        let outer = page.child().downcast::<GtkBox>().ok();
        let Some(outer) = outer else {
            return;
        };
        let Some(inner_widget) = outer.first_child() else {
            return;
        };
        let Some(container) = inner_widget.downcast::<GtkBox>().ok() else {
            return;
        };

        // Find the reconnect-banner widget
        let mut child = container.first_child();
        while let Some(widget) = child {
            if widget.widget_name() == "reconnect-banner" {
                if let Ok(banner) = widget.downcast::<GtkBox>() {
                    // Find or create the status label
                    let mut banner_child = banner.first_child();
                    while let Some(bc) = banner_child {
                        if bc.widget_name() == "reconnect-status" {
                            if let Ok(label) = bc.downcast::<gtk4::Label>() {
                                label.set_label(&i18n_f(
                                    "Auto-reconnecting (attempt {}/{})",
                                    &[&attempt.to_string(), &max_attempts.to_string()],
                                ));
                            }
                            return;
                        }
                        banner_child = bc.next_sibling();
                    }
                    // Status label not found — create it
                    let status_label = gtk4::Label::new(Some(&i18n_f(
                        "Auto-reconnecting (attempt {}/{})",
                        &[&attempt.to_string(), &max_attempts.to_string()],
                    )));
                    status_label.add_css_class("dim-label");
                    status_label.set_widget_name("reconnect-status");
                    if let Some(button) = banner.last_child() {
                        banner.insert_child_after(&status_label, button.prev_sibling().as_ref());
                    } else {
                        banner.append(&status_label);
                    }
                }
                break;
            }
            child = widget.next_sibling();
        }
    }

    /// Sets the callback invoked when a reconnect button is clicked
    ///
    /// The callback receives `(session_id, connection_id)`.
    pub fn set_on_reconnect<F>(&self, callback: F)
    where
        F: Fn(Uuid, Uuid) + 'static,
    {
        *self.on_reconnect.borrow_mut() = Some(Box::new(callback));
    }

    /// Returns a clone of the reconnect callback reference for use in auto-reconnect polling
    #[must_use]
    pub fn reconnect_callback(&self) -> Rc<RefCell<Option<Box<dyn Fn(Uuid, Uuid)>>>> {
        self.on_reconnect.clone()
    }

    /// Returns `true` if the session currently has a reconnect banner displayed.
    ///
    /// Used by the network monitor to identify sessions that need immediate
    /// reconnection after a network interface change.
    #[must_use]
    pub fn is_reconnect_shown(&self, session_id: Uuid) -> bool {
        self.reconnect_shown.borrow().contains(&session_id)
    }

    /// Sets a color indicator on a tab to show it's in a split pane
    /// Applies a colored left border to the tab's title in the TabBar
    pub fn set_tab_split_color(&self, session_id: Uuid, color_index: usize) {
        // Track split color so protocol/clear operations don't overwrite it
        self.split_session_colors
            .borrow_mut()
            .insert(session_id, color_index);

        if let Some(page) = self.sessions.borrow().get(&session_id) {
            // Remove any existing tab color classes from the page's child
            for (_, tab_class) in crate::split_view::SPLIT_PANE_COLORS {
                page.child().remove_css_class(tab_class);
            }
            // Remove old indicator classes
            for i in 0..6 {
                page.child()
                    .remove_css_class(&format!("split-indicator-{}", i));
            }

            // Add the new tab color class to the page's child
            let tab_class = crate::split_view::get_tab_color_class(color_index);
            page.child().add_css_class(tab_class);

            // Add indicator class for potential CSS styling
            let indicator_class = format!("split-indicator-{}", color_index);
            page.child().add_css_class(&indicator_class);

            // Create a colored circle icon for the indicator
            // This provides a visible colored indicator in the tab header
            if let Some(icon) = crate::split_view::create_colored_circle_icon(color_index, 16) {
                page.set_indicator_icon(Some(&icon));
            } else {
                // Fallback to symbolic icon if colored icon creation fails
                let icon = gio::ThemedIcon::new("media-record-symbolic");
                page.set_indicator_icon(Some(&icon));
            }
            page.set_indicator_activatable(false);
        }

        // R6.2: reflect the new split membership in the sidebar marker. The
        // borrows above are scoped to the block, so re-reading the map here is
        // safe.
        self.notify_split_colors_changed();
    }

    /// Removes the split color indicator from a tab
    pub fn clear_tab_split_color(&self, session_id: Uuid) {
        // Remove from split color tracking
        self.split_session_colors.borrow_mut().remove(&session_id);

        if let Some(page) = self.sessions.borrow().get(&session_id) {
            page.set_indicator_icon(gio::Icon::NONE);

            // Remove all tab color classes and indicator classes from the page's child
            let child = page.child();
            for (_, tab_class) in crate::split_view::SPLIT_PANE_COLORS {
                child.remove_css_class(tab_class);
            }
            // Remove indicator classes
            for i in 0..6 {
                child.remove_css_class(&format!("split-indicator-{}", i));
            }
        }

        // R6.2: a session left the split — clear/refresh its sidebar marker.
        self.notify_split_colors_changed();
    }

    /// Sets whether tabs should be colored by protocol type
    pub fn set_color_tabs_by_protocol(&self, enabled: bool) {
        *self.color_tabs_by_protocol.borrow_mut() = enabled;
        // Apply or remove protocol colors on all existing sessions
        let sessions: Vec<(Uuid, String)> = self
            .session_info
            .borrow()
            .iter()
            .map(|(id, info)| (*id, info.protocol.clone()))
            .collect();
        for (session_id, protocol) in sessions {
            if enabled {
                self.apply_protocol_color(session_id, &protocol);
            } else {
                self.clear_protocol_color(session_id);
            }
        }
    }

    /// Applies protocol-based color indicator to a tab
    fn apply_protocol_color(&self, session_id: Uuid, protocol: &str) {
        if let Some(page) = self.sessions.borrow().get(&session_id) {
            // Don't override split colors — split takes priority
            if self.split_session_colors.borrow().contains_key(&session_id) {
                return;
            }
            let (r, g, b) = rustconn_core::get_protocol_color_rgb(protocol);
            if let Some(icon) = Self::create_protocol_color_icon(r, g, b, 16) {
                page.set_indicator_icon(Some(&icon));
                page.set_indicator_activatable(false);
            }
        }
    }

    /// Removes protocol color indicator from a tab
    fn clear_protocol_color(&self, session_id: Uuid) {
        if let Some(page) = self.sessions.borrow().get(&session_id) {
            // Don't clear if split color is active
            if self.split_session_colors.borrow().contains_key(&session_id) {
                return;
            }
            page.set_indicator_icon(gio::Icon::NONE);
        }
    }

    /// Creates a colored circle icon for protocol tab indicators
    fn create_protocol_color_icon(r: u8, g: u8, b: u8, size: u32) -> Option<gio::Icon> {
        // Reuse the same circle-drawing logic as split colors
        let mut rgba_data = vec![0u8; (size * size * 4) as usize];
        let center = size as f32 / 2.0;
        let radius = center - 1.0;

        for y in 0..size {
            for x in 0..size {
                let dx = x as f32 - center;
                let dy = y as f32 - center;
                let distance = dx.hypot(dy);
                let idx = ((y * size + x) * 4) as usize;

                if distance <= radius {
                    let alpha = if distance > radius - 1.0 {
                        ((radius - distance + 1.0) * 255.0) as u8
                    } else {
                        255
                    };
                    rgba_data[idx] = r;
                    rgba_data[idx + 1] = g;
                    rgba_data[idx + 2] = b;
                    rgba_data[idx + 3] = alpha;
                }
            }
        }

        let pixbuf = gtk4::gdk_pixbuf::Pixbuf::from_bytes(
            &glib::Bytes::from(&rgba_data),
            gtk4::gdk_pixbuf::Colorspace::Rgb,
            true,
            8,
            size as i32,
            size as i32,
            (size * 4) as i32,
        );
        let texture = gtk4::gdk::Texture::for_pixbuf(&pixbuf);
        Some(texture.upcast::<gio::Icon>())
    }

    /// Gets the terminal widget for a session
    #[must_use]
    pub fn get_terminal(&self, session_id: Uuid) -> Option<Terminal> {
        self.terminals.borrow().get(&session_id).cloned()
    }

    /// Executes a key sequence on a terminal session
    ///
    /// Sends text, special keys (as VTE escape codes), and handles
    /// `{WAIT:ms}` delays using glib timers.
    pub fn execute_key_sequence(&self, session_id: Uuid, sequence: &KeySequence) {
        let Some(terminal) = self.get_terminal(session_id) else {
            tracing::warn!(%session_id, "Cannot execute key sequence: terminal not found");
            return;
        };

        tracing::info!(
            %session_id,
            elements = sequence.len(),
            "Executing key sequence"
        );

        // Collect elements and schedule them with cumulative delay
        let elements: Vec<KeyElement> = sequence.elements.clone();
        let mut cumulative_delay_ms: u64 = 0;

        for element in elements {
            if let KeyElement::Wait(ms) = &element {
                cumulative_delay_ms += u64::from(*ms);
            } else {
                let terminal_clone = terminal.clone();
                let delay = cumulative_delay_ms;

                match &element {
                    KeyElement::Text(text) => {
                        let text = text.clone();
                        if delay == 0 {
                            terminal_clone.feed_child(text.as_bytes());
                        } else {
                            glib::timeout_add_local_once(
                                std::time::Duration::from_millis(delay),
                                move || {
                                    terminal_clone.feed_child(text.as_bytes());
                                },
                            );
                        }
                    }
                    KeyElement::SpecialKey(key) => {
                        let bytes = key.to_vte_bytes();
                        if delay == 0 {
                            terminal_clone.feed_child(bytes);
                        } else {
                            glib::timeout_add_local_once(
                                std::time::Duration::from_millis(delay),
                                move || {
                                    terminal_clone.feed_child(bytes);
                                },
                            );
                        }
                    }
                    KeyElement::Variable(name) => {
                        // Variables should be substituted before reaching here
                        tracing::warn!(
                            variable = %name,
                            "Unresolved variable in key sequence"
                        );
                    }
                    KeyElement::Wait(_) => unreachable!(),
                }
            }
        }
    }

    /// Gets the cursor row of a terminal session
    ///
    /// VTE's `cursor_position()` returns `(column, row)`.
    pub fn get_terminal_cursor_row(&self, session_id: Uuid) -> Option<i64> {
        self.get_terminal(session_id).map(|t| t.cursor_position().1)
    }

    /// Gets session info for a session
    #[must_use]
    pub fn get_session_info(&self, session_id: Uuid) -> Option<TerminalSession> {
        self.session_info.borrow().get(&session_id).cloned()
    }

    /// Stores an SSH tunnel for a session. The tunnel is killed when the tab closes.
    pub fn store_ssh_tunnel(&self, session_id: Uuid, tunnel: rustconn_core::ssh_tunnel::SshTunnel) {
        self.ssh_tunnels.borrow_mut().insert(session_id, tunnel);
    }

    /// Gets the page container widget for a session
    ///
    /// Returns the `GtkBox` that holds the terminal.
    /// Returns the session's inner content container (the box holding the terminal overlay).
    ///
    /// Used by monitoring to prepend the monitoring bar above the terminal.
    #[must_use]
    pub fn get_session_container(&self, session_id: Uuid) -> Option<GtkBox> {
        let sessions = self.sessions.borrow();
        let page = sessions.get(&session_id)?;
        // page.child() is the TabPageContainer outer box.
        // Its first child is the inner content container (terminal overlay + monitoring bar).
        let outer = page.child();
        let outer_box = outer.downcast_ref::<GtkBox>()?;
        outer_box.first_child()?.downcast::<GtkBox>().ok()
    }

    /// Gets all active sessions
    #[must_use]
    pub fn get_all_sessions(&self) -> Vec<TerminalSession> {
        self.session_info.borrow().values().cloned().collect()
    }

    /// Sets the log file path for a session
    pub fn set_log_file(&self, session_id: Uuid, log_file: PathBuf) {
        if let Some(info) = self.session_info.borrow_mut().get_mut(&session_id) {
            info.log_file = Some(log_file);
        }
    }

    /// Sets the history entry ID for a session
    pub fn set_history_entry_id(&self, session_id: Uuid, history_entry_id: Uuid) {
        if let Some(info) = self.session_info.borrow_mut().get_mut(&session_id) {
            info.history_entry_id = Some(history_entry_id);
        }
    }

    /// Copies selected text from the active terminal to clipboard
    pub fn copy_to_clipboard(&self) {
        if let Some(terminal) = self.get_active_terminal()
            && let Some(text) = terminal.text_selected(vte4::Format::Text)
        {
            terminal.display().clipboard().set_text(&text);
        }
    }

    /// Pastes text from clipboard to the active terminal
    pub fn paste_from_clipboard(&self) {
        if let Some(terminal) = self.get_active_terminal() {
            terminal.paste_clipboard();
        }
    }

    /// Gets the terminal for the currently active tab
    #[must_use]
    pub fn get_active_terminal(&self) -> Option<Terminal> {
        let selected_page = self.tab_view.selected_page()?;
        let sessions = self.sessions.borrow();

        for (session_id, page) in sessions.iter() {
            if page == &selected_page {
                return self.terminals.borrow().get(session_id).cloned();
            }
        }
        None
    }

    /// Gets the session ID for the currently active tab
    #[must_use]
    pub fn get_active_session_id(&self) -> Option<Uuid> {
        let selected_page = self.tab_view.selected_page()?;
        let sessions = self.sessions.borrow();

        for (session_id, page) in sessions.iter() {
            if page == &selected_page {
                return Some(*session_id);
            }
        }
        None
    }

    /// Gets the session ID for a specific page number
    #[must_use]
    pub fn get_session_id_for_page(&self, page_num: u32) -> Option<Uuid> {
        if page_num >= self.tab_view.n_pages() as u32 {
            return None;
        }
        let page = self.tab_view.nth_page(page_num as i32);
        let sessions = self.sessions.borrow();

        for (session_id, stored_page) in sessions.iter() {
            if stored_page == &page {
                return Some(*session_id);
            }
        }
        None
    }

    /// Sends text to the active terminal
    pub fn send_text(&self, text: &str) {
        if let Some(terminal) = self.get_active_terminal() {
            terminal.feed_child(text.as_bytes());
        }
    }

    /// Sends text to a specific terminal session
    pub fn send_text_to_session(&self, session_id: Uuid, text: &str) {
        if let Some(terminal) = self.get_terminal(session_id) {
            terminal.feed_child(text.as_bytes());
        }
    }

    /// Rebuilds the shared snippet menu section based on current app state.
    ///
    /// Call this after snippets are created, edited, or deleted.
    pub fn rebuild_snippet_menu(&self, state: &crate::state::SharedAppState) {
        config::rebuild_snippet_menu_section(&self.snippet_menu_section, state);
    }

    /// Displays output text in a specific terminal session
    pub fn display_output(&self, session_id: Uuid, text: &str) {
        if let Some(terminal) = self.get_terminal(session_id) {
            terminal.feed(text.as_bytes());
        }
    }

    /// Returns the main container widget for this notebook
    #[must_use]
    pub fn widget(&self) -> &GtkBox {
        &self.container
    }

    /// Returns the TabView widget
    #[must_use]
    pub fn tab_view(&self) -> &adw::TabView {
        &self.tab_view
    }

    /// Returns the global split session colors map (session_id → color_index).
    ///
    /// Used by split view popover to show color indicators for sessions
    /// that are already displayed in any split view.
    #[must_use]
    pub fn split_colors(&self) -> &Rc<RefCell<HashMap<Uuid, usize>>> {
        &self.split_session_colors
    }

    /// Switches a session's tab page to split mode.
    ///
    /// Replaces the single-terminal content with the split view bridge widget
    /// inside the `TabPageContainer`. The `TabView` remains visible.
    pub fn switch_tab_to_split(&self, session_id: Uuid, split_widget: &GtkBox) {
        let mut containers = self.tab_containers.borrow_mut();
        if let Some(container) = containers.get_mut(&session_id) {
            container.switch_to_split(split_widget);
        }
        // TabView stays visible — no hide_tab_view_content()
        self.tab_view.set_visible(true);
        self.tab_view.set_vexpand(true);
    }

    /// Removes a session's standalone tab while it lives in another tab's split.
    ///
    /// The session's live widget already sits in the split panel, so this only
    /// drops the now-redundant tab page — the session's data (widget, terminal,
    /// history, monitoring state) stays alive. Keeping split guests out of the
    /// tab bar and Tab Overview avoids redundant placeholder tabs. The tab is
    /// recreated by [`Self::restore_session_tab`] when the session leaves the
    /// split. No-op if the session has no tab (already parked).
    pub fn park_session_tab(&self, session_id: Uuid) {
        let Some(page) = self.sessions.borrow().get(&session_id).cloned() else {
            return;
        };
        // Mark before closing so the close-page handler skips teardown and only
        // removes the tab page (see `setup_tab_view_signals`).
        self.parked_in_split.borrow_mut().insert(session_id);
        self.tab_view.close_page(&page);
    }

    /// Recreates the standalone tab for a session that was parked by
    /// [`Self::park_session_tab`], so its widget has a home again after it
    /// leaves the split. No-op if the session was not parked.
    ///
    /// The fresh tab starts with an empty single-mode container; the caller's
    /// subsequent [`Self::reparent_terminal_to_tab`] moves the live widget in.
    fn restore_session_tab(&self, session_id: Uuid) {
        if !self.parked_in_split.borrow_mut().remove(&session_id) {
            return;
        }
        let Some((title, protocol)) = self
            .session_info
            .borrow()
            .get(&session_id)
            .map(|info| (info.name.clone(), info.protocol.clone()))
        else {
            return;
        };

        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);
        let tab_container = TabPageContainer::single(&container);
        let page = self.tab_view.append(tab_container.widget());
        page.set_title(&title);
        page.set_icon(Some(&gio::ThemedIcon::new(Self::get_protocol_icon(
            &protocol,
        ))));

        self.sessions.borrow_mut().insert(session_id, page);
        self.tab_containers
            .borrow_mut()
            .insert(session_id, tab_container);
    }

    /// Closes (terminates) a session by id, running the standard tab-close
    /// teardown regardless of whether it currently has a standalone tab.
    ///
    /// A split guest has no tab, so its tab is recreated first (unselected, so
    /// the user sees no content switch) and then closed — the `close-page`
    /// handler disconnects the live widget and kills the child process via the
    /// session maps, which hold the widget wherever it currently lives. The
    /// caller is responsible for having removed the session's split panel first
    /// (e.g. via `close_pane`); `on_split_cleanup` clears any remaining split
    /// membership as part of the close.
    pub fn close_session(&self, session_id: Uuid) {
        self.restore_session_tab(session_id);
        let page = self.sessions.borrow().get(&session_id).cloned();
        if let Some(page) = page {
            self.tab_view.close_page(&page);
        }
    }

    /// Switches a session's tab page back to single-terminal mode.
    ///
    /// Removes the split widget and restores the single-terminal content.
    pub fn switch_tab_to_single(&self, session_id: Uuid, content: &GtkBox) {
        let mut containers = self.tab_containers.borrow_mut();
        if let Some(container) = containers.get_mut(&session_id) {
            container.switch_to_single(content);
        }
        self.tab_view.set_visible(true);
        self.tab_view.set_vexpand(true);
    }

    /// Returns the TabOverview widget
    #[must_use]
    pub fn tab_overview(&self) -> &adw::TabOverview {
        &self.tab_overview
    }

    /// Registers the one-time `open-notify` handler on `TabOverview` that
    /// Cleanup handler for TabOverview close.
    ///
    /// With the new per-tab split architecture, no pinning workarounds are
    /// needed, so this is a no-op placeholder kept for future use.
    fn setup_tab_overview_cleanup(&self) {
        // No cleanup needed — TabPageContainer guarantees non-zero allocation
        // for all TabPage children, so no temporary pinning is required.
    }

    /// Opens the Tab Overview.
    ///
    /// With the new per-tab split architecture, all `TabPage` children have
    /// non-zero allocation (guaranteed by `TabPageContainer`), so no pinning
    /// workarounds are needed.
    pub fn open_tab_overview(&self) {
        if self.sessions.borrow().is_empty() {
            return;
        }
        self.tab_overview.set_open(true);
    }

    /// Returns a clone of the sessions map for external use (e.g. activity indicator updates)
    #[must_use]
    pub fn sessions_map(&self) -> Rc<RefCell<HashMap<Uuid, adw::TabPage>>> {
        self.sessions.clone()
    }

    /// Returns the number of open tabs
    #[must_use]
    pub fn tab_count(&self) -> u32 {
        self.tab_view.n_pages() as u32
    }

    /// Returns the number of active sessions (excluding Welcome tab)
    #[must_use]
    pub fn session_count(&self) -> usize {
        self.sessions.borrow().len()
    }

    /// Switches to a specific tab by session ID
    pub fn switch_to_tab(&self, session_id: Uuid) {
        if let Some(page) = self.sessions.borrow().get(&session_id).cloned() {
            self.tab_view.set_selected_page(&page);
        }
    }

    /// Returns all session IDs
    #[must_use]
    pub fn session_ids(&self) -> Vec<Uuid> {
        self.sessions.borrow().keys().copied().collect()
    }

    /// Returns session IDs ordered by visible tab position (left to right).
    ///
    /// Unlike [`Self::session_ids`], which yields arbitrary `HashMap` order,
    /// this follows the on-screen tab order — used when saving a workspace so
    /// tabs restore in the same sequence.
    #[must_use]
    pub fn ordered_session_ids(&self) -> Vec<Uuid> {
        let sessions = self.sessions.borrow();
        let mut ordered = Vec::with_capacity(sessions.len());
        for i in 0..self.tab_view.n_pages() {
            let page = self.tab_view.nth_page(i);
            if let Some((id, _)) = sessions.iter().find(|(_, p)| **p == page) {
                ordered.push(*id);
            }
        }
        ordered
    }

    /// Connects a callback for when a terminal child exits
    pub fn connect_child_exited<F>(&self, session_id: Uuid, callback: F)
    where
        F: Fn(i32) + 'static,
    {
        if let Some(terminal) = self.get_terminal(session_id) {
            let pids = self.vte_child_pids.clone();
            terminal.connect_child_exited(move |_terminal, status| {
                // Remove PID — process already exited, no need to kill on tab close
                pids.borrow_mut().remove(&session_id);
                callback(status);
            });
        }
    }

    /// Connects a callback for terminal output (for logging)
    pub fn connect_contents_changed<F>(&self, session_id: Uuid, callback: F)
    where
        F: Fn() + 'static,
    {
        if let Some(terminal) = self.get_terminal(session_id) {
            terminal.connect_contents_changed(move |_terminal| {
                callback();
            });
        }
    }

    /// Connects a callback for cursor movement in terminal.
    ///
    /// `cursor-moved` fires more reliably than `contents-changed` for output
    /// that uses cursor positioning escape sequences without a trailing newline
    /// (e.g. SSH password prompts in no-echo mode). See issue #194.
    pub fn connect_cursor_moved<F>(&self, session_id: Uuid, callback: F)
    where
        F: Fn() + 'static,
    {
        if let Some(terminal) = self.get_terminal(session_id) {
            terminal.connect_cursor_moved(move |_terminal| {
                callback();
            });
        }
    }

    /// Connects a callback for user input (commit signal - data sent to PTY)
    pub fn connect_commit<F>(&self, session_id: Uuid, callback: F)
    where
        F: Fn(&str) + 'static,
    {
        if let Some(terminal) = self.get_terminal(session_id) {
            terminal.connect_commit(move |_terminal, text, _size| {
                callback(text);
            });
        }
    }

    /// Gets the current terminal text content for transcript logging
    #[must_use]
    pub fn get_terminal_text(&self, session_id: Uuid) -> Option<String> {
        self.get_terminal(session_id).map(|terminal| {
            let row_count = terminal.row_count();
            let col_count = terminal.column_count();
            let (text, _len) =
                terminal.text_range_format(vte4::Format::Text, 0, 0, row_count, col_count);
            text.map_or_else(String::new, |g| g.to_string())
        })
    }

    /// Returns the text of the line under the cursor, for password-prompt detection.
    ///
    /// Delegates to the session's VTE terminal: extracts the cursor's row via
    /// `text_range_format`, falling back to the last non-empty grid line when the
    /// cursor row is empty (e.g. prompt glyphs not yet committed). Returns `None`
    /// only when the session has no terminal. Never panics. See issue #194.
    #[must_use]
    pub fn get_cursor_line_text(&self, session_id: Uuid) -> Option<String> {
        let terminal = self.get_terminal(session_id)?;
        cursor_line_text(&terminal)
    }

    /// Applies terminal settings to all existing terminals
    pub fn apply_settings(&self, settings: &rustconn_core::config::TerminalSettings) {
        let terminals = self.terminals.borrow();
        for terminal in terminals.values() {
            config::configure_terminal_with_settings(terminal, settings);
        }
    }

    /// Re-applies per-connection theme overrides after global settings change.
    ///
    /// When global terminal settings are applied, they overwrite any
    /// per-connection color customizations. This method restores those
    /// overrides by looking up each session's connection and re-applying
    /// its `theme_override` (if any).
    pub fn reapply_theme_overrides<F>(&self, theme_name: &str, get_theme_override: F)
    where
        F: Fn(Uuid) -> Option<rustconn_core::models::ConnectionThemeOverride>,
    {
        let base_theme =
            TerminalTheme::by_name(theme_name).unwrap_or_else(TerminalTheme::dark_theme);
        let terminals = self.terminals.borrow();
        let session_info = self.session_info.borrow();
        for (session_id, terminal) in terminals.iter() {
            if let Some(info) = session_info.get(session_id)
                && let Some(theme_override) = get_theme_override(info.connection_id)
            {
                config::apply_theme_override_with_base(terminal, &theme_override, &base_theme);
            }
        }
    }

    /// Moves a session's content widget back into its `TabView` page.
    ///
    /// Used by the split close-pane / unsplit paths: when a session leaves a
    /// split panel, the *same* widget instance is reparented back into its
    /// single-session tab. For an embedded RDP/VNC/SPICE viewer this moves the
    /// live viewer widget (never disconnecting or recreating the connection);
    /// for a VTE session it rebuilds the terminal + scrollbar layout.
    pub fn reparent_terminal_to_tab(&self, session_id: Uuid) {
        // Option B: a split guest has no standalone tab (it was parked by
        // `park_session_tab`). Recreate the tab first so the widget has a home;
        // no-op for a session that still has its tab.
        self.restore_session_tab(session_id);

        // Embedded viewers have no VTE terminal — they travel as their own
        // widget instance (carrying their in-container toolbar and reconnect
        // banner). Handle them first; fall through to the VTE path otherwise.
        if self.reparent_embedded_to_tab(session_id) {
            return;
        }

        let Some(terminal) = self.terminals.borrow().get(&session_id).cloned() else {
            return;
        };

        // Remove terminal from current parent (split pane wrapper, etc.)
        if let Some(parent) = terminal.parent()
            && let Some(box_widget) = parent.downcast_ref::<GtkBox>()
        {
            box_widget.remove(&terminal);
        }

        // Rebuild a fresh single-terminal content box and switch TabPageContainer
        // back to single mode. This correctly handles the case where the tab was
        // previously in split mode (TabPageContainer contained the split bridge widget).
        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);

        // Re-wrap terminal with scrollbar (matching create_terminal_tab_with_settings layout)
        let terminal_row = GtkBox::new(Orientation::Horizontal, 0);
        terminal_row.set_hexpand(true);
        terminal_row.set_vexpand(true);
        terminal_row.append(&terminal);

        // Re-create overlay for highlight support
        let terminal_overlay = gtk4::Overlay::new();
        terminal_overlay.set_child(Some(&terminal_row));
        terminal_overlay.set_hexpand(true);
        terminal_overlay.set_vexpand(true);
        container.append(&terminal_overlay);

        // Update terminal overlay tracking
        self.terminal_overlays
            .borrow_mut()
            .insert(session_id, terminal_overlay);

        // Switch TabPageContainer to single mode with the new content
        let mut containers = self.tab_containers.borrow_mut();
        if let Some(tab_container) = containers.get_mut(&session_id) {
            tab_container.switch_to_single(&container);
        }

        terminal.set_visible(true);
    }

    /// Reparents an embedded viewer back into its single-session tab.
    ///
    /// Returns `true` when `session_id` is an embedded RDP/VNC/SPICE viewer and
    /// was handled; `false` when it is not embedded (the caller then falls back
    /// to the VTE terminal path). The same widget instance is moved, so the
    /// live protocol connection is preserved — nothing is disconnected.
    fn reparent_embedded_to_tab(&self, session_id: Uuid) -> bool {
        // Resolve the concrete widget while scoping the borrow, so no
        // `session_widgets` borrow is held across GTK reparenting.
        enum Embedded {
            Vnc(Rc<VncSessionWidget>),
            Rdp(Rc<EmbeddedRdpWidget>),
            #[cfg(feature = "web-embedded")]
            Web(Rc<crate::embedded_web::EmbeddedWebWidget>),
        }
        let embedded = {
            let widgets = self.session_widgets.borrow();
            match widgets.get(&session_id) {
                Some(SessionWidgetStorage::Vnc(w)) => Embedded::Vnc(Rc::clone(w)),
                Some(SessionWidgetStorage::EmbeddedRdp(w)) => Embedded::Rdp(Rc::clone(w)),
                #[cfg(feature = "web-embedded")]
                Some(SessionWidgetStorage::EmbeddedWeb(w)) => Embedded::Web(Rc::clone(w)),
                _ => return false,
            }
        };

        // Rebuild the single-mode content, mirroring the creation path so the
        // embedded widget is wrapped exactly as when its tab was first built.
        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);

        match embedded {
            Embedded::Vnc(w) => {
                let widget = w.widget();
                Self::detach_widget_from_parent(widget);
                widget.set_hexpand(true);
                widget.set_vexpand(true);
                container.append(widget);
                widget.set_visible(true);
            }
            Embedded::Rdp(w) => {
                let widget = w.widget();
                Self::detach_widget_from_parent(widget.upcast_ref());
                // Append the RDP container directly (mirroring the VNC
                // arm). Wrapping it in a freshly-created `adw::ToastOverlay`
                // here left the reparented `DrawingArea` unable to repaint (its
                // draw func was never re-invoked, so live frames landed in the
                // buffer but never reached the screen — a blank viewer). The
                // file-drop ToastOverlay is only needed while DnD is active and
                // is re-established elsewhere; a plain re-parent restores drawing.
                widget.set_hexpand(true);
                widget.set_vexpand(true);
                container.append(widget);
                widget.set_visible(true);
            }
            #[cfg(feature = "web-embedded")]
            Embedded::Web(w) => {
                let widget = w.widget();
                Self::detach_widget_from_parent(widget.upcast_ref());
                widget.set_hexpand(true);
                widget.set_vexpand(true);
                container.append(widget);
                widget.set_visible(true);
            }
        }

        {
            let mut containers = self.tab_containers.borrow_mut();
            if let Some(tab_container) = containers.get_mut(&session_id) {
                tab_container.switch_to_single(&container);
            }
        }

        // Nudge a repaint once the re-parented viewer has settled into its new
        // allocation (the live frame lives in a Rust-side buffer, not GTK's
        // surface cache).
        glib::idle_add_local_once(move || {
            container.queue_draw();
        });
        true
    }

    /// Detaches a widget from its current parent so the same instance can be
    /// re-attached elsewhere (GTK widgets may only have one parent).
    ///
    /// A `GtkBox` parent uses `remove`; any other parent uses `unparent`.
    fn detach_widget_from_parent(widget: &Widget) {
        if let Some(parent) = widget.parent() {
            if let Some(box_widget) = parent.downcast_ref::<GtkBox>() {
                box_widget.remove(widget);
            } else {
                widget.unparent();
            }
        }
    }

    /// Shows TabView content area (for RDP/VNC/SPICE sessions)
    /// Call this when switching to a non-SSH session that displays in TabView
    pub fn show_tab_view_content(&self) {
        self.tab_view.set_visible(true);
        self.tab_view.set_vexpand(true);
    }

    /// Returns whether the TabView content is currently visible
    #[must_use]
    pub fn is_tab_view_content_visible(&self) -> bool {
        self.tab_view.is_visible()
    }

    // ========================================================================
    // Tab Group Management
    // ========================================================================

    /// Assigns a session to a named tab group.
    ///
    /// The group is assigned a color from the palette. The tab indicator is
    /// updated to show the group color (unless a split color is active).
    pub fn set_tab_group(&self, session_id: Uuid, group_name: &str) {
        let color_index = self
            .tab_group_manager
            .borrow_mut()
            .get_or_assign_color(group_name);

        if let Some(info) = self.session_info.borrow_mut().get_mut(&session_id) {
            info.tab_group = Some(group_name.to_owned());
            info.tab_color_index = Some(color_index);
        }

        // Apply group label prefix to tab title (independent of split/protocol indicator)
        self.apply_group_color(session_id, color_index);

        // Update tooltip to include group name
        if let Some(page) = self.sessions.borrow().get(&session_id) {
            let current_tooltip = page.tooltip().unwrap_or_default();
            let base_tooltip = current_tooltip
                .as_str()
                .rsplit_once("\n[")
                .map_or(current_tooltip.as_str(), |(base, _)| base);
            page.set_tooltip(&format!("{base_tooltip}\n[{group_name}]"));
        }

        tracing::debug!(session_id = %session_id, group = group_name, color_index, "Tab assigned to group");
    }

    /// Returns the group name for a session, if any.
    #[must_use]
    pub fn get_tab_group(&self, session_id: Uuid) -> Option<String> {
        self.session_info
            .borrow()
            .get(&session_id)
            .and_then(|i| i.tab_group.clone())
    }

    /// Applies a group label prefix to a tab title.
    fn apply_group_color(&self, session_id: Uuid, _color_index: usize) {
        if let Some(page) = self.sessions.borrow().get(&session_id)
            && let Some(info) = self.session_info.borrow().get(&session_id)
            && let Some(ref group_name) = info.tab_group
        {
            let current_title = page.title().to_string();
            // Remove any existing group prefix first
            let base_title = current_title
                .find("] ")
                .and_then(|pos| {
                    if current_title.starts_with('[') {
                        Some(&current_title[pos + 2..])
                    } else {
                        None
                    }
                })
                .unwrap_or(&current_title);
            page.set_title(&format!("[{group_name}] {base_title}"));
        }
    }

    /// Sets the callback to be invoked when a page is closed.
    ///
    /// The callback receives the session ID and connection ID of the closed page.
    /// This is used to update the sidebar status when SSH tabs are closed via TabView.
    ///
    /// # Arguments
    ///
    /// * `callback` - A closure that takes (session_id, connection_id) as parameters
    pub fn set_on_page_closed<F>(&self, callback: F)
    where
        F: Fn(Uuid, Uuid) + 'static,
    {
        *self.on_page_closed.borrow_mut() = Some(Box::new(callback));
    }

    /// Sets the callback invoked when a new terminal session tab is created.
    ///
    /// The callback receives `(session_id, connection_id)`. It fires from
    /// [`Self::create_terminal_tab_with_settings`] — the single choke point
    /// for all terminal protocols and for both synchronous and async
    /// (port-checked) connection paths. Used to wire activity monitoring.
    pub fn set_on_session_created<F>(&self, callback: F)
    where
        F: Fn(Uuid, Uuid) + 'static,
    {
        *self.on_session_created.borrow_mut() = Some(Box::new(callback));
    }

    /// Sets a callback fired when ANY tab is added (all protocols).
    ///
    /// Unlike `on_session_created` (terminal-only), this fires for VNC, SPICE,
    /// embedded RDP, and external-process tabs too. Designed for one-shot use
    /// by workspace restore: the callback should clear itself once the target
    /// session is detected.
    pub fn set_on_tab_added<F>(&self, callback: F)
    where
        F: Fn(Uuid, Uuid) + 'static,
    {
        *self.on_tab_added.borrow_mut() = Some(Box::new(callback));
    }

    /// Clears the `on_tab_added` callback.
    pub fn clear_on_tab_added(&self) {
        *self.on_tab_added.borrow_mut() = None;
    }

    /// Fires the `on_tab_added` callback if set.
    fn notify_tab_added(&self, session_id: Uuid, connection_id: Uuid) {
        // Take the callback out to avoid holding a borrow across the call —
        // the callback may call `clear_on_tab_added()` which also borrows.
        let callback = self.on_tab_added.borrow_mut().take();
        if let Some(cb) = callback {
            cb(session_id, connection_id);
            // Restore the callback for future tab creations UNLESS it was
            // consumed. Convention: the callback sets the `on_tab_added` slot
            // to a new value (or the slot stays None if consumed). If the slot
            // is still None after the call, the callback was NOT self-clearing
            // from within (because take already emptied it), so we restore.
            // If the workspace callback wants to signal "done", it must NOT
            // call clear_on_tab_added — instead it signals via a shared flag
            // captured in the closure. We simply always restore here; the
            // workspace code uses an `Rc<Cell<bool>>` to stop re-firing.
            let mut slot = self.on_tab_added.borrow_mut();
            if slot.is_none() {
                *slot = Some(cb);
            }
        }
    }

    /// Sets the callback invoked when terminal (VTE) focus changes.
    ///
    /// The callback receives `true` when focus enters the terminal and `false`
    /// when it leaves. Wired from the window to suspend/restore the single-Ctrl
    /// accelerators that collide with readline chords (issue #197).
    pub fn set_on_terminal_focus<F>(&self, callback: F)
    where
        F: Fn(bool) + 'static,
    {
        *self.on_terminal_focus.borrow_mut() = Some(Box::new(callback));
    }

    /// Attaches a focus controller that drives the `on_terminal_focus` callback
    /// (`true` on enter, `false` on leave).
    ///
    /// Used for the VTE terminal and the embedded RDP/VNC/SPICE viewers so the
    /// single-Ctrl accelerators are suspended while any of them has focus,
    /// keeping the behavior identical across protocols (issue #197).
    /// `EventControllerFocus` reports focus for the widget and its descendants,
    /// so attaching to the top-level viewer widget fires when any child gains
    /// focus.
    fn attach_focus_passthrough<W: IsA<gtk4::Widget>>(&self, widget: &W) {
        let focus_ctrl = gtk4::EventControllerFocus::new();
        let on_focus_enter = self.on_terminal_focus.clone();
        focus_ctrl.connect_enter(move |_| {
            if let Some(cb) = on_focus_enter.borrow().as_ref() {
                cb(true);
            }
        });
        let on_focus_leave = self.on_terminal_focus.clone();
        focus_ctrl.connect_leave(move |_| {
            if let Some(cb) = on_focus_leave.borrow().as_ref() {
                cb(false);
            }
        });
        widget.add_controller(focus_ctrl);
    }

    /// Sets the callback invoked when session recording starts or stops.
    ///
    /// Receives the connection ID and the new recording state; used to
    /// drive the sidebar recording indicator.
    pub fn set_on_recording_changed<F>(&self, callback: F)
    where
        F: Fn(Uuid, bool) + 'static,
    {
        *self.on_recording_changed.borrow_mut() = Some(Box::new(callback));
    }

    /// Sets the callback invoked after the split-color map changes.
    ///
    /// Fired when a session joins or leaves a split, or a split tab closes.
    /// The handler re-syncs the sidebar split-membership marker from
    /// [`Self::split_colors`].
    pub fn set_on_split_colors_changed<F>(&self, callback: F)
    where
        F: Fn() + 'static,
    {
        *self.on_split_colors_changed.borrow_mut() = Some(Box::new(callback));
    }

    /// Fires the split-colors-changed callback, if one is registered.
    ///
    /// Callers must not hold a borrow of `split_session_colors`, `sessions`,
    /// or `session_info` when calling this — the handler re-reads them.
    fn notify_split_colors_changed(&self) {
        if let Some(ref callback) = *self.on_split_colors_changed.borrow() {
            callback();
        }
    }

    /// Sets the callback to be invoked for split view cleanup when a page is about to close.
    ///
    /// The callback receives the session ID of the page being closed.
    /// This is used to clear the session from split view panels before the tab is closed.
    ///
    /// # Arguments
    ///
    /// * `callback` - A closure that takes session_id as parameter
    pub fn set_on_split_cleanup<F>(&self, callback: F)
    where
        F: Fn(Uuid) + 'static,
    {
        *self.on_split_cleanup.borrow_mut() = Some(Box::new(callback));
    }

    // === Highlight rules integration ===

    /// Sets up highlight rules for a terminal session.
    ///
    /// Compiles global and per-connection [`HighlightRule`]s using
    /// [`CompiledHighlightRules::compile`], creates a transparent
    /// [`HighlightOverlay`] that draws colored backgrounds and foreground
    /// text on top of the VTE terminal, and wires `contents-changed` so
    /// the overlay repaints automatically.
    ///
    /// VTE's `match_add_regex()` is still registered for hover-underline
    /// feedback, but the actual colored rendering is done by the overlay.
    pub fn set_highlight_rules(
        &self,
        session_id: Uuid,
        global_rules: &[HighlightRule],
        per_conn_rules: &[HighlightRule],
    ) {
        let compiled = CompiledHighlightRules::compile(global_rules, per_conn_rules);

        if let Some(terminal) = self.terminals.borrow().get(&session_id) {
            // Still register with VTE for hover-underline feedback
            for rule in compiled.source_patterns() {
                let pattern = &rule.pattern;
                match vte4::Regex::for_match(pattern, PCRE2_MULTILINE) {
                    Ok(vte_regex) => {
                        terminal.match_add_regex(&vte_regex, 0);
                        tracing::trace!(
                            %session_id,
                            rule_name = %rule.name,
                            "Registered VTE highlight regex"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            %session_id,
                            rule_name = %rule.name,
                            pattern = %pattern,
                            "Failed to register VTE highlight regex: {e}"
                        );
                    }
                }
            }

            // Store compiled rules first so the overlay draw func can access them
            self.session_highlight_rules
                .borrow_mut()
                .insert(session_id, compiled);

            // Remove any previous overlay for this session
            self.highlight_overlays.borrow_mut().remove(&session_id);

            // Create and connect the colored highlight overlay
            if let Some(overlay_widget) = self.terminal_overlays.borrow().get(&session_id) {
                let hl_overlay = HighlightOverlay::new(overlay_widget, terminal);
                hl_overlay.connect(terminal, self.session_highlight_rules.clone(), session_id);
                self.highlight_overlays
                    .borrow_mut()
                    .insert(session_id, hl_overlay);
            }
        } else {
            self.session_highlight_rules
                .borrow_mut()
                .insert(session_id, compiled);
        }
    }

    // === Cluster terminal tracking ===

    /// Registers a terminal session as part of a cluster
    pub fn register_cluster_terminal(&self, cluster_id: Uuid, session_id: Uuid) {
        self.cluster_sessions
            .borrow_mut()
            .entry(cluster_id)
            .or_default()
            .push(session_id);
        self.session_to_cluster
            .borrow_mut()
            .insert(session_id, cluster_id);
    }

    /// Unregisters all terminals for a cluster
    pub fn unregister_cluster(&self, cluster_id: Uuid) {
        if let Some(sessions) = self.cluster_sessions.borrow_mut().remove(&cluster_id) {
            let mut reverse = self.session_to_cluster.borrow_mut();
            for sid in &sessions {
                reverse.remove(sid);
            }
        }
        // Clear any pending registrations for this cluster
        self.cluster_pending
            .borrow_mut()
            .retain(|_, cid| *cid != cluster_id);
    }

    /// Marks a connection as pending cluster registration.
    ///
    /// When the terminal tab for `connection_id` is eventually created
    /// (synchronously or after an async port check), it will automatically
    /// be registered as part of `cluster_id` and trigger the
    /// `on_cluster_session_registered` callback.
    pub fn mark_cluster_pending(&self, cluster_id: Uuid, connection_id: Uuid) {
        self.cluster_pending
            .borrow_mut()
            .insert(connection_id, cluster_id);
    }

    /// Resolves a pending cluster registration for a freshly created session.
    ///
    /// Called internally by `create_terminal_tab_with_settings`. If the
    /// connection had been marked as pending via `mark_cluster_pending`,
    /// this registers the new `session_id` in the cluster's session list.
    fn resolve_cluster_pending(&self, connection_id: Uuid, session_id: Uuid) {
        let Some(cluster_id) = self.cluster_pending.borrow_mut().remove(&connection_id) else {
            return;
        };
        self.register_cluster_terminal(cluster_id, session_id);
    }

    /// Gets all terminal session IDs for a cluster
    pub fn get_cluster_sessions(&self, cluster_id: Uuid) -> Vec<Uuid> {
        self.cluster_sessions
            .borrow()
            .get(&cluster_id)
            .cloned()
            .unwrap_or_default()
    }

    // ── Ad-hoc Broadcast ──────────────────────────────────────────────
    // (removed: superseded by the split-view broadcast toggle in the header bar)

    /// Sets the activity coordinator for tab context menu integration.
    ///
    /// Must be called after construction to enable the "Monitor: ..." context menu action.
    pub fn set_activity_coordinator(&self, coordinator: Rc<ActivityCoordinator>) {
        *self.activity_coordinator.borrow_mut() = Some(coordinator);
    }
}

impl Default for TerminalNotebook {
    fn default() -> Self {
        Self::new()
    }
}

/// Extracts the text of the line under the cursor of a VTE terminal.
///
/// Returns the cursor's row via `text_range_format`. When that row is empty
/// (prompt glyphs not yet committed to the grid), falls back to the last
/// non-empty line of the visible grid. Returns `None` when no non-empty text
/// can be extracted. Never panics. Backs `TerminalNotebook::get_cursor_line_text`
/// for cursor-position-based prompt detection (issue #194).
fn cursor_line_text(terminal: &Terminal) -> Option<String> {
    let col_count = terminal.column_count();
    // `cursor_position()` returns `(column, row)` in visible-grid coordinates,
    // matching the rows used by `text_range_format` in `get_terminal_text`.
    let (_col, row) = terminal.cursor_position();
    let (cursor_text, _len) =
        terminal.text_range_format(vte4::Format::Text, row, 0, row, col_count);
    if let Some(line) = cursor_text {
        let line = line.to_string();
        if !line.trim().is_empty() {
            return Some(line);
        }
    }

    // Fallback: last non-empty line of the visible grid.
    let row_count = terminal.row_count();
    let (grid_text, _len) =
        terminal.text_range_format(vte4::Format::Text, 0, 0, row_count, col_count);
    grid_text.and_then(|g| {
        g.to_string()
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .map(str::to_owned)
    })
}

#[cfg(test)]
mod split_eligibility_tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::{SessionWidgetStorage, SplitEligibility, eligibility_from};

    #[test]
    fn external_process_is_external_viewer() {
        // Constructible without GTK — carries only a child-process handle.
        let storage = SessionWidgetStorage::ExternalProcess(Rc::new(RefCell::new(None)));
        assert_eq!(
            eligibility_from(false, Some(&storage)),
            SplitEligibility::ExternalViewer
        );
    }

    #[test]
    fn stored_widget_wins_over_terminal_flag() {
        // Even if a stray terminal flag is set, an external viewer stays declined.
        let storage = SessionWidgetStorage::ExternalProcess(Rc::new(RefCell::new(None)));
        assert_eq!(
            eligibility_from(true, Some(&storage)),
            SplitEligibility::ExternalViewer
        );
    }

    #[test]
    fn terminal_only_session_is_embeddable() {
        assert_eq!(eligibility_from(true, None), SplitEligibility::Embeddable);
    }

    #[test]
    fn unknown_session_is_none() {
        assert_eq!(eligibility_from(false, None), SplitEligibility::None);
    }

    #[test]
    // GTK can only be initialized from one thread per process; the default
    // multi-threaded test harness makes this unsafe, so this widget-constructing
    // test is opt-in.
    #[ignore = "requires GTK init on a single thread; run with: cargo test -p rustconn -- --ignored --test-threads=1"]
    fn embedded_widget_variants_are_embeddable() {
        // The Vnc/EmbeddedRdp arms need real GTK widgets to
        // construct, so gate on a display; skip cleanly when headless.
        if gtk4::init().is_err() {
            return;
        }
        let widget = Rc::new(crate::session::VncSessionWidget::new());
        let storage = SessionWidgetStorage::Vnc(widget);
        assert_eq!(
            eligibility_from(false, Some(&storage)),
            SplitEligibility::Embeddable
        );
    }
}
