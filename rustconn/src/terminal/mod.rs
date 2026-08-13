//! Terminal notebook area using adw::TabView
//!
//! This module provides the tabbed terminal interface using VTE4
//! for SSH sessions and native GTK widgets for VNC/RDP/SPICE connections.
//!
//! # Module Structure
//!
//! - `types` - Data structures for sessions
//! - `config` - Terminal appearance and behavior configuration
//! - `tab_lifecycle` - Tab creation, parking for split view, restore, reparenting
//! - `session_lifecycle` - Reconnect, VTE reset, status indicators, reconnect banner
//!
// ponytail: `TerminalNotebook` is still one type with ~156 methods; 0.20.0 moved
// the tab- and session-lifecycle ones into the two modules named above, which cut
// this file by 30% but did not reduce coupling — it only made it visible, since
// the moved methods had to widen from private to `pub(super)`.
//
// The actual god object is the per-tab state those methods share: the notebook
// holds parallel collections keyed by tab, and every method reaches across them.
// The upgrade path is to extract that into a `TerminalTab` type owning its own
// widget, session handle, connection and reconnect state, after which most of
// these methods become methods on the tab and the notebook keeps only the ones
// that are genuinely about the collection. Do that before splitting another file
// off; a fourth module would move lines without moving the problem.
mod config;
mod detach;
pub use detach::{DetachMonitor, DetachPresentation};
pub mod file_drop;
pub mod highlight_overlay;
pub mod playback;
pub mod pty_relay;
pub mod pty_spawn;
mod recording;
mod session_lifecycle;
pub mod tab_container;
mod tab_lifecycle;
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
use rustconn_core::models::{AutomationConfig, BackspaceSends, DeleteSends};
use rustconn_core::terminal_themes::TerminalTheme;
pub use types::{
    ClusterTabs, PendingCluster, SessionWidgetStorage, TerminalSession, group_still_in_use,
    strip_group_prefix, tab_title,
};
use uuid::Uuid;
use vte4::Terminal;
use vte4::prelude::*;
use zeroize::Zeroizing;

/// PCRE2 multiline compile flag — required by VTE's `match_add_regex()`.
///
/// Without this flag VTE emits a runtime warning:
/// `_vte_regex_has_multiline_compile_flag(regex)` check failed.
const PCRE2_MULTILINE: u32 = 0x0000_0400;

/// `DECRST 1049` — leave the alternate screen, restoring the normal cursor.
///
/// `Terminal::reset` switches back to the normal screen only in its
/// `clear_history` branch, so every reset that keeps the scrollback has to do
/// the switch itself. Otherwise a session that died inside a full-screen app
/// (vim, htop, less) keeps showing that app's frozen screen and hides the very
/// scrollback the tab was kept open for (issue #253). VTE applies the mode's
/// side effect unconditionally, so feeding this is a no-op on the normal
/// screen. Feed it *after* `reset`, which discards unprocessed input.
const LEAVE_ALTERNATE_SCREEN: &[u8] = b"\x1b[?1049l";

/// How often a live session's grid is compared with the size its child knows.
///
/// Nothing in VTE reports a resize (see the `vte_contract_tests` module), so
/// this is the delay between the user letting go of the window edge and the
/// child receiving `SIGWINCH`. Two integer reads per session at this interval is
/// nothing, and a shorter one would only chase an event the user cannot perceive
/// anyway.
const GRID_SIZE_POLL: std::time::Duration = std::time::Duration::from_millis(250);

use rustconn_core::automation::{KeyElement, KeySequence};
use rustconn_core::highlight::CompiledHighlightRules;
use rustconn_core::models::HighlightRule;
use rustconn_core::session::recording::{RecordingMetadata, metadata_path, write_metadata};
use rustconn_core::split::tab_groups::TabGroupManager;

use crate::activity_coordinator::ActivityCoordinator;
use crate::automation::{AutomationSession, prepare_rules_from_config};
use crate::embedded_rdp::EmbeddedRdpWidget;
use crate::i18n::{i18n, i18n_f};
use crate::monitoring::MonitoringCoordinator;
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
    /// Sessions whose connection has ended while their tab is still open.
    ///
    /// A tab is kept after the child exits (unless `close_on_clean_exit`) so the
    /// scrollback stays readable and the reconnect banner has somewhere to live.
    /// Such a session is still in `session_info`, which made the smart
    /// double-click focus the dead tab instead of connecting (issue #242). This
    /// set is the liveness signal every "is there a session to focus?" check
    /// must consult; it is broader than `reconnect_shown`, which only tracks
    /// tabs that actually got a banner.
    disconnected_sessions: Rc<RefCell<HashSet<Uuid>>>,
    /// Whether an in-place reconnect keeps the previous session's scrollback
    /// (`TerminalSettings::keep_history_on_reconnect`, issue #253).
    keep_history_on_reconnect: Rc<std::cell::Cell<bool>>,
    /// Maximum scrollback lines to retain after a reconnect (None = unlimited).
    ///
    /// VTE's own `scrollback_lines` property is a *per-session* cap, but with
    /// history preserved across reconnects the total buffer grows without bound.
    /// When set, the old scrollback is trimmed to this many lines by temporarily
    /// lowering VTE's cap before feeding the reconnect separator.
    max_scrollback_on_reconnect: Rc<std::cell::Cell<Option<u32>>>,
    /// Absolute VTE row at which a session's current connection started.
    ///
    /// VTE cursor rows are absolute buffer coordinates — they include the
    /// scrollback — so every "the cursor advanced past the connect banner"
    /// check needs a baseline once a reconnect keeps the previous session's
    /// output (issue #253). `None` means "capture the baseline on the next
    /// read": `prepare_for_reconnect` feeds its separator through VTE, which
    /// processes input asynchronously, so the row is only meaningful once
    /// that output has actually landed. A session with no entry started on an
    /// empty buffer and needs no baseline.
    cursor_row_base: Rc<RefCell<HashMap<Uuid, Option<i64>>>>,
    /// First buffer row a session's transcript has not been logged yet
    /// (issue [#247](https://github.com/totoshko88/RustConn/issues/247)).
    ///
    /// Like [`Self::cursor_row_base`] this is an absolute VTE buffer row, so it
    /// survives scrolling: everything that scrolls off the viewport stays
    /// addressable in the scrollback and still reaches the log. A missing entry
    /// means "start at row 0", which is where a fresh buffer begins.
    /// The PTY each live session is running on
    /// (issue [#247](https://github.com/totoshko88/RustConn/issues/247)).
    ///
    /// VTE renders the session but owns no descriptor, so this is what the
    /// child is actually attached to: output is read here and fed to the
    /// widget, input the widget reports through `commit` is written back, and
    /// the window size is pushed down. Dropping the entry stops the session's
    /// relay threads, so it is removed when the tab closes and replaced when a
    /// reconnect starts a new child on the same terminal.
    pty_relays: Rc<RefCell<HashMap<Uuid, pty_relay::PtyRelay>>>,
    /// Observers of raw PTY output, per session.
    ///
    /// Session logging registers one of these, which is how a transcript
    /// records what the child wrote rather than what the widget ended up
    /// displaying. They are `Rc` so the delivery loop can clone the list and
    /// release its borrow before calling anything.
    output_observers: Rc<RefCell<HashMap<Uuid, Vec<Rc<dyn Fn(&[u8])>>>>>,
    /// Sessions whose terminal already forwards `commit` to its relay.
    ///
    /// The handler resolves the relay through [`Self::pty_relays`] on every
    /// keystroke rather than capturing one, so it keeps working across a
    /// reconnect — and must therefore be connected only once per terminal.
    commit_forwarded: Rc<RefCell<HashSet<Uuid>>>,
    /// Window-size poll for each live session.
    ///
    /// Nothing in VTE announces a geometry change (see the `vte_contract_tests`
    /// module), so the grid is compared against the size last pushed to the
    /// child. Removed together with the relay.
    pty_size_timers: Rc<RefCell<HashMap<Uuid, glib::SourceId>>>,
    /// Cluster tab tracking: cluster_id → its open tabs and their group name.
    cluster_sessions: Rc<RefCell<HashMap<Uuid, ClusterTabs>>>,
    /// Reverse lookup: session_id → cluster_id
    session_to_cluster: Rc<RefCell<HashMap<Uuid, Uuid>>>,
    /// Pending cluster registrations awaiting their session_id.
    ///
    /// When a connection is launched as part of a cluster but its tab is created
    /// asynchronously (e.g. after a TCP port check), we cannot register the
    /// session_id at launch time. Instead we record connection_id →
    /// [`PendingCluster`] here and resolve it the moment a tab is created.
    cluster_pending: Rc<RefCell<HashMap<Uuid, PendingCluster>>>,
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
    /// Sessions an *unattended* sweep is allowed to bring back.
    ///
    /// A visible reconnect banner is not consent: the disconnect path shows one
    /// for a shell the user closed with `exit`, for a failed authentication and
    /// for a process that crashed on startup — precisely the cases where it
    /// deliberately refuses to reconnect. Membership here records that decision
    /// so a network change or a resume from sleep honours it instead of
    /// re-running the login by itself.
    auto_reconnect_eligible: Rc<RefCell<std::collections::HashSet<Uuid>>>,
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
    /// Sessions whose widget currently lives in a detached window and which
    /// therefore have no `TabPage`. Session data stays alive, exactly as for
    /// `parked_in_split`; the `close-page` handler skips teardown for them.
    detached: Rc<RefCell<HashSet<Uuid>>>,
    /// Invoked by [`Self::switch_to_tab`] when the target session is detached,
    /// so the window layer can present its window instead of selecting a tab.
    on_focus_detached: Rc<RefCell<Option<Box<dyn Fn(Uuid)>>>>,
    /// Invoked when the tab context menu requests a detach.
    on_detach_request: Rc<RefCell<Option<Box<dyn Fn(Uuid, DetachPresentation) -> bool>>>>,
    /// Invoked once a session's teardown has run, whatever ended it: a tab
    /// close, a remote disconnect, a child exit, or a terminate from the
    /// session manager. Parked and detached sessions do not reach it, because
    /// their `close-page` pass skips teardown. The window layer uses it to
    /// close a detached window whose session disappeared, so no empty window
    /// is left behind (issue #236).
    on_session_ended: Rc<RefCell<Option<Box<dyn Fn(Uuid)>>>>,
    /// Monitoring coordinator, set after construction. Detach and attach
    /// suspend and resume the monitoring bar around the widget move, exactly
    /// as the split path does.
    monitoring: Rc<RefCell<Option<Rc<MonitoringCoordinator>>>>,
    /// Shared snippet menu section for terminal context menus.
    /// Updated when snippets are created/edited/deleted; all terminals
    /// share the same live `gio::Menu` model so changes propagate automatically.
    snippet_menu_section: Rc<gio::Menu>,
    /// VTE child process PIDs per session.
    /// Used to send SIGTERM/SIGKILL to the process group on tab close.
    /// Some terminal clients (e.g. telnet) do not exit on PTY close (SIGHUP),
    /// so an explicit kill is needed (#172).
    vte_child_pids: Rc<RefCell<HashMap<Uuid, i32>>>,
    /// Whether to show the Welcome tab when no sessions are open (issue #232).
    /// Shared with signal handlers via `Rc<Cell<bool>>`.
    show_welcome: Rc<std::cell::Cell<bool>>,
}

impl TerminalNotebook {
    /// Creates a new terminal notebook using adw::TabView
    ///
    /// When `show_welcome` is `false`, the Welcome tab is not created at
    /// startup — useful when a startup action will immediately open a session
    /// (issue #232).
    #[must_use]
    pub fn new(show_welcome: bool) -> Self {
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

        // Add a welcome page only if the setting allows it (issue #232)
        if show_welcome {
            let welcome = Self::create_welcome_tab();
            let welcome_container = TabPageContainer::welcome(&welcome.upcast::<gtk4::Widget>());
            let welcome_page = tab_view.append(welcome_container.widget());
            welcome_page.set_title(&i18n("Welcome"));
            welcome_page.set_icon(Some(&gio::ThemedIcon::new("go-home-symbolic")));
        }

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
            disconnected_sessions: Rc::new(RefCell::new(HashSet::new())),
            keep_history_on_reconnect: Rc::new(std::cell::Cell::new(true)),
            max_scrollback_on_reconnect: Rc::new(std::cell::Cell::new(None)),
            cursor_row_base: Rc::new(RefCell::new(HashMap::new())),
            pty_relays: Rc::new(RefCell::new(HashMap::new())),
            output_observers: Rc::new(RefCell::new(HashMap::new())),
            commit_forwarded: Rc::new(RefCell::new(HashSet::new())),
            pty_size_timers: Rc::new(RefCell::new(HashMap::new())),
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
            auto_reconnect_eligible: Rc::new(RefCell::new(std::collections::HashSet::new())),
            ssh_tunnels: Rc::new(RefCell::new(HashMap::new())),
            activity_coordinator: Rc::new(RefCell::new(None)),
            tab_containers: Rc::new(RefCell::new(HashMap::new())),
            parked_in_split: Rc::new(RefCell::new(HashSet::new())),
            detached: Rc::new(RefCell::new(HashSet::new())),
            on_focus_detached: Rc::new(RefCell::new(None)),
            on_detach_request: Rc::new(RefCell::new(None)),
            on_session_ended: Rc::new(RefCell::new(None)),
            monitoring: Rc::new(RefCell::new(None)),
            snippet_menu_section: Rc::new(gio::Menu::new()),
            vte_child_pids: Rc::new(RefCell::new(HashMap::new())),
            show_welcome: Rc::new(std::cell::Cell::new(show_welcome)),
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
        let automation_sessions_close = Rc::clone(&self.automation_sessions);
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
        let detached_close = Rc::clone(&self.detached);
        let on_session_ended = Rc::clone(&self.on_session_ended);
        let vte_child_pids = self.vte_child_pids.clone();
        let auto_reconnect_on_close = Rc::clone(&self.auto_reconnect_eligible);
        let show_welcome_on_close = self.show_welcome.clone();
        let disconnected_on_close = Rc::clone(&self.disconnected_sessions);
        let cursor_row_base_on_close = Rc::clone(&self.cursor_row_base);
        let pty_relays_on_close = Rc::clone(&self.pty_relays);
        let output_observers_on_close = Rc::clone(&self.output_observers);
        let commit_forwarded_on_close = Rc::clone(&self.commit_forwarded);
        let pty_size_timers_on_close = Rc::clone(&self.pty_size_timers);

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

            // Parked (Option B): the tab is being removed because the session
            // moved into another tab's split or into a detached window, NOT
            // closed. Its live widget lives elsewhere and its session data must
            // survive, so drop only the tab page and its (now-stale) container
            // mapping — skip all teardown. `restore_session_tab` recreates the
            // tab when the session comes back.
            let is_parked = !session_id.is_nil()
                && (parked_in_split.borrow().contains(&session_id)
                    || detached_close.borrow().contains(&session_id));
            if is_parked {
                sessions.borrow_mut().remove(&session_id);
                tab_containers.borrow_mut().remove(&session_id);
                view.close_page_finish(page, true);
                // Parking the *last* tab leaves the content area empty, which
                // only detaching can do — a split guest's owner tab always
                // survives. Give the main window its Welcome tab back, exactly
                // as a normal close does (issue #236).
                if show_welcome_on_close.get() && tab_view.n_pages() == 0 {
                    Self::append_welcome_page(&tab_view);
                }
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
                // Dropping the automation session cancels its poll source and
                // scrubs any resolved credential responses still in the engine.
                automation_sessions_close.borrow_mut().remove(&session_id);

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
                disconnected_on_close.borrow_mut().remove(&session_id);
                cursor_row_base_on_close.borrow_mut().remove(&session_id);

                // Stop the session's PTY: dropping the relay ends its reader and
                // writer threads and closes the descriptor. The child is killed
                // just below, which is what releases a reader still waiting on
                // output (#247, #172).
                if let Some(timer) = pty_size_timers_on_close.borrow_mut().remove(&session_id) {
                    timer.remove();
                }
                drop(pty_relays_on_close.borrow_mut().remove(&session_id));
                output_observers_on_close.borrow_mut().remove(&session_id);
                commit_forwarded_on_close.borrow_mut().remove(&session_id);
                auto_reconnect_on_close.borrow_mut().remove(&session_id);

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

                // The session is gone for good now. Fired last, and outside
                // every borrow above, so a handler may freely re-enter the
                // notebook (issue #236: closing a leftover detached window).
                if let Some(ref callback) = *on_session_ended.borrow() {
                    callback(session_id);
                }
            }

            // Confirm close
            view.close_page_finish(page, true);

            // If no more sessions, show welcome page (respecting user preference #232)
            if show_welcome_on_close.get()
                && sessions.borrow().is_empty()
                && tab_view.n_pages() == 0
            {
                Self::append_welcome_page(&tab_view);
            }

            glib::Propagation::Stop
        });
    }

    /// Stops expect polling and scrubs resolved responses for a finished child.
    pub fn clear_automation_session(&self, session_id: Uuid) {
        self.automation_sessions.borrow_mut().remove(&session_id);
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

    /// Spawns a session command on its own PTY and attaches it to the terminal.
    ///
    /// `envv` entries override the inherited environment assembled by
    /// [`build_child_env`]. Returns `false` when the session has no terminal or
    /// the command could not be started; a failure also marks the tab
    /// disconnected and offers a reconnect banner.
    pub fn spawn_command(
        &self,
        session_id: Uuid,
        argv: &[&str],
        envv: Option<&[&str]>,
        working_directory: Option<&str>,
        ssh_agent_socket: Option<&str>,
    ) -> bool {
        let Some(terminal) = self.get_terminal(session_id) else {
            return false;
        };

        // A reconnect starts a new child on the same terminal, so the previous
        // session's relay has to go first: dropping it stops its threads and
        // closes its descriptors, and leaving it would keep feeding the widget
        // from a PTY nobody is on any more.
        self.teardown_relay(session_id);

        let env_vec = build_child_env(envv, ssh_agent_socket);
        let env_refs: Vec<&str> = env_vec.iter().map(|e| e.as_str()).collect();
        let command_name = (*argv.first().unwrap_or(&"")).to_owned();
        let size = grid_size(&terminal);

        tracing::debug!(
            command = %command_name,
            %session_id,
            argv = ?argv,
            working_directory = ?working_directory,
            env_count = env_refs.len(),
            rows = size.0,
            cols = size.1,
            "Spawning session command"
        );

        let child = match pty_spawn::spawn_on_pty(argv, &env_refs, working_directory, size) {
            Ok(child) => child,
            Err(e) => {
                tracing::error!(
                    command = %command_name,
                    %session_id,
                    %e,
                    "Failed to spawn session command"
                );
                self.show_spawn_failure(session_id, &command_name, &e);
                return false;
            }
        };

        let (relay, output) = match pty_relay::PtyRelay::start(child.master, size) {
            Ok(started) => started,
            Err(e) => {
                // The child is already running on a PTY nothing will read, so
                // it has to go rather than linger on an invisible terminal.
                reap_child(child.pid);
                tracing::error!(
                    command = %command_name,
                    %session_id,
                    %e,
                    "Failed to start the PTY relay"
                );
                self.show_spawn_failure(
                    session_id,
                    &command_name,
                    &pty_spawn::SpawnError::Failed(e.to_string()),
                );
                return false;
            }
        };

        self.pty_relays.borrow_mut().insert(session_id, relay);
        self.vte_child_pids
            .borrow_mut()
            .insert(session_id, child.pid as i32);

        self.deliver_output_to(session_id, &terminal, output);
        self.forward_input_from(session_id, &terminal);
        self.watch_grid_size(session_id, &terminal);
        watch_child_exit(&terminal, child.pid);

        true
    }

    /// Feeds a session's PTY output to its terminal and to its observers.
    ///
    /// Runs on the GTK main thread for as long as the relay lives; when the
    /// relay is dropped the stream ends and this loop finishes on its own.
    fn deliver_output_to(
        &self,
        session_id: Uuid,
        terminal: &Terminal,
        output: pty_relay::OutputStream,
    ) {
        let terminal = terminal.downgrade();
        let observers = Rc::clone(&self.output_observers);
        glib::spawn_future_local(async move {
            while let Ok(chunk) = output.recv().await {
                if let Some(terminal) = terminal.upgrade() {
                    terminal.feed(&chunk);
                }
                // The list is cloned so that no borrow is held while an observer
                // runs: session logging is then free to touch the notebook.
                let handlers = observers
                    .borrow()
                    .get(&session_id)
                    .cloned()
                    .unwrap_or_default();
                for handler in handlers {
                    handler(&chunk);
                }
            }
        });
    }

    /// Sends whatever the terminal reports as input to the session's PTY.
    ///
    /// Keys, pasted text, mouse reports and the replies VTE makes to terminal
    /// queries all arrive through `commit`, which VTE emits whether or not it
    /// owns a PTY. Connected once per terminal, because the handler resolves the
    /// relay by session and so keeps working across a reconnect.
    fn forward_input_from(&self, session_id: Uuid, terminal: &Terminal) {
        if !self.commit_forwarded.borrow_mut().insert(session_id) {
            return;
        }
        let relays = Rc::clone(&self.pty_relays);
        terminal.connect_commit(move |_terminal, text, size| {
            if let Some(relay) = relays.borrow().get(&session_id) {
                relay.write_input(&pty_relay::commit_bytes(text, size));
            }
        });
    }

    /// Keeps the child's idea of the window size in step with the widget.
    ///
    /// A poll rather than a signal, because VTE has none to offer: no row-count
    /// or column-count notification, `char-size-changed` covers font metrics
    /// only, and GTK4 removed `size-allocate` (see `vte_contract_tests`). The
    /// check is two integer reads, and `TIOCSWINSZ` is only sent when the grid
    /// really changed — a redundant `SIGWINCH` makes full-screen programs
    /// repaint for nothing.
    fn watch_grid_size(&self, session_id: Uuid, terminal: &Terminal) {
        let terminal = terminal.downgrade();
        let relays = Rc::clone(&self.pty_relays);
        let timers = Rc::clone(&self.pty_size_timers);
        let source = glib::timeout_add_local(GRID_SIZE_POLL, move || {
            let Some(terminal) = terminal.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let relays = relays.borrow();
            let Some(relay) = relays.get(&session_id) else {
                // No relay means the session is over, and so is this poll.
                timers.borrow_mut().remove(&session_id);
                return glib::ControlFlow::Break;
            };
            let (rows, cols) = grid_size(&terminal);
            if relay.sync_size(rows, cols) {
                tracing::debug!(%session_id, rows, cols, "Pushed new window size to child");
            }
            glib::ControlFlow::Continue
        });
        if let Some(previous) = self.pty_size_timers.borrow_mut().insert(session_id, source) {
            previous.remove();
        }
    }

    /// Registers a callback for a session's raw PTY output.
    ///
    /// The callback runs on the GTK main thread with each chunk exactly as the
    /// child wrote it — no viewport, no rewrapping, no de-duplication — which is
    /// what a session transcript needs. Several observers can coexist.
    pub fn add_output_observer<F>(&self, session_id: Uuid, observer: F)
    where
        F: Fn(&[u8]) + 'static,
    {
        self.output_observers
            .borrow_mut()
            .entry(session_id)
            .or_default()
            .push(Rc::new(observer));
    }

    /// Stops a session's relay and its window-size poll.
    ///
    /// Output observers deliberately survive: an in-place reconnect keeps the
    /// same session log, and re-registering them would double every line.
    fn teardown_relay(&self, session_id: Uuid) {
        if let Some(timer) = self.pty_size_timers.borrow_mut().remove(&session_id) {
            timer.remove();
        }
        drop(self.pty_relays.borrow_mut().remove(&session_id));
    }

    /// Marks a session's tab as failed and explains why.
    ///
    /// A missing CLI tool is the common case and gets its own wording, because
    /// "not installed" tells the user what to do while an errno does not. The
    /// banner carries a Reconnect button so the tab stays useful after the tool
    /// has been installed.
    fn show_spawn_failure(
        &self,
        session_id: Uuid,
        command_name: &str,
        error: &pty_spawn::SpawnError,
    ) {
        let not_found = error.is_not_found();

        if let Some(page) = self.sessions.borrow().get(&session_id) {
            page.set_indicator_icon(Some(&gio::ThemedIcon::new("network-offline-symbolic")));
            page.set_indicator_activatable(false);

            if let Ok(outer) = page.child().downcast::<GtkBox>()
                && let Some(inner) = outer.first_child()
                && let Ok(container) = inner.downcast::<GtkBox>()
            {
                let connection_id = self
                    .session_info
                    .borrow()
                    .get(&session_id)
                    .map_or_else(Uuid::nil, |i| i.connection_id);

                let banner = GtkBox::new(Orientation::Horizontal, 6);
                banner.set_margin_start(12);
                banner.set_margin_end(12);
                banner.set_margin_top(6);
                banner.set_margin_bottom(6);
                banner.set_halign(gtk4::Align::Center);
                banner.set_widget_name("reconnect-banner");

                let label = gtk4::Label::new(Some(&if not_found {
                    i18n_f("Command not found: {}", &[command_name])
                } else {
                    i18n_f("Failed to start '{}'", &[command_name])
                }));
                label.add_css_class("dim-label");

                let button = gtk4::Button::with_label(&i18n("Reconnect"));
                button.add_css_class("suggested-action");
                button.set_tooltip_text(Some(&i18n("Reconnect to this session")));

                banner.append(&label);
                banner.append(&button);
                container.append(&banner);

                let on_reconnect = self.on_reconnect.clone();
                button.connect_clicked(move |_| {
                    if let Some(ref cb) = *on_reconnect.borrow() {
                        cb(session_id, connection_id);
                    }
                });
            }
        }

        let msg = if not_found {
            i18n_f("'{}' is not installed", &[command_name])
        } else {
            i18n_f(
                "Failed to start '{}': {}",
                &[command_name, &error.to_string()],
            )
        };
        crate::toast::show_error_toast_on_active_window(&msg);
    }
}

/// Returns a terminal's grid as `(rows, columns)`, clamped to `u16`.
///
/// VTE reports these as `i64`; a terminal larger than 65535 cells in either
/// direction does not exist, and `TIOCSWINSZ` could not express it anyway.
fn grid_size(terminal: &Terminal) -> (u16, u16) {
    let clamp = |value: i64| u16::try_from(value.max(0)).unwrap_or(u16::MAX);
    (clamp(terminal.row_count()), clamp(terminal.column_count()))
}

/// Kills a child and collects it, so a failed startup leaves no zombie.
fn reap_child(pid: u32) {
    let pid = nix::unistd::Pid::from_raw(pid as i32);
    let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGKILL);
    let _ = nix::sys::wait::waitpid(pid, None);
}

/// Raises `child-exited` on a terminal when its child process ends.
///
/// VTE only emits that signal for a child it spawned itself, and the whole
/// teardown path hangs off it — reconnect banner, log flush, monitoring, tab
/// state. The GLib watch also performs the `waitpid`, which is why
/// [`pty_spawn::spawn_on_pty`] deliberately leaks its `Child` handle.
fn watch_child_exit(terminal: &Terminal, pid: u32) {
    let terminal = terminal.downgrade();
    glib::child_watch_add_local(glib::Pid(pid as i32), move |_pid, status| {
        tracing::debug!(status, pid, "Session child exited");
        if let Some(terminal) = terminal.upgrade() {
            terminal.emit_by_name::<()>("child-exited", &[&status]);
        }
    });
}

/// Assembles the complete environment for a session's child process.
///
/// The parent environment is inherited so the child sees `HOME`, `DISPLAY` and
/// friends, then four things are layered on top; `envv` overrides all of them.
///
/// Both spawn paths used to carry their own copy of this, which is how the
/// macOS `SSH_ASKPASS_REQUIRE` guard (#161) ended up applied on one path only.
///
/// Entries are zeroizing: a jump host's password is handed to `ssh` through
/// `envv` as an askpass variable, so this vector holds a credential for as long
/// as it takes to spawn.
fn build_child_env(
    envv: Option<&[&str]>,
    ssh_agent_socket: Option<&str>,
) -> Vec<Zeroizing<String>> {
    /// Replaces any existing definitions of `KEY=` before appending `entry`.
    fn set(env_vec: &mut Vec<Zeroizing<String>>, entry: String) {
        if let Some(eq_pos) = entry.find('=') {
            let key_prefix = &entry[..=eq_pos];
            env_vec.retain(|existing| !existing.starts_with(key_prefix));
        }
        env_vec.push(Zeroizing::new(entry));
    }

    // PATH is replaced with the extended version so CLI tools RustConn
    // downloaded itself (Flatpak, snap) are found by name.
    let extended_path = rustconn_core::cli_download::get_extended_path();
    let mut env_vec: Vec<Zeroizing<String>> = std::env::vars()
        .map(|(key, value)| {
            Zeroizing::new(if key == "PATH" {
                format!("PATH={extended_path}")
            } else {
                format!("{key}={value}")
            })
        })
        .collect();
    if std::env::var_os("PATH").is_none() {
        env_vec.push(Zeroizing::new(format!("PATH={extended_path}")));
    }

    // SSH agent: an explicit socket wins over the process-wide agent RustConn
    // started, which in turn wins over whatever the desktop session exported.
    if let Some(custom_socket) = ssh_agent_socket {
        set(&mut env_vec, format!("SSH_AUTH_SOCK={custom_socket}"));
    } else if let Some(agent_info) = rustconn_core::sftp::get_agent_info() {
        set(
            &mut env_vec,
            format!("SSH_AUTH_SOCK={}", agent_info.socket_path),
        );
        if let Some(ref pid) = agent_info.pid {
            set(&mut env_vec, format!("SSH_AGENT_PID={pid}"));
        }
    }

    // Strip host SSH_ASKPASS — RustConn types passwords into the session
    // itself, so the host askpass program (e.g. ksshaskpass) is never needed
    // and may not exist inside a sandbox (#48).
    env_vec.retain(|e| !e.starts_with("SSH_ASKPASS="));

    // On macOS, ssh may still try its compiled-in askpass path (XQuartz) with
    // SSH_ASKPASS unset; OpenSSH >= 8.4 honours this instead (#161).
    #[cfg(target_os = "macos")]
    set(&mut env_vec, "SSH_ASKPASS_REQUIRE=never".to_owned());

    push_sandbox_cli_config(&mut env_vec);

    // ncurses programs (mc, htop) need TERM to detect colour and mouse
    // support; a GUI process usually has none, and a Flatpak sandbox may
    // inherit TERM=dumb. xterm-256color is universally available.
    if !env_vec.iter().any(|e| e.starts_with("TERM="))
        || rustconn_core::flatpak::is_flatpak()
        || env_vec.iter().any(|e| e.as_str() == "TERM=dumb")
    {
        set(&mut env_vec, "TERM=xterm-256color".to_owned());
    }

    if let Some(user_env) = envv {
        for entry in user_env {
            set(&mut env_vec, (*entry).to_owned());
        }
    }

    env_vec
}

/// Redirects CLI config directories to writable locations inside a sandbox.
///
/// Host directories are either mounted read-only (gcloud, Azure, kubectl) or
/// not mounted at all (Teleport, OCI). Boundary needs nothing: it reaches the
/// system keyring over D-Bus, which works in a sandbox. Cloudflare Tunnel needs
/// nothing either: `cloudflared access ssh` authenticates in a browser with
/// short-lived tokens and keeps no config.
fn push_sandbox_cli_config(env_vec: &mut Vec<Zeroizing<String>>) {
    let dirs: [(&str, Option<PathBuf>); 4] = if rustconn_core::flatpak::is_flatpak() {
        [
            (
                "CLOUDSDK_CONFIG",
                rustconn_core::flatpak::get_flatpak_gcloud_config_dir(),
            ),
            (
                "AZURE_CONFIG_DIR",
                rustconn_core::flatpak::get_flatpak_azure_config_dir(),
            ),
            (
                "TELEPORT_HOME",
                rustconn_core::flatpak::get_flatpak_teleport_config_dir(),
            ),
            (
                "OCI_CLI_CONFIG_FILE",
                rustconn_core::flatpak::get_flatpak_oci_config_dir().map(|dir| dir.join("config")),
            ),
        ]
    } else if rustconn_core::is_snap() {
        // The personal-files plugs expose host credentials read-only, so
        // writable config lives under $SNAP_USER_DATA.
        [
            (
                "CLOUDSDK_CONFIG",
                rustconn_core::snap::get_snap_gcloud_config_dir(),
            ),
            (
                "AZURE_CONFIG_DIR",
                rustconn_core::snap::get_snap_azure_config_dir(),
            ),
            (
                "TELEPORT_HOME",
                rustconn_core::snap::get_snap_teleport_config_dir(),
            ),
            (
                "OCI_CLI_CONFIG_FILE",
                rustconn_core::snap::get_snap_oci_config_dir().map(|dir| dir.join("config")),
            ),
        ]
    } else {
        return;
    };

    for (key, dir) in dirs {
        let prefix = format!("{key}=");
        // An inherited or caller-provided value is left alone.
        if let Some(dir) = dir
            && !env_vec.iter().any(|e| e.starts_with(&prefix))
        {
            env_vec.push(Zeroizing::new(format!("{prefix}{}", dir.display())));
        }
    }
}

impl TerminalNotebook {
    /// Spawns an SSH command in the terminal
    ///
    /// Unlike [`Self::spawn_telnet`], this does **not** apply the connection's
    /// erase mode: every caller must call [`Self::set_erase_mode`] itself with
    /// `conn.protocol_config.erase_modes()` before spawning, or the session
    /// silently falls back to the global defaults (issue
    /// [#271](https://github.com/totoshko88/RustConn/issues/271)). The two
    /// bytes are not parameters here because this signature is already at the
    /// point where the project's argument limit is waived below, and one of the
    /// callers (Quick Connect in `window::edit_dialogs`) has no stored
    /// connection to read them from — it wants the defaults.
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
        use_mptcp: bool,
    ) -> bool {
        let mut argv = if use_waypipe {
            if use_mptcp {
                vec!["mptcpize", "run", "waypipe", "ssh"]
            } else {
                vec!["waypipe", "ssh"]
            }
        } else if use_mptcp {
            vec!["mptcpize", "run", "ssh"]
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

    /// Points a session's Backspace and Delete keys at the configured bytes.
    ///
    /// Applies to the session's live VTE widget, so it has to run after the tab
    /// exists and after [`config::configure_terminal_with_settings`] has
    /// installed the defaults — see [`config::apply_erase_mode`]. Re-applying it
    /// on every spawn is what keeps the choice in force across a reconnect into
    /// the same terminal. A session that has already gone away is ignored.
    pub fn set_erase_mode(
        &self,
        session_id: Uuid,
        backspace_sends: BackspaceSends,
        delete_sends: DeleteSends,
    ) {
        if let Some(terminal) = self.terminals.borrow().get(&session_id) {
            config::apply_erase_mode(terminal, backspace_sends, delete_sends);
        }
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
        backspace_sends: BackspaceSends,
        delete_sends: DeleteSends,
    ) -> bool {
        self.set_erase_mode(session_id, backspace_sends, delete_sends);

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
        self.disconnected_sessions.borrow_mut().remove(&session_id);
        // Cancel any background polling (auto-reconnect, host check) for this session
        self.cancel_poll(session_id);
        // A detached session has no tab page to close, so route it through the
        // tabless path — otherwise "close this session" from the session
        // manager or a clean exit with close-on-clean-exit would silently do
        // nothing and leave the detached window behind (issue #236).
        if self.is_detached(session_id) {
            self.close_session(session_id);
            return;
        }
        let page = self.sessions.borrow().get(&session_id).cloned();
        if let Some(page) = page {
            self.tab_view.close_page(&page);
        }
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

    /// Sets whether an in-place reconnect keeps the previous scrollback (#253).
    pub fn set_keep_history_on_reconnect(&self, enabled: bool) {
        self.keep_history_on_reconnect.set(enabled);
    }

    /// Sets the maximum scrollback lines to retain after a reconnect.
    pub fn set_max_scrollback_on_reconnect(&self, limit: Option<u32>) {
        self.max_scrollback_on_reconnect.set(limit);
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

    /// Updates whether the Welcome tab is shown when no sessions are open (issue #232)
    pub fn set_show_welcome(&self, enabled: bool) {
        self.show_welcome.set(enabled);
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

    /// Returns the live session → terminal map.
    ///
    /// Handed to a split-view bridge so it can tell a terminal session from an
    /// embedded viewer. A bridge with its own empty map reports every session as
    /// embedded, which silently disables keystroke broadcast.
    #[must_use]
    pub fn shared_terminals(&self) -> Rc<RefCell<HashMap<Uuid, Terminal>>> {
        Rc::clone(&self.terminals)
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

    /// Gets the cursor row of a terminal session, relative to its connect.
    ///
    /// VTE's `cursor_position()` returns `(column, row)` with the row in
    /// absolute buffer coordinates — scrollback included. Callers use the row
    /// to tell whether the session produced output past its connect banner, so
    /// the value is reported relative to the row the current connection started
    /// on. That is 0 for a fresh session and, when a reconnect keeps the
    /// previous scrollback, the row the preserved history ended on (issue #253).
    pub fn get_terminal_cursor_row(&self, session_id: Uuid) -> Option<i64> {
        let row = self.get_terminal(session_id)?.cursor_position().1;
        let mut bases = self.cursor_row_base.borrow_mut();
        let Some(base) = bases.get_mut(&session_id) else {
            return Some(row);
        };
        Some((row - *base.get_or_insert(row)).max(0))
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

    /// Returns the content box that currently hosts a session's live widget.
    ///
    /// A tabbed session resolves through its page, exactly as
    /// [`Self::get_session_container`] does. A detached session has no page, so
    /// its box is the parent of the widget [`Self::build_session_content`]
    /// wrapped — which is the very box handed to its window. Split guests
    /// deliberately resolve to `None`: their widget lives inside another
    /// session's layout, which is not theirs to add chrome to.
    ///
    /// Used by everything that decorates a session in place (reconnect banner,
    /// monitoring bar) so the decoration follows the session between windows
    /// (issue #236).
    #[must_use]
    pub fn session_content_box(&self, session_id: Uuid) -> Option<GtkBox> {
        if let Some(container) = self.get_session_container(session_id) {
            return Some(container);
        }
        if !self.is_detached(session_id) {
            return None;
        }
        // A VTE session sits one level deeper than an embedded viewer: its
        // overlay is the direct child of the content box.
        let overlay = self.terminal_overlays.borrow().get(&session_id).cloned();
        let anchor: Widget = match overlay {
            Some(overlay) => overlay.upcast(),
            None => self.get_session_display_widget(session_id)?,
        };
        anchor.parent()?.downcast::<GtkBox>().ok()
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

    /// Switches to a specific tab by session ID.
    ///
    /// A detached session has no tab, so the request is routed to the
    /// `on_focus_detached` callback instead, which presents its window. Every
    /// existing focus call site (sidebar activation, session manager, workspace
    /// restore) therefore works unchanged for detached sessions.
    pub fn switch_to_tab(&self, session_id: Uuid) {
        if self.is_detached(session_id) {
            self.notify_focus_detached(session_id);
            return;
        }
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

    /// Gets the current terminal text content for prompt and banner detection.
    ///
    /// Reads the visible viewport. VTE addresses the whole scrollback and the
    /// visible area in one coordinate system, so rows `0..row_count` are the
    /// *oldest* scrollback lines as soon as anything has scrolled off — the
    /// same trap the highlight overlay had to fix (issue #154). Anchoring to
    /// the viewport keeps this correct now that a reconnect can start on a
    /// non-empty buffer (issue #253).
    #[must_use]
    pub fn get_terminal_text(&self, session_id: Uuid) -> Option<String> {
        self.get_terminal(session_id).map(|terminal| {
            let row_count = terminal.row_count();
            let col_count = terminal.column_count();
            #[expect(
                clippy::cast_possible_truncation,
                reason = "adjustment value is a row index bounded by the scrollback size"
            )]
            let top = terminal
                .vadjustment()
                .map_or(0_i64, |adjustment| adjustment.value() as i64);
            let (text, _len) =
                terminal.text_range_format(vte4::Format::Text, top, 0, top + row_count, col_count);
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

    /// Re-applies per-connection erase modes after global settings change.
    ///
    /// [`Self::apply_settings`] runs every live terminal back through
    /// [`config::configure_terminal_with_settings`], which reinstalls the global
    /// defaults — so saving anything in Preferences → Terminal used to silently
    /// put Backspace back to `^?` on a session that had asked for `^H`, until it
    /// was reconnected (issue
    /// [#271](https://github.com/totoshko88/RustConn/issues/271)). Call this
    /// straight after `apply_settings`, exactly as
    /// [`Self::reapply_theme_overrides`] is called for the same reason.
    ///
    /// A session whose connection has since been deleted, or whose protocol has
    /// no such setting, is re-set to the defaults rather than skipped: its
    /// terminal has just been overwritten too, and the defaults are what it
    /// should be showing.
    pub fn reapply_erase_modes<F>(&self, get_erase_modes: F)
    where
        F: Fn(Uuid) -> Option<(BackspaceSends, DeleteSends)>,
    {
        let terminals = self.terminals.borrow();
        let session_info = self.session_info.borrow();
        for (session_id, terminal) in terminals.iter() {
            let (backspace_sends, delete_sends) = session_info
                .get(session_id)
                .and_then(|info| get_erase_modes(info.connection_id))
                .unwrap_or_default();
            config::apply_erase_mode(terminal, backspace_sends, delete_sends);
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

    /// Renames every open session of a connection, returning the ids it touched.
    ///
    /// A connection rename used to leave open sessions showing the old name.
    /// Updates the session metadata and the tab chrome (title, tooltip, group
    /// prefix); the caller updates whatever else names the session — the title
    /// of a detached window, for one (issue #236).
    pub fn rename_connection_sessions(&self, connection_id: Uuid, new_name: &str) -> Vec<Uuid> {
        let affected: Vec<(Uuid, Option<String>, Option<String>)> = self
            .session_info
            .borrow_mut()
            .iter_mut()
            .filter(|(_, info)| info.connection_id == connection_id)
            .map(|(id, info)| {
                info.name = new_name.to_owned();
                (*id, info.tab_group.clone(), info.host.clone())
            })
            .collect();

        for (session_id, group, host) in &affected {
            // The page is bound to its own `let` first: an `if let` scrutinee
            // temporary would keep the `sessions` borrow alive across the two
            // GTK setters below.
            let page = self.sessions.borrow().get(session_id).cloned();
            if let Some(page) = page {
                page.set_title(&tab_title(new_name, group.as_deref()));
                page.set_tooltip(&Self::tab_tooltip(
                    new_name,
                    host.as_deref(),
                    group.as_deref(),
                ));
            }
        }
        if !affected.is_empty() {
            tracing::debug!(
                connection = %connection_id,
                sessions = affected.len(),
                "renamed open sessions after a connection rename"
            );
        }
        affected.into_iter().map(|(id, _, _)| id).collect()
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
            // The rendered title is the only record of the base name here, so an
            // existing prefix comes off before the new one goes on.
            let current_title = page.title().to_string();
            page.set_title(&tab_title(
                strip_group_prefix(&current_title),
                Some(group_name),
            ));
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
        // Cluster membership is resolved here rather than in each creation path
        // so that every protocol is covered, and before the callback so an
        // observer sees the tab already labelled with its cluster.
        self.resolve_cluster_pending(connection_id, session_id);

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

    /// Registers a session as part of a cluster, under the cluster's group name.
    pub fn register_cluster_terminal(&self, cluster_id: Uuid, group: &str, session_id: Uuid) {
        self.cluster_sessions
            .borrow_mut()
            .entry(cluster_id)
            .or_insert_with(|| ClusterTabs {
                group: group.to_owned(),
                sessions: Vec::new(),
            })
            .sessions
            .push(session_id);
        self.session_to_cluster
            .borrow_mut()
            .insert(session_id, cluster_id);
    }

    /// Unregisters all tabs of a cluster, retiring its tab group if it is unused.
    ///
    /// Called after the member tabs have been closed, so by now their sessions
    /// are gone from `session_info` and the only sessions that could still be
    /// wearing the name are unrelated ones — a tab a user labelled by hand, or a
    /// second cluster with the same name. Without this, every cluster ever opened
    /// would leave its name in the "Set Group…" chooser for the lifetime of the
    /// window, which is a new problem created by naming groups automatically.
    pub fn unregister_cluster(&self, cluster_id: Uuid) {
        if let Some(tabs) = self.cluster_sessions.borrow_mut().remove(&cluster_id) {
            let mut reverse = self.session_to_cluster.borrow_mut();
            for sid in &tabs.sessions {
                reverse.remove(sid);
            }
            drop(reverse);

            // The cluster's own sessions are excluded rather than assumed gone:
            // `close_page` teardown is not guaranteed to have removed them from
            // `session_info` by the time the caller unregisters, and the question
            // being asked is whether anything *else* wears the name.
            //
            // ponytail: O(members × open tabs); a cluster is a handful of hosts,
            // so a set would cost more than it saves.
            let still_used = {
                let info = self.session_info.borrow();
                group_still_in_use(
                    &tabs.group,
                    info.iter()
                        .filter(|(session_id, _)| !tabs.sessions.contains(session_id))
                        .map(|(_, info)| info.tab_group.as_deref()),
                )
            };
            if still_used {
                tracing::debug!(
                    group = tabs.group,
                    "cluster closed; keeping its group name, other tabs still carry it"
                );
            } else {
                self.tab_group_manager
                    .borrow_mut()
                    .remove_group(&tabs.group);
            }
        }
        // Clear any pending registrations for this cluster
        self.cluster_pending
            .borrow_mut()
            .retain(|_, pending| pending.cluster_id != cluster_id);
    }

    /// Marks a connection as pending cluster registration.
    ///
    /// When the tab for `connection_id` is eventually created — synchronously,
    /// or after an async port check — it is registered as part of `cluster_id`
    /// and labelled with the cluster's `group` name.
    pub fn mark_cluster_pending(&self, cluster_id: Uuid, group: &str, connection_id: Uuid) {
        self.cluster_pending.borrow_mut().insert(
            connection_id,
            PendingCluster {
                cluster_id,
                group: group.to_owned(),
            },
        );
    }

    /// Resolves a pending cluster registration for a freshly created tab.
    ///
    /// Called from [`Self::notify_tab_added`], which every tab-creation path goes
    /// through — terminal, VNC, embedded RDP, embedded Web and external process
    /// alike. It used to be called from the terminal path only, so an RDP or VNC
    /// member of a cluster opened a tab that was never registered: it stayed in
    /// `cluster_pending` forever and "Disconnect all cluster sessions" could not
    /// see it.
    ///
    /// Labelling the tab with the cluster's name is what makes a cluster visible
    /// after it is open: the tab reads `[cluster] host`, and every tab-group
    /// operation — Close All in Group above all — then applies to the cluster
    /// without needing a cluster-specific command.
    fn resolve_cluster_pending(&self, connection_id: Uuid, session_id: Uuid) {
        let Some(pending) = self.cluster_pending.borrow_mut().remove(&connection_id) else {
            return;
        };
        self.register_cluster_terminal(pending.cluster_id, &pending.group, session_id);
        self.set_tab_group(session_id, &pending.group);
    }

    /// Gets all session IDs for a cluster
    pub fn get_cluster_sessions(&self, cluster_id: Uuid) -> Vec<Uuid> {
        self.cluster_sessions
            .borrow()
            .get(&cluster_id)
            .map(|tabs| tabs.sessions.clone())
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

    /// Sets the monitoring coordinator used by the detach and attach paths.
    ///
    /// Must be called after construction so moving a session between its tab
    /// and a detached window suspends the monitoring bar before the widget move
    /// and resumes it into the new content box afterwards. Without it the
    /// detach paths simply leave monitoring untouched.
    pub fn set_monitoring_coordinator(&self, coordinator: Rc<MonitoringCoordinator>) {
        *self.monitoring.borrow_mut() = Some(coordinator);
    }
}

impl Default for TerminalNotebook {
    fn default() -> Self {
        Self::new(true)
    }
}

/// Builds the separator fed into a terminal that reconnects with its history.
///
/// Opens a fresh line (the dead session's output may end mid-line), then a dim
/// rule carrying `label`, so the preserved scrollback and the new session's
/// output stay visually apart (issue #253). The returned string contains VTE
/// escape sequences and is meant for `Terminal::feed`, not for display.
fn reconnect_separator(label: &str) -> String {
    format!("\r\n\x1b[2m── {label} ──\x1b[0m\r\n")
}

/// Extracts the text of the line under the cursor of a VTE terminal.
///
/// Returns the cursor's row via `text_range_format`. When that row is empty
/// (prompt glyphs not yet committed to the grid), falls back to the last
/// non-empty line of the screen ending at the cursor. Returns `None` when no
/// non-empty text can be extracted. Never panics. Backs
/// `TerminalNotebook::get_cursor_line_text` for cursor-position-based prompt
/// detection (issue #194).
fn cursor_line_text(terminal: &Terminal) -> Option<String> {
    let col_count = terminal.column_count();
    // `cursor_position()` returns `(column, row)` with the row in absolute
    // buffer coordinates — the same coordinates `text_range_format` takes.
    let (_col, row) = terminal.cursor_position();
    let (cursor_text, _len) =
        terminal.text_range_format(vte4::Format::Text, row, 0, row, col_count);
    if let Some(line) = cursor_text {
        let line = line.to_string();
        if !line.trim().is_empty() {
            return Some(line);
        }
    }

    // Fallback: last non-empty line of the screen ending at the cursor.
    // Rows are absolute buffer coordinates, so reading from 0 would return the
    // oldest scrollback lines whenever the buffer is not empty — which is the
    // normal case once a reconnect keeps the previous history (issue #253).
    let row_count = terminal.row_count();
    let start = (row - row_count + 1).max(0);
    let (grid_text, _len) =
        terminal.text_range_format(vte4::Format::Text, start, 0, row, col_count);
    grid_text.and_then(|g| {
        g.to_string()
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .map(str::to_owned)
    })
}

#[cfg(test)]
mod reconnect_separator_tests {
    use super::reconnect_separator;

    #[test]
    fn separator_opens_and_closes_its_own_line() {
        let separator = reconnect_separator("Reconnected at 2026-08-02 14:33:07");
        // A fresh line on both sides: the dead session's output may end
        // mid-line, and the new session must not start on the rule itself.
        assert!(separator.starts_with("\r\n"));
        assert!(separator.ends_with("\r\n"));
    }

    #[test]
    fn separator_carries_the_label_and_resets_the_attributes() {
        let separator = reconnect_separator("Reconnected at 2026-08-02 14:33:07");
        assert!(separator.contains("Reconnected at 2026-08-02 14:33:07"));
        // Dim only the rule — leaving SGR set would tint the new session.
        assert!(separator.contains("\x1b[2m"));
        assert!(separator.contains("\x1b[0m"));
        assert!(
            separator.rfind("\x1b[0m") > separator.rfind("\x1b[2m"),
            "the reset has to come after the dim attribute"
        );
    }
}

#[cfg(test)]
mod tab_tooltip_tests {
    use super::TerminalNotebook;

    #[test]
    fn title_only_tooltip_is_just_the_title() {
        assert_eq!(
            TerminalNotebook::tab_tooltip("prod-db", None, None),
            "prod-db"
        );
        // An empty host must not add a blank second line.
        assert_eq!(
            TerminalNotebook::tab_tooltip("prod-db", Some(""), None),
            "prod-db"
        );
    }

    #[test]
    fn host_and_group_each_get_their_own_line() {
        assert_eq!(
            TerminalNotebook::tab_tooltip("prod-db", Some("10.0.0.5"), None),
            "prod-db\n10.0.0.5"
        );
        assert_eq!(
            TerminalNotebook::tab_tooltip("prod-db", None, Some("Production")),
            "prod-db\n[Production]"
        );
        // The group line stays last, so the group strip/append logic in
        // `set_tab_group` keeps finding it and leaves the host line alone.
        assert_eq!(
            TerminalNotebook::tab_tooltip("prod-db", Some("10.0.0.5"), Some("Production")),
            "prod-db\n10.0.0.5\n[Production]"
        );
    }
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
    #[ignore = "initialises GTK: needs a display and its own process; run alone with `cargo test -p rustconn --bin rustconn -- --ignored --exact <this test path>`"]
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

/// Contract tests for the VTE behaviours the PTY relay is built on (#247).
///
/// The transcript has been rewritten three times because each attempt reasoned
/// about how VTE addresses rows and delivers input instead of checking. These
/// tests check. They need a display and initialise GTK, which can only happen
/// once per process, so they are opt-in:
///
/// ```text
/// cargo test -p rustconn --bin rustconn -- --ignored --exact \
///     terminal::vte_contract_tests::<name>
/// ```
#[cfg(test)]
mod vte_contract_tests {
    use vte4::TerminalExt;

    const OPT_IN: &str = "initialises GTK: needs a display and its own process";

    /// Pumps the GLib main context until `ready` holds, or the deadline passes.
    ///
    /// VTE parses fed bytes from a queued source rather than inside `feed`, so
    /// nothing is observable until the loop runs.
    fn pump_until(ready: impl Fn() -> bool) -> bool {
        let ctx = gtk4::glib::MainContext::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !ready() && std::time::Instant::now() < deadline {
            ctx.iteration(false);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        ready()
    }

    /// `commit` carries keyboard and pasted input even with no PTY attached.
    ///
    /// This is what lets RustConn own the master file descriptor: VTE renders
    /// and handles keys, and every byte it would have written to a PTY of its
    /// own arrives here instead. VTE guarantees it explicitly — `send_child`
    /// emits `commit` before checking for a PTY, for
    /// [vte#222](https://gitlab.gnome.org/GNOME/vte/-/issues/222) — and the
    /// whole relay collapses without it, so it is pinned here.
    #[test]
    #[ignore = "initialises GTK: needs a display and its own process"]
    fn commit_fires_without_a_pty() {
        if gtk4::init().is_err() {
            return;
        }
        let terminal = super::Terminal::new();
        assert!(terminal.pty().is_none(), "no PTY is the point of this test");

        let seen = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
        let seen_in_handler = seen.clone();
        terminal.connect_commit(move |_t, text, _size| {
            seen_in_handler.borrow_mut().push_str(text);
        });

        terminal.feed_child(b"whoami\r");
        assert!(
            pump_until(|| !seen.borrow().is_empty()),
            "commit must fire without a PTY, or the relay cannot receive input ({OPT_IN})"
        );
        assert_eq!(
            *seen.borrow(),
            "whoami\r",
            "commit must carry the bytes verbatim"
        );
    }

    /// Neither erase binding may be left for VTE to resolve on its own.
    ///
    /// Owning the PTY means VTE has no termios to read `VERASE` from, and
    /// vte 0.84 does not treat that as a reason to stop: `map_erase_binding()`
    /// reaches `assert(auto_mode != eTTY)` and aborts the process, so one
    /// Backspace press was enough to take the window down. A configured
    /// terminal must therefore name both bindings — see
    /// [`super::config::apply_erase_bindings`].
    #[test]
    #[ignore = "initialises GTK: needs a display and its own process"]
    fn erase_bindings_are_never_left_to_vte() {
        if gtk4::init().is_err() {
            return;
        }
        let terminal = super::Terminal::new();
        assert!(terminal.pty().is_none(), "no PTY is the point of this test");

        super::config::configure_terminal_with_settings(
            &terminal,
            &rustconn_core::config::TerminalSettings::default(),
        );

        for (key, binding) in [
            ("Backspace", terminal.backspace_binding()),
            ("Delete", terminal.delete_binding()),
        ] {
            assert!(
                !matches!(binding, vte4::EraseBinding::Auto | vte4::EraseBinding::Tty),
                "{key} is left as {binding:?}: with no PTY, vte 0.84 aborts \
                 while resolving that ({OPT_IN})"
            );
        }
    }

    /// A per-connection erase mode may not reintroduce the abort either.
    ///
    /// `Automatic` is the tempting place to hand the decision back to VTE, and
    /// the option exists for hosts whose users press Backspace constantly, so
    /// every combination offered by the connection editor (issue
    /// [#271](https://github.com/totoshko88/RustConn/issues/271)) is checked
    /// against the same rule as the defaults above.
    #[test]
    #[ignore = "initialises GTK: needs a display and its own process"]
    fn configured_erase_modes_are_never_left_to_vte() {
        use rustconn_core::models::{BackspaceSends, DeleteSends};

        if gtk4::init().is_err() {
            return;
        }
        let terminal = super::Terminal::new();
        assert!(terminal.pty().is_none(), "no PTY is the point of this test");

        for backspace in BackspaceSends::all() {
            for delete in DeleteSends::all() {
                super::config::apply_erase_mode(&terminal, *backspace, *delete);

                for (key, binding) in [
                    ("Backspace", terminal.backspace_binding()),
                    ("Delete", terminal.delete_binding()),
                ] {
                    assert!(
                        !matches!(binding, vte4::EraseBinding::Auto | vte4::EraseBinding::Tty),
                        "{backspace:?}/{delete:?} leaves {key} as {binding:?}: \
                         with no PTY, vte 0.84 aborts while resolving that ({OPT_IN})"
                    );
                }
            }
        }
    }

    /// No VTE signal announces a geometry change, which is why the relay polls.
    ///
    /// Owning the PTY means RustConn has to tell the child when the window
    /// changes size, and there is no event to hang that on: VTE exposes no
    /// row-count or column-count signal, `char-size-changed` covers font
    /// metrics only, GTK4 removed `GtkWidget::size-allocate`, and
    /// `contents-changed` — the obvious candidate, already used elsewhere in
    /// this file — does not fire for a resize, as asserted below. So
    /// [`super::pty_relay::PtyRelay`] compares the grid size on a timer
    /// instead. Should a future VTE grow a suitable signal, this test starts
    /// failing and the poll can go.
    #[test]
    #[ignore = "initialises GTK: needs a display and its own process"]
    fn geometry_change_raises_no_contents_changed() {
        if gtk4::init().is_err() {
            return;
        }
        let terminal = super::Terminal::new();
        terminal.set_size(80, 24);
        terminal.feed(b"anchor\r\n");
        assert!(pump_until(|| terminal.cursor_position().1 > 0), "{OPT_IN}");

        let fired = std::rc::Rc::new(std::cell::Cell::new(false));
        let fired_in_handler = fired.clone();
        terminal.connect_contents_changed(move |_t| fired_in_handler.set(true));

        terminal.set_size(100, 30);
        // Give it every chance to arrive before concluding that it does not.
        pump_until(|| fired.get());
        assert!(
            !fired.get(),
            "contents-changed now fires on a resize: the relay's winsize poll \
             can be replaced by this signal"
        );
        assert_eq!(
            (terminal.column_count(), terminal.row_count()),
            (100, 30),
            "the grid itself did change, so polling it detects the resize"
        );
    }

    /// `cursor_position` and `text_range_format` address the same rows.
    ///
    /// Kept from the row-anchored transcript that the relay replaced: prompt
    /// detection (`cursor_line_text`) still mixes the two, and issue
    /// [#253](https://github.com/totoshko88/RustConn/issues/253) came from
    /// getting this wrong.
    #[test]
    #[ignore = "initialises GTK: needs a display and its own process"]
    fn cursor_row_and_text_range_share_coordinates() {
        if gtk4::init().is_err() {
            return;
        }
        let terminal = super::Terminal::new();
        terminal.set_size(80, 24);
        terminal.feed(b"alpha\r\nbravo\r\ncharlie\r\ndelta\r\n");
        assert!(pump_until(|| terminal.cursor_position().1 > 0), "{OPT_IN}");

        assert_eq!(
            terminal.cursor_position().1,
            4,
            "four newline-terminated lines leave the cursor on row 4"
        );

        let (first, _) = terminal.text_range_format(vte4::Format::Text, 0, 0, 0, 80);
        assert_eq!(
            first.map(|g| g.to_string().trim_end().to_owned()),
            Some("alpha".to_owned()),
            "row 0 is the first line fed, so both APIs count rows the same way"
        );

        let (bounded, _) = terminal.text_range_format(vte4::Format::Text, 1, 0, 2, 80);
        let bounded = bounded.map(|g| g.to_string()).unwrap_or_default();
        assert!(
            bounded.contains("bravo") && bounded.contains("charlie"),
            "requested rows must be present: {bounded:?}"
        );
        assert!(
            !bounded.contains("delta"),
            "the end row must bound the range: {bounded:?}"
        );
    }

    /// Widening the terminal renumbers every row below a wrapped line.
    ///
    /// This is why the transcript is no longer scraped from the widget at all.
    /// A session's terminal is resized when the widget receives its real
    /// allocation — around the first capture of a session — and again whenever
    /// the user resizes the window. VTE rewraps, a wrapped logical line stops
    /// occupying two rows, and everything below it moves: an absolute row
    /// recorded before the change points at different text afterwards, which
    /// duplicated some lines and skipped others (issue #247).
    #[test]
    #[ignore = "initialises GTK: needs a display and its own process"]
    fn widening_renumbers_rows_below_a_wrapped_line() {
        if gtk4::init().is_err() {
            return;
        }
        let terminal = super::Terminal::new();
        terminal.set_size(40, 10);
        terminal.feed(b"short\r\n");
        terminal.feed("wrapped-".repeat(9).as_bytes()); // 72 chars: two rows at 40
        terminal.feed(b"\r\n");
        assert!(pump_until(|| terminal.cursor_position().1 >= 3), "{OPT_IN}");

        let before_cursor = terminal.cursor_position().1;
        let (before_row0, _) = terminal.text_range_format(vte4::Format::Text, 0, 0, 0, 40);

        terminal.set_size(100, 10);
        assert!(
            pump_until(|| terminal.cursor_position().1 < before_cursor),
            "{OPT_IN}"
        );

        let (after_row0, _) = terminal.text_range_format(vte4::Format::Text, 0, 0, 0, 100);
        assert_eq!(
            before_row0.map(|g| g.to_string().trim_end().to_owned()),
            after_row0.map(|g| g.to_string().trim_end().to_owned()),
            "a row above the wrap point keeps its content"
        );
        assert_eq!(
            (before_cursor, terminal.cursor_position().1),
            (3, 2),
            "the wrapped line collapses to one row and pulls the rest upwards"
        );
    }
}
