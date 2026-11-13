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
mod child_teardown;
mod config;
mod detach;
pub use detach::{DetachMonitor, DetachPresentation};
pub use output_start::ConnectionOutput;
mod failed_marker;
pub mod file_drop;
pub mod highlight_overlay;
mod mark_registry;
pub mod osc52;
mod output_start;
pub mod playback;
pub mod pty_relay;
pub mod pty_spawn;
mod recording;
pub mod safe_paste;
mod selection_hold;
mod session_lifecycle;
mod split_title;
pub mod tab_container;
mod tab_lifecycle;
mod tab_menu;
pub mod termprops;
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
use rustconn_core::models::{AutomationConfig, BackspaceSends, DeleteSends, RdpKeyboardHook};
use rustconn_core::terminal_themes::TerminalTheme;
pub use types::{
    ClusterTabs, LOCAL_SHELL_PROTOCOL, PendingCluster, SessionWidgetStorage, TerminalSession,
    group_still_in_use, local_shell_label, tab_title,
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
            on_cloud_login: Rc::new(RefCell::new(None)),
            split_pane_box_provider: Rc::new(RefCell::new(None)),
            focused_session_provider: Rc::new(RefCell::new(None)),
            on_tab_broadcast_toggle: Rc::new(RefCell::new(None)),
            tab_broadcast_membership: Rc::new(RefCell::new(None)),
            tab_connection_menu: Rc::new(RefCell::new(None)),
            keyboard_passthrough: Rc::new(std::cell::Cell::new(false)),
            auto_capture_shortcuts: Rc::new(std::cell::Cell::new(true)),
            focused_viewer: Rc::new(RefCell::new(None)),
            focused_hook: Rc::new(std::cell::Cell::new(None)),
            on_terminal_focus: Rc::new(RefCell::new(None)),
            reconnect_shown: Rc::new(RefCell::new(HashSet::new())),
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
            child_exited_handlers: Rc::new(RefCell::new(HashMap::new())),
            show_welcome: Rc::new(std::cell::Cell::new(show_welcome)),
            show_directional_close: Rc::new(std::cell::Cell::new(false)),
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
        let child_exited_on_close = Rc::clone(&self.child_exited_handlers);
        let show_welcome_on_close = self.show_welcome.clone();
        let disconnected_on_close = Rc::clone(&self.disconnected_sessions);
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

                // Remove highlight overlay for this session. Dropped outside the
                // map borrow: its `Drop` detaches the layer and its handlers.
                let highlight_overlay = highlight_overlays.borrow_mut().remove(&session_id);
                drop(highlight_overlay);

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
                mark_registries_on_close.borrow_mut().remove(&session_id);
                failed_markers_on_close.borrow_mut().remove(&session_id);
                session_cwd_on_close.borrow_mut().remove(&session_id);
                split_tab_titles_on_close.borrow_mut().remove(&session_id);

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
        self.spawn_command_with_cleanup(
            session_id,
            argv,
            envv,
            working_directory,
            ssh_agent_socket,
            ChildCleanup::default(),
        )
    }

    /// [`Self::spawn_command`], running `cleanup` when the child exits or the
    /// spawn fails, whether or not the tab is still open.
    pub(crate) fn spawn_command_with_cleanup(
        &self,
        session_id: Uuid,
        argv: &[&str],
        envv: Option<&[&str]>,
        working_directory: Option<&str>,
        ssh_agent_socket: Option<&str>,
        cleanup: ChildCleanup,
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
        // Diagnostics keep naming the session's own command, not `sh`, so a
        // "command not found" message still names the thing the user configured.
        let command_name = (*argv.first().unwrap_or(&"")).to_owned();
        let size = grid_size(&terminal);

        // An output filter turns the command into a shell pipeline. Built here
        // rather than in each launcher because this is the one point every
        // terminal protocol passes through.
        let filtered = self.wrap_in_output_filter(session_id, argv);
        let spawn_argv: Vec<&str> = match filtered.as_deref() {
            Some(script) => vec!["sh", "-c", script],
            None => argv.to_vec(),
        };

        // The session's own argv, not `spawn_argv`: with a filter the latter is
        // the whole `sh -c` script with the session command quoted inside it, so
        // logging it would put the same content in the log twice as much noise.
        // Which filter was chosen is already logged, by name, in
        // `set_output_filter`.
        tracing::debug!(
            command = %command_name,
            %session_id,
            argv = ?argv,
            working_directory = ?working_directory,
            env_count = env_refs.len(),
            rows = size.0,
            cols = size.1,
            filtered = filtered.is_some(),
            "Spawning session command"
        );

        let child = match pty_spawn::spawn_on_pty(&spawn_argv, &env_refs, working_directory, size) {
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
        watch_child_exit(&terminal, child.pid, cleanup);

        true
    }

    /// Registers, or clears, the output filter a session's command is piped through.
    ///
    /// Call before spawning. A filter whose executable cannot be found is
    /// refused here rather than at spawn time: the session then starts unfiltered,
    /// which is the documented fallback, where a pipeline to a missing command
    /// would start and immediately lose every byte the session wrote.
    pub fn set_output_filter(
        &self,
        session_id: Uuid,
        postpend: Option<&rustconn_core::models::PostpendCommand>,
    ) {
        let Some(argv) = postpend.and_then(rustconn_core::models::PostpendCommand::filter_argv)
        else {
            self.output_filters.borrow_mut().remove(&session_id);
            return;
        };

        // `argv[0]` is guaranteed non-empty by `filter_argv`.
        let program = argv[0].clone();
        if !filter_program_is_runnable(&program) {
            tracing::warn!(
                %session_id,
                filter = %program,
                "Output filter is not installed or not executable; session output is unfiltered"
            );
            // A log line is not feedback: the session starts and looks completely
            // normal, just unfiltered, so nothing tells the user their filter did
            // not run. A background failure the user can act on is a toast, per
            // the project's error-feedback rule.
            crate::toast::show_error_toast_on_active_window(&i18n_f(
                "Output filter '{}' is not installed; session output is not filtered",
                &[&program],
            ));
            self.output_filters.borrow_mut().remove(&session_id);
            return;
        }

        tracing::info!(%session_id, filter = %program, "Session output will be piped through a filter");
        self.output_filters.borrow_mut().insert(session_id, argv);
    }

    /// Returns the `sh -c` script for a filtered session, or `None` when unfiltered.
    ///
    /// See [`filter_program_is_runnable`] for why availability is decided before
    /// this point rather than at spawn time.
    ///
    /// ponytail: the pipeline's exit status is the *filter's*, not the session's,
    /// because POSIX `sh` has no portable `PIPESTATUS`. A filtered session that
    /// fails therefore looks like a clean exit to the reconnect heuristics. Worth
    /// revisiting only if a filter turns out to be common enough that the lost
    /// status matters.
    fn wrap_in_output_filter(&self, session_id: Uuid, argv: &[&str]) -> Option<String> {
        let filters = self.output_filters.borrow();
        let filter = filters.get(&session_id)?;
        let filter_refs: Vec<&str> = filter.iter().map(String::as_str).collect();
        Some(rustconn_core::shell_escape::pipe_argv_through(
            argv,
            &filter_refs,
        ))
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
        let hold = selection_hold::SelectionHold::attach(terminal);
        let terminal = terminal.downgrade();
        let observers = Rc::clone(&self.output_observers);
        glib::spawn_future_local(async move {
            // One filter per stream, because it has to remember a sequence split
            // across a chunk boundary and its state belongs to exactly one PTY.
            let mut osc52_filter = osc52::Osc52Filter::default();
            while let Ok(chunk) = output.recv().await {
                hold.wait_while_selecting(|| terminal.upgrade().is_some_and(|t| t.has_selection()))
                    .await;
                let live_terminal = terminal.upgrade();
                // The clipboard offer is written here, inside the sink, so it
                // only fires while the terminal is alive to supply a display.
                // The length is the only part of a payload worth logging: it is
                // remote-supplied output and may be a secret the user yanked.
                let mut sink = |text: &str| {
                    tracing::debug!(
                        %session_id,
                        bytes = text.len(),
                        "OSC 52 clipboard offer from the remote side"
                    );
                    if let Some(terminal) = &live_terminal {
                        osc52::offer_to_clipboard(terminal, text);
                    }
                };
                // Run the filter on every chunk, alive widget or not — it has to
                // keep its cross-chunk state consistent. The result is owned so
                // the same bytes reach VTE *and* the observers: a lifted OSC 52
                // offer must not survive in the session transcript either, which
                // is exactly where the raw chunk used to leak it.
                let filtered = osc52_filter.push(&chunk, &mut sink).to_vec();
                if let Some(terminal) = &live_terminal {
                    // A chunk can filter to nothing while the filter holds back
                    // the start of a sequence (a lone `ESC`, an OSC string that
                    // is still open). A removed offer is not empty: it leaves a
                    // `CAN`. `feed` is not called on an empty slice: VTE has been
                    // known to reach an `assert()` on degenerate input, and an
                    // assertion in a library takes the whole process down.
                    if !filtered.is_empty() {
                        terminal.feed(&filtered);
                    }
                }
                // The list is cloned so that no borrow is held while an observer
                // runs: session logging is then free to touch the notebook.
                let handlers = observers
                    .borrow()
                    .get(&session_id)
                    .cloned()
                    .unwrap_or_default();
                for handler in handlers {
                    handler(&filtered);
                }
            }
            // The stream can end inside a sequence. A lone `ESC` or `c2` is
            // handed to VTE rather than dropped with the filter. An OSC string
            // still open depends on the setting: with OSC 52 enabled `finish`
            // replaces it with a `CAN`, so a later `BEL` or `ST` cannot
            // complete it; with it disabled the fragment is released as it
            // came.
            if let Some(terminal) = terminal.upgrade() {
                hold.detach(&terminal);
                let tail = osc52_filter.finish();
                if !tail.is_empty() {
                    terminal.feed(tail);
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
        // Issue #294: push the real window size on the first main-loop idle
        // after the session is spawned. Without this, the child starts at the
        // default 24×80 and programs that read geometry only at startup (`mc`,
        // `htop`) render at the wrong size until they are restarted.
        //
        // Best effort by construction: the idle runs at
        // `G_PRIORITY_DEFAULT_IDLE`, below GTK's frame clock, so in the common
        // case layout has already happened — but if it has not, VTE reports its
        // own 24×80 defaults rather than zero, the guard below does not catch it,
        // and `sync_size` finds nothing to change. That case falls back to the
        // 250 ms poll, which is the behaviour this replaced, so there is no
        // retry: the cost of being early is a slower fix, not a wrong size.
        {
            let terminal_weak = terminal.downgrade();
            let relays_early = Rc::clone(&self.pty_relays);
            glib::idle_add_local_once(move || {
                let Some(terminal) = terminal_weak.upgrade() else {
                    return;
                };
                let (rows, cols) = grid_size(&terminal);
                if rows == 0 || cols == 0 {
                    return;
                }
                let relays = relays_early.borrow();
                if let Some(relay) = relays.get(&session_id)
                    && relay.sync_size(rows, cols)
                {
                    tracing::debug!(
                        %session_id, rows, cols,
                        "Early size push after widget allocation"
                    );
                }
            });
        }

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

    /// Renders text in the terminal widget without sending it to the session.
    ///
    /// `feed` is what VTE uses for output coming from the child; feeding bytes
    /// here draws them locally and never reaches the remote shell. That is the
    /// difference between a script printing a status line and a script typing
    /// that line at a prompt.
    pub fn feed_local_output(&self, session_id: Uuid, bytes: &[u8]) {
        if let Some(terminal) = self.get_terminal(session_id) {
            terminal.feed(bytes);
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

    /// Ends every live session's child process, for the application-exit paths.
    ///
    /// The `close-page` handler already does this per tab (issue #172), but
    /// quitting never closes the pages: the window's `close_request` and the
    /// `app.quit` action both went straight to `Proceed`, so an open `telnet`
    /// tab left its process running after RustConn was gone
    /// (issue [#304](https://github.com/totoshko88/RustConn/issues/304)). The
    /// leak was not visible for VTE-spawned children, but RustConn owns the PTY
    /// itself now: dropping the notebook closes the master and nothing else, and
    /// `telnet` is precisely the client that ignores the resulting `SIGHUP`.
    ///
    /// Deliberately does not go through `close_page`. That path fires the
    /// session-ended callbacks, rebuilds the Welcome tab and touches the sidebar,
    /// all pointless on the way out and all of it re-entering state the caller is
    /// in the middle of persisting. This only releases what would outlive the
    /// process.
    ///
    /// Idempotent, so it is safe for more than one exit path to run — and they
    /// do: the three of them funnel through
    /// [`crate::window::shutdown_sessions_for_exit`], and the window button and
    /// Ctrl+Q can both fire for one quit.
    ///
    /// Must run *after* `flush_active_recordings`, which is why the caller owns
    /// the ordering: this clears the PTY relays, and a recorder still being
    /// flushed through one would lose its tail.
    pub fn shutdown_sessions(&self) {
        // External viewers hosted in a tab (`xfreerdp`, `vncviewer`): the same
        // omission, and the same fix as the registry's tabless ones (issue #209).
        //
        // Only their `Child` is taken, and the widget map is left alone. An
        // embedded RDP/VNC/Web widget needs `disconnect()`, not a kill, and that
        // is what the `close-page` path does — which still runs, for every
        // detached session, immediately after this. Draining the map here would
        // silently take that away. Taking the child leaves `None` behind, so the
        // `take()` in `close-page` is a no-op rather than a double kill.
        let external: Vec<_> = self
            .session_widgets
            .borrow()
            .values()
            .filter_map(|storage| match storage {
                SessionWidgetStorage::ExternalProcess(process) => Some(Rc::clone(process)),
                _ => None,
            })
            .collect();
        for process in external {
            if let Some(mut child) = process.borrow_mut().take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }

        // Stop reading before signalling, so a relay thread does not sit in a
        // read on a PTY whose child is going away.
        for (_, timer) in self.pty_size_timers.borrow_mut().drain() {
            timer.remove();
        }
        self.pty_relays.borrow_mut().clear();

        let pids: Vec<i32> = self
            .vte_child_pids
            .borrow_mut()
            .drain()
            .map(|(_, pid)| pid)
            .collect();
        if !pids.is_empty() {
            tracing::info!(
                count = pids.len(),
                "terminating session children on shutdown"
            );
        }
        child_teardown::terminate_child_groups_blocking(&pids);

        // `SshTunnel::drop` kills its ssh process. Tabless external sessions park
        // their tunnel here too (see `spawn_and_register_external_viewer`), which
        // is the only thing that ever collected them.
        self.ssh_tunnels.borrow_mut().clear();
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

/// The text of the active screen's rows `first_row..=last_row`, with a
/// wrapped line joined into one.
fn rows_text(terminal: &Terminal, first_row: i64, last_row: i64) -> String {
    terminal
        .text_range_format(
            vte4::Format::Text,
            first_row,
            0,
            last_row,
            terminal.column_count(),
        )
        .0
        .map_or_else(String::new, |g| g.to_string())
}

/// [`TerminalNotebook::connection_output`] for one terminal, given the
/// session's recorded start (`None`: count from row 0).
///
/// Returns the output, or `None` while the banner has not landed, and the
/// start to record in place of `start` when this read resolved it, so the
/// banner search runs once. Kept free of the notebook so an opt-in test can
/// drive it with a bare VTE terminal.
fn read_connection_output(
    terminal: &Terminal,
    start: Option<&output_start::OutputStart>,
) -> (Option<ConnectionOutput>, Option<output_start::OutputStart>) {
    let cursor_row = terminal.cursor_position().1;
    let Some((base, banner, resolved)) = connection_start_row(terminal, start, cursor_row) else {
        return (None, None);
    };
    // At most the last BANNER_SEARCH_ROWS rows: a detector the heuristic
    // never decides (a jump-host session whose prompt it misses) reads on
    // every change for the whole session, and copying everything from the
    // start row grew with the session. The verdict reads the last line, the
    // last prompt or question and the failures after it; a failure further
    // up than this with no prompt or question below it would need a client
    // that printed a screenful after failing, and one that fails exits.
    // `row` still counts from the start row.
    let first_row = base
        .min(cursor_row)
        .max(cursor_row - output_start::BANNER_SEARCH_ROWS);
    let text = rows_text(terminal, first_row, cursor_row);
    // Only a cursor above the start row can mean another screen, so the
    // wider read runs only then.
    let screen_replaced = cursor_row < base
        && banner.is_some_and(|last_line| {
            let first_row = (cursor_row - output_start::BANNER_SEARCH_ROWS).max(0);
            !output_start::shows_banner(&rows_text(terminal, first_row, cursor_row), &last_line)
        });
    let output = ConnectionOutput {
        row: (cursor_row - base).max(0),
        text,
        screen_replaced,
    };
    (Some(output), resolved)
}

/// The absolute row the current connection output starts on, with the
/// banner line above it when it was found by that line, and the start to
/// record when this call resolved a pending one.
///
/// Resolves a pending [`output_start::OutputStart::AfterBanner`] by finding
/// the banner in the rows above the cursor. `None` while the banner has not
/// landed.
fn connection_start_row(
    terminal: &Terminal,
    start: Option<&output_start::OutputStart>,
    cursor_row: i64,
) -> Option<(i64, Option<String>, Option<output_start::OutputStart>)> {
    let (last_line, fed_at_row) = match start {
        None => return Some((0, None, None)),
        Some(output_start::OutputStart::Row(base)) => return Some((*base, None, None)),
        Some(output_start::OutputStart::BelowBanner { row, last_line }) => {
            return Some((*row, Some(last_line.clone()), None));
        }
        Some(output_start::OutputStart::AfterBanner {
            last_line,
            fed_at_row,
        }) => (last_line, *fed_at_row),
    };
    let first_row = output_start::search_first_row(fed_at_row, cursor_row);
    let col_count = terminal.column_count();
    let rows: Vec<String> = (first_row..=cursor_row)
        .map(|row| {
            terminal
                .text_range_format(vte4::Format::Text, row, 0, row, col_count)
                .0
                .map_or_else(String::new, |g| g.to_string())
        })
        .collect();
    if let Some(row) = output_start::row_below_banner(&rows, first_row, last_line) {
        let resolved = output_start::OutputStart::BelowBanner {
            row,
            last_line: last_line.clone(),
        };
        Some((row, Some(last_line.clone()), Some(resolved)))
    } else {
        // A screenful printed since the banner: count from the window top.
        let row = output_start::base_without_banner(fed_at_row, cursor_row)?;
        Some((row, None, Some(output_start::OutputStart::Row(row))))
    }
}

/// Whether an output filter's program can actually be executed.
///
/// Both shapes a filter can take — a bare command name and an absolute path — go
/// through the one lookup, because `find_in_path` checks the **executable bit**
/// and also knows about Flatpak's `/app/bin`, Snap's bundled directories and the
/// sandboxed CLI download dirs. A leading `~/` has already been expanded by
/// `PostpendCommand::filter_argv`, so it arrives here absolute.
///
/// The absolute-path case used to be checked with `Path::is_file()` alone. That
/// passes for `/etc/passwd`, which then fails at exec time with the pipeline
/// already built — losing every byte the session wrote, the exact failure this
/// gate exists to prevent, and inconsistent with the bare-name case that was
/// checked properly.
///
/// A free function rather than a method so it is testable without a GTK notebook.
fn filter_program_is_runnable(program: &str) -> bool {
    rustconn_core::which::find_in_path(program).is_some()
}

/// Kills a child and collects it, so a failed startup leaves no zombie.
fn reap_child(pid: u32) {
    let pid = nix::unistd::Pid::from_raw(pid as i32);
    let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGKILL);
    let _ = nix::sys::wait::waitpid(pid, None);
}

/// What is undone when a session child exits or fails to spawn.
///
/// Runs from the child watch, so it runs after the tab is closed too, unlike a
/// `child-exited` handler, which needs the terminal widget.
#[derive(Default)]
pub(crate) struct ChildCleanup {
    /// Files to remove, such as a secret file the child reads.
    pub(crate) paths: Vec<PathBuf>,
    /// Runs once, after the files are removed.
    pub(crate) on_exit: Option<Box<dyn FnOnce()>>,
}

impl From<Vec<PathBuf>> for ChildCleanup {
    fn from(paths: Vec<PathBuf>) -> Self {
        Self {
            paths,
            on_exit: None,
        }
    }
}

impl Drop for ChildCleanup {
    fn drop(&mut self) {
        for path in &self.paths {
            if let Err(error) = std::fs::remove_file(path)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!(%error, path = %path.display(), "Failed to remove child secret file");
            }
        }
        if let Some(on_exit) = self.on_exit.take() {
            on_exit();
        }
    }
}

/// Raises `child-exited` on a terminal when its child process ends.
///
/// `cleanup` stays owned by the child watch for the complete process lifetime
/// and runs on exit, even when the terminal is gone. Early spawn failures drop
/// the same guard before returning. VTE does not emit this signal for a child
/// it did not spawn; the GLib watch also performs `waitpid`, which is why
/// [`pty_spawn::spawn_on_pty`] deliberately leaks its `Child` handle.
fn watch_child_exit(terminal: &Terminal, pid: u32, cleanup: ChildCleanup) {
    let terminal = terminal.downgrade();
    let mut cleanup = Some(cleanup);
    glib::child_watch_add_local(glib::Pid(pid as i32), move |_pid, status| {
        tracing::debug!(status, pid, "Session child exited");
        if let Some(terminal) = terminal.upgrade() {
            terminal.emit_by_name::<()>("child-exited", &[&status]);
        }
        drop(cleanup.take());
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
/// Entries are zeroizing because callers may pass short-lived sensitive
/// values. SSH askpass uses only owner-only secret-file paths here; password
/// bytes never enter the child environment.
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
        self.spawn_ssh_with_cleanup(
            session_id,
            host,
            port,
            username,
            identity_file,
            extra_args,
            use_waypipe,
            ssh_agent_socket,
            startup_command,
            extra_env,
            use_mptcp,
            Vec::new(),
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "function parameters mirror upstream API or struct fields 1:1; cleanup ownership is the only additional concern"
    )]
    pub(crate) fn spawn_ssh_with_cleanup(
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
        cleanup_paths: Vec<PathBuf>,
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
        if !has_known_hosts_opt
            && let Some(option) = rustconn_core::get_flatpak_known_hosts_path()
                .and_then(|kh_path| rustconn_core::ssh_tunnel::user_known_hosts_option(&kh_path))
        {
            kh_option = option;
            argv.push("-o");
            argv.push(&kh_option);
        }

        // Keep-alive defaults (ServerAliveInterval=15, ServerAliveCountMax=3)
        // are now emitted by `SshConfig::build_command_args` for every SSH
        // command — the GUI terminal, the CLI, and the tunnels — so they already
        // sit in `extra_args`. They used to be injected here, which meant only
        // GUI sessions got them; the fallback moved into the shared builder in
        // 0.21.11 so "all SSH sessions" is finally true (#217).
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

        self.spawn_command_with_cleanup(
            session_id,
            &argv,
            extra_env,
            None,
            ssh_agent_socket,
            cleanup_paths.into(),
        )
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

    /// Marks or unmarks a tab as a member of the cross-tab broadcast group.
    ///
    /// The marker is a title prefix (`⇄ `) rather than the tab indicator icon,
    /// which is already claimed by the split colour, the protocol colour and the
    /// offline state; the prefix stacks with all of them and with a group label
    /// prefix. Idempotent: the prefix is stripped before being re-applied, so
    /// repeated calls never double it, and a tooltip line is kept in sync so the
    /// membership is discoverable on hover (issue #329).
    pub fn set_broadcast_member_marker(&self, session_id: Uuid, member: bool) {
        const MARKER: &str = "⇄ ";

        if let Some(page) = self.sessions.borrow().get(&session_id) {
            let title = page.title().to_string();
            let base = title.strip_prefix(MARKER).unwrap_or(&title);
            if member {
                page.set_title(&format!("{MARKER}{base}"));
            } else {
                page.set_title(base);
            }

            let tooltip = page.tooltip().unwrap_or_default().to_string();
            page.set_tooltip(&Self::broadcast_tooltip(&tooltip, member));
        }
    }

    /// Re-appends the lines a tooltip mutator lifted out, in their fixed order:
    /// a split tab's panes line (issue #421), then the working-directory line.
    fn with_lifted_lines(
        mut tooltip: String,
        panes: Option<String>,
        cwd: Option<String>,
    ) -> String {
        if let Some(panes) = panes {
            tooltip.push_str(&panes);
        }
        if let Some(cwd) = cwd {
            tooltip.push_str(&cwd);
        }
        tooltip
    }

    /// The tooltip with its broadcast line added (`member`) or removed.
    ///
    /// The CWD and panes lines are lifted out first, so neither is mistaken for
    /// the broadcast suffix nor dropped by a toggle, and re-appended after it.
    pub(super) fn broadcast_tooltip(tooltip: &str, member: bool) -> String {
        const TOOLTIP_LINE: &str = "\n[Broadcast]";
        let (without_cwd, cwd_line) = Self::split_cwd_line(tooltip);
        let (without_cwd, panes_line) = split_title::split_panes_line(&without_cwd);
        let base = without_cwd
            .strip_suffix(TOOLTIP_LINE)
            .unwrap_or(&without_cwd);
        let rebuilt = if member {
            format!("{base}{TOOLTIP_LINE}")
        } else {
            base.to_owned()
        };
        Self::with_lifted_lines(rebuilt, panes_line, cwd_line)
    }

    /// The tooltip with its group line set to `group_name`.
    ///
    /// Shared by [`Self::set_tab_group`] and the tab menu's **Set Group…**, so
    /// the two cannot drift. The CWD and panes lines are lifted out first, so
    /// the `\n[` split cannot land on a `[Broadcast]`/old group line and
    /// discard the lines beyond it, and re-appended after the group line.
    pub(super) fn grouped_tooltip(tooltip: &str, group_name: &str) -> String {
        let (without_cwd, cwd_line) = Self::split_cwd_line(tooltip);
        let (without_cwd, panes_line) = split_title::split_panes_line(&without_cwd);
        let base = without_cwd
            .rsplit_once("\n[")
            .map_or(without_cwd.as_str(), |(base, _)| base);
        Self::with_lifted_lines(format!("{base}\n[{group_name}]"), panes_line, cwd_line)
    }

    /// The tooltip without its group line, or `None` when it has none.
    ///
    /// Lifts the CWD and panes lines like [`Self::grouped_tooltip`], so
    /// removing a group keeps the directory line rather than cutting it off.
    pub(super) fn ungrouped_tooltip(tooltip: &str) -> Option<String> {
        let (without_cwd, cwd_line) = Self::split_cwd_line(tooltip);
        let (without_cwd, panes_line) = split_title::split_panes_line(&without_cwd);
        let (base, _) = without_cwd.rsplit_once("\n[")?;
        Some(Self::with_lifted_lines(
            base.to_owned(),
            panes_line,
            cwd_line,
        ))
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
                    // `Wait` is peeled off by the `if let KeyElement::Wait(ms)`
                    // above and folded into `cumulative_delay_ms`; this `match`
                    // only runs in the `else` branch, so it can never be `Wait`.
                    KeyElement::Wait(_) => {
                        unreachable!("Wait is handled by the outer if-let and never reaches here")
                    }
                }
            }
        }
    }

    /// What the session's current process has printed below its connect
    /// banner, for the connection status detectors.
    ///
    /// VTE's `cursor_position()` returns `(column, row)` with the row in
    /// absolute buffer coordinates — scrollback included — so the row and the
    /// text are counted from the row the current connection's output starts
    /// on: the row just below RustConn's connect banner, pinned by
    /// [`Self::display_connect_banner`]. Neither the banner nor, after an
    /// in-place reconnect, the kept scrollback counts (issue #253). See
    /// [`ConnectionOutput::screen_replaced`] for a client on the alternate
    /// screen, whose rows cannot be compared with that start row.
    ///
    /// Returns `None` when the session has no terminal or the banner has not
    /// been processed yet; callers wait for the next output.
    #[must_use]
    pub fn connection_output(&self, session_id: Uuid) -> Option<ConnectionOutput> {
        let terminal = self.get_terminal(session_id)?;
        let start = self.cursor_row_base.borrow().get(&session_id).cloned();
        let (output, resolved) = read_connection_output(&terminal, start.as_ref());
        if let Some(resolved) = resolved {
            self.cursor_row_base
                .borrow_mut()
                .insert(session_id, resolved);
        }
        output
    }

    /// Runs `decide` on [`Self::connection_output`] after every change in the
    /// session's terminal, until it returns `true`.
    ///
    /// The handler disconnects itself once `decide` returns `true`, and also,
    /// without calling it again, once an in-place reconnect has started
    /// another process in the session: a detector left armed by a failed
    /// attempt never counts a later attempt's output, and detectors do not
    /// pile up across reconnects. Closing the tab drops the terminal, and the
    /// handler with it.
    pub fn watch_connection_output<F>(self: &Rc<Self>, session_id: Uuid, decide: F)
    where
        F: Fn(Option<&ConnectionOutput>) -> bool + 'static,
    {
        let Some(terminal) = self.get_terminal(session_id) else {
            return;
        };
        let started_at = self.session_started_at(session_id);
        let notebook = Rc::downgrade(self);
        let handler: Rc<RefCell<Option<glib::SignalHandlerId>>> = Rc::default();
        let handler_in_callback = Rc::clone(&handler);
        let id = terminal.connect_contents_changed(move |terminal| {
            let done = notebook.upgrade().is_none_or(|notebook| {
                notebook.session_started_at(session_id) != started_at
                    || decide(notebook.connection_output(session_id).as_ref())
            });
            if done && let Some(id) = handler_in_callback.borrow_mut().take() {
                terminal.disconnect(id);
            }
        });
        *handler.borrow_mut() = Some(id);
    }

    /// Feeds a terminal connect's banner and pins the connection start below
    /// it.
    ///
    /// Call this, not [`Self::display_output`], for the banner every terminal
    /// connect and in-place reconnect prints before it spawns its client: from
    /// then on [`Self::connection_output`] counts only what the client prints.
    pub fn display_connect_banner(&self, session_id: Uuid, banner: &str) {
        // The row the banner starts on: VTE has not processed it yet, so the
        // cursor is still where the buffer (or the kept history) ended.
        let Some(fed_at_row) = self
            .get_terminal(session_id)
            .map(|terminal| terminal.cursor_position().1)
        else {
            return;
        };
        self.display_output(session_id, banner);
        self.cursor_row_base.borrow_mut().insert(
            session_id,
            output_start::OutputStart::AfterBanner {
                last_line: output_start::banner_last_line(banner),
                fed_at_row,
            },
        );
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
    /// nowhere resolves to `None`.
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
            safe_paste::paste_into_terminal(&terminal);
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

    /// Scrolls the active terminal to the previous or next shell prompt.
    ///
    /// Reads the active session's OSC 133 [`mark_registry::MarkRegistry`], finds
    /// the nearest prompt strictly above (`previous`) or below the top of the
    /// current viewport, and scrolls there. A silent no-op when there is no such
    /// prompt, no shell integration on this session (empty registry), or on a
    /// build below the `vte-0-78` feature (the registry is never populated).
    /// (shell-integration-osc133 spec, task 4.)
    pub fn scroll_to_adjacent_prompt(&self, previous: bool) {
        tracing::debug!(previous, "scroll_to_adjacent_prompt: action invoked");
        let Some(session_id) = self.get_active_session_id() else {
            tracing::debug!("scroll_to_adjacent_prompt: no active session");
            return;
        };
        let Some(terminal) = self.get_terminal(session_id) else {
            tracing::debug!(%session_id, "scroll_to_adjacent_prompt: no terminal");
            return;
        };
        let registry = self.mark_registries.borrow().get(&session_id).cloned();
        let Some(registry) = registry else {
            tracing::debug!(%session_id, "scroll_to_adjacent_prompt: no registry");
            return;
        };
        tracing::debug!(
            %session_id,
            registry_empty = registry.borrow().is_empty(),
            "scroll_to_adjacent_prompt: registry resolved"
        );

        let Some(adjustment) = terminal.vadjustment() else {
            tracing::debug!(%session_id, "scroll_to_adjacent_prompt: no vadjustment");
            return;
        };
        // The viewport's top buffer row — the same coordinate space the registry
        // stores, documented in `highlight_overlay`.
        let viewport_top = adjustment.value() as i64;
        // The adjustment's lower bound is the oldest row still in the buffer:
        // VTE moves it up as scrollback is trimmed. Discard marks below it before
        // querying so a stale row can never be a navigation target (task 2.5).
        let oldest_row = adjustment.lower() as i64;

        let mut registry = registry.borrow_mut();
        registry.trim_below(oldest_row);
        let target = if previous {
            registry.prev_before(viewport_top)
        } else {
            registry.next_after(viewport_top)
        };
        tracing::debug!(
            viewport_top,
            oldest_row,
            ?target,
            "scroll_to_adjacent_prompt: computed target"
        );
        if let Some(row) = target {
            adjustment.set_value(row as f64);
        }
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

    /// Returns the TabBar.
    ///
    /// Outside fullscreen it is the first child of [`widget`](Self::widget);
    /// the window's fullscreen chrome moves it under the header bar while the
    /// window is fullscreen and back again afterwards (issue #354).
    #[must_use]
    pub fn tab_bar(&self) -> &adw::TabBar {
        &self.tab_bar
    }

    /// Turns the TabView's built-in tab shortcuts off for keyboard passthrough.
    ///
    /// `AdwTabView` carries its own shortcut controller (Ctrl+Tab,
    /// Ctrl+Shift+Tab, Ctrl+Page Up/Down, Ctrl+Home/End,
    /// Ctrl+Shift+Page Up/Down/Home/End, Alt+0…9) with window-wide scope. It is not a `GApplication`
    /// accelerator, so clearing the accelerator table in passthrough mode does
    /// not reach it, and because it runs before the session widget's key
    /// controller it switched tabs instead of forwarding the chord (issue #356).
    ///
    /// Also hands the desktop's own shortcuts (Alt+Tab, Super, Super+number)
    /// to an embedded RDP or VNC viewer while one has focus; see
    /// [`Self::attach_shortcut_inhibit`].
    pub fn set_keyboard_passthrough(&self, enabled: bool) {
        let focused = self
            .focused_viewer
            .borrow()
            .as_ref()
            .and_then(glib::WeakRef::upgrade);
        let viewer_takes = focused.as_ref().is_some_and(|viewer| {
            viewer_takes_shortcuts(
                viewer,
                self.focused_hook.get(),
                self.auto_capture_shortcuts.get(),
            )
        });
        self.tab_view
            .set_shortcuts(tab_view_shortcuts_for_passthrough(enabled || viewer_takes));
        self.keyboard_passthrough.set(enabled);

        if let Some(viewer) = focused {
            set_system_shortcuts_inhibited(&viewer, enabled || viewer_takes);
        } else if !enabled {
            // The viewer may have gone away while it held the shortcuts.
            set_system_shortcuts_inhibited(self.container.upcast_ref(), false);
        }
    }

    /// Enables or disables automatic system shortcut capture for embedded
    /// remote desktop viewers (RDP and VNC) on focus.
    pub fn set_auto_capture_shortcuts(&self, enabled: bool) {
        self.auto_capture_shortcuts.set(enabled);
        if let Some(viewer) = self
            .focused_viewer
            .borrow()
            .as_ref()
            .and_then(glib::WeakRef::upgrade)
        {
            let should_inhibit = self.keyboard_passthrough.get()
                || viewer_takes_shortcuts(&viewer, self.focused_hook.get(), enabled);
            set_system_shortcuts_inhibited(&viewer, should_inhibit);
            if should_inhibit {
                self.tab_view.set_shortcuts(adw::TabViewShortcuts::NONE);
            } else {
                self.tab_view
                    .set_shortcuts(adw::TabViewShortcuts::ALL_SHORTCUTS);
            }
        }
    }

    /// Whether automatic system shortcut capture for remote desktop viewers is on.
    #[must_use]
    pub fn auto_capture_shortcuts(&self) -> bool {
        self.auto_capture_shortcuts.get()
    }

    /// Asks the compositor for the desktop shortcuts while `widget` (an
    /// embedded RDP or VNC viewer) has focus and passthrough or auto-capture
    /// is on, and gives them back when focus leaves (issue #356).
    ///
    /// `hook` is an RDP connection's own choice of where Windows key
    /// combinations go (`RdpConfig::keyboard_hook`), read on every focus change;
    /// `None`, or a cell holding `None`, follows auto-capture. "Only in full
    /// screen" also follows the window going in and out of full screen while
    /// the viewer has focus.
    ///
    /// This is the Wayland keyboard-shortcuts-inhibit request that an external
    /// FreeRDP window already makes. GNOME asks the user once whether to allow
    /// it, and Super+Esc always gives the shortcuts back. Terminals are left
    /// out on purpose: in an SSH tab Alt+Tab is expected to switch windows.
    fn attach_shortcut_inhibit<W: IsA<gtk4::Widget>>(
        &self,
        widget: &W,
        hook: Option<Rc<std::cell::Cell<Option<RdpKeyboardHook>>>>,
    ) {
        let focus_ctrl = gtk4::EventControllerFocus::new();
        // The full-screen watch of a focused "Only in full screen" viewer.
        let fullscreen_watch: Rc<
            RefCell<Option<(glib::WeakRef<gtk4::Window>, glib::SignalHandlerId)>>,
        > = Rc::new(RefCell::new(None));
        let passthrough = self.keyboard_passthrough.clone();
        let auto_capture = self.auto_capture_shortcuts.clone();
        let focused = self.focused_viewer.clone();
        let focused_hook = self.focused_hook.clone();
        let tab_view = self.tab_view.clone();
        let watch = fullscreen_watch.clone();
        let viewer = widget.upcast_ref::<gtk4::Widget>().downgrade();
        focus_ctrl.connect_enter(move |_| {
            let Some(viewer) = viewer.upgrade() else {
                return;
            };
            *focused.borrow_mut() = Some(viewer.downgrade());
            let viewer_hook = hook.as_ref().and_then(|cell| cell.get());
            focused_hook.set(viewer_hook);
            if passthrough.get() || viewer_takes_shortcuts(&viewer, viewer_hook, auto_capture.get())
            {
                set_system_shortcuts_inhibited(&viewer, true);
                tab_view.set_shortcuts(adw::TabViewShortcuts::NONE);
            }
            if viewer_hook == Some(RdpKeyboardHook::FullscreenOnly)
                && let Some(window) = viewer.root().and_downcast::<gtk4::Window>()
            {
                let passthrough = passthrough.clone();
                let tab_view = tab_view.clone();
                let viewer = viewer.downgrade();
                let handler = window.connect_fullscreened_notify(move |window| {
                    let Some(viewer) = viewer.upgrade() else {
                        return;
                    };
                    let take = passthrough.get() || window.is_fullscreen();
                    set_system_shortcuts_inhibited(&viewer, take);
                    tab_view.set_shortcuts(tab_view_shortcuts_for_passthrough(take));
                });
                if let Some((old_window, old_handler)) =
                    watch.borrow_mut().replace((window.downgrade(), handler))
                    && let Some(old_window) = old_window.upgrade()
                {
                    old_window.disconnect(old_handler);
                }
            }
        });
        let passthrough = self.keyboard_passthrough.clone();
        let focused = self.focused_viewer.clone();
        let focused_hook = self.focused_hook.clone();
        let tab_view = self.tab_view.clone();
        let viewer = widget.upcast_ref::<gtk4::Widget>().downgrade();
        focus_ctrl.connect_leave(move |_| {
            *focused.borrow_mut() = None;
            focused_hook.set(None);
            if let Some((window, handler)) = fullscreen_watch.borrow_mut().take()
                && let Some(window) = window.upgrade()
            {
                window.disconnect(handler);
            }
            if let Some(viewer) = viewer.upgrade()
                && !passthrough.get()
            {
                set_system_shortcuts_inhibited(&viewer, false);
            }
            if !passthrough.get() {
                tab_view.set_shortcuts(adw::TabViewShortcuts::ALL_SHORTCUTS);
            }
        });
        widget.add_controller(focus_ctrl);
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

    /// Connects a callback for when a terminal child exits.
    ///
    /// `hook` identifies the caller. Registering the same hook twice for a
    /// session replaces the earlier handler; different hooks coexist. Both
    /// halves matter: an in-place reconnect re-registers
    /// [`ChildExitHook::Disconnect`] on the same VTE widget and a leftover
    /// handler would run the whole disconnect path once per reconnect the
    /// session ever had (two post-disconnect tasks, two host-probe polls, the
    /// sidebar's active-session count decremented twice for one exit) — while
    /// evicting a *different* caller's handler is what left the reconnect
    /// banner unreachable whenever session logging was enabled (issue #297).
    pub fn connect_child_exited<F>(&self, session_id: Uuid, hook: ChildExitHook, callback: F)
    where
        F: Fn(i32) + 'static,
    {
        if let Some(terminal) = self.get_terminal(session_id) {
            if let Some((previous_terminal, handler)) = self
                .child_exited_handlers
                .borrow_mut()
                .remove(&(session_id, hook))
                && let Some(previous_terminal) = previous_terminal.upgrade()
            {
                previous_terminal.disconnect(handler);
            }

            let pids = self.vte_child_pids.clone();
            let handler = terminal.connect_child_exited(move |_terminal, status| {
                // Remove PID — process already exited, no need to kill on tab close
                pids.borrow_mut().remove(&session_id);
                callback(status);
            });

            self.child_exited_handlers
                .borrow_mut()
                .insert((session_id, hook), (terminal.downgrade(), handler));
        } else {
            // The signal is attached to the widget, so with no terminal there
            // is nothing to attach to and the caller's cleanup would never run.
            // Silent before, and silence here is how issue #297 stayed
            // invisible for two releases.
            tracing::warn!(
                %session_id,
                ?hook,
                "No terminal for session — child-exited handler not connected"
            );
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
        let base_theme = TerminalTheme::resolve(theme_name, crate::app::system_is_dark());
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

    /// Repaints every live terminal with the current colour theme.
    ///
    /// For the case no setting changed: `color_theme` is
    /// [`FOLLOW_SYSTEM_THEME`][rustconn_core::terminal_themes::FOLLOW_SYSTEM_THEME]
    /// and the desktop flipped light/dark, so the *resolved* palette is different
    /// while `settings.toml` is untouched. Nothing else on that path repaints.
    ///
    /// Colours only — unlike [`Self::apply_settings`] this leaves font, scrollback
    /// and erase bindings alone, so it needs no `reapply_erase_modes` chaser and
    /// cannot undo a per-session Backspace choice (issue #271).
    /// Per-connection overrides are layered back on for the same reason
    /// [`Self::reapply_theme_overrides`] exists (issue #99).
    pub fn reapply_colors<F>(&self, theme_name: &str, get_theme_override: F)
    where
        F: Fn(Uuid) -> Option<rustconn_core::models::ConnectionThemeOverride>,
    {
        let base_theme = TerminalTheme::resolve(theme_name, crate::app::system_is_dark());
        let terminals = self.terminals.borrow();
        let session_info = self.session_info.borrow();
        for (session_id, terminal) in terminals.iter() {
            config::setup_colors_with_theme(terminal, theme_name);
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

        // Update tooltip to include group name.
        if let Some(page) = self.sessions.borrow().get(&session_id) {
            let current_tooltip = page.tooltip().unwrap_or_default().to_string();
            page.set_tooltip(&Self::grouped_tooltip(&current_tooltip, group_name));
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
        let affected: Vec<Uuid> = self
            .session_info
            .borrow()
            .iter()
            .filter(|(_, info)| info.connection_id == connection_id)
            .map(|(id, _)| *id)
            .collect();

        let chrome = self.tab_chrome();
        for session_id in &affected {
            Self::apply_session_label(&chrome, *session_id, new_name);
        }
        if !affected.is_empty() {
            tracing::debug!(
                connection = %connection_id,
                sessions = affected.len(),
                "renamed open sessions after a connection rename"
            );
        }
        affected
    }

    /// Applies a new label to one session: its metadata, the tab title and the
    /// tooltip.
    ///
    /// Takes the two maps rather than `&self` because the tab context menu
    /// reaches it from a `'static` action closure, which holds clones of both
    /// and cannot borrow the notebook.
    ///
    /// The tab chrome is touched here, and the split pane header showing the
    /// session through the relabel hook. A session moved to its own window has
    /// no tab, so for it only the stored name changes here; its window title is
    /// renamed by the caller or read from `info.name` when it is placed.
    ///
    /// The tab hosting a split layout names its focused pane (issue #421), so
    /// a relabelled session — the owner, or a guest in one of its panes, which
    /// Rename Tab targets when the guest's pane has focus — is re-derived there
    /// once the label is stored.
    pub(super) fn apply_session_label(
        chrome: &split_title::TabChrome,
        session_id: Uuid,
        label: &str,
    ) {
        let (group, host) = {
            let mut info_ref = chrome.session_info.borrow_mut();
            let Some(info) = info_ref.get_mut(&session_id) else {
                return;
            };
            info.name = label.to_owned();
            (info.tab_group.clone(), info.host.clone())
        };
        Self::label_session_view(
            &chrome.terminals,
            &chrome.session_widgets,
            session_id,
            label,
        );

        // The page is bound to its own `let` first: an `if let` scrutinee
        // temporary would keep the `sessions` borrow alive across the two GTK
        // setters below.
        let page = chrome.sessions.borrow().get(&session_id).cloned();
        let retitled = page.is_some();
        if let Some(page) = page {
            page.set_title(&tab_title(label, group.as_deref()));
            // A split tab's header, panes and directory lines are put back by
            // the refresh below.
            let old_tooltip = page.tooltip().unwrap_or_default();
            page.set_tooltip(&Self::relabelled_tooltip(
                old_tooltip.as_str(),
                label,
                host.as_deref(),
                group.as_deref(),
            ));
        }

        Self::refresh_split_tabs_for(chrome, session_id, retitled.then_some(label));

        // The split pane header showing the session follows the rename too.
        // The hook is cloned out first, so it runs with no notebook borrow.
        let hook = chrome.on_session_relabelled.borrow().clone();
        if let Some(hook) = hook {
            hook(session_id, label);
        }
    }

    /// Gives a session's view the name a screen reader announces for it.
    ///
    /// Without one Orca says only "terminal" or reads an unnamed image, and in
    /// a split layout every pane sounds the same. A terminal is named after
    /// `label`, the session name its tab shows; an embedded RDP or VNC viewer
    /// gets "Remote desktop <code>label</code>" and a hint on taking the keyboard back
    /// ([`crate::embedded::label_remote_view`]). The name lives on the widget,
    /// so it moves with it into a split pane or a detached window. Called when
    /// the tab is created and from [`Self::apply_session_label`] on a rename.
    pub(super) fn label_session_view(
        terminals: &RefCell<HashMap<Uuid, Terminal>>,
        session_widgets: &RefCell<HashMap<Uuid, SessionWidgetStorage>>,
        session_id: Uuid,
        label: &str,
    ) {
        // Cloned out so no map borrow is held while GTK updates the widget.
        let terminal = terminals.borrow().get(&session_id).cloned();
        if let Some(terminal) = terminal {
            terminal.update_property(&[gtk4::accessible::Property::Label(label)]);
            return;
        }
        let viewer = match session_widgets.borrow().get(&session_id) {
            Some(SessionWidgetStorage::EmbeddedRdp(widget)) => Some(widget.drawing_area().clone()),
            Some(SessionWidgetStorage::Vnc(widget)) => Some(widget.drawing_area().clone()),
            _ => None,
        };
        if let Some(viewer) = viewer {
            crate::embedded::label_remote_view(&viewer, label);
        }
    }

    /// The tooltip of a relabelled tab, built from scratch by `tab_tooltip`.
    ///
    /// `tab_tooltip` knows nothing of the CWD line, so the one in `old` is
    /// carried over and re-appended, keeping it alive across a rename.
    pub(super) fn relabelled_tooltip(
        old: &str,
        label: &str,
        host: Option<&str>,
        group: Option<&str>,
    ) -> String {
        let cwd_line = Self::split_cwd_line(old).1;
        Self::with_lifted_lines(Self::tab_tooltip(label, host, group), None, cwd_line)
    }

    /// Returns the group name for a session, if any.
    #[must_use]
    pub fn get_tab_group(&self, session_id: Uuid) -> Option<String> {
        self.session_info
            .borrow()
            .get(&session_id)
            .and_then(|i| i.tab_group.clone())
    }

    /// Returns whether a session's tab is pinned.
    ///
    /// Reads the TabView page rather than session metadata: pinning is live
    /// page state owned by the TabView, and the tab context menu flips it
    /// through `set_page_pinned` without telling the notebook. A session with
    /// no page — parked in a split, or in its own window — reports `false`,
    /// which is what the restore path wants: it comes back as an ordinary tab.
    #[must_use]
    pub fn is_session_pinned(&self, session_id: Uuid) -> bool {
        self.sessions
            .borrow()
            .get(&session_id)
            .is_some_and(|page| page.is_pinned())
    }

    /// Pins or unpins a session's tab.
    ///
    /// The same entry point the session restore path uses to reapply a pin that
    /// was set before the restart, so there is one way to reach page pinning
    /// rather than two that can drift. The page is cloned out of the map first:
    /// `set_page_pinned` emits `page-pinned`, and holding the `sessions` borrow
    /// across the call would let a handler that reaches the notebook re-borrow
    /// it.
    pub fn set_session_pinned(&self, session_id: Uuid, pinned: bool) {
        let page = self.sessions.borrow().get(&session_id).cloned();
        if let Some(page) = page {
            self.tab_view.set_page_pinned(&page, pinned);
        }
    }

    /// Applies a group label prefix to a tab title.
    fn apply_group_color(&self, session_id: Uuid, _color_index: usize) {
        if let Some(page) = self.sessions.borrow().get(&session_id)
            && let Some(info) = self.session_info.borrow().get(&session_id)
            && let Some(ref group_name) = info.tab_group
        {
            // Compose from `info.name`, the structural base label, not from the
            // rendered title: stripping a prefix off the title would eat the
            // first segment of a user label that happens to look like `[x] y`.
            // A split tab names its focused pane instead (issue #421).
            let name = self
                .split_tab_shown_name(session_id)
                .unwrap_or_else(|| info.name.clone());
            page.set_title(&tab_title(&name, Some(group_name)));
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

    /// Sets the persistent all-protocol callback fired when a session is shown.
    ///
    /// The callback receives `(session_id, connection_id)` and fires from
    /// [`Self::notify_tab_added`], which every protocol's tab-creation path
    /// calls. Unlike the one-shot `on_tab_added`, this slot is restored after
    /// firing. Wired from the window to drive auto-hide-on-connect (issue #401).
    pub fn set_on_session_shown<F>(&self, callback: F)
    where
        F: Fn(Uuid, Uuid) + 'static,
    {
        *self.on_session_shown.borrow_mut() = Some(Box::new(callback));
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

    /// Fires the `on_tab_added` callback if set.
    fn notify_tab_added(&self, session_id: Uuid, connection_id: Uuid) {
        // Cluster membership is resolved here rather than in each creation path
        // so that every protocol is covered, and before the callback so an
        // observer sees the tab already labelled with its cluster.
        self.resolve_cluster_pending(connection_id, session_id);

        // Take the callback out to avoid holding a borrow across the call —
        // the callback may set `on_tab_added` itself, which borrows it too.
        let callback = self.on_tab_added.borrow_mut().take();
        if let Some(cb) = callback {
            cb(session_id, connection_id);
            // Restore the callback for future tab creations UNLESS it was
            // consumed. Convention: the callback sets the `on_tab_added` slot
            // to a new value (or the slot stays None if consumed). If the slot
            // is still None after the call, the callback was NOT self-clearing
            // from within (because take already emptied it), so we restore.
            // If the workspace callback wants to signal "done", it must NOT
            // clear the slot — instead it signals via a shared flag
            // captured in the closure. We simply always restore here; the
            // workspace code uses an `Rc<Cell<bool>>` to stop re-firing.
            let mut slot = self.on_tab_added.borrow_mut();
            if slot.is_none() {
                *slot = Some(cb);
            }
        }

        // Auto-hide-on-connect and any future "a session appeared" observers.
        // Taken out of the slot so the callback may re-enter the notebook
        // without a BorrowMutError; restored afterwards because, unlike
        // on_tab_added, this one is persistent (issue #401).
        let shown_cb = self.on_session_shown.borrow_mut().take();
        if let Some(cb) = shown_cb {
            cb(session_id, connection_id);
            let mut slot = self.on_session_shown.borrow_mut();
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
    /// [`CompiledHighlightRules::compile_with_options`] and gives the session a
    /// fresh [`HighlightOverlay`], the transparent layer that draws colored
    /// backgrounds and underlines on top of the VTE terminal and repaints
    /// itself as the terminal changes.
    ///
    /// When `include_builtin_defaults` is `false` the built-in
    /// ERROR/WARNING/CRITICAL/FATAL rules are not applied (issue #343).
    ///
    /// Safe to call again for the same session — on reconnect, or when
    /// [`Self::reapply_highlight_rules`] pushes new Settings: the previous
    /// overlay is dropped first, which removes its layer, its signal handlers
    /// and its hover regexes instead of stacking a second set (issue #343).
    ///
    /// VTE's `match_add_regex()` is still registered for hover-underline
    /// feedback, but the actual colored rendering is done by the overlay.
    pub fn set_highlight_rules(
        &self,
        session_id: Uuid,
        global_rules: &[HighlightRule],
        per_conn_rules: &[HighlightRule],
        include_builtin_defaults: bool,
    ) {
        let compiled = CompiledHighlightRules::compile_with_options(
            global_rules,
            per_conn_rules,
            include_builtin_defaults,
        );

        // Drop the previous overlay outside the map borrow: its `Drop` talks to
        // GTK and VTE.
        let previous = self.highlight_overlays.borrow_mut().remove(&session_id);
        drop(previous);

        let terminal = self.terminals.borrow().get(&session_id).cloned();
        let Some(terminal) = terminal else {
            self.session_highlight_rules
                .borrow_mut()
                .insert(session_id, compiled);
            return;
        };

        // Still register with VTE for hover-underline feedback. The tags go to
        // the overlay, which unregisters them when it is replaced.
        let mut match_tags = Vec::new();
        for rule in compiled.source_patterns() {
            let pattern = &rule.pattern;
            match vte4::Regex::for_match(pattern, PCRE2_MULTILINE) {
                Ok(vte_regex) => {
                    match_tags.push(terminal.match_add_regex(&vte_regex, 0));
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

        let overlay = HighlightOverlay::attach(
            &terminal,
            Rc::clone(&self.session_highlight_rules),
            session_id,
            match_tags,
        );
        self.highlight_overlays
            .borrow_mut()
            .insert(session_id, overlay);
    }

    /// Re-applies highlight rules to every open session that has them.
    ///
    /// Settings used to reach only sessions started afterwards, so turning the
    /// built-in rules off or editing a global rule changed nothing on screen
    /// until a reconnect (issue #343). `get_connection_rules` returns a
    /// connection's own rules; a session whose connection is gone (a quick
    /// connect, a deleted entry) gets the global rules alone. Sessions that
    /// never had highlighting — the local shell, the mc file browser, embedded
    /// viewers — are left alone.
    pub fn reapply_highlight_rules<F>(
        &self,
        global_rules: &[HighlightRule],
        include_builtin_defaults: bool,
        get_connection_rules: F,
    ) where
        F: Fn(Uuid) -> Option<Vec<HighlightRule>>,
    {
        let sessions: Vec<(Uuid, Uuid)> = {
            let highlighted = self.session_highlight_rules.borrow();
            let session_info = self.session_info.borrow();
            highlighted
                .keys()
                .map(|session_id| {
                    let connection_id = session_info
                        .get(session_id)
                        .map_or_else(Uuid::nil, |info| info.connection_id);
                    (*session_id, connection_id)
                })
                .collect()
        };
        for (session_id, connection_id) in sessions {
            let per_conn_rules = get_connection_rules(connection_id).unwrap_or_default();
            self.set_highlight_rules(
                session_id,
                global_rules,
                &per_conn_rules,
                include_builtin_defaults,
            );
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

/// The `AdwTabView` shortcut set for a keyboard-passthrough state.
///
/// Passthrough disables every one of them so the chord reaches the session;
/// leaving it restores the full default set, which is what the TabView is built
/// with, so toggling twice is a no-op.
const fn tab_view_shortcuts_for_passthrough(enabled: bool) -> adw::TabViewShortcuts {
    if enabled {
        adw::TabViewShortcuts::NONE
    } else {
        adw::TabViewShortcuts::ALL_SHORTCUTS
    }
}

/// Whether the desktop's shortcuts should go to `viewer` while it has focus.
///
/// `hook` is the connection's own choice; `None` follows `auto_capture`, the
/// application's "Auto-capture shortcuts in remote desktop" setting.
/// Keyboard passthrough, which sends them in every case, is checked by the
/// callers.
fn viewer_takes_shortcuts(
    viewer: &gtk4::Widget,
    hook: Option<RdpKeyboardHook>,
    auto_capture: bool,
) -> bool {
    hook_takes_shortcuts(hook, auto_capture, || {
        viewer
            .root()
            .and_downcast::<gtk4::Window>()
            .is_some_and(|window| window.is_fullscreen())
    })
}

/// The decision behind [`viewer_takes_shortcuts`], with the window's full-screen
/// state asked for only when the hook depends on it.
fn hook_takes_shortcuts(
    hook: Option<RdpKeyboardHook>,
    auto_capture: bool,
    fullscreen: impl FnOnce() -> bool,
) -> bool {
    match hook {
        None => auto_capture,
        Some(RdpKeyboardHook::Local) => false,
        Some(RdpKeyboardHook::Remote) => true,
        Some(RdpKeyboardHook::FullscreenOnly) => fullscreen(),
    }
}

/// Inhibits or restores the compositor's shortcuts for the window `widget` is
/// in, so a detached session window asks for its own surface.
///
/// On Wayland this is `zwp_keyboard_shortcuts_inhibit_v1`, on X11 a keyboard
/// grab; GTK picks. It only has effect while that window has keyboard focus.
fn set_system_shortcuts_inhibited(widget: &gtk4::Widget, inhibit: bool) {
    let Some(toplevel) = widget
        .native()
        .and_then(|native| native.surface())
        .and_downcast::<gtk4::gdk::Toplevel>()
    else {
        return;
    };
    tracing::debug!(inhibit, "keyboard passthrough: system shortcuts");
    if inhibit {
        toplevel.inhibit_system_shortcuts(None::<&gtk4::gdk::Event>);
    } else {
        toplevel.restore_system_shortcuts();
    }
}

#[cfg(test)]
mod passthrough_shortcut_tests {
    use libadwaita as adw;
    use rustconn_core::models::RdpKeyboardHook;

    use super::{hook_takes_shortcuts, tab_view_shortcuts_for_passthrough};

    /// A connection's own choice wins over the app's auto-capture setting, and
    /// "only in full screen" follows the window.
    #[test]
    fn keyboard_hook_decides_where_shortcuts_go() {
        for auto in [true, false] {
            assert_eq!(hook_takes_shortcuts(None, auto, || unreachable!()), auto);
            assert!(!hook_takes_shortcuts(
                Some(RdpKeyboardHook::Local),
                auto,
                || true
            ));
            assert!(hook_takes_shortcuts(
                Some(RdpKeyboardHook::Remote),
                auto,
                || false
            ));
            for fullscreen in [true, false] {
                assert_eq!(
                    hook_takes_shortcuts(Some(RdpKeyboardHook::FullscreenOnly), auto, || {
                        fullscreen
                    }),
                    fullscreen
                );
            }
        }
    }

    /// Issue #356: Ctrl+Shift+Tab and friends switched tabs in passthrough.
    #[test]
    fn passthrough_disables_every_tab_view_shortcut() {
        assert_eq!(
            tab_view_shortcuts_for_passthrough(true),
            adw::TabViewShortcuts::NONE
        );
    }

    /// Leaving passthrough must give back exactly the set `adw::TabView::new()`
    /// starts with — Ctrl+Tab, Ctrl+Page Up/Down and Alt+digit keep working.
    #[test]
    fn leaving_passthrough_restores_the_default_set() {
        assert_eq!(
            tab_view_shortcuts_for_passthrough(false),
            adw::TabViewShortcuts::ALL_SHORTCUTS
        );
    }
}

/// The output-filter availability gate.
///
/// Extracted into a free function precisely so this can be checked without a GTK
/// notebook — the absolute-path branch used to accept any existing file.
#[cfg(test)]
mod output_filter_gate_tests {
    use super::filter_program_is_runnable;

    #[test]
    fn a_bare_command_on_path_is_runnable() {
        assert!(filter_program_is_runnable("sh"));
    }

    #[test]
    fn a_command_that_does_not_exist_is_not_runnable() {
        assert!(!filter_program_is_runnable(
            "rustconn-no-such-filter-binary"
        ));
        assert!(!filter_program_is_runnable(""));
    }

    /// The regression this gate was rewritten for: an absolute path that exists
    /// but is not executable must be refused *before* the pipeline is built.
    /// Checking `Path::is_file()` alone passed it and then lost every byte of the
    /// session at exec time.
    #[test]
    fn an_existing_but_non_executable_absolute_path_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("not-executable");
        std::fs::write(&path, b"#!/bin/sh\necho hi\n").expect("write");

        assert!(
            !filter_program_is_runnable(path.to_str().expect("utf-8 path")),
            "a file without the executable bit is not a runnable filter"
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
            assert!(
                filter_program_is_runnable(path.to_str().expect("utf-8 path")),
                "the same path becomes runnable once it is executable"
            );
        }
    }

    #[test]
    fn a_directory_is_not_a_runnable_filter() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(!filter_program_is_runnable(
            dir.path().to_str().expect("utf-8 path")
        ));
    }
}

/// The mc tab passes its wrapper `PATH` as an `envv` entry; this pins that it
/// replaces the extended `PATH` rather than sitting next to it (issue #422).
#[cfg(test)]
mod child_env_tests {
    use super::build_child_env;

    #[test]
    fn a_path_entry_from_the_caller_replaces_the_extended_path() {
        // An explicit agent socket keeps the test away from the agent lookup.
        let env = build_child_env(
            Some(&["PATH=/run/rc-mc-wrapper:/usr/bin"]),
            Some("/run/rc-test-agent.sock"),
        );
        let paths: Vec<&str> = env
            .iter()
            .map(|entry| entry.as_str())
            .filter(|entry| entry.starts_with("PATH="))
            .collect();
        assert_eq!(paths, ["PATH=/run/rc-mc-wrapper:/usr/bin"]);
    }

    #[test]
    fn without_a_caller_path_the_child_gets_the_extended_one() {
        let env = build_child_env(None, Some("/run/rc-test-agent.sock"));
        let paths: Vec<String> = env
            .iter()
            .filter(|entry| entry.starts_with("PATH="))
            .map(|entry| entry.as_str().to_owned())
            .collect();
        assert_eq!(
            paths,
            [format!(
                "PATH={}",
                rustconn_core::cli_download::get_extended_path()
            )]
        );
    }
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

    // The working-directory line is added by `update_session_cwd` through
    // `strip_cwd_line`, which must remove exactly its own line wherever it sits.
    const CWD: &str = TerminalNotebook::CWD_TOOLTIP_PREFIX; // "\n📁 "

    #[test]
    fn strip_cwd_line_is_a_noop_without_one() {
        assert_eq!(
            TerminalNotebook::strip_cwd_line("prod-db\n10.0.0.5"),
            "prod-db\n10.0.0.5"
        );
    }

    #[test]
    fn strip_cwd_line_removes_a_trailing_cwd() {
        let t = format!("prod-db\n10.0.0.5{CWD}/var/log");
        assert_eq!(TerminalNotebook::strip_cwd_line(&t), "prod-db\n10.0.0.5");
    }

    #[test]
    fn strip_cwd_line_removes_a_cwd_followed_by_a_group_line() {
        // A group toggle may append after the CWD line; the strip must remove
        // only the CWD line and keep the group line and its leading newline.
        let t = format!("prod-db{CWD}/home/me\n[Production]");
        assert_eq!(
            TerminalNotebook::strip_cwd_line(&t),
            "prod-db\n[Production]"
        );
    }

    #[test]
    fn stripping_then_appending_replaces_in_place() {
        let t = format!("prod-db{CWD}/old\n[Production]");
        let base = TerminalNotebook::strip_cwd_line(&t);
        assert_eq!(base, "prod-db\n[Production]");
        // Appending the fresh CWD line, then stripping again, returns the base —
        // the update is idempotent however many times the directory changes.
        let updated = format!("{base}{CWD}/new");
        assert_eq!(updated, format!("prod-db\n[Production]{CWD}/new"));
        assert_eq!(TerminalNotebook::strip_cwd_line(&updated), base);
    }

    #[test]
    fn split_cwd_line_returns_the_line_for_reappending() {
        let t = format!("prod-db\n10.0.0.5{CWD}/var/log");
        let (without, cwd) = TerminalNotebook::split_cwd_line(&t);
        assert_eq!(without, "prod-db\n10.0.0.5");
        assert_eq!(cwd.as_deref(), Some(format!("{CWD}/var/log").as_str()));
        // Re-appending the lifted line restores the original exactly.
        assert_eq!(format!("{without}{}", cwd.unwrap()), t);
    }

    #[test]
    fn split_cwd_line_is_none_without_one() {
        let (without, cwd) = TerminalNotebook::split_cwd_line("prod-db\n[Production]");
        assert_eq!(without, "prod-db\n[Production]");
        assert_eq!(cwd, None);
    }

    // The tooltip mutators that rebuild the host/group/broadcast portion must
    // lift the CWD line out first and re-append it, so a group assignment, a
    // broadcast toggle or a rename never drops or mis-orders the directory.
    // The tests call the transformations the mutators themselves use.

    fn broadcast_tooltip(tooltip: &str, member: bool) -> String {
        TerminalNotebook::broadcast_tooltip(tooltip, member)
    }

    fn group_tooltip(tooltip: &str, group: &str) -> String {
        TerminalNotebook::grouped_tooltip(tooltip, group)
    }

    #[test]
    fn tab_menu_set_group_keeps_the_cwd_line_last() {
        // Review of #421, finding 5: the tab menu's Set Group lifted the panes
        // line but not the directory line, so the group line landed after 📁.
        let t = format!("prod-db\n10.0.0.5{CWD}/var/log");
        assert_eq!(
            group_tooltip(&t, "Production"),
            format!("prod-db\n10.0.0.5\n[Production]{CWD}/var/log")
        );
        // Regrouping replaces the group line in place.
        let regrouped = group_tooltip(&group_tooltip(&t, "Production"), "Staging");
        assert_eq!(
            regrouped,
            format!("prod-db\n10.0.0.5\n[Staging]{CWD}/var/log")
        );
    }

    #[test]
    fn removing_a_group_keeps_the_cwd_and_panes_lines() {
        const PANES: &str = "\nPanes: server_A, server_B";
        let t = format!("server_A\n10.0.0.5\n[Production]{PANES}{CWD}/var/log");
        assert_eq!(
            TerminalNotebook::ungrouped_tooltip(&t),
            Some(format!("server_A\n10.0.0.5{PANES}{CWD}/var/log"))
        );
        // No group line: nothing to remove, the tooltip is left alone.
        assert_eq!(
            TerminalNotebook::ungrouped_tooltip(&format!("prod-db{CWD}/var/log")),
            None
        );
    }

    #[test]
    fn a_split_tabs_panes_line_survives_broadcast_and_group_changes() {
        // Issue #421: the panes line is appended last, after a broadcast line;
        // turning broadcast off must still find and strip `[Broadcast]`, and
        // a group assignment must not cut the panes line off with it.
        const PANES: &str = "\nPanes: server_A, server_B";
        let split = format!("server_A\n10.0.0.5\n[Broadcast]{PANES}");
        assert_eq!(
            broadcast_tooltip(&split, false),
            format!("server_A\n10.0.0.5{PANES}")
        );
        assert_eq!(
            group_tooltip(&split, "Production"),
            format!("server_A\n10.0.0.5\n[Production]{PANES}")
        );
        // With a directory line too, both lines come back.
        let with_cwd = format!("server_A{PANES}{CWD}/var/log");
        assert_eq!(
            broadcast_tooltip(&with_cwd, true),
            format!("server_A\n[Broadcast]{PANES}{CWD}/var/log")
        );
    }

    #[test]
    fn broadcast_toggle_keeps_the_cwd_line() {
        let with_cwd = format!("prod-db\n10.0.0.5{CWD}/var/log");
        // Enabling broadcast keeps the directory, with broadcast before it.
        let on = broadcast_tooltip(&with_cwd, true);
        assert_eq!(on, format!("prod-db\n10.0.0.5\n[Broadcast]{CWD}/var/log"));
        // Disabling it again strips only the broadcast line, directory intact.
        let off = broadcast_tooltip(&on, false);
        assert_eq!(off, with_cwd);
    }

    #[test]
    fn group_assignment_keeps_the_cwd_line_even_with_broadcast_present() {
        // The exact clobber case: broadcast line sits before the CWD line.
        let t = format!("prod-db\n10.0.0.5\n[Broadcast]{CWD}/var/log");
        let grouped = group_tooltip(&t, "Production");
        // The group split lands on `[Broadcast]` (not the CWD line), and the
        // directory is re-appended last rather than discarded.
        assert_eq!(
            grouped,
            format!("prod-db\n10.0.0.5\n[Production]{CWD}/var/log")
        );
    }

    #[test]
    fn osc7_uri_keeps_the_path_for_a_remote_host() {
        // The remote case GIO's File::path() drops to None — we must keep the
        // path and ignore the authority.
        assert_eq!(
            TerminalNotebook::path_from_osc7_uri("file://remotehost/var/log"),
            Some("/var/log".to_string())
        );
        // Local / empty authority still works.
        assert_eq!(
            TerminalNotebook::path_from_osc7_uri("file:///home/me"),
            Some("/home/me".to_string())
        );
        assert_eq!(
            TerminalNotebook::path_from_osc7_uri("file://localhost/etc"),
            Some("/etc".to_string())
        );
    }

    #[test]
    fn osc7_uri_percent_decodes_and_rejects_non_file() {
        assert_eq!(
            TerminalNotebook::path_from_osc7_uri("file://host/home/a%20b/%D1%82"),
            Some("/home/a b/т".to_string())
        );
        // Non-file scheme or no path → None (CWD line removed).
        assert_eq!(TerminalNotebook::path_from_osc7_uri("http://host/x"), None);
        assert_eq!(TerminalNotebook::path_from_osc7_uri("file://host"), None);
    }

    #[test]
    fn rename_keeps_the_cwd_line() {
        // `apply_session_label` rebuilds via `tab_tooltip` then re-appends CWD.
        let old = format!("prod-db\n10.0.0.5{CWD}/var/log");
        let rebuilt = TerminalNotebook::relabelled_tooltip(&old, "renamed", Some("10.0.0.5"), None);
        assert_eq!(rebuilt, format!("renamed\n10.0.0.5{CWD}/var/log"));
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

    /// A scroll position between two rows stays where it was put.
    ///
    /// Touchpad scrolling and a drag on the scrollbar leave the view part-way
    /// through a row, and the highlight overlay draws its rows at the same
    /// sub-row offset VTE does (issue #343). That offset exists only because
    /// VTE keeps the fraction instead of snapping the adjustment to a whole row,
    /// so it is pinned here: should a future VTE start rounding, this fails and
    /// the overlay's offset is always zero.
    #[test]
    #[ignore = "initialises GTK: needs a display and its own process"]
    fn fractional_scroll_position_is_kept() {
        use gtk4::prelude::{AdjustmentExt, GtkWindowExt, ScrollableExt};

        if gtk4::init().is_err() {
            return;
        }
        let terminal = super::Terminal::new();
        // VTE sizes its vertical adjustment (upper, page_size) in size_allocate,
        // so a terminal never placed in a mapped window reports 0 for
        // both (measured on VTE 0.84.1). Give it a real allocation first.
        let window = gtk4::Window::new();
        window.set_child(Some(&terminal));
        window.present();
        terminal.set_size(80, 10);
        for line in 0..40 {
            terminal.feed(format!("line {line}\r\n").as_bytes());
        }
        let adjustment = terminal
            .vadjustment()
            .expect("VTE creates its own vertical adjustment");
        assert!(
            pump_until(|| adjustment.upper() - adjustment.page_size() >= 20.0),
            "forty lines in ten rows leave scrollback to scroll through ({OPT_IN})"
        );

        adjustment.set_value(12.5);
        // Give VTE every chance to snap it before concluding that it does not.
        pump_until(|| (adjustment.value() - 12.5).abs() > f64::EPSILON);
        assert!(
            (adjustment.value() - 12.5).abs() <= f64::EPSILON,
            "VTE moved the scroll position from 12.5 to {}",
            adjustment.value()
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

    const MOSH_BANNER: &str =
        "🔗 Connecting via MOSH to web...\r\n⚡ Executing: mosh user@web\r\n\r\n";

    /// What mosh-client does when it starts: switch to the alternate screen,
    /// clear it, home the cursor and draw the remote screen.
    const MOSH_DRAW: &[u8] =
        "\x1b[?1049h\x1b[2J\x1b[HLast login: today\r\nuser@web:~ ❯ ".as_bytes();

    /// A terminal with 40 rows of kept history in 24 visible rows, as after a
    /// reconnect with "keep history" on, and the start a connect banner
    /// records before VTE has processed it.
    fn terminal_with_history() -> (super::Terminal, Option<super::output_start::OutputStart>) {
        let terminal = super::Terminal::new();
        terminal.set_size(80, 24);
        for line in 0..40 {
            terminal.feed(format!("old session output {line}\r\n").as_bytes());
        }
        assert!(
            pump_until(|| terminal.cursor_position().1 >= 40),
            "{OPT_IN}"
        );
        let start = Some(super::output_start::OutputStart::AfterBanner {
            last_line: super::output_start::banner_last_line(MOSH_BANNER),
            fed_at_row: terminal.cursor_position().1,
        });
        (terminal, start)
    }

    /// The alternate-screen route of the status verdict, against a real VTE.
    ///
    /// Once the banner has been processed in a pass of its own, a client's
    /// switch to the alternate screen keeps the buffer's row numbering: VTE
    /// (0.8x, measured with 40 rows of kept history) puts the alternate
    /// screen's rows below the start row, so the rows from the start to the
    /// cursor hold what the client drew and the row-based verdict reads it.
    /// `ConnectionOutput::screen_replaced`, for a cursor above the start row,
    /// does not arise on this route.
    #[test]
    #[ignore = "initialises GTK: needs a display and its own process"]
    fn a_client_on_the_alternate_screen_is_read_below_the_start_row() {
        if gtk4::init().is_err() {
            return;
        }
        let (terminal, mut start) = terminal_with_history();
        let base = terminal.cursor_position().1 + 3;
        terminal.feed(MOSH_BANNER.as_bytes());
        assert!(
            pump_until(|| terminal.cursor_position().1 >= base),
            "{OPT_IN}"
        );

        let (output, resolved) = super::read_connection_output(&terminal, start.as_ref());
        let output = output.expect("the banner alone resolves the start");
        assert_eq!((output.row, output.screen_replaced), (0, false));
        assert!(resolved.is_some(), "the start is resolved once");
        start = resolved;

        terminal.feed(MOSH_DRAW);
        assert!(pump_until(|| terminal.cursor_position().0 > 0), "{OPT_IN}");
        let (output, _) = super::read_connection_output(&terminal, start.as_ref());
        assert!(
            output.as_ref().is_some_and(|o| !o.screen_replaced
                && o.row > 0
                && o.text.contains("user@web:~ ❯")
                && !o.text.contains("old session output")),
            "cursor {:?}, start row {base}, output {output:?}",
            terminal.cursor_position(),
        );
    }

    /// The known limit of that route: when VTE processes the banner and the
    /// switch to the alternate screen in one pass, the banner is never seen
    /// on the normal screen, the start is never resolved, and the detectors
    /// stay undecided (the session stays on "connecting"). Documented in the
    /// CHANGELOG; this pins it so a change in VTE or in the search shows up.
    #[test]
    #[ignore = "initialises GTK: needs a display and its own process"]
    fn a_banner_and_the_alternate_screen_in_one_pass_stay_undecided() {
        if gtk4::init().is_err() {
            return;
        }
        let (terminal, start) = terminal_with_history();
        let fed_at_row = terminal.cursor_position().1;
        terminal.feed(MOSH_BANNER.as_bytes());
        terminal.feed(MOSH_DRAW);
        assert!(pump_until(|| terminal.cursor_position().0 > 0), "{OPT_IN}");
        let (output, resolved) = super::read_connection_output(&terminal, start.as_ref());
        assert_eq!(
            (&output, &resolved),
            (&None, &None),
            "cursor {:?}, fed at {fed_at_row}",
            terminal.cursor_position(),
        );
    }
}
