//! Property-based and unit tests for the `EncryptedFileBackend`.
//!
//! These tests validate the correctness properties for the application-managed
//! encrypted-file secret backend (Part B of the resilient-secret-storage spec).
//!
//! **Feature: resilient-secret-storage, Task 2.7**
//! **Validates: Requirements 18.1, 18.2, 5.5, 6.5**
//!
//! - Round-trip (R18.1, R5.5): `store` then `retrieve` yields an equivalent
//!   credential across arbitrary field values (empty, unicode, long).
//! - Debug-leak (R18.2): neither the backend's `Debug` nor a retrieved
//!   `Credentials`' `Debug` exposes a secret value.
//! - Delete-isolation (R6.5): deleting one entry leaves the others intact.
//!
//! **Feature: resilient-secret-storage, Task 3.4**
//! **Validates: Requirements 3.1, 3.3**
//!
//! - Fallback (R3.1, R3.3): when the preferred backend's `store` fails, the
//!   `SecretManager` falls back to the encrypted-file backend, reports the
//!   fallback backend id, and the credential is retrievable afterwards.

use async_trait::async_trait;
use proptest::prelude::*;
use rustconn_core::error::{SecretError, SecretResult};
use rustconn_core::models::Credentials;
use rustconn_core::secret::{EncryptedFileBackend, SecretBackend, SecretManager, StoreOutcome};
use secrecy::SecretString;
use std::sync::Arc;
use tempfile::TempDir;

// ========== Generators ==========

/// Strategy for an optional, possibly-secret string that spans the interesting
/// input space: absent, empty, ASCII, unicode, and long values.
fn arb_field() -> impl Strategy<Value = Option<String>> {
    prop_oneof![
        Just(None),
        Just(Some(String::new())),
        "[a-zA-Z0-9 _.@-]{1,40}".prop_map(Some),
        // Unicode incl. Cyrillic, CJK, emoji.
        "[\\p{Cyrillic}\\p{Han}\u{1F300}-\u{1F5FF}]{1,16}".prop_map(Some),
        // Long value to exercise larger ciphertext blobs.
        "[a-zA-Z0-9]{200,400}".prop_map(Some),
    ]
}

/// Strategy for a non-empty connection id (the flat store key).
fn arb_connection_id() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9/_-]{1,40}".prop_map(String::from)
}

/// Builds `Credentials` from generated optional fields, wrapping secret values
/// in `SecretString`.
fn make_credentials(
    username: Option<String>,
    password: Option<String>,
    key_passphrase: Option<String>,
    domain: Option<String>,
) -> Credentials {
    Credentials {
        username,
        password: password.map(SecretString::from),
        key_passphrase: key_passphrase.map(SecretString::from),
        domain,
    }
}

/// Builds a current-thread tokio runtime for driving the async backend methods
/// from within a synchronous proptest closure.
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime for test")
}

// ========== Round-trip property (R18.1, R5.5) ==========

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// **Validates: Requirements 18.1, 5.5**
    ///
    /// For arbitrary credentials, `store` followed by `retrieve` yields an
    /// equivalent credential — every field (username/password/passphrase/domain)
    /// is preserved across the encrypt -> on-disk -> decrypt round trip.
    #[test]
    fn store_then_retrieve_preserves_credentials(
        connection_id in arb_connection_id(),
        username in arb_field(),
        password in arb_field(),
        key_passphrase in arb_field(),
        domain in arb_field(),
    ) {
        let creds = make_credentials(username, password, key_passphrase, domain);

        // Isolated temp store per case so nothing leaks between iterations.
        let dir = TempDir::new().expect("failed to create temp dir");
        let backend = EncryptedFileBackend::with_path(dir.path().join("credentials.enc"));

        let rt = runtime();
        rt.block_on(async {
            backend
                .store(&connection_id, &creds)
                .await
                .expect("store should succeed");
            let retrieved = backend
                .retrieve(&connection_id)
                .await
                .expect("retrieve should succeed");

            // PartialEq on Credentials compares the exposed secret values too.
            prop_assert_eq!(retrieved, Some(creds));
            Ok(())
        })?;
    }

    /// **Validates: Requirement 6.5**
    ///
    /// Retrieving an absent connection id returns `None` (no panic, no error).
    #[test]
    fn retrieve_absent_returns_none(connection_id in arb_connection_id()) {
        let dir = TempDir::new().expect("failed to create temp dir");
        let backend = EncryptedFileBackend::with_path(dir.path().join("credentials.enc"));

        let rt = runtime();
        rt.block_on(async {
            let retrieved = backend
                .retrieve(&connection_id)
                .await
                .expect("retrieve should succeed");
            prop_assert!(retrieved.is_none());
            Ok(())
        })?;
    }
}

// ========== Debug-leak (R18.2) ==========

/// **Validates: Requirement 18.2**
///
/// The backend's `Debug` representation never contains a stored secret value;
/// the backend holds only the (non-secret) path, so a sentinel password stored
/// through it must not appear in `format!("{backend:?}")`.
#[test]
fn backend_debug_does_not_leak_secret() {
    const SENTINEL: &str = "S3ntinel-Pa55w0rd-do-not-leak";

    let dir = TempDir::new().expect("failed to create temp dir");
    let backend = EncryptedFileBackend::with_path(dir.path().join("credentials.enc"));

    let rt = runtime();
    rt.block_on(async {
        let creds = Credentials::with_password("alice", SENTINEL);
        backend
            .store("conn-debug", &creds)
            .await
            .expect("store should succeed");

        let rendered = format!("{backend:?}");
        assert!(
            !rendered.contains(SENTINEL),
            "backend Debug leaked the secret: {rendered}"
        );
    });
}

/// **Validates: Requirement 18.2**
///
/// A `Credentials` value retrieved from the backend redacts its secret fields in
/// `Debug` (via `SecretString`), so a round-tripped sentinel password is absent
/// from the rendered debug string.
#[test]
fn retrieved_credentials_debug_does_not_leak_secret() {
    const SENTINEL_PW: &str = "R3trieved-Secret-XYZ";
    const SENTINEL_PASS: &str = "Passphrase-LEAK-CHECK-42";

    let dir = TempDir::new().expect("failed to create temp dir");
    let backend = EncryptedFileBackend::with_path(dir.path().join("credentials.enc"));

    let rt = runtime();
    rt.block_on(async {
        let creds = Credentials {
            username: Some("bob".to_string()),
            password: Some(SecretString::from(SENTINEL_PW.to_string())),
            key_passphrase: Some(SecretString::from(SENTINEL_PASS.to_string())),
            domain: Some("EXAMPLE".to_string()),
        };
        backend
            .store("conn-redact", &creds)
            .await
            .expect("store should succeed");

        let retrieved = backend
            .retrieve("conn-redact")
            .await
            .expect("retrieve should succeed")
            .expect("entry should exist");

        let rendered = format!("{retrieved:?}");
        assert!(
            !rendered.contains(SENTINEL_PW),
            "retrieved Credentials Debug leaked the password: {rendered}"
        );
        assert!(
            !rendered.contains(SENTINEL_PASS),
            "retrieved Credentials Debug leaked the passphrase: {rendered}"
        );
    });
}

// ========== Delete-isolation (R6.5) ==========

/// **Validates: Requirement 6.5**
///
/// Storing two entries under different connection ids and deleting one leaves
/// the other retrievable while the deleted one returns `None`.
#[test]
fn delete_one_entry_leaves_others_intact() {
    let dir = TempDir::new().expect("failed to create temp dir");
    let backend = EncryptedFileBackend::with_path(dir.path().join("credentials.enc"));

    let rt = runtime();
    rt.block_on(async {
        let keep = Credentials::with_password("keep-user", "keep-password");
        let to_drop = Credentials::with_password("drop-user", "drop-password");

        backend
            .store("conn-keep", &keep)
            .await
            .expect("store keep should succeed");
        backend
            .store("conn-drop", &to_drop)
            .await
            .expect("store drop should succeed");

        backend
            .delete("conn-drop")
            .await
            .expect("delete should succeed");

        let kept = backend
            .retrieve("conn-keep")
            .await
            .expect("retrieve keep should succeed");
        assert_eq!(kept, Some(keep), "surviving entry must be unchanged");

        let dropped = backend
            .retrieve("conn-drop")
            .await
            .expect("retrieve drop should succeed");
        assert!(dropped.is_none(), "deleted entry must return None");
    });
}

// ========== Fallback wiring (Task 3.4 — R3.1, R3.3) ==========

/// Stub primary backend that is available but always fails to store.
///
/// Models the issue #201 case: a legitimate, present backend (e.g. a system
/// keyring whose service is up enough to report available) whose `store` never
/// succeeds. It reports `is_available() == true` so it is a genuine — not
/// skipped — primary, forcing the `SecretManager` to exercise its fallback path.
#[derive(Debug)]
struct AlwaysFailsBackend;

#[async_trait]
impl SecretBackend for AlwaysFailsBackend {
    async fn store(&self, _connection_id: &str, _credentials: &Credentials) -> SecretResult<()> {
        Err(SecretError::StoreFailed(
            "always_fails backend refuses every write".to_string(),
        ))
    }

    async fn retrieve(&self, _connection_id: &str) -> SecretResult<Option<Credentials>> {
        Ok(None)
    }

    async fn delete(&self, _connection_id: &str) -> SecretResult<()> {
        Ok(())
    }

    async fn is_available(&self) -> bool {
        true
    }

    fn backend_id(&self) -> &'static str {
        "always_fails"
    }

    fn display_name(&self) -> &'static str {
        "Always-fails (test stub)"
    }
}

/// **Validates: Requirements 3.1, 3.3**
///
/// With a failing primary and a real `EncryptedFileBackend` as the next backend
/// in the chain, `store_reported(.., allow_fallback = true)` reports a fallback
/// to `encrypted_file`, and the credential is subsequently retrievable through
/// the manager — proving it was actually written to the encrypted-file backend.
#[test]
fn fallback_to_encrypted_file_when_primary_fails() {
    let dir = TempDir::new().expect("failed to create temp dir");
    let file_backend = EncryptedFileBackend::with_path(dir.path().join("credentials.enc"));

    let manager = SecretManager::new(vec![
        Arc::new(AlwaysFailsBackend) as Arc<dyn SecretBackend>,
        Arc::new(file_backend) as Arc<dyn SecretBackend>,
    ]);

    let creds = Credentials {
        username: Some("fallback-user".to_string()),
        password: Some(SecretString::from("fallback-secret-pw")),
        key_passphrase: None,
        domain: Some("EXAMPLE".to_string()),
    };

    let rt = runtime();
    rt.block_on(async {
        let outcome = manager
            .store_reported("conn-fallback", &creds, true)
            .await
            .expect("store should succeed via fallback");

        assert_eq!(
            outcome,
            StoreOutcome::Fallback {
                backend_id: "encrypted_file".to_string(),
            },
            "primary failed, so the encrypted-file backend must be reported as the fallback"
        );

        let retrieved = manager
            .retrieve("conn-fallback")
            .await
            .expect("retrieve should succeed")
            .expect("credential must be retrievable after fallback store");
        assert_eq!(
            retrieved, creds,
            "the credential written to the fallback backend must round-trip"
        );
    });
}

/// **Validates: Requirement 14.2**
///
/// When `allow_fallback` is `false` and the primary backend fails, the primary's
/// original error is returned unchanged — the fallback chain is not consulted.
#[test]
fn no_fallback_surfaces_primary_error() {
    let dir = TempDir::new().expect("failed to create temp dir");
    let file_backend = EncryptedFileBackend::with_path(dir.path().join("credentials.enc"));

    let manager = SecretManager::new(vec![
        Arc::new(AlwaysFailsBackend) as Arc<dyn SecretBackend>,
        Arc::new(file_backend) as Arc<dyn SecretBackend>,
    ]);

    let creds = Credentials::with_password("no-fallback-user", "no-fallback-pw");

    let rt = runtime();
    rt.block_on(async {
        let err = manager
            .store_reported("conn-no-fallback", &creds, false)
            .await
            .expect_err("store must fail with the primary error when fallback is disallowed");
        assert!(
            matches!(err, SecretError::StoreFailed(_)),
            "the primary StoreFailed error must be surfaced unchanged, got: {err:?}"
        );

        // Nothing should have been written to the fallback backend.
        let retrieved = manager
            .retrieve("conn-no-fallback")
            .await
            .expect("retrieve should succeed");
        assert!(
            retrieved.is_none(),
            "no credential should be stored when fallback is disallowed"
        );
    });
}
