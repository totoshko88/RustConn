//! Credential verification tracking
//!
//! This module provides functionality to track whether credentials have been
//! successfully used for authentication. This allows the application to skip
//! the password dialog for connections with verified credentials.
//!
//! # Requirements Coverage
//!
//! - Requirement 2.1: Skip dialog for verified credentials
//! - Requirement 2.3: Mark credentials as requiring verification on auth failure
//! - Requirement 2.5: Mark credentials as verified after successful auth

use chrono::{DateTime, Utc};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Status of credential verification for a connection
///
/// Tracks whether credentials have been successfully used and when
/// they were last verified.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct CredentialStatus {
    /// Whether the credentials have been verified (used successfully)
    pub verified: bool,
    /// Timestamp of last successful verification
    pub verified_at: Option<DateTime<Utc>>,
    /// Timestamp of last failed verification attempt
    pub failed_at: Option<DateTime<Utc>>,
    /// Number of consecutive failures since last success
    pub failure_count: u32,
    /// Error message from last failure (if any)
    pub last_error: Option<String>,
}

impl CredentialStatus {
    /// Creates a new unverified credential status
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a verified credential status with current timestamp
    #[must_use]
    pub fn verified() -> Self {
        Self {
            verified: true,
            verified_at: Some(Utc::now()),
            failed_at: None,
            failure_count: 0,
            last_error: None,
        }
    }

    /// Marks credentials as verified (successful authentication)
    ///
    /// This resets the failure count and updates the verification timestamp.
    pub fn mark_verified(&mut self) {
        self.verified = true;
        self.verified_at = Some(Utc::now());
        self.failure_count = 0;
        self.last_error = None;
    }

    /// Marks credentials as unverified (failed authentication)
    ///
    /// This increments the failure count and records the error.
    ///
    /// # Arguments
    /// * `error` - Optional error message describing the failure
    pub fn mark_unverified(&mut self, error: Option<String>) {
        self.verified = false;
        self.failed_at = Some(Utc::now());
        self.failure_count = self.failure_count.saturating_add(1);
        self.last_error = error;
    }

    /// Checks if credentials are verified and can be used automatically
    #[must_use]
    pub const fn is_verified(&self) -> bool {
        self.verified
    }

    /// Checks if credentials require re-verification (dialog should be shown)
    #[must_use]
    pub const fn requires_verification(&self) -> bool {
        !self.verified
    }

    /// Returns the number of consecutive failures
    #[must_use]
    pub const fn failure_count(&self) -> u32 {
        self.failure_count
    }

    /// Checks if there have been recent failures
    #[must_use]
    pub const fn has_failures(&self) -> bool {
        self.failure_count > 0
    }
}

/// Verified credentials with status information
///
/// This struct wraps credentials with their verification status,
/// allowing the caller to make decisions about whether to use
/// them automatically or prompt the user.
#[derive(Debug, Clone)]
pub struct VerifiedCredentials {
    /// The username (if available)
    pub username: Option<String>,
    /// The password (if available) - stored securely
    pub password: Option<SecretString>,
    /// The domain (for RDP connections)
    pub domain: Option<String>,
    /// Verification status
    pub status: CredentialStatus,
}

impl VerifiedCredentials {
    /// Creates new verified credentials
    #[must_use]
    pub const fn new(
        username: Option<String>,
        password: Option<SecretString>,
        domain: Option<String>,
        status: CredentialStatus,
    ) -> Self {
        Self {
            username,
            password,
            domain,
            status,
        }
    }

    /// Creates verified credentials from a successful lookup
    #[must_use]
    pub fn from_verified(
        username: Option<String>,
        password: Option<SecretString>,
        domain: Option<String>,
    ) -> Self {
        Self {
            username,
            password,
            domain,
            status: CredentialStatus::verified(),
        }
    }

    /// Creates verified credentials from string password (convenience method)
    #[must_use]
    pub fn from_verified_str(
        username: Option<String>,
        password: Option<String>,
        domain: Option<String>,
    ) -> Self {
        Self {
            username,
            password: password.map(SecretString::from),
            domain,
            status: CredentialStatus::verified(),
        }
    }

    /// Creates unverified credentials (need to prompt user)
    #[must_use]
    pub fn unverified() -> Self {
        Self {
            username: None,
            password: None,
            domain: None,
            status: CredentialStatus::new(),
        }
    }

    /// Checks if these credentials can be used without prompting
    #[must_use]
    pub const fn can_use_automatically(&self) -> bool {
        self.status.is_verified() && self.password.is_some()
    }

    /// Checks if the password dialog should be shown
    #[must_use]
    pub const fn should_show_dialog(&self) -> bool {
        !self.can_use_automatically()
    }

    /// Returns true if credentials have a password
    #[must_use]
    pub const fn has_password(&self) -> bool {
        self.password.is_some()
    }

    /// Exposes the password for use (should be used carefully)
    #[must_use]
    pub fn expose_password(&self) -> Option<&str> {
        self.password
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret)
    }
}

/// Data for pre-filling the password dialog
///
/// This struct represents the values that should be pre-filled in the
/// password dialog based on connection settings.
///
/// # Requirements Coverage
/// - Requirement 2.4: Pre-fill username and domain from saved connection settings
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DialogPreFillData {
    /// Username to pre-fill (from connection settings)
    pub username: Option<String>,
    /// Domain to pre-fill (from connection settings)
    pub domain: Option<String>,
    /// Connection name for dialog title
    pub connection_name: Option<String>,
    /// Whether to show the "Save to `KeePass`" migration button
    pub show_migrate_button: bool,
}

impl DialogPreFillData {
    /// Creates new pre-fill data
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates pre-fill data from connection settings
    ///
    /// # Arguments
    /// * `username` - Optional username from connection
    /// * `domain` - Optional domain from connection
    /// * `connection_name` - Name of the connection for dialog title
    #[must_use]
    pub const fn from_connection(
        username: Option<String>,
        domain: Option<String>,
        connection_name: String,
    ) -> Self {
        Self {
            username,
            domain,
            connection_name: Some(connection_name),
            show_migrate_button: false,
        }
    }

    /// Creates pre-fill data from verified credentials
    ///
    /// This is used when credentials have been resolved but the dialog
    /// still needs to be shown (e.g., credentials not verified or no password).
    ///
    /// # Arguments
    /// * `verified_creds` - The resolved credentials with verification status
    /// * `connection_name` - Name of the connection for dialog title
    #[must_use]
    pub fn from_verified_credentials(
        verified_creds: &VerifiedCredentials,
        connection_name: String,
    ) -> Self {
        Self {
            username: verified_creds.username.clone(),
            domain: verified_creds.domain.clone(),
            connection_name: Some(connection_name),
            show_migrate_button: false,
        }
    }

    /// Sets whether to show the migration button
    #[must_use]
    pub const fn with_migrate_button(mut self, show: bool) -> Self {
        self.show_migrate_button = show;
        self
    }

    /// Returns true if username should be pre-filled
    #[must_use]
    pub fn has_username(&self) -> bool {
        self.username.as_ref().is_some_and(|u| !u.is_empty())
    }

    /// Returns true if domain should be pre-filled
    #[must_use]
    pub fn has_domain(&self) -> bool {
        self.domain.as_ref().is_some_and(|d| !d.is_empty())
    }

    /// Returns true if any field should be pre-filled
    #[must_use]
    pub fn has_prefill_data(&self) -> bool {
        self.has_username() || self.has_domain()
    }
}

/// Manager for credential verification status
///
/// Stores and retrieves verification status for connections,
/// persisted to a JSON file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CredentialVerificationManager {
    /// Map of connection ID to verification status
    statuses: HashMap<Uuid, CredentialStatus>,
}

impl CredentialVerificationManager {
    /// Creates a new empty manager
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Gets the verification status for a connection
    ///
    /// Returns the stored status or a default unverified status.
    #[must_use]
    pub fn get_status(&self, connection_id: Uuid) -> CredentialStatus {
        self.statuses
            .get(&connection_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Sets the verification status for a connection
    pub fn set_status(&mut self, connection_id: Uuid, status: CredentialStatus) {
        self.statuses.insert(connection_id, status);
    }

    /// Marks credentials as verified for a connection
    pub fn mark_verified(&mut self, connection_id: Uuid) {
        let status = self.statuses.entry(connection_id).or_default();
        status.mark_verified();
    }

    /// Marks credentials as unverified for a connection
    ///
    /// # Arguments
    /// * `connection_id` - The connection ID
    /// * `error` - Optional error message
    pub fn mark_unverified(&mut self, connection_id: Uuid, error: Option<String>) {
        let status = self.statuses.entry(connection_id).or_default();
        status.mark_unverified(error);
    }

    /// Checks if credentials are verified for a connection
    #[must_use]
    pub fn is_verified(&self, connection_id: Uuid) -> bool {
        self.statuses
            .get(&connection_id)
            .is_some_and(CredentialStatus::is_verified)
    }

    /// Removes verification status for a connection
    ///
    /// Called when a connection is deleted.
    pub fn remove(&mut self, connection_id: Uuid) {
        self.statuses.remove(&connection_id);
    }

    /// Clears all verification statuses
    pub fn clear(&mut self) {
        self.statuses.clear();
    }

    /// Returns the number of tracked connections
    #[must_use]
    pub fn len(&self) -> usize {
        self.statuses.len()
    }

    /// Checks if there are no tracked connections
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.statuses.is_empty()
    }

    /// Returns all connection IDs with verified credentials
    #[must_use]
    pub fn verified_connections(&self) -> Vec<Uuid> {
        self.statuses
            .iter()
            .filter(|(_, s)| s.is_verified())
            .map(|(id, _)| *id)
            .collect()
    }

    /// Returns all connection IDs with failed credentials
    #[must_use]
    pub fn failed_connections(&self) -> Vec<Uuid> {
        self.statuses
            .iter()
            .filter(|(_, s)| s.has_failures())
            .map(|(id, _)| *id)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_credential_status_default() {
        let status = CredentialStatus::default();
        assert!(!status.verified);
        assert!(status.verified_at.is_none());
        assert!(status.failed_at.is_none());
        assert_eq!(status.failure_count, 0);
        assert!(status.last_error.is_none());
    }

    #[test]
    fn test_credential_status_verified() {
        let status = CredentialStatus::verified();
        assert!(status.verified);
        assert!(status.verified_at.is_some());
        assert_eq!(status.failure_count, 0);
    }

    #[test]
    fn test_mark_verified() {
        let mut status = CredentialStatus::new();
        status.mark_unverified(Some("test error".to_string()));
        assert!(!status.verified);
        assert_eq!(status.failure_count, 1);

        status.mark_verified();
        assert!(status.verified);
        assert_eq!(status.failure_count, 0);
        assert!(status.last_error.is_none());
    }

    #[test]
    fn test_mark_unverified() {
        let mut status = CredentialStatus::verified();
        status.mark_unverified(Some("auth failed".to_string()));

        assert!(!status.verified);
        assert!(status.failed_at.is_some());
        assert_eq!(status.failure_count, 1);
        assert_eq!(status.last_error, Some("auth failed".to_string()));
    }

    #[test]
    fn test_failure_count_increments() {
        let mut status = CredentialStatus::new();
        status.mark_unverified(None);
        assert_eq!(status.failure_count, 1);
        status.mark_unverified(None);
        assert_eq!(status.failure_count, 2);
        status.mark_unverified(None);
        assert_eq!(status.failure_count, 3);
    }

    #[test]
    fn test_verified_credentials_can_use_automatically() {
        let creds = VerifiedCredentials::from_verified_str(
            Some("user".to_string()),
            Some("pass".to_string()),
            None,
        );
        assert!(creds.can_use_automatically());
        assert!(!creds.should_show_dialog());
    }

    #[test]
    fn test_verified_credentials_without_password() {
        let creds = VerifiedCredentials::from_verified_str(Some("user".to_string()), None, None);
        assert!(!creds.can_use_automatically());
        assert!(creds.should_show_dialog());
    }

    #[test]
    fn test_unverified_credentials() {
        let creds = VerifiedCredentials::unverified();
        assert!(!creds.can_use_automatically());
        assert!(creds.should_show_dialog());
    }

    #[test]
    fn test_manager_mark_verified() {
        let mut manager = CredentialVerificationManager::new();
        let id = Uuid::new_v4();

        assert!(!manager.is_verified(id));

        manager.mark_verified(id);
        assert!(manager.is_verified(id));
    }

    #[test]
    fn test_manager_mark_unverified() {
        let mut manager = CredentialVerificationManager::new();
        let id = Uuid::new_v4();

        manager.mark_verified(id);
        assert!(manager.is_verified(id));

        manager.mark_unverified(id, Some("error".to_string()));
        assert!(!manager.is_verified(id));

        let status = manager.get_status(id);
        assert_eq!(status.failure_count, 1);
        assert_eq!(status.last_error, Some("error".to_string()));
    }

    #[test]
    fn test_manager_remove() {
        let mut manager = CredentialVerificationManager::new();
        let id = Uuid::new_v4();

        manager.mark_verified(id);
        assert_eq!(manager.len(), 1);

        manager.remove(id);
        assert_eq!(manager.len(), 0);
        assert!(!manager.is_verified(id));
    }

    #[test]
    fn test_manager_verified_connections() {
        let mut manager = CredentialVerificationManager::new();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let id3 = Uuid::new_v4();

        manager.mark_verified(id1);
        manager.mark_verified(id2);
        manager.mark_unverified(id3, None);

        let verified = manager.verified_connections();
        assert_eq!(verified.len(), 2);
        assert!(verified.contains(&id1));
        assert!(verified.contains(&id2));
        assert!(!verified.contains(&id3));
    }

    #[test]
    fn test_manager_failed_connections() {
        let mut manager = CredentialVerificationManager::new();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();

        manager.mark_verified(id1);
        manager.mark_unverified(id2, None);

        let failed = manager.failed_connections();
        assert_eq!(failed.len(), 1);
        assert!(failed.contains(&id2));
    }

    #[test]
    fn test_credential_status_serialization() {
        let mut status = CredentialStatus::verified();
        status.mark_unverified(Some("test".to_string()));

        let json = serde_json::to_string(&status).unwrap();
        let deserialized: CredentialStatus = serde_json::from_str(&json).unwrap();

        assert_eq!(status.verified, deserialized.verified);
        assert_eq!(status.failure_count, deserialized.failure_count);
        assert_eq!(status.last_error, deserialized.last_error);
    }

    #[test]
    fn test_manager_serialization() {
        let mut manager = CredentialVerificationManager::new();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();

        manager.mark_verified(id1);
        manager.mark_unverified(id2, Some("error".to_string()));

        let json = serde_json::to_string(&manager).unwrap();
        let deserialized: CredentialVerificationManager = serde_json::from_str(&json).unwrap();

        assert!(deserialized.is_verified(id1));
        assert!(!deserialized.is_verified(id2));
        assert_eq!(deserialized.get_status(id2).failure_count, 1);
    }
}
