//! Property tests for bulk credential operations

use proptest::prelude::*;
use rustconn_core::models::Credentials;
use rustconn_core::secret::{BulkOperationResult, CredentialUpdate};
use secrecy::ExposeSecret;

proptest! {
    /// Property: BulkOperationResult starts empty
    #[test]
    fn bulk_result_starts_empty(_dummy in 0..1) {
        let result = BulkOperationResult::new();
        prop_assert_eq!(result.success_count, 0);
        prop_assert_eq!(result.failure_count, 0);
        prop_assert!(result.failed_ids.is_empty());
        prop_assert!(result.errors.is_empty());
    }

    /// Property: Empty result is success
    #[test]
    fn empty_result_is_success(_dummy in 0..1) {
        let result = BulkOperationResult::new();
        prop_assert!(result.is_success());
        prop_assert!(!result.has_failures());
    }

    /// Property: Total equals success + failure
    #[test]
    fn total_equals_sum(success in 0usize..100, failure in 0usize..100) {
        let result = BulkOperationResult {
            success_count: success,
            failure_count: failure,
            failed_ids: Vec::new(),
            errors: Vec::new(),
        };
        prop_assert_eq!(result.total(), success + failure);
    }

    /// Property: has_failures is true when failure_count > 0
    #[test]
    fn has_failures_when_nonzero(failure in 1usize..100) {
        let result = BulkOperationResult {
            success_count: 0,
            failure_count: failure,
            failed_ids: Vec::new(),
            errors: Vec::new(),
        };
        prop_assert!(result.has_failures());
        prop_assert!(!result.is_success());
    }

    /// Property: CredentialUpdate starts with no changes
    #[test]
    fn credential_update_starts_empty(_dummy in 0..1) {
        let update = CredentialUpdate::new();
        prop_assert!(update.username.is_none());
        prop_assert!(update.password.is_none());
        prop_assert!(update.domain.is_none());
        prop_assert!(!update.clear_password);
    }

    /// Property: with_username sets username
    #[test]
    fn with_username_sets_value(username in "[a-z]{3,10}") {
        let update = CredentialUpdate::new().with_username(&username);
        prop_assert_eq!(update.username.as_deref(), Some(username.as_str()));
    }

    /// Property: with_domain sets domain
    #[test]
    fn with_domain_sets_value(domain in "[A-Z]{3,10}") {
        let update = CredentialUpdate::new().with_domain(&domain);
        prop_assert_eq!(update.domain.as_deref(), Some(domain.as_str()));
    }

    /// Property: with_password sets password
    #[test]
    fn with_password_sets_value(password in "[a-zA-Z0-9]{8,20}") {
        let update = CredentialUpdate::new().with_password(&password);
        prop_assert!(update.password.is_some());
        let stored = update.password.as_ref().unwrap().expose_secret();
        prop_assert_eq!(stored, &password);
    }

    /// Property: with_clear_password sets flag
    #[test]
    fn with_clear_password_sets_flag(_dummy in 0..1) {
        let update = CredentialUpdate::new().with_clear_password();
        prop_assert!(update.clear_password);
    }

    /// Property: apply preserves existing username when not updated
    #[test]
    fn apply_preserves_existing_username(existing_user in "[a-z]{3,10}") {
        let existing = Credentials::with_username(&existing_user);
        let update = CredentialUpdate::new(); // No username change
        let result = update.apply(&existing);
        prop_assert_eq!(result.username.as_deref(), Some(existing_user.as_str()));
    }

    /// Property: apply replaces username when updated
    #[test]
    fn apply_replaces_username(
        existing_user in "[a-z]{3,10}",
        new_user in "[a-z]{3,10}",
    ) {
        let existing = Credentials::with_username(&existing_user);
        let update = CredentialUpdate::new().with_username(&new_user);
        let result = update.apply(&existing);
        prop_assert_eq!(result.username.as_deref(), Some(new_user.as_str()));
    }

    /// Property: apply clears password when clear_password is set
    #[test]
    fn apply_clears_password(
        username in "[a-z]{3,10}",
        password in "[a-zA-Z0-9]{8,20}",
    ) {
        let existing = Credentials::with_password(&username, &password);
        let update = CredentialUpdate::new().with_clear_password();
        let result = update.apply(&existing);
        prop_assert!(result.password.is_none());
    }

    /// Property: apply preserves password when not cleared
    #[test]
    fn apply_preserves_password(
        username in "[a-z]{3,10}",
        password in "[a-zA-Z0-9]{8,20}",
    ) {
        let existing = Credentials::with_password(&username, &password);
        let update = CredentialUpdate::new(); // No password change
        let result = update.apply(&existing);
        prop_assert!(result.password.is_some());
        prop_assert_eq!(result.expose_password(), Some(password.as_str()));
    }

    /// Property: apply sets new password
    #[test]
    fn apply_sets_new_password(
        username in "[a-z]{3,10}",
        old_password in "[a-zA-Z0-9]{8,20}",
        new_password in "[a-zA-Z0-9]{8,20}",
    ) {
        let existing = Credentials::with_password(&username, &old_password);
        let update = CredentialUpdate::new().with_password(&new_password);
        let result = update.apply(&existing);
        prop_assert_eq!(result.expose_password(), Some(new_password.as_str()));
    }

    /// Property: apply preserves domain when not updated
    #[test]
    fn apply_preserves_domain(_dummy in 0..1) {
        let mut existing = Credentials::empty();
        existing.domain = Some("DOMAIN".to_string());
        let update = CredentialUpdate::new();
        let result = update.apply(&existing);
        prop_assert_eq!(result.domain.as_deref(), Some("DOMAIN"));
    }

    /// Property: apply sets new domain
    #[test]
    fn apply_sets_new_domain(new_domain in "[A-Z]{3,10}") {
        let existing = Credentials::empty();
        let update = CredentialUpdate::new().with_domain(&new_domain);
        let result = update.apply(&existing);
        prop_assert_eq!(result.domain.as_deref(), Some(new_domain.as_str()));
    }

    /// Property: apply preserves key_passphrase (never modified by update)
    #[test]
    fn apply_preserves_key_passphrase(_dummy in 0..1) {
        use secrecy::SecretString;
        let mut existing = Credentials::empty();
        existing.key_passphrase = Some(SecretString::from("passphrase".to_string()));
        let update = CredentialUpdate::new().with_username("newuser");
        let result = update.apply(&existing);
        prop_assert!(result.key_passphrase.is_some());
        prop_assert_eq!(result.expose_key_passphrase(), Some("passphrase"));
    }
}

#[test]
fn test_bulk_result_default() {
    let result = BulkOperationResult::default();
    assert_eq!(result.success_count, 0);
    assert_eq!(result.failure_count, 0);
    assert!(result.is_success());
}

#[test]
fn test_credential_update_default() {
    let update = CredentialUpdate::default();
    assert!(update.username.is_none());
    assert!(update.password.is_none());
    assert!(update.domain.is_none());
    assert!(!update.clear_password);
}

#[test]
fn test_credential_update_chaining() {
    let update = CredentialUpdate::new()
        .with_username("user")
        .with_password("pass")
        .with_domain("DOMAIN");

    assert_eq!(update.username.as_deref(), Some("user"));
    assert!(update.password.is_some());
    assert_eq!(update.domain.as_deref(), Some("DOMAIN"));
}

#[test]
fn test_apply_empty_to_empty() {
    let existing = Credentials::empty();
    let update = CredentialUpdate::new();
    let result = update.apply(&existing);
    assert!(result.is_empty());
}

#[test]
fn test_apply_full_update() {
    let existing = Credentials::with_password("olduser", "oldpass");
    let update = CredentialUpdate::new()
        .with_username("newuser")
        .with_password("newpass")
        .with_domain("NEWDOMAIN");

    let result = update.apply(&existing);
    assert_eq!(result.username.as_deref(), Some("newuser"));
    assert_eq!(result.expose_password(), Some("newpass"));
    assert_eq!(result.domain.as_deref(), Some("NEWDOMAIN"));
}
