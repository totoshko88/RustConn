//! Property-based tests for credential verification tracking
//!
//! These tests validate the correctness properties for credential verification
//! as defined in Requirements 2.1, 2.2, 2.3, 2.5.

use proptest::prelude::*;
use rustconn_core::{
    CredentialStatus, CredentialVerificationManager, DialogPreFillData, VerifiedCredentials,
};
use secrecy::SecretString;
use uuid::Uuid;

// ========== Generators ==========

/// Strategy for generating UUIDs
fn arb_uuid() -> impl Strategy<Value = Uuid> {
    any::<[u8; 16]>().prop_map(Uuid::from_bytes)
}

/// Strategy for generating optional error messages
fn arb_error_message() -> impl Strategy<Value = Option<String>> {
    prop_oneof![Just(None), "[a-zA-Z0-9 ]{1,50}".prop_map(Some),]
}

/// Strategy for generating optional usernames
fn arb_username() -> impl Strategy<Value = Option<String>> {
    prop_oneof![Just(None), "[a-zA-Z][a-zA-Z0-9_]{0,20}".prop_map(Some),]
}

/// Strategy for generating optional passwords
fn arb_password() -> impl Strategy<Value = Option<String>> {
    prop_oneof![Just(None), "[a-zA-Z0-9!@#$%^&*]{8,32}".prop_map(Some),]
}

/// Strategy for generating optional domains
fn arb_domain() -> impl Strategy<Value = Option<String>> {
    prop_oneof![Just(None), "[A-Z]{2,10}".prop_map(Some),]
}

// ========== Property Tests: CredentialStatus ==========

proptest! {
    /// Property: Default status is unverified
    #[test]
    fn default_status_is_unverified(_seed in any::<u64>()) {
        let status = CredentialStatus::default();
        prop_assert!(!status.is_verified());
        prop_assert!(status.requires_verification());
        prop_assert_eq!(status.failure_count(), 0);
    }

    /// Property: Verified status constructor creates verified status
    #[test]
    fn verified_constructor_creates_verified_status(_seed in any::<u64>()) {
        let status = CredentialStatus::verified();
        prop_assert!(status.is_verified());
        prop_assert!(!status.requires_verification());
        prop_assert!(status.verified_at.is_some());
    }

    /// Property: mark_verified sets verified to true and resets failures
    #[test]
    fn mark_verified_sets_verified_and_resets_failures(
        initial_failures in 0u32..100,
        error in arb_error_message(),
    ) {
        let mut status = CredentialStatus::new();

        // Add some failures
        for _ in 0..initial_failures {
            status.mark_unverified(error.clone());
        }

        // Mark as verified
        status.mark_verified();

        prop_assert!(status.is_verified());
        prop_assert_eq!(status.failure_count(), 0);
        prop_assert!(status.last_error.is_none());
        prop_assert!(status.verified_at.is_some());
    }

    /// Property: mark_unverified increments failure count
    #[test]
    fn mark_unverified_increments_failure_count(
        num_failures in 1u32..50,
        error in arb_error_message(),
    ) {
        let mut status = CredentialStatus::verified();

        for i in 1..=num_failures {
            status.mark_unverified(error.clone());
            prop_assert_eq!(status.failure_count(), i);
            prop_assert!(!status.is_verified());
        }
    }

    /// Property: mark_unverified records error message
    #[test]
    fn mark_unverified_records_error(error in arb_error_message()) {
        let mut status = CredentialStatus::verified();
        status.mark_unverified(error.clone());

        prop_assert_eq!(status.last_error, error);
        prop_assert!(status.failed_at.is_some());
    }

    /// Property: has_failures returns true iff failure_count > 0
    #[test]
    fn has_failures_consistency(num_failures in 0u32..10) {
        let mut status = CredentialStatus::new();

        for _ in 0..num_failures {
            status.mark_unverified(None);
        }

        prop_assert_eq!(status.has_failures(), num_failures > 0);
    }
}

// ========== Property Tests: VerifiedCredentials ==========

proptest! {
    /// Property 6: Verified credentials with password can be used automatically
    /// Validates: Requirement 2.1 - Skip dialog for verified credentials
    #[test]
    fn verified_credentials_skip_dialog(
        username in arb_username(),
        password in "[a-zA-Z0-9]{8,32}".prop_map(Some), // Always has password
        domain in arb_domain(),
    ) {
        let creds = VerifiedCredentials::from_verified_str(username, password, domain);

        // Verified credentials with password should skip dialog
        prop_assert!(creds.can_use_automatically());
        prop_assert!(!creds.should_show_dialog());
        prop_assert!(creds.status.is_verified());
    }

    /// Property 7: Missing credentials require dialog
    /// Validates: Requirement 2.2 - Show dialog for missing credentials
    #[test]
    fn missing_credentials_require_dialog(
        username in arb_username(),
        domain in arb_domain(),
    ) {
        // Unverified credentials (no password)
        let creds = VerifiedCredentials::unverified();
        prop_assert!(creds.should_show_dialog());
        prop_assert!(!creds.can_use_automatically());

        // Verified but no password
        let creds_no_pass = VerifiedCredentials::from_verified_str(username, None, domain);
        prop_assert!(creds_no_pass.should_show_dialog());
        prop_assert!(!creds_no_pass.can_use_automatically());
    }

    /// Property: has_password returns true iff password is Some
    #[test]
    fn has_password_consistency(
        username in arb_username(),
        password in arb_password(),
        domain in arb_domain(),
    ) {
        let creds = VerifiedCredentials::from_verified_str(username, password.clone(), domain);
        prop_assert_eq!(creds.has_password(), password.is_some());
    }

    /// Property: can_use_automatically requires both verified and password
    #[test]
    fn can_use_automatically_requires_both(
        username in arb_username(),
        password in arb_password(),
        domain in arb_domain(),
    ) {
        // Verified with password
        let verified_with_pass = VerifiedCredentials::from_verified_str(
            username.clone(),
            password.clone(),
            domain.clone(),
        );

        // Unverified with password
        let unverified_with_pass = VerifiedCredentials::new(
            username.clone(),
            password.clone().map(SecretString::from),
            domain.clone(),
            CredentialStatus::new(),
        );

        // can_use_automatically should be true only if verified AND has password
        let expected = password.is_some();
        prop_assert_eq!(verified_with_pass.can_use_automatically(), expected);
        prop_assert!(!unverified_with_pass.can_use_automatically());
    }
}

// ========== Property Tests: CredentialVerificationManager ==========

proptest! {
    /// Property 9: Successful auth marks verified
    /// Validates: Requirement 2.5 - Mark credentials as verified after successful auth
    #[test]
    fn successful_auth_marks_verified(id in arb_uuid()) {
        let mut manager = CredentialVerificationManager::new();

        // Initially not verified
        prop_assert!(!manager.is_verified(id));

        // Mark as verified (simulating successful auth)
        manager.mark_verified(id);

        // Now should be verified
        prop_assert!(manager.is_verified(id));

        let status = manager.get_status(id);
        prop_assert!(status.is_verified());
        prop_assert!(status.verified_at.is_some());
    }

    /// Property: Failed auth marks unverified
    /// Validates: Requirement 2.3 - Mark credentials as requiring verification on auth failure
    #[test]
    fn failed_auth_marks_unverified(
        id in arb_uuid(),
        error in arb_error_message(),
    ) {
        let mut manager = CredentialVerificationManager::new();

        // First mark as verified
        manager.mark_verified(id);
        prop_assert!(manager.is_verified(id));

        // Then mark as unverified (simulating auth failure)
        manager.mark_unverified(id, error.clone());

        // Now should not be verified
        prop_assert!(!manager.is_verified(id));

        let status = manager.get_status(id);
        prop_assert!(!status.is_verified());
        prop_assert_eq!(status.failure_count(), 1);
        prop_assert_eq!(status.last_error, error);
    }

    /// Property: Manager tracks multiple connections independently
    #[test]
    fn manager_tracks_connections_independently(
        id1 in arb_uuid(),
        id2 in arb_uuid(),
    ) {
        prop_assume!(id1 != id2);

        let mut manager = CredentialVerificationManager::new();

        manager.mark_verified(id1);
        manager.mark_unverified(id2, Some("error".to_string()));

        prop_assert!(manager.is_verified(id1));
        prop_assert!(!manager.is_verified(id2));

        // Modifying one doesn't affect the other
        manager.mark_unverified(id1, None);
        prop_assert!(!manager.is_verified(id1));
        prop_assert!(!manager.is_verified(id2));
    }

    /// Property: Remove clears status for connection
    #[test]
    fn remove_clears_status(id in arb_uuid()) {
        let mut manager = CredentialVerificationManager::new();

        manager.mark_verified(id);
        prop_assert!(manager.is_verified(id));
        prop_assert_eq!(manager.len(), 1);

        manager.remove(id);
        prop_assert!(!manager.is_verified(id));
        prop_assert_eq!(manager.len(), 0);
    }

    /// Property: Clear removes all statuses
    #[test]
    fn clear_removes_all(ids in prop::collection::vec(arb_uuid(), 1..10)) {
        let mut manager = CredentialVerificationManager::new();

        for id in &ids {
            manager.mark_verified(*id);
        }

        prop_assert!(!manager.is_empty());

        manager.clear();

        prop_assert!(manager.is_empty());
        prop_assert_eq!(manager.len(), 0);

        for id in &ids {
            prop_assert!(!manager.is_verified(*id));
        }
    }

    /// Property: verified_connections returns only verified IDs
    #[test]
    fn verified_connections_returns_only_verified(
        verified_ids in prop::collection::vec(arb_uuid(), 0..5),
        unverified_ids in prop::collection::vec(arb_uuid(), 0..5),
    ) {
        let mut manager = CredentialVerificationManager::new();

        for id in &verified_ids {
            manager.mark_verified(*id);
        }

        for id in &unverified_ids {
            manager.mark_unverified(*id, None);
        }

        let result = manager.verified_connections();

        // All verified IDs should be in result
        for id in &verified_ids {
            prop_assert!(result.contains(id));
        }

        // No unverified IDs should be in result
        for id in &unverified_ids {
            prop_assert!(!result.contains(id));
        }
    }

    /// Property: failed_connections returns only failed IDs
    #[test]
    fn failed_connections_returns_only_failed(
        verified_ids in prop::collection::vec(arb_uuid(), 0..5),
        failed_ids in prop::collection::vec(arb_uuid(), 0..5),
    ) {
        let mut manager = CredentialVerificationManager::new();

        for id in &verified_ids {
            manager.mark_verified(*id);
        }

        for id in &failed_ids {
            manager.mark_unverified(*id, None);
        }

        let result = manager.failed_connections();

        // All failed IDs should be in result
        for id in &failed_ids {
            prop_assert!(result.contains(id));
        }

        // No verified IDs should be in result (they have 0 failures)
        for id in &verified_ids {
            prop_assert!(!result.contains(id));
        }
    }
}

// ========== Property Tests: Serialization ==========

proptest! {
    /// Property: CredentialStatus JSON round-trip
    #[test]
    fn credential_status_json_round_trip(
        verified in any::<bool>(),
        failure_count in 0u32..100,
        error in arb_error_message(),
    ) {
        let mut status = if verified {
            CredentialStatus::verified()
        } else {
            CredentialStatus::new()
        };

        for _ in 0..failure_count {
            status.mark_unverified(error.clone());
        }

        let json = serde_json::to_string(&status).unwrap();
        let deserialized: CredentialStatus = serde_json::from_str(&json).unwrap();

        prop_assert_eq!(status.verified, deserialized.verified);
        prop_assert_eq!(status.failure_count, deserialized.failure_count);
        prop_assert_eq!(status.last_error, deserialized.last_error);
    }

    /// Property: CredentialVerificationManager JSON round-trip
    #[test]
    fn manager_json_round_trip(
        verified_ids in prop::collection::vec(arb_uuid(), 0..5),
        failed_ids in prop::collection::vec(arb_uuid(), 0..5),
    ) {
        let mut manager = CredentialVerificationManager::new();

        for id in &verified_ids {
            manager.mark_verified(*id);
        }

        for id in &failed_ids {
            manager.mark_unverified(*id, Some("error".to_string()));
        }

        let json = serde_json::to_string(&manager).unwrap();
        let deserialized: CredentialVerificationManager = serde_json::from_str(&json).unwrap();

        // Verify all statuses preserved
        for id in &verified_ids {
            prop_assert_eq!(manager.is_verified(*id), deserialized.is_verified(*id));
        }

        for id in &failed_ids {
            let orig = manager.get_status(*id);
            let deser = deserialized.get_status(*id);
            prop_assert_eq!(orig.failure_count, deser.failure_count);
        }
    }
}

// ========== Property Tests: DialogPreFillData ==========

/// Strategy for generating connection names
fn arb_connection_name() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9 _-]{0,30}".prop_map(String::from)
}

proptest! {
    /// **Feature: native-protocol-embedding, Property 8: Dialog Pre-fill from Connection**
    /// **Validates: Requirements 2.4**
    ///
    /// For any connection with saved username and domain, the password dialog
    /// should be pre-filled with those values.
    #[test]
    fn dialog_prefill_from_connection(
        username in arb_username(),
        domain in arb_domain(),
        connection_name in arb_connection_name(),
    ) {
        let prefill = DialogPreFillData::from_connection(
            username.clone(),
            domain.clone(),
            connection_name.clone(),
        );

        // Pre-fill data should match connection settings
        prop_assert_eq!(&prefill.username, &username);
        prop_assert_eq!(&prefill.domain, &domain);
        prop_assert_eq!(&prefill.connection_name, &Some(connection_name));

        // has_username should be true iff username is Some and non-empty
        let expected_has_username = username.as_ref().is_some_and(|u| !u.is_empty());
        prop_assert_eq!(prefill.has_username(), expected_has_username);

        // has_domain should be true iff domain is Some and non-empty
        let expected_has_domain = domain.as_ref().is_some_and(|d| !d.is_empty());
        prop_assert_eq!(prefill.has_domain(), expected_has_domain);

        // has_prefill_data should be true if either username or domain is present
        prop_assert_eq!(
            prefill.has_prefill_data(),
            expected_has_username || expected_has_domain
        );
    }

    /// Property: Default DialogPreFillData has no pre-fill values
    #[test]
    fn dialog_prefill_default_empty(_seed in any::<u64>()) {
        let prefill = DialogPreFillData::new();

        prop_assert!(prefill.username.is_none());
        prop_assert!(prefill.domain.is_none());
        prop_assert!(prefill.connection_name.is_none());
        prop_assert!(!prefill.show_migrate_button);
        prop_assert!(!prefill.has_username());
        prop_assert!(!prefill.has_domain());
        prop_assert!(!prefill.has_prefill_data());
    }

    /// Property: with_migrate_button sets the flag correctly
    #[test]
    fn dialog_prefill_migrate_button(
        username in arb_username(),
        domain in arb_domain(),
        connection_name in arb_connection_name(),
        show_migrate in any::<bool>(),
    ) {
        let prefill = DialogPreFillData::from_connection(
            username,
            domain,
            connection_name,
        ).with_migrate_button(show_migrate);

        prop_assert_eq!(prefill.show_migrate_button, show_migrate);
    }
}
