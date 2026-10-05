//! Terminal types and data structures
//!
//! This module contains type definitions for terminal sessions.

use std::cell::RefCell;
use std::path::PathBuf;
use std::process::Child;
use std::rc::Rc;

use uuid::Uuid;

use crate::embedded_rdp::EmbeddedRdpWidget;
#[cfg(feature = "web-embedded")]
use crate::embedded_web::EmbeddedWebWidget;
use crate::session::VncSessionWidget;

/// Terminal session information
#[derive(Debug, Clone)]
pub struct TerminalSession {
    /// Session UUID for session management
    pub id: Uuid,
    /// Connection ID this session is for
    pub connection_id: Uuid,
    /// Connection name for display
    pub name: String,
    /// Protocol type (ssh, rdp, vnc, spice)
    pub protocol: String,
    /// Whether this is an embedded terminal or external window
    pub is_embedded: bool,
    /// Host shown on the tab tooltip's second line, when the creation path knew
    /// one (embedded VNC today).
    ///
    /// Carried with the session so a tab rebuilt after a detach or a rename
    /// keeps the same tooltip the creation path produced (Requirement 2.3,
    /// issue #236).
    pub host: Option<String>,
    /// Log file path if logging is enabled
    pub log_file: Option<PathBuf>,
    /// History entry ID for tracking connection history
    pub history_entry_id: Option<Uuid>,
    /// Tab group name (e.g., "Production", "Staging")
    pub tab_group: Option<String>,
    /// Color index from palette for visual grouping
    pub tab_color_index: Option<usize>,
    /// Timestamp when the session was created/connected
    pub connected_at: chrono::DateTime<chrono::Utc>,
    /// When the session was last reconnected in place, if it ever was.
    ///
    /// Kept apart from `connected_at`, which the duration display and the
    /// rapid-crash check read as the tab's start.
    pub reconnected_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl TerminalSession {
    /// When the process now running in the session was started: the last
    /// in-place reconnect, or the session's creation.
    pub fn started_at(&self) -> chrono::DateTime<chrono::Utc> {
        self.reconnected_at.unwrap_or(self.connected_at)
    }

    /// Formats the session duration as a human-readable string.
    pub fn format_duration(&self) -> String {
        let elapsed = chrono::Utc::now()
            .signed_duration_since(self.connected_at)
            .num_seconds()
            .max(0);
        let hours = elapsed / 3600;
        let minutes = (elapsed % 3600) / 60;
        let seconds = elapsed % 60;
        if hours > 0 {
            format!("{hours}h {minutes:02}m")
        } else if minutes > 0 {
            format!("{minutes}m {seconds:02}s")
        } else {
            format!("{seconds}s")
        }
    }
}

/// Session widget storage for non-SSH sessions
pub enum SessionWidgetStorage {
    /// VNC session widget
    Vnc(Rc<VncSessionWidget>),
    /// Embedded RDP widget (with dynamic resolution)
    EmbeddedRdp(Rc<EmbeddedRdpWidget>),
    /// Embedded Web browser widget (WebKitGTK 6.0)
    #[cfg(feature = "web-embedded")]
    EmbeddedWeb(Rc<EmbeddedWebWidget>),
    /// External process (xfreerdp, vncviewer, etc.) — killed on tab close
    ExternalProcess(Rc<RefCell<Option<Child>>>),
}

// ============================================================================
// Cluster tab tracking
// ============================================================================

/// A cluster whose connections are being opened, remembered until each tab appears.
///
/// A cluster member's tab may be created long after the launch — an SSH session
/// waits on a TCP port check first — so the notebook records the cluster against
/// the *connection* id and resolves it when the tab shows up. The name travels
/// with the id because it becomes the tab group's name, and the notebook has no
/// way to look a cluster up: cluster definitions live in `AppState`.
#[derive(Debug, Clone)]
pub struct PendingCluster {
    /// The cluster this connection is being opened as part of.
    pub cluster_id: Uuid,
    /// The cluster's name, used verbatim as the tab group name.
    pub group: String,
}

/// The open tabs of one cluster, and the tab group they were labelled with.
///
/// The name is kept here rather than looked up on demand so that closing a
/// cluster can retire its group from the group registry — by then the member
/// sessions are gone and nothing else remembers what they were called.
#[derive(Debug, Clone)]
pub struct ClusterTabs {
    /// The tab group name every member tab carries.
    pub group: String,
    /// Session ids of the cluster's open tabs, in the order they appeared.
    pub sessions: Vec<Uuid>,
}

/// Reports whether any live session still carries `group`.
///
/// The guard on retiring a group name: a cluster's name is not necessarily
/// exclusive to it — a user may have typed the same name into "Set Group…" on an
/// unrelated tab, and two clusters may share a name — so a name is only forgotten
/// once nothing is labelled with it any more.
pub fn group_still_in_use<'a, I>(group: &str, live: I) -> bool
where
    I: IntoIterator<Item = Option<&'a str>>,
{
    live.into_iter().any(|live| live == Some(group))
}

/// Composes a tab title from a session name and its group.
///
/// One place decides the `[group] ` prefix, so a tab rebuilt after a park, a
/// detach or a rename is titled exactly as the creation path titled it. The
/// brackets are not translated: they are punctuation around user data, and the
/// group name is whatever the user or the cluster is called.
#[must_use]
pub fn tab_title(name: &str, group: Option<&str>) -> String {
    match group {
        Some(group) => format!("[{group}] {name}"),
        None => name.to_owned(),
    }
}

/// Strips a `[group] ` prefix composed by [`tab_title`] from a rendered title.
///
/// Needed because the group can change on a tab that already has one, and the
/// only record of the base name in that moment is the title itself.
///
/// Deliberately conservative: a name that merely *starts* with `[` is left alone
/// unless it also contains `"] "`, and even then the caller may be re-splitting a
/// connection genuinely named `[dc1] web`. That ambiguity is inherent to encoding
/// the group into the title string and is why the group is also carried
/// structurally on the session.
#[must_use]
pub fn strip_group_prefix(title: &str) -> &str {
    if !title.starts_with('[') {
        return title;
    }
    title
        .find("] ")
        .map_or(title, |pos| &title[pos + "] ".len()..])
}

/// Protocol marker of a Local Shell tab (its connection id is nil).
///
/// One definition, because three places have to agree on it: the creation
/// path that stamps the protocol, the tab menu that decides whether a tab may
/// be relabelled, and the restore path that recognises the entry again.
pub const LOCAL_SHELL_PROTOCOL: &str = "local";

/// Title of a Local Shell tab that carries no label of its own.
///
/// Deliberately not translated at the point of use: the value is written into
/// the session-restore snapshot and read back on the next start, so a default
/// that shifted with the locale would restore in the language it was saved in.
/// A label the user typed is user data and is not translated either.
const DEFAULT_LOCAL_SHELL_LABEL: &str = "Local Shell";

/// The title a Local Shell tab opens with: the label the user gave it, or the
/// default when there is none.
///
/// Blank and whitespace-only labels fall back rather than producing an empty
/// tab title, which is indistinguishable from a broken tab in the tab bar.
#[must_use]
pub fn local_shell_label(saved: &str) -> &str {
    let trimmed = saved.trim();
    if trimmed.is_empty() {
        DEFAULT_LOCAL_SHELL_LABEL
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_LOCAL_SHELL_LABEL, group_still_in_use, local_shell_label, strip_group_prefix,
        tab_title,
    };

    /// The shape the creation paths produce.
    #[test]
    fn a_grouped_tab_is_titled_with_its_group() {
        assert_eq!(tab_title("web1", Some("dc1")), "[dc1] web1");
        assert_eq!(tab_title("web1", None), "web1");
    }

    /// Round trip: the prefix a grouped title carries is the one that comes off.
    #[test]
    fn stripping_undoes_composing() {
        let titled = tab_title("ssh1 via ssh2", Some("staging"));
        assert_eq!(strip_group_prefix(&titled), "ssh1 via ssh2");
    }

    /// An ungrouped title is returned untouched, including one containing
    /// brackets somewhere other than the start.
    #[test]
    fn an_ungrouped_title_is_left_alone() {
        assert_eq!(strip_group_prefix("web1"), "web1");
        assert_eq!(strip_group_prefix("web1 [prod]"), "web1 [prod]");
    }

    /// A leading `[` with no closing `"] "` is not a prefix and must survive.
    #[test]
    fn a_bare_leading_bracket_is_not_a_prefix() {
        assert_eq!(strip_group_prefix("[unclosed"), "[unclosed");
        assert_eq!(strip_group_prefix("[a]b"), "[a]b");
    }

    /// A name is in use while any session carries it.
    #[test]
    fn a_group_worn_by_a_live_session_is_in_use() {
        let live = [Some("dc1"), None, Some("staging")];
        assert!(group_still_in_use("dc1", live.iter().copied()));
        assert!(group_still_in_use("staging", live.iter().copied()));
    }

    /// A name nothing carries may be retired — including when every session is
    /// ungrouped, and when there are no sessions left at all.
    #[test]
    fn a_group_nothing_carries_may_be_retired() {
        assert!(!group_still_in_use("dc1", [None, None].iter().copied()));
        assert!(!group_still_in_use("dc1", std::iter::empty()));
        assert!(!group_still_in_use("dc1", std::iter::once(Some("dc2"))));
    }

    /// The check is exact, not a prefix match: retiring "dc1" must not be blocked
    /// by a live "dc10", and must not be allowed by a live "dc1" spelt otherwise.
    #[test]
    fn the_in_use_check_matches_the_whole_name() {
        assert!(!group_still_in_use("dc1", std::iter::once(Some("dc10"))));
        assert!(!group_still_in_use("dc1", std::iter::once(Some("DC1"))));
    }

    /// The label the user typed is the title, trimmed.
    #[test]
    fn a_typed_label_becomes_the_title() {
        assert_eq!(local_shell_label("build logs"), "build logs");
        assert_eq!(local_shell_label("  build logs  "), "build logs");
    }

    /// A tab with no usable label must not end up untitled: an empty title is
    /// indistinguishable from a broken tab in the tab bar.
    #[test]
    fn an_empty_label_falls_back_to_the_default() {
        assert_eq!(local_shell_label(""), DEFAULT_LOCAL_SHELL_LABEL);
        assert_eq!(local_shell_label("   \t "), DEFAULT_LOCAL_SHELL_LABEL);
    }

    /// The restore path reads back exactly what was saved, including a label
    /// that merely looks like the default.
    #[test]
    fn a_saved_label_survives_being_read_back() {
        assert_eq!(local_shell_label(DEFAULT_LOCAL_SHELL_LABEL), "Local Shell");
        assert_eq!(local_shell_label("Local Shell 2"), "Local Shell 2");
    }
}
