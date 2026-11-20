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
}

/// Manager for stored cluster definitions
#[derive(Debug, Default)]
pub struct ClusterManager {
    /// Stored cluster definitions
    clusters: HashMap<Uuid, Cluster>,
}

impl ClusterManager {
    /// Creates a new cluster manager
    #[must_use]
    pub fn new() -> Self {
        Self {
            clusters: HashMap::new(),
        }
    }

    /// Adds a cluster definition
    pub fn add_cluster(&mut self, cluster: Cluster) {
        self.clusters.insert(cluster.id, cluster);
    }

    /// Removes a cluster definition
    pub fn remove_cluster(&mut self, cluster_id: Uuid) -> Option<Cluster> {
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
}

#[cfg(test)]
mod manager_tests {
    use super::*;

    #[test]
    fn test_cluster_manager_creation() {
        let manager = ClusterManager::new();
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
