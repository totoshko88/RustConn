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
mod callbacks;
mod child_env;
mod child_teardown;
mod cluster;
mod config;
mod detach;
pub use detach::{DetachMonitor, DetachPresentation};
pub use output_start::ConnectionOutput;
pub use split_title::title_session;
mod failed_marker;
pub mod file_drop;
pub mod highlight_overlay;
mod mark_registry;
mod notebook_output;
pub mod osc52;
mod output_start;
pub mod playback;
pub mod pty_relay;
pub mod pty_spawn;
mod recording;
pub mod safe_paste;
pub(crate) mod script_recorder;
mod selection_hold;
mod session_lifecycle;
mod shortcuts;
mod spawn;
mod split_close;
mod split_title;
pub mod tab_container;
mod tab_labels;
mod tab_lifecycle;
mod tab_menu;
mod tab_transfer;
mod terminal_ops;
pub mod termprops;
mod types;
#[cfg(test)]
mod vte_contract_tests;

pub(crate) use tab_transfer::{
    accept_tab_drops, claim_tab_drop, clamp_drop_position, drag_single_tab, new_window_view,
    return_transferred_tabs, set_drop_handler, set_new_window_handler,
};

pub(crate) use child_env::ChildCleanup;
use child_env::{build_child_env, filter_program_is_runnable, reap_child, watch_child_exit};
use notebook_output::grid_size;
#[cfg(test)]
use notebook_output::read_connection_output;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Orientation, Widget, gio, glib};
use libadwaita as adw;
use libadwaita::prelude::*;
use rustconn_core::models::{AutomationConfig, BackspaceSends, DeleteSends, RdpKeyboardHook};
use rustconn_core::terminal_themes::TerminalTheme;
pub use types::{
    BROADCAST_TITLE_MARKER, ClusterTabs, LOCAL_SHELL_PROTOCOL, PendingCluster, SCRIPT_TITLE_MARKER,
    SHARED_TITLE_MARKER, SessionWidgetStorage, TerminalSession, group_still_in_use,
    local_shell_label, regroup_title, tab_title, with_title_marker,
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
use crate::session::VncSessionWidget;
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
    /// The connection's own ssh-agent socket (connection or group), if any
    pub agent_socket: Option<String>,
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

/// Which part of the application owns a session's `child-exited` handler.
///
/// A single VTE widget carries several independent handlers, and each of them
/// has to be replaceable on its own: an in-place reconnect re-registers the
/// disconnect path on the same widget and must not end up running it twice
/// (issue #283), while session logging registers from an async callback long
/// after the disconnect path is already wired. Keying the handler map by
/// purpose is what lets a re-registration replace its own predecessor instead
/// of evicting somebody else's handler (issue #297).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChildExitHook {
    /// Session teardown: disconnect indicator, reconnect banner, history
    /// record, post-disconnect task and auto-reconnect.
    Disconnect,
    /// Shutting the activity monitor's timers down.
    ActivityMonitor,
    /// Flushing the session-log transcript's last partial line.
    Transcript,
    /// Flushing the session log so it is readable while the tab lingers.
    SessionLog,
}

/// Shared slot for the reconnect banner's cloud login callback.
type CloudLoginCallback =
    Rc<RefCell<Option<Box<dyn Fn(Uuid, Uuid, rustconn_core::protocol::CloudLogin)>>>>;

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
    /// Persistent all-protocol callback fired when a session is shown
    /// (session_id, connection_id). Parallel to `on_page_closed`; drives the
    /// auto-hide-on-connect feature (issue #401). Unlike the one-shot
    /// `on_tab_added`, this one is restored after firing.
    on_session_shown: Rc<RefCell<Option<Box<dyn Fn(Uuid, Uuid)>>>>,
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
    /// Direct tracking of split view colors per session (session_id → color_index).
    /// Used to prevent clear operations from overwriting split indicators.
    split_session_colors: Rc<RefCell<HashMap<Uuid, usize>>>,
    /// Tab group manager for assigning colors to named groups
    tab_group_manager: Rc<RefCell<TabGroupManager>>,
    /// Callback for reconnect button clicks (session_id, connection_id)
    on_reconnect: Rc<RefCell<Option<Box<dyn Fn(Uuid, Uuid)>>>>,
    /// Callback run with the connection ID when a session's command could not
    /// be started (missing client binary, PTY failure). Such a session has no
    /// process, so no `child-exited` ever follows to mark it failed.
    on_spawn_failed: Rc<RefCell<Option<Box<dyn Fn(Uuid)>>>>,
    /// Callback for the reconnect banner's cloud login button
    /// (session_id, connection_id, login command).
    on_cloud_login: CloudLoginCallback,
    /// Resolves the split-pane container box a session is displayed in, when it
    /// is a split guest with no `TabPage` of its own (issue #328).
    ///
    /// Wired by the window to look the session up across the per-tab split
    /// bridges. `session_content_box` consults this as a last resort so the
    /// reconnect banner reaches every pane, not only the split owner's.
    split_pane_box_provider: Rc<RefCell<Option<Rc<dyn Fn(Uuid) -> Option<GtkBox>>>>>,
    /// Resolves the focused pane's session for a tab that hosts a split, given
    /// the tab owner's session id (issue #371).
    ///
    /// The focused pane is known only to the tab's `SplitViewBridge`, which
    /// lives at the window layer in `session_split_bridges`, not in the
    /// notebook. Wired by the window the same way as
    /// [`Self::split_pane_box_provider`]. `Save Output` consults it so a split
    /// tab saves the pane the user is actually looking at, not always the
    /// owner. Returns `None` for a tab with no split, so the caller falls back
    /// to the owner session.
    focused_session_provider: Rc<RefCell<Option<Rc<dyn Fn(Uuid) -> Option<Uuid>>>>>,
    /// Makes the split pane that shows a session the focused pane of its split,
    /// and reports whether the session is shown in a pane at all.
    ///
    /// `win.close-pane` closes the *focused* pane, and the bridge tracks focus
    /// from pointer clicks, not keyboard focus. Q on an ended session in a split
    /// sets it here first, so the pane that got the key is the one closed.
    /// Wired by the window like [`Self::split_pane_box_provider`].
    split_pane_focuser: Rc<RefCell<Option<Rc<dyn Fn(Uuid) -> bool>>>>,
    /// Toggles a tab's membership in the cross-tab broadcast group (issue #329).
    ///
    /// Wired by the window; the tab context menu calls it to activate the
    /// `win.toggle-tab-broadcast` action. `None` until the window wires it.
    on_tab_broadcast_toggle: Rc<RefCell<Option<Box<dyn Fn(Uuid)>>>>,
    /// Reports whether a tab is currently in the broadcast group, so the tab
    /// context menu can label its toggle item. Wired by the window (issue #329).
    tab_broadcast_membership: Rc<RefCell<Option<Rc<dyn Fn(Uuid) -> bool>>>>,
    /// Supplies the tab menu's Edit Connection and Copy sections for a
    /// connection id. Wired by the window, which owns the connection data
    /// (issue #357).
    tab_connection_menu: Rc<RefCell<Option<tab_menu::TabConnectionMenuProvider>>>,
    /// Whether keyboard passthrough is on; mirrors `win.toggle-passthrough`.
    keyboard_passthrough: Rc<std::cell::Cell<bool>>,
    /// Whether to automatically capture system shortcuts (Alt+Tab, Super)
    /// when an embedded RDP/VNC viewer gains keyboard focus.
    auto_capture_shortcuts: Rc<std::cell::Cell<bool>>,
    /// The embedded RDP/VNC viewer that has keyboard focus, if any. Its window
    /// asks the compositor for the desktop shortcuts while passthrough is on
    /// (issue #356).
    focused_viewer: Rc<RefCell<Option<glib::WeakRef<gtk4::Widget>>>>,
    /// The focused viewer's `keyboard_hook` (an RDP connection's choice of
    /// where Windows key combinations go); `None` follows auto-capture.
    focused_hook: Rc<std::cell::Cell<Option<RdpKeyboardHook>>>,
    /// Callback fired when terminal focus changes (`true` = focus entered the
    /// VTE, `false` = focus left). Drives focus-based accelerator suspend (#197).
    on_terminal_focus: Rc<RefCell<Option<Box<dyn Fn(bool)>>>>,
    /// Sessions that should have a reconnect banner (prevents duplicates).
    ///
    /// The mark outlives the banner widget: moving a session between a tab, a
    /// split pane and a window rebuilds the box around its terminal and drops
    /// the banner with it. [`Self::reconcile_reconnect_banner`] runs whenever
    /// the terminal is mapped again and puts the banner back for a marked,
    /// disconnected session.
    reconnect_shown: Rc<RefCell<HashSet<Uuid>>>,
    /// The cloud login offered on a session's banner, kept so a banner shown
    /// again after a move gets its "Log In to …" button back.
    cloud_login_offers: Rc<RefCell<HashMap<Uuid, rustconn_core::protocol::CloudLogin>>>,
    /// Each terminal's Reconnect entry in its right-click menu, offered only
    /// while the session has ended. Absent in right-click-pastes mode.
    vte_reconnect_items: Rc<RefCell<HashMap<Uuid, config::VteReconnectItem>>>,
    /// Runs when a session's terminal is mapped, which every move between a
    /// tab, a split pane and a window does. Wired by the window to
    /// [`Self::reconcile_reconnect_banner`] through a `Weak` notebook handle.
    on_terminal_mapped: Rc<RefCell<Option<Rc<dyn Fn(Uuid)>>>>,
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
    /// Where a session's current connection output starts.
    ///
    /// VTE cursor rows are absolute buffer coordinates — they include the
    /// scrollback — so every "the cursor advanced past the connect banner"
    /// check needs a baseline: the banner itself, and after a reconnect the
    /// previous session's output (issue #253), are above it. Every terminal
    /// connect pins it with [`Self::display_connect_banner`]; see
    /// [`output_start`] for why the row is located by content rather than read
    /// when the banner is fed. A session with no entry counts from row 0.
    cursor_row_base: Rc<RefCell<HashMap<Uuid, output_start::OutputStart>>>,
    /// OSC 133 prompt marks per session (shell-integration-osc133 spec).
    ///
    /// Populated by [`termprops::connect_shell_integration`] from the remote
    /// shell's command-end events and read by the prompt-navigation actions and
    /// the failed-command marker. Shared via `Rc` so the VTE termprop handler and
    /// the window actions see the same registry. Empty — and the whole feature
    /// dormant — on a host without shell integration, and on a build below the
    /// `vte-0-78` feature (the hook is a no-op there). Dropped with the session.
    mark_registries: Rc<RefCell<HashMap<Uuid, Rc<RefCell<mark_registry::MarkRegistry>>>>>,
    /// The failed-command marker layer per session, kept alive so it keeps
    /// painting and is removed from the terminal when dropped on tab close.
    /// Reads the matching [`Self::mark_registries`] entry; invisible when that
    /// registry has no failed rows.
    failed_markers: Rc<RefCell<HashMap<Uuid, failed_marker::FailedMarkerOverlay>>>,
    /// The remote working directory a session last reported via OSC 7, shown as
    /// the last line of the tab tooltip. Updated from VTE's
    /// `current-directory-uri-changed`; absent until the shell reports one (it
    /// needs OSC 7, which the same `vte.sh` that drives shell integration emits).
    session_cwd: Rc<RefCell<HashMap<Uuid, String>>>,
    /// Title state of every tab that hosts a split layout, keyed by the owner
    /// session (issue #421). Such a tab is named after its focused pane; see
    /// `split_title.rs`. Absent for an ordinary tab.
    split_tab_titles: split_title::SplitTabTitles,
    /// Told every session rename, so the split pane header follows it
    /// (issue #421); see `set_on_session_relabelled`.
    on_session_relabelled: split_title::SessionRelabelHook,
    /// Told after a split tab's title source changes, so the header subtitle
    /// follows the focused pane (issue #421); see `set_on_split_title_changed`.
    on_split_title_changed: Rc<RefCell<Option<Rc<dyn Fn()>>>>,
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
    /// Output filter (postpend command) argv for sessions that configured one.
    ///
    /// Registered by the protocol launcher before it spawns, and read by
    /// [`Self::spawn_command_with_cleanup`], which wraps the session command in a
    /// shell pipeline. It is a per-session map rather than a spawn parameter
    /// because the notebook holds no connections: the launcher has the
    /// `PostpendCommand`, the spawn does not, and every terminal protocol shares
    /// one spawn. Kept across a reconnect on purpose, so the filter comes back
    /// with the session; removed when the tab closes.
    output_filters: Rc<RefCell<HashMap<Uuid, Vec<String>>>>,
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
    /// Called with `(cluster_id, cluster_name, session_id)` when the tab of a
    /// cluster member opened "in a new window" appears. The window layer moves
    /// that tab into the cluster's own window.
    on_cluster_window_tab: Rc<RefCell<Option<Box<dyn Fn(Uuid, &str, Uuid)>>>>,
    /// Active recording sessions (tracked by session_id)
    active_recordings: Rc<RefCell<HashSet<Uuid>>>,
    /// Recording paths and start times: session_id → (data_path, timing_path, connection_name, start_time)
    recording_paths: RefCell<HashMap<Uuid, (PathBuf, PathBuf, String, Instant)>>,
    /// Remote recording info for SSH sessions: session_id → RemoteRecordingInfo
    remote_recordings: RefCell<HashMap<Uuid, RemoteRecordingInfo>>,
    /// Compiled highlight rules per session: session_id → CompiledHighlightRules
    session_highlight_rules: Rc<RefCell<HashMap<Uuid, CompiledHighlightRules>>>,
    /// Script recording in progress, if any. At most one notebook-wide: the
    /// `commit` subscription is registered once per start, and two concurrent
    /// recordings would have to share the user's keystrokes between two files.
    script_recording: Rc<RefCell<Option<script_recorder::ActiveScriptRecording>>>,
    /// Sessions whose output is being shared read-only; drives the tab menu.
    shared_sessions: Rc<RefCell<HashSet<Uuid>>>,
    /// Shared script section for terminal context menus. One live model shared
    /// by every terminal, rebuilt when recording starts or stops so the single
    /// item's label can flip between Record and Stop.
    script_menu_section: Rc<gio::Menu>,
    /// Highlight overlay widgets per session: session_id → HighlightOverlay
    highlight_overlays: Rc<RefCell<HashMap<Uuid, HighlightOverlay>>>,
    /// The theme the quick picker last painted on a session, until a global
    /// repaint, a reconnect or the session's end paints over it.
    session_themes: Rc<RefCell<HashMap<Uuid, String>>>,
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
    /// Consecutive in-place reconnects per session whose process ended
    /// quickly (`rustconn_core::connection::next_short_lived_streak`); stops
    /// auto-reconnect against a host that answers but keeps closing.
    short_lived_reconnects: Rc<RefCell<HashMap<Uuid, u32>>>,
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
    /// Who asked for the next close of a session's tab. A missing entry means
    /// the user did (tab close button, middle-click); programmatic callers
    /// mark the session first. Consumed by the `close-page` handler.
    close_intents: Rc<RefCell<HashMap<Uuid, detach::CloseIntent>>>,
    /// Asks the window layer which sessions a tab's split shows, owner first.
    split_members: split_close::SplitMembersProvider,
    /// Split tabs whose close is waiting for the user's answer.
    split_close_pending: Rc<RefCell<HashSet<Uuid>>>,
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
    /// The live `child-exited` handlers of each session, with the terminal each
    /// one is attached to.
    ///
    /// An in-place reconnect calls `connect_child_exited` again on the *same*
    /// VTE widget, so without this the previous handler stays attached and the
    /// entire disconnect path runs once per handler: one post-disconnect task,
    /// one host-probe poll and one sidebar decrement for every reconnect the
    /// session has ever had. The terminal is kept beside the id because a
    /// handler id is only meaningful to the widget it came from — disconnecting
    /// a stale one from a fresh widget is a GLib error, hence the weak ref.
    ///
    /// The key carries a [`ChildExitHook`] as well as the session id because
    /// four unrelated parts of the app watch the same signal. Keying by session
    /// alone made every registration evict the previous one, so whichever
    /// registered last was the only one left — and session logging registers
    /// from an async callback, i.e. *after* the disconnect path (issue #297).
    child_exited_handlers: Rc<
        RefCell<HashMap<(Uuid, ChildExitHook), (glib::WeakRef<Terminal>, glib::SignalHandlerId)>>,
    >,
    /// Whether to show the Welcome tab when no sessions are open (issue #232).
    /// Shared with signal handlers via `Rc<Cell<bool>>`.
    show_welcome: Rc<std::cell::Cell<bool>>,
    /// Whether the tab context menu offers "Close to the Left" / "Close to the
    /// Right". Default `false` (the directional closes are hidden to keep the
    /// menu short); the window flips it from the user's preference and whenever
    /// Settings changes it. Read at menu-build time, so a change takes effect on
    /// the next right-click without rebuilding the notebook.
    show_directional_close: Rc<std::cell::Cell<bool>>,
}

/// A VTE terminal with no PTY and no input, for read-only views.
///
/// Erase bindings still have to be mapped: VTE resolves Backspace even when
/// input is disabled, and with no PTY attached that aborts the process.
pub(crate) fn read_only_terminal() -> Terminal {
    let terminal = Terminal::new();
    terminal.set_input_enabled(false);
    config::apply_erase_bindings(&terminal);
    terminal.set_scrollback_lines(2000);
    terminal
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
        // libadwaita moves the window instead of dragging a lone tab; give that
        // tab a drag of its own so it can still go to another window.
        drag_single_tab(&tab_bar, &tab_view, |page| {
            !TabPageContainer::is_welcome(&page.child())
        });

        // Create TabOverview for grid view of all tabs (GNOME Web-style)
        let tab_overview = adw::TabOverview::new();
        tab_overview.set_view(Some(&tab_view));
        tab_overview.set_enable_new_tab(false);

        // Add overview button to the end of the TabBar
        let overview_button = gtk4::Button::from_icon_name("view-grid-symbolic");
        overview_button.set_tooltip_text(Some(&crate::i18n::with_accel(
            &i18n("Tab Overview"),
            "win.tab-overview",
        )));
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
            on_session_shown: Rc::new(RefCell::new(None)),
            on_session_created: Rc::new(RefCell::new(None)),
            on_tab_added: Rc::new(RefCell::new(None)),
            on_recording_changed: Rc::new(RefCell::new(None)),
            on_split_colors_changed: Rc::new(RefCell::new(None)),
            on_split_cleanup: Rc::new(RefCell::new(None)),
            terminals: Rc::new(RefCell::new(HashMap::new())),
            session_widgets: Rc::new(RefCell::new(HashMap::new())),
            automation_sessions: Rc::new(RefCell::new(HashMap::new())),
            session_info: Rc::new(RefCell::new(HashMap::new())),
            split_session_colors: Rc::new(RefCell::new(HashMap::new())),
            tab_group_manager: Rc::new(RefCell::new(TabGroupManager::new())),
            on_reconnect: Rc::new(RefCell::new(None)),
            on_spawn_failed: Rc::new(RefCell::new(None)),
            on_cloud_login: Rc::new(RefCell::new(None)),
            split_pane_box_provider: Rc::new(RefCell::new(None)),
            focused_session_provider: Rc::new(RefCell::new(None)),
            split_pane_focuser: Rc::new(RefCell::new(None)),
            on_tab_broadcast_toggle: Rc::new(RefCell::new(None)),
            tab_broadcast_membership: Rc::new(RefCell::new(None)),
            tab_connection_menu: Rc::new(RefCell::new(None)),
            keyboard_passthrough: Rc::new(std::cell::Cell::new(false)),
            auto_capture_shortcuts: Rc::new(std::cell::Cell::new(true)),
            focused_viewer: Rc::new(RefCell::new(None)),
            focused_hook: Rc::new(std::cell::Cell::new(None)),
            on_terminal_focus: Rc::new(RefCell::new(None)),
            reconnect_shown: Rc::new(RefCell::new(HashSet::new())),
            cloud_login_offers: Rc::new(RefCell::new(HashMap::new())),
            vte_reconnect_items: Rc::new(RefCell::new(HashMap::new())),
            on_terminal_mapped: Rc::new(RefCell::new(None)),
            disconnected_sessions: Rc::new(RefCell::new(HashSet::new())),
            keep_history_on_reconnect: Rc::new(std::cell::Cell::new(true)),
            max_scrollback_on_reconnect: Rc::new(std::cell::Cell::new(None)),
            cursor_row_base: Rc::new(RefCell::new(HashMap::new())),
            mark_registries: Rc::new(RefCell::new(HashMap::new())),
            failed_markers: Rc::new(RefCell::new(HashMap::new())),
            session_cwd: Rc::new(RefCell::new(HashMap::new())),
            split_tab_titles: Rc::new(RefCell::new(HashMap::new())),
            on_session_relabelled: Rc::new(RefCell::new(None)),
            on_split_title_changed: Rc::new(RefCell::new(None)),
            pty_relays: Rc::new(RefCell::new(HashMap::new())),
            output_observers: Rc::new(RefCell::new(HashMap::new())),
            output_filters: Rc::new(RefCell::new(HashMap::new())),
            commit_forwarded: Rc::new(RefCell::new(HashSet::new())),
            pty_size_timers: Rc::new(RefCell::new(HashMap::new())),
            cluster_sessions: Rc::new(RefCell::new(HashMap::new())),
            session_to_cluster: Rc::new(RefCell::new(HashMap::new())),
            cluster_pending: Rc::new(RefCell::new(HashMap::new())),
            on_cluster_window_tab: Rc::new(RefCell::new(None)),
            recording_paths: RefCell::new(HashMap::new()),
            session_highlight_rules: Rc::new(RefCell::new(HashMap::new())),
            script_recording: Rc::new(RefCell::new(None)),
            shared_sessions: Rc::new(RefCell::new(HashSet::new())),
            script_menu_section: Rc::new(gio::Menu::new()),
            highlight_overlays: Rc::new(RefCell::new(HashMap::new())),
            session_themes: Rc::new(RefCell::new(HashMap::new())),
            terminal_overlays: Rc::new(RefCell::new(HashMap::new())),
            active_recordings: Rc::new(RefCell::new(HashSet::new())),
            remote_recordings: RefCell::new(HashMap::new()),
            poll_cancel_tokens: Rc::new(RefCell::new(HashMap::new())),
            auto_reconnect_eligible: Rc::new(RefCell::new(std::collections::HashSet::new())),
            short_lived_reconnects: Rc::new(RefCell::new(HashMap::new())),
            ssh_tunnels: Rc::new(RefCell::new(HashMap::new())),
            activity_coordinator: Rc::new(RefCell::new(None)),
            tab_containers: Rc::new(RefCell::new(HashMap::new())),
            parked_in_split: Rc::new(RefCell::new(HashSet::new())),
            detached: Rc::new(RefCell::new(HashSet::new())),
            close_intents: Rc::new(RefCell::new(HashMap::new())),
            split_members: Rc::new(RefCell::new(None)),
            split_close_pending: Rc::new(RefCell::new(HashSet::new())),
            on_focus_detached: Rc::new(RefCell::new(None)),
            on_detach_request: Rc::new(RefCell::new(None)),
            on_session_ended: Rc::new(RefCell::new(None)),
            monitoring: Rc::new(RefCell::new(None)),
            snippet_menu_section: Rc::new(gio::Menu::new()),
            vte_child_pids: Rc::new(RefCell::new(HashMap::new())),
            child_exited_handlers: Rc::new(RefCell::new(HashMap::new())),
            show_welcome: Rc::new(std::cell::Cell::new(show_welcome)),
            show_directional_close: Rc::new(std::cell::Cell::new(false)),
        };

        term_notebook.setup_tab_view_signals();
        term_notebook.setup_tab_context_menu();
        term_notebook.setup_tab_overview_cleanup();
        // The section starts empty; without this first fill, the Record Script
        // item that would change the state is itself never shown.
        term_notebook.rebuild_script_menu_section();
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
        let session_themes = self.session_themes.clone();
        let terminal_overlays = self.terminal_overlays.clone();
        let ssh_tunnels = self.ssh_tunnels.clone();
        let tab_containers = self.tab_containers.clone();
        let parked_in_split = self.parked_in_split.clone();
        let detached_close = Rc::clone(&self.detached);
        let on_session_ended = Rc::clone(&self.on_session_ended);
        let vte_child_pids = self.vte_child_pids.clone();
        let auto_reconnect_on_close = Rc::clone(&self.auto_reconnect_eligible);
        let short_lived_on_close = Rc::clone(&self.short_lived_reconnects);
        let child_exited_on_close = Rc::clone(&self.child_exited_handlers);
        let show_welcome_on_close = self.show_welcome.clone();
        let disconnected_on_close = Rc::clone(&self.disconnected_sessions);
        let reconnect_shown_on_close = Rc::clone(&self.reconnect_shown);
        let cloud_login_offers_on_close = Rc::clone(&self.cloud_login_offers);
        let vte_reconnect_items_on_close = Rc::clone(&self.vte_reconnect_items);
        let poll_tokens_on_close = Rc::clone(&self.poll_cancel_tokens);
        let cursor_row_base_on_close = Rc::clone(&self.cursor_row_base);
        let mark_registries_on_close = Rc::clone(&self.mark_registries);
        let failed_markers_on_close = Rc::clone(&self.failed_markers);
        let session_cwd_on_close = Rc::clone(&self.session_cwd);
        let split_tab_titles_on_close = Rc::clone(&self.split_tab_titles);
        let pty_relays_on_close = Rc::clone(&self.pty_relays);
        let output_observers_on_close = Rc::clone(&self.output_observers);
        let output_filters_on_close = Rc::clone(&self.output_filters);
        let commit_forwarded_on_close = Rc::clone(&self.commit_forwarded);
        let pty_size_timers_on_close = Rc::clone(&self.pty_size_timers);
        let script_recording_on_close = Rc::clone(&self.script_recording);
        let script_menu_section_on_close = Rc::clone(&self.script_menu_section);
        let split_close_on_close = self.split_close_hooks();

        // A tab dropped outside every window opens a new window, whose view the
        // tab then lands on (see `tab_transfer`).
        self.tab_view.connect_create_window(|_| new_window_view());

        // A cluster window has a tab view of its own, and libadwaita lets a tab
        // be dragged between any two: hand back a session tab dragged out of
        // here, after which the window layer moves its session where it was
        // dropped. The Welcome placeholder is handed back too, and stays.
        let sessions_home = self.sessions.clone();
        return_transferred_tabs(&self.tab_view, move |page| {
            TabPageContainer::is_welcome(&page.child())
                || sessions_home
                    .try_borrow()
                    .is_ok_and(|sessions| sessions.values().any(|p| p == page))
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

            // Consume the close intent first, so no mark outlives this close
            // whichever branch below handles it.
            let intent = if session_id.is_nil() {
                detach::CloseIntent::Programmatic
            } else {
                split_close_on_close.take_intent(session_id)
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

            // A user's close of a tab hosting a split of two or more sessions
            // is held until the user answers (item 11). The members are asked
            // for only on that path and outside every map borrow above, so a
            // programmatic close never touches the window's bridge map.
            if intent == detach::CloseIntent::User {
                let members = split_close_on_close.members(session_id);
                if detach::split_close_decision(!members.is_empty(), members.len(), intent)
                    == detach::SplitCloseDecision::Ask
                {
                    split_close_on_close.present_single(page, session_id, &members);
                    return glib::Propagation::Stop;
                }
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

                // Remove highlight overlay for this session. Dropped outside the
                // map borrow: its `Drop` detaches the layer and its handlers.
                let highlight_overlay = highlight_overlays.borrow_mut().remove(&session_id);
                drop(highlight_overlay);
                // The theme the quick picker painted ends with the session.
                session_themes.borrow_mut().remove(&session_id);

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
                // `close_tab` clears these two itself, but a tab closed from its
                // own close button, or a session ended from a detached or
                // cluster window through `close_session`, reaches only this
                // handler: without it an auto-reconnect probe kept polling a
                // session that no longer exists.
                reconnect_shown_on_close.borrow_mut().remove(&session_id);
                cloud_login_offers_on_close.borrow_mut().remove(&session_id);
                vte_reconnect_items_on_close
                    .borrow_mut()
                    .remove(&session_id);
                let poll_token = poll_tokens_on_close.borrow_mut().remove(&session_id);
                if let Some(cancel) = poll_token {
                    cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                cursor_row_base_on_close.borrow_mut().remove(&session_id);
                mark_registries_on_close.borrow_mut().remove(&session_id);
                failed_markers_on_close.borrow_mut().remove(&session_id);
                session_cwd_on_close.borrow_mut().remove(&session_id);
                split_tab_titles_on_close.borrow_mut().remove(&session_id);

                // A recording running against this session cannot outlive it:
                // write the steps out rather than dropping them on the floor.
                if let Some(recorded) = script_recorder::finalize_for_session(
                    &script_recording_on_close,
                    &script_menu_section_on_close,
                    session_id,
                ) {
                    tracing::info!(
                        %session_id,
                        steps = recorded.step_count,
                        path = %recorded.path.display(),
                        "Saved script recording when its session closed"
                    );
                }

                // Stop the session's PTY: dropping the relay ends its reader and
                // writer threads and closes the descriptor. The child is killed
                // just below, which is what releases a reader still waiting on
                // output (#247, #172).
                if let Some(timer) = pty_size_timers_on_close.borrow_mut().remove(&session_id) {
                    timer.remove();
                }
                drop(pty_relays_on_close.borrow_mut().remove(&session_id));
                output_observers_on_close.borrow_mut().remove(&session_id);
                output_filters_on_close.borrow_mut().remove(&session_id);
                commit_forwarded_on_close.borrow_mut().remove(&session_id);
                auto_reconnect_on_close.borrow_mut().remove(&session_id);
                short_lived_on_close.borrow_mut().remove(&session_id);
                // The widget goes with the tab, so the handlers die with it —
                // this only keeps the map from growing for the app's lifetime.
                child_exited_on_close
                    .borrow_mut()
                    .retain(|(id, _), _| *id != session_id);

                // Kill VTE child process group explicitly (#172).
                // Some CLI clients (notably telnet) do not exit on SIGHUP
                // when the PTY master fd is closed. Sending SIGTERM to the
                // process group ensures all children terminate.
                if let Some(pid) = vte_child_pids.borrow_mut().remove(&session_id) {
                    child_teardown::terminate_child_group(pid);
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
}

impl TerminalNotebook {
    /// Closes a terminal tab by session ID
    pub fn close_tab(&self, session_id: Uuid) {
        self.reconnect_shown.borrow_mut().remove(&session_id);
        self.cloud_login_offers.borrow_mut().remove(&session_id);
        self.disconnected_sessions.borrow_mut().remove(&session_id);
        // Cancel any background polling (auto-reconnect, host check) for this session
        self.cancel_poll(session_id);
        // A parked session — a split guest or a detached window — has no tab
        // page to close, so it goes through the tabless path. Checking only the
        // detached mark left a split guest running with no pane and no tab when
        // the sidebar's Close, the session manager, a script or a clean exit
        // closed it by id (issue #236 for the detached case). The split pane is
        // taken down by the `on_split_cleanup` hook the close runs.
        let page = self.sessions.borrow().get(&session_id).cloned();
        let route = detach::close_route(
            self.parked_in_split.borrow().contains(&session_id),
            self.is_detached(session_id),
            page.is_some(),
        );
        match (route, page) {
            (detach::CloseRoute::Parked, _) => self.close_session(session_id),
            (detach::CloseRoute::Page, Some(page)) => {
                // Closed on the application's behalf (script, cluster, clean
                // exit, reconnect, sidebar Disconnect): never ask.
                self.mark_programmatic_close(session_id);
                self.tab_view.close_page(&page);
            }
            (detach::CloseRoute::Page | detach::CloseRoute::Unknown, _) => {
                tracing::debug!(session = %session_id, "close_tab: no tab or parked session");
            }
        }
    }

    /// Sets whether an in-place reconnect keeps the previous scrollback (#253).
    pub fn set_keep_history_on_reconnect(&self, enabled: bool) {
        self.keep_history_on_reconnect.set(enabled);
    }

    /// Sets the maximum scrollback lines to retain after a reconnect.
    pub fn set_max_scrollback_on_reconnect(&self, limit: Option<u32>) {
        self.max_scrollback_on_reconnect.set(limit);
    }

    /// Updates whether the Welcome tab is shown when no sessions are open (issue #232)
    pub fn set_show_welcome(&self, enabled: bool) {
        self.show_welcome.set(enabled);
    }

    /// Updates whether the tab context menu offers the directional "Close to
    /// the Left" / "Close to the Right" items. Takes effect on the next
    /// right-click; no rebuild needed.
    pub fn set_show_directional_close(&self, enabled: bool) {
        self.show_directional_close.set(enabled);
    }

    /// Gets the terminal widget for a session
    #[must_use]
    pub fn get_terminal(&self, session_id: Uuid) -> Option<Terminal> {
        self.terminals.borrow().get(&session_id).cloned()
    }

    /// Grid of a session's VTE terminal as `(columns, rows)`.
    ///
    /// The share page needs it so xterm.js renders escape sequences at the
    /// size the PTY produced them for.
    #[must_use]
    pub fn terminal_grid(&self, session_id: Uuid) -> Option<(u16, u16)> {
        let terminal = self.get_terminal(session_id)?;
        let (rows, cols) = grid_size(&terminal);
        Some((cols, rows))
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

    /// When the session's current process started: its in-place reconnect, or
    /// the session's creation.
    ///
    /// A status detector captures this when it is installed and stops once it
    /// changes, so a detector left armed by a failed attempt does not count a
    /// later attempt's output as its own.
    #[must_use]
    pub fn session_started_at(&self, session_id: Uuid) -> Option<chrono::DateTime<chrono::Utc>> {
        self.session_info
            .borrow()
            .get(&session_id)
            .map(TerminalSession::started_at)
    }

    /// Gets session info for a session
    #[must_use]
    pub fn get_session_info(&self, session_id: Uuid) -> Option<TerminalSession> {
        self.session_info.borrow().get(&session_id).cloned()
    }

    /// Finds the live session for a saved connection.
    ///
    /// Prefers the active tab, then the first session of that connection. The
    /// sidebar offers session tools only when a connection has exactly one
    /// session, so "first" is unambiguous there.
    #[must_use]
    pub fn session_for_connection(&self, connection_id: Uuid) -> Option<Uuid> {
        let info = self.session_info.borrow();
        if let Some(active) = self.get_active_session_id()
            && info
                .get(&active)
                .is_some_and(|s| s.connection_id == connection_id)
        {
            return Some(active);
        }
        info.iter()
            .find(|(_, s)| s.connection_id == connection_id)
            .map(|(id, _)| *id)
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
    /// wrapped — which is the very box handed to its window. A split guest owns
    /// no page either; it resolves through the wired split-pane box provider to
    /// its own pane container, so a reconnect banner attaches to that pane
    /// rather than to the split owner's (issue #328). Only a session displayed
    /// nowhere resolves to `None`. A split owner resolves to its tab's box,
    /// which holds the whole split; the reconnect banner looks up the owner's
    /// own pane first instead (`reconnect_banner_box`).
    ///
    /// Used by everything that decorates a session in place (reconnect banner,
    /// monitoring bar) so the decoration follows the session between windows
    /// (issue #236) and reaches every split pane (issue #328).
    #[must_use]
    pub fn session_content_box(&self, session_id: Uuid) -> Option<GtkBox> {
        if let Some(container) = self.get_session_container(session_id) {
            return Some(container);
        }
        // A split guest owns no TabPage; its widget lives in another session's
        // Paned tree. Resolve the pane's own container so a reconnect banner can
        // attach there too (issue #328). Checked before the detached branch
        // because a split guest is not detached.
        let provider = self
            .split_pane_box_provider
            .borrow()
            .as_ref()
            .map(Rc::clone);
        if let Some(provider) = provider
            && let Some(pane_box) = provider(session_id)
        {
            return Some(pane_box);
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

    /// Sessions that currently have a tab, left to right.
    ///
    /// Unlike [`Self::get_all_sessions`] (hash order, includes sessions parked
    /// in a split or detached), this is the order the user sees in the tab bar
    /// and only lists sessions a tab selection could name.
    #[must_use]
    pub fn sessions_in_tab_order(&self) -> Vec<TerminalSession> {
        let sessions = self.sessions.borrow();
        let info = self.session_info.borrow();
        (0..self.tab_view.n_pages())
            .filter_map(|i| {
                let page = self.tab_view.nth_page(i);
                let (id, _) = sessions.iter().find(|(_, p)| **p == page)?;
                info.get(id).cloned()
            })
            .collect()
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

    /// Rebuilds the shared snippet menu section based on current app state.
    ///
    /// Call this after snippets are created, edited, or deleted.
    pub fn rebuild_snippet_menu(&self, state: &crate::state::SharedAppState) {
        config::rebuild_snippet_menu_section(&self.snippet_menu_section, state);
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

    /// Returns the TabBar.
    ///
    /// Outside fullscreen it is the first child of [`widget`](Self::widget);
    /// the window's fullscreen chrome moves it under the header bar while the
    /// window is fullscreen and back again afterwards (issue #354).
    #[must_use]
    pub fn tab_bar(&self) -> &adw::TabBar {
        &self.tab_bar
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

    /// Shows TabView content area (for RDP/VNC/SPICE sessions)
    /// Call this when switching to a non-SSH session that displays in TabView
    pub fn show_tab_view_content(&self) {
        self.tab_view.set_visible(true);
        self.tab_view.set_vexpand(true);
    }

    // ========================================================================
    // Tab Group Management
    // ========================================================================
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
