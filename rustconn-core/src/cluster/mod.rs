//! Cluster management for `RustConn`
//!
//! This module provides cluster functionality for managing multiple connections
//! as a group, including broadcast mode for sending input to all sessions simultaneously.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
    /// Creates a new cluster session
    #[must_use]
    pub fn new(cluster: &Cluster) -> Self {
        let sessions = cluster
            .connection_ids
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

    /// Queues input to be broadcast to all sessions
    /// Returns the list of connection IDs that should receive the input
    #[must_use]
    pub fn broadcast_input(&self, _input: &str) -> Vec<Uuid> {
        if !self.broadcast_mode {
            return Vec::new();
        }

        // Return IDs of all connected sessions
        self.sessions
            .iter()
            .filter(|(_, state)| state.status == ClusterSessionStatus::Connected)
            .map(|(id, _)| *id)
            .collect()
    }

    /// Returns the IDs of all sessions that should receive input
    /// In broadcast mode, returns all connected sessions
    /// Otherwise, returns an empty vec (caller should handle single session focus)
    #[must_use]
    pub fn get_input_targets(&self) -> Vec<Uuid> {
        if self.broadcast_mode {
            self.broadcast_input("")
        } else {
            Vec::new()
        }
    }

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

    #[test]
    fn test_cluster_session_broadcast_input() {
        let mut cluster = Cluster::new("Test".to_string());
        let conn1 = Uuid::new_v4();
        let conn2 = Uuid::new_v4();
        cluster.add_connection(conn1);
        cluster.add_connection(conn2);

        let mut session = ClusterSession::new(&cluster);

        // Without broadcast mode, no targets
        let targets = session.broadcast_input("test");
        assert!(targets.is_empty());

        // Enable broadcast mode
        session.set_broadcast_mode(true);

        // Still no targets because sessions aren't connected
        let targets = session.broadcast_input("test");
        assert!(targets.is_empty());

        // Connect one session
        session.update_session_status(conn1, ClusterSessionStatus::Connected);
        let targets = session.broadcast_input("test");
        assert_eq!(targets.len(), 1);
        assert!(targets.contains(&conn1));

        // Connect second session
        session.update_session_status(conn2, ClusterSessionStatus::Connected);
        let targets = session.broadcast_input("test");
        assert_eq!(targets.len(), 2);
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

    /// Starts a cluster session
    ///
    /// # Errors
    /// Returns an error if the cluster is not found or is empty
    ///
    /// # Panics
    /// This function will not panic as the session is inserted before retrieval.
    pub fn start_session(&mut self, cluster_id: Uuid) -> ClusterResult<&mut ClusterSession> {
        let cluster = self
            .clusters
            .get(&cluster_id)
            .ok_or(ClusterError::NotFound(cluster_id))?;

        if cluster.is_empty() {
            return Err(ClusterError::EmptyCluster);
        }

        let session = ClusterSession::new(cluster);
        self.active_sessions.insert(cluster_id, session);

        // Safe to unwrap: we just inserted the session above
        Ok(self.active_sessions.get_mut(&cluster_id).unwrap())
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

    /// Gets the broadcast targets for a cluster (if in broadcast mode)
    #[must_use]
    pub fn get_broadcast_targets(&self, cluster_id: Uuid) -> Vec<Uuid> {
        self.active_sessions
            .get(&cluster_id)
            .map(ClusterSession::get_input_targets)
            .unwrap_or_default()
    }

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

        let session = manager.start_session(cluster_id).unwrap();
        assert_eq!(session.session_count(), 1);
        assert_eq!(manager.active_session_count(), 1);
    }

    #[test]
    fn test_cluster_manager_empty_cluster_error() {
        let mut manager = ClusterManager::new();
        let cluster = Cluster::new("Empty".to_string());
        let cluster_id = cluster.id;

        manager.add_cluster(cluster);

        let result = manager.start_session(cluster_id);
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
        manager.start_session(cluster_id).unwrap();

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
        manager.start_session(cluster_id).unwrap();

        let summary = manager.get_session_summary(cluster_id).unwrap();
        assert_eq!(summary.cluster_name, "Test Cluster");
        assert_eq!(summary.total_sessions, 2);
        assert_eq!(summary.connected_count, 0);
        assert!(summary.broadcast_mode);
    }
}
