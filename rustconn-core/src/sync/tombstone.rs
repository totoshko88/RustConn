//! Tombstone model for Simple Sync deletion tracking.
//!
//! When an entity is deleted on one device, a [`Tombstone`] record is created
//! so that the deletion propagates to other devices during bidirectional merge.
//! Tombstones are cleaned up after `SyncSettings::tombstone_retention_days`.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Type of entity that was deleted, used in tombstone records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncEntityType {
    /// A connection was deleted.
    Connection,
    /// A connection group was deleted.
    Group,
    /// A connection template was deleted.
    Template,
    /// A snippet was deleted.
    Snippet,
    /// A cluster was deleted.
    Cluster,
    /// A variable was deleted.
    Variable,
}

/// A deletion record for Simple Sync bidirectional merge.
///
/// When entity E is deleted locally, a tombstone `(entity_type, id, deleted_at)`
/// is created. During merge, if `tombstone.deleted_at > remote_entity.updated_at`,
/// the remote entity is also deleted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tombstone {
    /// The type of entity that was deleted.
    pub entity_type: SyncEntityType,
    /// UUID of the deleted entity.
    pub id: Uuid,
    /// When the entity was deleted.
    pub deleted_at: DateTime<Utc>,
}

impl Tombstone {
    /// Creates a new tombstone for the given entity.
    #[must_use]
    pub fn new(entity_type: SyncEntityType, id: Uuid) -> Self {
        Self {
            entity_type,
            id,
            deleted_at: Utc::now(),
        }
    }

    /// Creates a tombstone with a specific deletion timestamp.
    #[must_use]
    pub fn with_deleted_at(
        entity_type: SyncEntityType,
        id: Uuid,
        deleted_at: DateTime<Utc>,
    ) -> Self {
        Self {
            entity_type,
            id,
            deleted_at,
        }
    }

    /// Returns `true` if this tombstone is older than `retention_days`.
    #[must_use]
    pub fn is_expired(&self, retention_days: u32, now: DateTime<Utc>) -> bool {
        let cutoff = now - Duration::days(i64::from(retention_days));
        self.deleted_at < cutoff
    }
}

/// Removes tombstones older than `retention_days` from the given list.
///
/// Returns the cleaned list with expired tombstones removed.
#[must_use]
pub fn cleanup_tombstones(tombstones: &[Tombstone], retention_days: u32) -> Vec<Tombstone> {
    let now = Utc::now();
    tombstones
        .iter()
        .filter(|t| !t.is_expired(retention_days, now))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tombstone_new_sets_current_time() {
        let id = Uuid::new_v4();
        let before = Utc::now();
        let tombstone = Tombstone::new(SyncEntityType::Connection, id);
        let after = Utc::now();

        assert_eq!(tombstone.entity_type, SyncEntityType::Connection);
        assert_eq!(tombstone.id, id);
        assert!(tombstone.deleted_at >= before);
        assert!(tombstone.deleted_at <= after);
    }

    #[test]
    fn tombstone_with_deleted_at() {
        let id = Uuid::new_v4();
        let ts = Utc::now() - Duration::hours(5);
        let tombstone = Tombstone::with_deleted_at(SyncEntityType::Group, id, ts);
        assert_eq!(tombstone.deleted_at, ts);
    }

    #[test]
    fn tombstone_is_expired() {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let old = now - Duration::days(31);
        let recent = now - Duration::days(5);

        let expired = Tombstone::with_deleted_at(SyncEntityType::Connection, id, old);
        let fresh = Tombstone::with_deleted_at(SyncEntityType::Connection, id, recent);

        assert!(expired.is_expired(30, now));
        assert!(!fresh.is_expired(30, now));
    }

    #[test]
    fn cleanup_tombstones_removes_expired() {
        let now = Utc::now();
        let tombstones = vec![
            Tombstone::with_deleted_at(
                SyncEntityType::Connection,
                Uuid::new_v4(),
                now - Duration::days(31),
            ),
            Tombstone::with_deleted_at(
                SyncEntityType::Group,
                Uuid::new_v4(),
                now - Duration::days(5),
            ),
            Tombstone::with_deleted_at(
                SyncEntityType::Snippet,
                Uuid::new_v4(),
                now - Duration::days(60),
            ),
        ];

        let cleaned = cleanup_tombstones(&tombstones, 30);
        assert_eq!(cleaned.len(), 1);
        assert_eq!(cleaned[0].entity_type, SyncEntityType::Group);
    }

    #[test]
    fn serialization_round_trip() {
        let tombstone = Tombstone::new(SyncEntityType::Template, Uuid::new_v4());
        let json = serde_json::to_string(&tombstone).unwrap();
        let deserialized: Tombstone = serde_json::from_str(&json).unwrap();
        assert_eq!(tombstone, deserialized);
    }

    #[test]
    fn entity_type_serializes_as_snake_case() {
        let json = serde_json::to_string(&SyncEntityType::Connection).unwrap();
        assert_eq!(json, "\"connection\"");
        let json = serde_json::to_string(&SyncEntityType::Group).unwrap();
        assert_eq!(json, "\"group\"");
    }
}
