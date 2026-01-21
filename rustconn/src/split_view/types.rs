//! GUI-specific types for the split view redesign
//!
//! This module contains types that are specific to the GUI layer and are not
//! part of the core data models in `rustconn-core`.

use rustconn_core::split::{DropResult, PanelId, SessionId, TabId};
use uuid::Uuid;

/// Unique identifier for a connection in the sidebar
///
/// This wraps a UUID to provide type safety when working with connection
/// identifiers from the sidebar. Connections are distinct from sessions -
/// a connection represents a saved server configuration, while a session
/// represents an active connection instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectionId(pub Uuid);

impl ConnectionId {
    /// Creates a new random connection ID
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Creates a connection ID from an existing UUID
    #[must_use]
    pub const fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// Returns the inner UUID
    #[must_use]
    pub const fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for ConnectionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ConnectionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Uuid> for ConnectionId {
    fn from(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl From<ConnectionId> for Uuid {
    fn from(id: ConnectionId) -> Self {
        id.0
    }
}

/// Source of a drag-and-drop operation
///
/// This enum represents the different sources from which a drag operation
/// can originate. The drop handler uses this to determine how to process
/// the drop and what cleanup actions are needed at the source.
///
/// # Variants
///
/// - `RootTab`: A standalone tab being dragged from the tab bar
/// - `SplitPane`: A panel from within a split container
/// - `SidebarItem`: A connection entry from the sidebar (creates new session)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropSource {
    /// A root tab being dragged from the tab bar
    ///
    /// When dropped on a panel, the tab's session moves to that panel
    /// and the source tab is removed from the tab bar.
    RootTab {
        /// The session ID of the tab being dragged
        session_id: SessionId,
    },

    /// A panel from a split container being dragged
    ///
    /// When dropped on another panel, the session moves to the target
    /// and the source panel becomes empty or is removed.
    SplitPane {
        /// The tab ID containing the source split container
        source_tab_id: TabId,
        /// The panel ID within the split container
        panel_id: PanelId,
        /// The session ID being moved
        session_id: SessionId,
    },

    /// A sidebar connection item being dragged
    ///
    /// When dropped on a panel, a new session is created for this
    /// connection and placed in the target panel.
    SidebarItem {
        /// The connection ID from the sidebar
        connection_id: ConnectionId,
    },
}

impl DropSource {
    /// Creates a new `RootTab` drop source
    #[must_use]
    pub const fn root_tab(session_id: SessionId) -> Self {
        Self::RootTab { session_id }
    }

    /// Creates a new `SplitPane` drop source
    #[must_use]
    pub const fn split_pane(
        source_tab_id: TabId,
        panel_id: PanelId,
        session_id: SessionId,
    ) -> Self {
        Self::SplitPane {
            source_tab_id,
            panel_id,
            session_id,
        }
    }

    /// Creates a new `SidebarItem` drop source
    #[must_use]
    pub const fn sidebar_item(connection_id: ConnectionId) -> Self {
        Self::SidebarItem { connection_id }
    }

    /// Returns the session ID if this source has one
    ///
    /// `SidebarItem` sources don't have a session ID because they
    /// represent a connection that hasn't been instantiated yet.
    #[must_use]
    pub const fn session_id(&self) -> Option<SessionId> {
        match self {
            Self::RootTab { session_id } | Self::SplitPane { session_id, .. } => Some(*session_id),
            Self::SidebarItem { .. } => None,
        }
    }

    /// Returns true if this is a root tab source
    #[must_use]
    pub const fn is_root_tab(&self) -> bool {
        matches!(self, Self::RootTab { .. })
    }

    /// Returns true if this is a split pane source
    #[must_use]
    pub const fn is_split_pane(&self) -> bool {
        matches!(self, Self::SplitPane { .. })
    }

    /// Returns true if this is a sidebar item source
    #[must_use]
    pub const fn is_sidebar_item(&self) -> bool {
        matches!(self, Self::SidebarItem { .. })
    }
}

/// Describes what cleanup action is needed at the source after a drop.
///
/// When a drag-and-drop operation completes, the source location may need
/// cleanup. This enum describes what action the UI layer should take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceCleanup {
    /// No cleanup needed (e.g., sidebar item creates new session).
    None,
    /// Remove the source tab from the tab bar.
    ///
    /// Used when a root tab is dropped onto a panel.
    /// Requirements 9.2: Remove source tab from tab bar.
    RemoveTab {
        /// The session ID of the tab to remove.
        session_id: SessionId,
    },
    /// Clear or remove the source panel.
    ///
    /// Used when a panel from a split container is dropped elsewhere.
    /// The source panel should become empty or be removed.
    ClearPanel {
        /// The tab containing the source split container.
        source_tab_id: TabId,
        /// The panel that was the source of the drag.
        panel_id: PanelId,
    },
}

impl SourceCleanup {
    /// Returns true if cleanup is needed.
    #[must_use]
    pub const fn is_needed(&self) -> bool {
        !matches!(self, Self::None)
    }
}

/// Describes what action is needed for an evicted session.
///
/// When a session is dropped onto an occupied panel, the existing session
/// is evicted and needs to be placed somewhere else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvictionAction {
    /// No eviction occurred (panel was empty).
    None,
    /// Create a new root tab for the evicted session.
    ///
    /// Requirements 10.2, 10.3, 10.4: Evicted connection goes to new Root_Tab.
    CreateTab {
        /// The session that was evicted and needs a new tab.
        evicted_session: SessionId,
    },
}

impl EvictionAction {
    /// Returns true if an eviction occurred.
    #[must_use]
    pub const fn is_evicted(&self) -> bool {
        matches!(self, Self::CreateTab { .. })
    }

    /// Returns the evicted session ID, if any.
    #[must_use]
    pub const fn evicted_session(&self) -> Option<SessionId> {
        match self {
            Self::None => None,
            Self::CreateTab { evicted_session } => Some(*evicted_session),
        }
    }
}

/// The complete outcome of a drop operation.
///
/// This struct encapsulates all the information the UI layer needs to
/// properly handle a drop operation, including:
/// - The core model's `DropResult`
/// - What cleanup is needed at the source
/// - What action is needed for any evicted session
///
/// # Requirements
///
/// This type supports the following requirements:
/// - 9.1: Move connection to empty panel
/// - 9.2: Remove source tab from tab bar
/// - 9.3: Move connection from another split container
/// - 9.4: Create new connection from sidebar item
/// - 10.1: Move new connection into occupied panel
/// - 10.2: Evict existing connection to new root tab
/// - 10.3: Swap connections and evict displaced one
/// - 10.4: Create new connection and evict existing one
///
/// # Example
///
/// ```ignore
/// let outcome = adapter.handle_drop(panel_id, &source)?;
///
/// // Handle source cleanup
/// match outcome.source_cleanup {
///     SourceCleanup::RemoveTab { session_id } => {
///         tab_manager.close_tab(session_id);
///     }
///     SourceCleanup::ClearPanel { source_tab_id, panel_id } => {
///         // Clear the source panel in the other split container
///     }
///     SourceCleanup::None => {}
/// }
///
/// // Handle eviction
/// if let EvictionAction::CreateTab { evicted_session } = outcome.eviction {
///     tab_manager.create_tab_for_session(evicted_session);
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropOutcome {
    /// The result from the core model's `place_in_panel()`.
    pub drop_result: DropResult,
    /// What cleanup is needed at the drag source.
    pub source_cleanup: SourceCleanup,
    /// What action is needed for any evicted session.
    pub eviction: EvictionAction,
    /// The session that was placed in the panel.
    ///
    /// For `RootTab` and `SplitPane` sources, this is the existing session.
    /// For `SidebarItem` sources, this is the newly created session.
    pub placed_session: SessionId,
}

impl DropOutcome {
    /// Creates a new `DropOutcome` for a successful placement.
    #[must_use]
    pub const fn new(
        drop_result: DropResult,
        source_cleanup: SourceCleanup,
        eviction: EvictionAction,
        placed_session: SessionId,
    ) -> Self {
        Self {
            drop_result,
            source_cleanup,
            eviction,
            placed_session,
        }
    }

    /// Returns true if an eviction occurred.
    #[must_use]
    pub const fn is_evicted(&self) -> bool {
        self.eviction.is_evicted()
    }

    /// Returns true if source cleanup is needed.
    #[must_use]
    pub const fn needs_source_cleanup(&self) -> bool {
        self.source_cleanup.is_needed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_id_new_creates_unique_ids() {
        let id1 = ConnectionId::new();
        let id2 = ConnectionId::new();
        assert_ne!(id1, id2);
    }

    #[test]
    fn connection_id_from_uuid_roundtrip() {
        let uuid = Uuid::new_v4();
        let id = ConnectionId::from_uuid(uuid);
        assert_eq!(id.as_uuid(), uuid);
    }

    #[test]
    fn connection_id_display() {
        let uuid = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let id = ConnectionId::from_uuid(uuid);
        assert_eq!(id.to_string(), "550e8400-e29b-41d4-a716-446655440000");
    }

    #[test]
    fn drop_source_root_tab() {
        let session_id = SessionId::new();
        let source = DropSource::root_tab(session_id);

        assert!(source.is_root_tab());
        assert!(!source.is_split_pane());
        assert!(!source.is_sidebar_item());
        assert_eq!(source.session_id(), Some(session_id));
    }

    #[test]
    fn drop_source_split_pane() {
        let tab_id = TabId::new();
        let panel_id = PanelId::new();
        let session_id = SessionId::new();
        let source = DropSource::split_pane(tab_id, panel_id, session_id);

        assert!(!source.is_root_tab());
        assert!(source.is_split_pane());
        assert!(!source.is_sidebar_item());
        assert_eq!(source.session_id(), Some(session_id));
    }

    #[test]
    fn drop_source_sidebar_item() {
        let connection_id = ConnectionId::new();
        let source = DropSource::sidebar_item(connection_id);

        assert!(!source.is_root_tab());
        assert!(!source.is_split_pane());
        assert!(source.is_sidebar_item());
        assert_eq!(source.session_id(), None);
    }

    // ========================================================================
    // SourceCleanup Tests
    // ========================================================================

    #[test]
    fn source_cleanup_none_is_not_needed() {
        let cleanup = SourceCleanup::None;
        assert!(!cleanup.is_needed());
    }

    #[test]
    fn source_cleanup_remove_tab_is_needed() {
        let session_id = SessionId::new();
        let cleanup = SourceCleanup::RemoveTab { session_id };
        assert!(cleanup.is_needed());
    }

    #[test]
    fn source_cleanup_clear_panel_is_needed() {
        let tab_id = TabId::new();
        let panel_id = PanelId::new();
        let cleanup = SourceCleanup::ClearPanel {
            source_tab_id: tab_id,
            panel_id,
        };
        assert!(cleanup.is_needed());
    }

    // ========================================================================
    // EvictionAction Tests
    // ========================================================================

    #[test]
    fn eviction_action_none_is_not_evicted() {
        let action = EvictionAction::None;
        assert!(!action.is_evicted());
        assert!(action.evicted_session().is_none());
    }

    #[test]
    fn eviction_action_create_tab_is_evicted() {
        let session_id = SessionId::new();
        let action = EvictionAction::CreateTab {
            evicted_session: session_id,
        };
        assert!(action.is_evicted());
        assert_eq!(action.evicted_session(), Some(session_id));
    }

    // ========================================================================
    // DropOutcome Tests
    // ========================================================================

    #[test]
    fn drop_outcome_placed_no_eviction() {
        let session_id = SessionId::new();
        let outcome = DropOutcome::new(
            DropResult::Placed,
            SourceCleanup::RemoveTab { session_id },
            EvictionAction::None,
            session_id,
        );

        assert!(!outcome.is_evicted());
        assert!(outcome.needs_source_cleanup());
        assert_eq!(outcome.placed_session, session_id);
    }

    #[test]
    fn drop_outcome_evicted_with_cleanup() {
        let placed_session = SessionId::new();
        let evicted_session = SessionId::new();
        let outcome = DropOutcome::new(
            DropResult::Evicted { evicted_session },
            SourceCleanup::RemoveTab {
                session_id: placed_session,
            },
            EvictionAction::CreateTab { evicted_session },
            placed_session,
        );

        assert!(outcome.is_evicted());
        assert!(outcome.needs_source_cleanup());
        assert_eq!(outcome.placed_session, placed_session);
    }

    #[test]
    fn drop_outcome_sidebar_item_no_cleanup() {
        let placed_session = SessionId::new();
        let outcome = DropOutcome::new(
            DropResult::Placed,
            SourceCleanup::None,
            EvictionAction::None,
            placed_session,
        );

        assert!(!outcome.is_evicted());
        assert!(!outcome.needs_source_cleanup());
    }
}
