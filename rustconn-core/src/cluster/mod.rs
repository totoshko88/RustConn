//! Cluster management for `RustConn`
//!
//! This module provides cluster functionality for managing multiple connections
//! as a group, including broadcast mode for sending input to all sessions simultaneously.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// Errors related to cluster operations
#[derive(Debug, Error)]
pub enum ClusterError {
    /// Cluster not found
    #[error("Cluster not found: {0}")]
    NotFound(Uuid),

    /// Cluster already exists
    #[error("Cluster already exists: {0}")]
    AlreadyExists(String),

    /// Invalid cluster configuration
    #[error("Invalid cluster configuration: {0}")]
    InvalidConfig(String),

    /// Session error within cluster
    #[error("Session error for connection {connection_id}: {message}")]
    SessionError {
        /// The connection ID that failed
        connection_id: Uuid,
        /// Error message
        message: String,
    },

    /// No connections in cluster
    #[error("Cluster has no connections")]
    EmptyCluster,

    /// The auto-membership pattern is not a valid regular expression
    #[error("Invalid auto-membership pattern: {0}")]
    InvalidPattern(#[source] regex::Error),
}

/// A compiled cluster auto-membership rule.
///
/// The single definition of what an auto-membership pattern means, shared by
/// [`Cluster::resolve_members`], the GUI's live preview and its save-time
/// validation so the three cannot drift apart:
///
/// - surrounding whitespace is ignored, and a blank pattern is "no rule";
/// - the pattern is a [`regex`] crate expression, matched against the
///   connection's **name** and, separately, its **host** — either is enough;
/// - matching is **unanchored** (`web` matches `prod-web1`; use `^…$` to pin
///   it) and **case-sensitive** (prefix `(?i)` to ignore case), i.e. plain
///   `regex` semantics with nothing added.
#[derive(Debug, Clone)]
pub struct AutoMembership {
    re: regex::Regex,
}

impl AutoMembership {
    /// Compiles a pattern, returning `Ok(None)` when it is blank.
    ///
    /// # Errors
    /// Returns [`ClusterError::InvalidPattern`] when the trimmed pattern is not
    /// a valid regular expression.
    pub fn parse(pattern: &str) -> ClusterResult<Option<Self>> {
        let pattern = pattern.trim();
        if pattern.is_empty() {
            return Ok(None);
        }
        regex::Regex::new(pattern)
            .map(|re| Some(Self { re }))
            .map_err(ClusterError::InvalidPattern)
    }

    /// Returns `true` when the rule matches the connection name or host.
    #[must_use]
    pub fn matches(&self, name: &str, host: &str) -> bool {
        self.re.is_match(name) || self.re.is_match(host)
    }
}

/// Result type alias for cluster operations
pub type ClusterResult<T> = std::result::Result<T, ClusterError>;

/// Status of a session within a cluster
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ClusterSessionStatus {
    /// Session is pending connection
    #[default]
    Pending,
    /// Session is connecting
    Connecting,
    /// Session is active and connected
    Connected,
    /// Session has been disconnected
    Disconnected,
    /// Session encountered an error
    Error,
}

/// State of an individual session within a cluster
#[derive(Debug, Clone)]
pub struct ClusterMemberState {
    /// The connection ID
    pub connection_id: Uuid,
    /// Current status of this session
    pub status: ClusterSessionStatus,
    /// Error message if status is Error
    pub error_message: Option<String>,
}

impl ClusterMemberState {
    /// Creates a new cluster member state
    #[must_use]
    pub const fn new(connection_id: Uuid) -> Self {
        Self {
            connection_id,
            status: ClusterSessionStatus::Pending,
            error_message: None,
        }
    }

    /// Sets the status to connecting
    pub fn set_connecting(&mut self) {
        self.status = ClusterSessionStatus::Connecting;
        self.error_message = None;
    }

    /// Sets the status to connected
    pub fn set_connected(&mut self) {
        self.status = ClusterSessionStatus::Connected;
        self.error_message = None;
    }

    /// Sets the status to disconnected
    pub fn set_disconnected(&mut self) {
        self.status = ClusterSessionStatus::Disconnected;
        self.error_message = None;
    }

    /// Sets the status to error with a message
    pub fn set_error(&mut self, message: String) {
        self.status = ClusterSessionStatus::Error;
        self.error_message = Some(message);
    }

    /// Returns true if the session is in an active state (connecting or connected)
    #[must_use]
    pub const fn is_active(&self) -> bool {
        matches!(
            self.status,
            ClusterSessionStatus::Connecting | ClusterSessionStatus::Connected
        )
    }
}

/// A cluster of connections that can be managed together
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cluster {
    /// Unique identifier for this cluster
    pub id: Uuid,
    /// Display name for the cluster
    pub name: String,
    /// IDs of connections that belong to this cluster
    pub connection_ids: Vec<Uuid>,
    /// Whether broadcast mode is enabled by default
    pub broadcast_enabled: bool,
    /// Optional regular expression for automatic membership. When set, any
    /// connection whose name OR host matches is included in the cluster's
    /// resolved members in addition to the explicit `connection_ids` — so a rule
    /// like `^prod-web\d+` auto-collects every matching host for
    /// mass-connect/broadcast. `None` (the default) means explicit membership
    /// only. Read-widening: it never removes an explicit member and never
    /// mutates `connection_ids`. Matching semantics are [`AutoMembership`]'s.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_membership: Option<String>,
}

impl Cluster {
    /// Creates a new cluster with the given name
    #[must_use]
    pub fn new(name: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            connection_ids: Vec::new(),
            broadcast_enabled: false,
            auto_membership: None,
        }
    }

    /// Creates a new cluster with specific ID (for deserialization)
    #[must_use]
    pub const fn with_id(id: Uuid, name: String) -> Self {
        Self {
            id,
            name,
            connection_ids: Vec::new(),
            broadcast_enabled: false,
            auto_membership: None,
        }
    }

    /// Adds a connection to the cluster
    pub fn add_connection(&mut self, connection_id: Uuid) {
        if !self.connection_ids.contains(&connection_id) {
            self.connection_ids.push(connection_id);
        }
    }

    /// Removes a connection from the cluster
    pub fn remove_connection(&mut self, connection_id: Uuid) {
        self.connection_ids.retain(|id| *id != connection_id);
    }

    /// Returns true if the cluster contains the given connection
    #[must_use]
    pub fn contains_connection(&self, connection_id: Uuid) -> bool {
        self.connection_ids.contains(&connection_id)
    }

    /// Returns the number of connections in the cluster
    #[must_use]
    pub fn connection_count(&self) -> usize {
        self.connection_ids.len()
    }

    /// Returns true if the cluster has no connections
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.connection_ids.is_empty()
    }

    /// Resolves the cluster's effective membership: the explicit
    /// [`Self::connection_ids`] plus every connection whose name OR host matches
    /// the [`Self::auto_membership`] regex, de-duplicated and preserving order
    /// (explicit members first, then regex matches in `connections` order).
    ///
    /// This is **read-widening only** — it never mutates `connection_ids` and
    /// never drops an explicit member. An absent or invalid regex yields just the
    /// explicit members (an invalid pattern is logged and ignored rather than
    /// panicking, so a bad rule can never break a mass-connect). The regex is
    /// compiled once per call; matching follows [`AutoMembership`].
    ///
    /// Takes any iterator of borrowed connections, so a caller holding them in
    /// a map does not have to clone the whole list first.
    #[must_use]
    pub fn resolve_members<'a>(
        &self,
        connections: impl IntoIterator<Item = &'a crate::Connection>,
    ) -> Vec<Uuid> {
        let mut members = self.connection_ids.clone();

        let rule = match AutoMembership::parse(self.auto_membership.as_deref().unwrap_or("")) {
            Ok(Some(rule)) => rule,
            Ok(None) => return members,
            Err(e) => {
                tracing::warn!(
                    cluster = %self.name,
                    error = %e,
                    "cluster auto-membership regex is invalid; using explicit members only"
                );
                return members;
            }
        };

        let explicit: std::collections::HashSet<Uuid> =
            self.connection_ids.iter().copied().collect();
        for conn in connections {
            if explicit.contains(&conn.id) {
                continue; // already an explicit member, keep it once
            }
            if rule.matches(&conn.name, &conn.host) {
                members.push(conn.id);
            }
        }
        members
    }
}

/// An active cluster session managing multiple connection sessions
#[derive(Debug)]
pub struct ClusterSession {
    /// The cluster ID this session is for
    pub cluster_id: Uuid,
    /// The cluster name (for display)
    pub cluster_name: String,
    /// State of each member session, keyed by connection ID
    sessions: HashMap<Uuid, ClusterMemberState>,
    /// Whether broadcast mode is currently enabled
    broadcast_mode: bool,
}

impl ClusterSession {
    /// Creates a session tracking the cluster's explicit members only.
    ///
    /// Auto-membership matches need the connection list; use
    /// [`Self::with_members`] with [`Cluster::resolve_members`] for those, as
    /// [`ClusterManager::start_session`] does.
    #[must_use]
    pub fn new(cluster: &Cluster) -> Self {
        Self::with_members(cluster, &cluster.connection_ids)
    }

    /// Creates a session tracking exactly `members` (normally the cluster's
    /// resolved membership).
    #[must_use]
    pub fn with_members(cluster: &Cluster, members: &[Uuid]) -> Self {
        let sessions = members
            .iter()
            .map(|id| (*id, ClusterMemberState::new(*id)))
            .collect();

        Self {
            cluster_id: cluster.id,
            cluster_name: cluster.name.clone(),
            sessions,
            broadcast_mode: cluster.broadcast_enabled,
        }
    }

    /// Returns whether broadcast mode is enabled
    #[must_use]
    pub const fn is_broadcast_mode(&self) -> bool {
        self.broadcast_mode
    }

    /// Enables or disables broadcast mode
    pub const fn set_broadcast_mode(&mut self, enabled: bool) {
        self.broadcast_mode = enabled;
    }

    /// Toggles broadcast mode and returns the new state
    pub const fn toggle_broadcast_mode(&mut self) -> bool {
        self.broadcast_mode = !self.broadcast_mode;
        self.broadcast_mode
    }

    /// Gets the state of a specific session
    #[must_use]
    pub fn get_session_state(&self, connection_id: Uuid) -> Option<&ClusterMemberState> {
        self.sessions.get(&connection_id)
    }

    /// Gets a mutable reference to a session state
    pub fn get_session_state_mut(
        &mut self,
        connection_id: Uuid,
    ) -> Option<&mut ClusterMemberState> {
        self.sessions.get_mut(&connection_id)
    }

    /// Updates the status of a session
    pub fn update_session_status(&mut self, connection_id: Uuid, status: ClusterSessionStatus) {
        if let Some(state) = self.sessions.get_mut(&connection_id) {
            state.status = status;
            if status != ClusterSessionStatus::Error {
                state.error_message = None;
            }
        }
    }

    /// Sets a session to error state with a message
    pub fn set_session_error(&mut self, connection_id: Uuid, message: String) {
        if let Some(state) = self.sessions.get_mut(&connection_id) {
            state.set_error(message);
        }
    }

    /// Returns the status of all sessions
    #[must_use]
    pub fn get_all_statuses(&self) -> Vec<(Uuid, ClusterSessionStatus)> {
        self.sessions
            .iter()
            .map(|(id, state)| (*id, state.status))
            .collect()
    }

    /// Returns all session states
    #[must_use]
    pub const fn get_all_states(&self) -> &HashMap<Uuid, ClusterMemberState> {
        &self.sessions
    }

    /// Returns the number of sessions in the cluster
    #[must_use]
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Returns the number of connected sessions
    #[must_use]
    pub fn connected_count(&self) -> usize {
        self.sessions
            .values()
            .filter(|s| s.status == ClusterSessionStatus::Connected)
            .count()
    }

    /// Returns the number of sessions with errors
    #[must_use]
    pub fn error_count(&self) -> usize {
        self.sessions
            .values()
            .filter(|s| s.status == ClusterSessionStatus::Error)
            .count()
    }

    /// Returns true if all sessions are connected
    #[must_use]
    pub fn all_connected(&self) -> bool {
        !self.sessions.is_empty()
            && self
                .sessions
                .values()
                .all(|s| s.status == ClusterSessionStatus::Connected)
    }

    /// Returns true if any session is connected
    #[must_use]
    pub fn any_connected(&self) -> bool {
        self.sessions
            .values()
            .any(|s| s.status == ClusterSessionStatus::Connected)
    }

    /// Returns true if all sessions are disconnected or in error state
    #[must_use]
    pub fn all_inactive(&self) -> bool {
        self.sessions.values().all(|s| {
            matches!(
                s.status,
                ClusterSessionStatus::Disconnected | ClusterSessionStatus::Error
            )
        })
    }

    // The `broadcast_input` / `get_input_targets` pair that used to live here was
    // removed in 0.21.11. It computed the set of connected sessions that a typed
    // line should be mirrored to, from the days when a cluster owned the
    // broadcast. Since 0.14.8 broadcast is a split-view feature that mirrors VTE
    // `commit` events directly to the visible terminal panels
    // (`wire_broadcast_for_session` in the GUI), and nothing called this path —
    // the `broadcast_mode` flag survives only to feed `ClusterSessionSummary`.
    // Keeping the dead computation invited a future editor to wire it back in
    // parallel to the real one.

    /// Returns connection IDs of sessions that failed
    #[must_use]
    pub fn get_failed_sessions(&self) -> Vec<(Uuid, Option<String>)> {
        self.sessions
            .iter()
            .filter(|(_, state)| state.status == ClusterSessionStatus::Error)
            .map(|(id, state)| (*id, state.error_message.clone()))
            .collect()
    }

    /// Returns connection IDs of sessions that are still active
    #[must_use]
    pub fn get_active_sessions(&self) -> Vec<Uuid> {
        self.sessions
            .iter()
            .filter(|(_, state)| state.is_active())
            .map(|(id, _)| *id)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cluster_creation() {
        let cluster = Cluster::new("Test Cluster".to_string());
        assert!(!cluster.id.is_nil());
        assert_eq!(cluster.name, "Test Cluster");
        assert!(cluster.connection_ids.is_empty());
        assert!(!cluster.broadcast_enabled);
    }

    #[test]
    fn test_cluster_add_remove_connection() {
        let mut cluster = Cluster::new("Test".to_string());
        let conn_id = Uuid::new_v4();

        cluster.add_connection(conn_id);
        assert!(cluster.contains_connection(conn_id));
        assert_eq!(cluster.connection_count(), 1);

        // Adding same connection again should not duplicate
        cluster.add_connection(conn_id);
        assert_eq!(cluster.connection_count(), 1);

        cluster.remove_connection(conn_id);
        assert!(!cluster.contains_connection(conn_id));
        assert!(cluster.is_empty());
    }

    #[test]
    fn test_cluster_session_creation() {
        let mut cluster = Cluster::new("Test".to_string());
        let conn1 = Uuid::new_v4();
        let conn2 = Uuid::new_v4();
        cluster.add_connection(conn1);
        cluster.add_connection(conn2);

        let session = ClusterSession::new(&cluster);
        assert_eq!(session.cluster_id, cluster.id);
        assert_eq!(session.session_count(), 2);
        assert!(!session.is_broadcast_mode());

        // All sessions should start as Pending
        let state1 = session.get_session_state(conn1).unwrap();
        assert_eq!(state1.status, ClusterSessionStatus::Pending);
    }

    #[test]
    fn test_cluster_session_broadcast_mode() {
        let mut cluster = Cluster::new("Test".to_string());
        cluster.add_connection(Uuid::new_v4());
        cluster.broadcast_enabled = true;

        let mut session = ClusterSession::new(&cluster);
        assert!(session.is_broadcast_mode());

        session.set_broadcast_mode(false);
        assert!(!session.is_broadcast_mode());

        let new_state = session.toggle_broadcast_mode();
        assert!(new_state);
        assert!(session.is_broadcast_mode());
    }

    #[test]
    fn test_cluster_session_status_updates() {
        let mut cluster = Cluster::new("Test".to_string());
        let conn_id = Uuid::new_v4();
        cluster.add_connection(conn_id);

        let mut session = ClusterSession::new(&cluster);

        session.update_session_status(conn_id, ClusterSessionStatus::Connecting);
        assert_eq!(
            session.get_session_state(conn_id).unwrap().status,
            ClusterSessionStatus::Connecting
        );

        session.update_session_status(conn_id, ClusterSessionStatus::Connected);
        assert_eq!(session.connected_count(), 1);
        assert!(session.all_connected());

        session.set_session_error(conn_id, "Connection lost".to_string());
        assert_eq!(session.error_count(), 1);
        assert_eq!(
            session.get_session_state(conn_id).unwrap().error_message,
            Some("Connection lost".to_string())
        );
    }
}

/// Manager for active cluster sessions
#[derive(Debug, Default)]
pub struct ClusterManager {
    /// Active cluster sessions, keyed by cluster ID
    active_sessions: HashMap<Uuid, ClusterSession>,
    /// Stored cluster definitions
    clusters: HashMap<Uuid, Cluster>,
}

impl ClusterManager {
    /// Creates a new cluster manager
    #[must_use]
    pub fn new() -> Self {
        Self {
            active_sessions: HashMap::new(),
            clusters: HashMap::new(),
        }
    }

    /// Adds a cluster definition
    pub fn add_cluster(&mut self, cluster: Cluster) {
        self.clusters.insert(cluster.id, cluster);
    }

    /// Removes a cluster definition
    pub fn remove_cluster(&mut self, cluster_id: Uuid) -> Option<Cluster> {
        // Also remove any active session
        self.active_sessions.remove(&cluster_id);
        self.clusters.remove(&cluster_id)
    }

    /// Gets a cluster by ID
    #[must_use]
    pub fn get_cluster(&self, cluster_id: Uuid) -> Option<&Cluster> {
        self.clusters.get(&cluster_id)
    }

    /// Gets a mutable reference to a cluster
    pub fn get_cluster_mut(&mut self, cluster_id: Uuid) -> Option<&mut Cluster> {
        self.clusters.get_mut(&cluster_id)
    }

    /// Updates an existing cluster
    ///
    /// # Errors
    /// Returns an error if the cluster is not found
    pub fn update_cluster(&mut self, cluster_id: Uuid, updated: Cluster) -> ClusterResult<()> {
        if !self.clusters.contains_key(&cluster_id) {
            return Err(ClusterError::NotFound(cluster_id));
        }
        self.clusters.insert(cluster_id, updated);
        Ok(())
    }

    /// Returns all clusters
    #[must_use]
    pub fn get_all_clusters(&self) -> Vec<&Cluster> {
        self.clusters.values().collect()
    }

    /// Returns the number of clusters
    #[must_use]
    pub fn cluster_count(&self) -> usize {
        self.clusters.len()
    }

    /// Loads clusters from a vector (for persistence)
    pub fn load_clusters(&mut self, clusters: Vec<Cluster>) {
        self.clusters = clusters.into_iter().map(|c| (c.id, c)).collect();
    }

    /// Returns all clusters as a vector (for persistence)
    #[must_use]
    pub fn clusters_to_vec(&self) -> Vec<Cluster> {
        self.clusters.values().cloned().collect()
    }

    /// Starts a cluster session over the cluster's resolved membership.
    ///
    /// Members are [`Cluster::resolve_members`] against `connections`, so a
    /// cluster whose members all come from its auto-membership pattern starts
    /// a session like any other.
    ///
    /// # Errors
    /// Returns [`ClusterError::NotFound`] if the cluster does not exist, or
    /// [`ClusterError::EmptyCluster`] if it resolves to no members.
    ///
    /// # Panics
    /// This function will not panic as the session is inserted before retrieval.
    pub fn start_session<'a>(
        &mut self,
        cluster_id: Uuid,
        connections: impl IntoIterator<Item = &'a crate::Connection>,
    ) -> ClusterResult<&mut ClusterSession> {
        let cluster = self
            .clusters
            .get(&cluster_id)
            .ok_or(ClusterError::NotFound(cluster_id))?;

        let members = cluster.resolve_members(connections);
        if members.is_empty() {
            return Err(ClusterError::EmptyCluster);
        }

        let session = ClusterSession::with_members(cluster, &members);
        self.active_sessions.insert(cluster_id, session);

        // INVARIANT: we just inserted the session above, so get_mut always succeeds.
        Ok(self
            .active_sessions
            .get_mut(&cluster_id)
            .expect("session was just inserted"))
    }

    /// Gets an active cluster session
    #[must_use]
    pub fn get_session(&self, cluster_id: Uuid) -> Option<&ClusterSession> {
        self.active_sessions.get(&cluster_id)
    }

    /// Gets a mutable reference to an active cluster session
    pub fn get_session_mut(&mut self, cluster_id: Uuid) -> Option<&mut ClusterSession> {
        self.active_sessions.get_mut(&cluster_id)
    }

    /// Ends a cluster session
    pub fn end_session(&mut self, cluster_id: Uuid) -> Option<ClusterSession> {
        self.active_sessions.remove(&cluster_id)
    }

    /// Returns all active sessions
    #[must_use]
    pub fn get_active_sessions(&self) -> Vec<&ClusterSession> {
        self.active_sessions.values().collect()
    }

    /// Returns the number of active sessions
    #[must_use]
    pub fn active_session_count(&self) -> usize {
        self.active_sessions.len()
    }

    /// Handles a session failure within a cluster
    /// Returns true if there are still active sessions in the cluster
    pub fn handle_session_failure(
        &mut self,
        cluster_id: Uuid,
        connection_id: Uuid,
        error_message: String,
    ) -> bool {
        self.active_sessions
            .get_mut(&cluster_id)
            .is_some_and(|session| {
                session.set_session_error(connection_id, error_message);
                // Return true if there are still active sessions
                session.any_connected() || session.get_active_sessions().len() > 1
            })
    }

    /// Updates the status of a connection within a cluster session
    pub fn update_connection_status(
        &mut self,
        cluster_id: Uuid,
        connection_id: Uuid,
        status: ClusterSessionStatus,
    ) {
        if let Some(session) = self.active_sessions.get_mut(&cluster_id) {
            session.update_session_status(connection_id, status);
        }
    }

    // `get_broadcast_targets` was removed in 0.21.11 together with the
    // `ClusterSession` broadcast pair it delegated to — a legacy path with no
    // callers, superseded by split-view broadcast. See the note in
    // `ClusterSession`.

    /// Checks if a cluster session has any failures
    #[must_use]
    pub fn has_failures(&self, cluster_id: Uuid) -> bool {
        self.active_sessions
            .get(&cluster_id)
            .is_some_and(|s| s.error_count() > 0)
    }

    /// Gets the summary of a cluster session
    #[must_use]
    pub fn get_session_summary(&self, cluster_id: Uuid) -> Option<ClusterSessionSummary> {
        self.active_sessions
            .get(&cluster_id)
            .map(|session| ClusterSessionSummary {
                cluster_id,
                cluster_name: session.cluster_name.clone(),
                total_sessions: session.session_count(),
                connected_count: session.connected_count(),
                error_count: session.error_count(),
                broadcast_mode: session.is_broadcast_mode(),
            })
    }
}

/// Summary of a cluster session's state
#[derive(Debug, Clone)]
pub struct ClusterSessionSummary {
    /// The cluster ID
    pub cluster_id: Uuid,
    /// The cluster name
    pub cluster_name: String,
    /// Total number of sessions
    pub total_sessions: usize,
    /// Number of connected sessions
    pub connected_count: usize,
    /// Number of sessions with errors
    pub error_count: usize,
    /// Whether broadcast mode is enabled
    pub broadcast_mode: bool,
}

#[cfg(test)]
mod manager_tests {
    use super::*;

    #[test]
    fn test_cluster_manager_creation() {
        let manager = ClusterManager::new();
        assert_eq!(manager.active_session_count(), 0);
        assert!(manager.get_all_clusters().is_empty());
    }

    #[test]
    fn test_cluster_manager_add_remove_cluster() {
        let mut manager = ClusterManager::new();
        let cluster = Cluster::new("Test".to_string());
        let cluster_id = cluster.id;

        manager.add_cluster(cluster);
        assert!(manager.get_cluster(cluster_id).is_some());

        let removed = manager.remove_cluster(cluster_id);
        assert!(removed.is_some());
        assert!(manager.get_cluster(cluster_id).is_none());
    }

    #[test]
    fn test_cluster_manager_start_session() {
        let mut manager = ClusterManager::new();
        let mut cluster = Cluster::new("Test".to_string());
        let conn_id = Uuid::new_v4();
        cluster.add_connection(conn_id);
        let cluster_id = cluster.id;

        manager.add_cluster(cluster);

        let session = manager
            .start_session(cluster_id, std::iter::empty())
            .unwrap();
        assert_eq!(session.session_count(), 1);
        assert_eq!(manager.active_session_count(), 1);
    }

    #[test]
    fn test_cluster_manager_empty_cluster_error() {
        let mut manager = ClusterManager::new();
        let cluster = Cluster::new("Empty".to_string());
        let cluster_id = cluster.id;

        manager.add_cluster(cluster);

        let result = manager.start_session(cluster_id, std::iter::empty());
        assert!(matches!(result, Err(ClusterError::EmptyCluster)));
    }

    #[test]
    fn test_cluster_manager_handle_failure() {
        let mut manager = ClusterManager::new();
        let mut cluster = Cluster::new("Test".to_string());
        let conn1 = Uuid::new_v4();
        let conn2 = Uuid::new_v4();
        cluster.add_connection(conn1);
        cluster.add_connection(conn2);
        let cluster_id = cluster.id;

        manager.add_cluster(cluster);
        manager
            .start_session(cluster_id, std::iter::empty())
            .unwrap();

        // Connect both sessions
        manager.update_connection_status(cluster_id, conn1, ClusterSessionStatus::Connected);
        manager.update_connection_status(cluster_id, conn2, ClusterSessionStatus::Connected);

        // Fail one session - should still have active sessions
        let has_active =
            manager.handle_session_failure(cluster_id, conn1, "Connection lost".to_string());
        assert!(has_active);
        assert!(manager.has_failures(cluster_id));

        // Verify the other session is still connected
        let session = manager.get_session(cluster_id).unwrap();
        assert_eq!(session.connected_count(), 1);
        assert_eq!(session.error_count(), 1);
    }

    #[test]
    fn test_cluster_manager_session_summary() {
        let mut manager = ClusterManager::new();
        let mut cluster = Cluster::new("Test Cluster".to_string());
        cluster.add_connection(Uuid::new_v4());
        cluster.add_connection(Uuid::new_v4());
        cluster.broadcast_enabled = true;
        let cluster_id = cluster.id;

        manager.add_cluster(cluster);
        manager
            .start_session(cluster_id, std::iter::empty())
            .unwrap();

        let summary = manager.get_session_summary(cluster_id).unwrap();
        assert_eq!(summary.cluster_name, "Test Cluster");
        assert_eq!(summary.total_sessions, 2);
        assert_eq!(summary.connected_count, 0);
        assert!(summary.broadcast_mode);
    }

    #[test]
    fn resolve_members_without_pattern_returns_explicit_only() {
        let mut cluster = Cluster::new("c".to_string());
        let a = Uuid::new_v4();
        cluster.add_connection(a);
        let conns = vec![crate::Connection::new_ssh(
            "prod-web1".to_string(),
            "prod-web1.example.com".to_string(),
            22,
        )];
        // No auto_membership -> only the explicit member, regardless of conns.
        assert_eq!(cluster.resolve_members(&conns), vec![a]);
    }

    #[test]
    fn resolve_members_matches_name_or_host_by_regex() {
        let mut cluster = Cluster::new("c".to_string());
        cluster.auto_membership = Some(r"^prod-web\d+".to_string());
        let web1 = crate::Connection::new_ssh("prod-web1".into(), "10.0.0.1".into(), 22);
        let web2 = crate::Connection::new_ssh("box".into(), "prod-web2".into(), 22);
        let db = crate::Connection::new_ssh("prod-db1".into(), "10.0.0.9".into(), 22);
        let conns = vec![web1.clone(), web2.clone(), db];
        let members = cluster.resolve_members(&conns);
        // web1 (name match) and web2 (host match) included; db excluded.
        assert!(members.contains(&web1.id));
        assert!(members.contains(&web2.id));
        assert_eq!(members.len(), 2);
    }

    #[test]
    fn resolve_members_dedups_explicit_and_matched() {
        let mut cluster = Cluster::new("c".to_string());
        let web1 = crate::Connection::new_ssh("prod-web1".into(), "10.0.0.1".into(), 22);
        cluster.add_connection(web1.id); // explicit AND would match the regex
        cluster.auto_membership = Some(r"^prod-web".to_string());
        let members = cluster.resolve_members(std::slice::from_ref(&web1));
        assert_eq!(
            members,
            vec![web1.id],
            "explicit member must not be duplicated"
        );
    }

    #[test]
    fn resolve_members_invalid_regex_falls_back_to_explicit() {
        let mut cluster = Cluster::new("c".to_string());
        let a = Uuid::new_v4();
        cluster.add_connection(a);
        cluster.auto_membership = Some("[invalid(".to_string()); // unbalanced
        let conns = vec![crate::Connection::new_ssh(
            "prod-web1".into(),
            "h".into(),
            22,
        )];
        // Invalid regex must not panic and must not add matches.
        assert_eq!(cluster.resolve_members(&conns), vec![a]);
    }

    #[test]
    fn auto_only_cluster_starts_a_session_over_its_matches() {
        // Zero explicit members, everything from the pattern: the session must
        // track the matched connections instead of failing as EmptyCluster.
        let web1 = crate::Connection::new_ssh("prod-web1".into(), "10.0.0.1".into(), 22);
        let web2 = crate::Connection::new_ssh("prod-web2".into(), "10.0.0.2".into(), 22);
        let db = crate::Connection::new_ssh("prod-db1".into(), "10.0.0.9".into(), 22);
        let mut cluster = Cluster::new("web".to_string());
        cluster.auto_membership = Some(r"^prod-web".to_string());
        let cluster_id = cluster.id;

        let mut manager = ClusterManager::new();
        manager.add_cluster(cluster);
        let conns = [web1.clone(), web2.clone(), db.clone()];
        let session = manager.start_session(cluster_id, &conns).unwrap();
        assert_eq!(session.session_count(), 2);
        assert!(session.get_session_state(web1.id).is_some());
        assert!(session.get_session_state(web2.id).is_some());
        assert!(session.get_session_state(db.id).is_none());
    }

    #[test]
    fn auto_only_cluster_matching_nothing_is_empty() {
        let mut cluster = Cluster::new("web".to_string());
        cluster.auto_membership = Some(r"^nothing-matches$".to_string());
        let cluster_id = cluster.id;
        let mut manager = ClusterManager::new();
        manager.add_cluster(cluster);
        let conns = [crate::Connection::new_ssh(
            "prod-web1".into(),
            "h".into(),
            22,
        )];
        assert!(matches!(
            manager.start_session(cluster_id, &conns),
            Err(ClusterError::EmptyCluster)
        ));
    }

    #[test]
    fn auto_membership_pattern_is_trimmed() {
        // The editor saves the trimmed text; the matcher must agree with it so
        // the live preview and the connect-time result are the same set.
        let rule = AutoMembership::parse("  ^prod-web  ").unwrap().unwrap();
        assert!(rule.matches("prod-web1", "10.0.0.1"));
        assert!(AutoMembership::parse("   ").unwrap().is_none());
        assert!(AutoMembership::parse("").unwrap().is_none());
    }

    #[test]
    fn auto_membership_is_unanchored_and_case_sensitive() {
        let rule = AutoMembership::parse("web").unwrap().unwrap();
        assert!(
            rule.matches("prod-web1", "x"),
            "unanchored: substring match"
        );
        assert!(!rule.matches("PROD-WEB1", "X"), "case-sensitive by default");
        let ci = AutoMembership::parse("(?i)web").unwrap().unwrap();
        assert!(ci.matches("PROD-WEB1", "X"), "(?i) opts into ignoring case");
    }

    #[test]
    fn auto_membership_invalid_pattern_is_an_error() {
        assert!(matches!(
            AutoMembership::parse("[invalid("),
            Err(ClusterError::InvalidPattern(_))
        ));
    }

    #[test]
    fn auto_membership_absent_not_serialized() {
        let cluster = Cluster::new("c".to_string());
        let json = serde_json::to_string(&cluster).unwrap();
        assert!(!json.contains("auto_membership"));
    }

    #[test]
    fn cluster_without_auto_membership_field_still_loads() {
        // Backward compat: a cluster written before the field existed.
        let json = r#"{"id":"00000000-0000-0000-0000-000000000000","name":"c","connection_ids":[],"broadcast_enabled":false}"#;
        let cluster: Cluster = serde_json::from_str(json).unwrap();
        assert!(cluster.auto_membership.is_none());
    }
}
