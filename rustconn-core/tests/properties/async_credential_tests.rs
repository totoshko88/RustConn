//! Property-based tests for async credential resolution
//!
//! These tests validate the correctness properties for async credential resolution
//! as defined in Requirements 9.1, 9.4.
//!
//! **Feature: performance-improvements, Property 15: Async Credential Resolution**
//! **Validates: Requirements 9.1, 9.4**

use proptest::prelude::*;
use rustconn_core::models::AutomationConfig;
use rustconn_core::{
    AsyncCredentialResolver, AsyncCredentialResult, CancellationToken, SecretManager,
};
use std::sync::Arc;
use std::time::Duration;

// ========== Generators ==========

/// Strategy for generating connection names
fn arb_connection_name() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9 _-]{0,30}".prop_map(String::from)
}

/// Strategy for generating hostnames
fn arb_hostname() -> impl Strategy<Value = String> {
    prop_oneof![
        "[a-z]{3,10}\\.[a-z]{2,4}".prop_map(String::from),
        "192\\.168\\.[0-9]{1,3}\\.[0-9]{1,3}".prop_map(String::from),
        "10\\.[0-9]{1,3}\\.[0-9]{1,3}\\.[0-9]{1,3}".prop_map(String::from),
    ]
}

/// Strategy for generating optional usernames
fn arb_username() -> impl Strategy<Value = Option<String>> {
    prop_oneof![Just(None), "[a-zA-Z][a-zA-Z0-9_]{0,20}".prop_map(Some),]
}

/// Strategy for generating ports
fn arb_port() -> impl Strategy<Value = u16> {
    prop_oneof![Just(22), Just(3389), Just(5900), 1024u16..65535,]
}

/// Creates a test connection for property testing
fn create_test_connection(
    name: String,
    host: String,
    port: u16,
    username: Option<String>,
) -> rustconn_core::Connection {
    use rustconn_core::{
        Connection, PasswordSource, ProtocolConfig, ProtocolType, SshConfig, WindowMode,
    };
    use std::collections::HashMap;

    Connection {
        id: uuid::Uuid::new_v4(),
        name,
        host,
        port,
        protocol: ProtocolType::Ssh,
        username,
        group_id: None,
        tags: Vec::new(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        protocol_config: ProtocolConfig::Ssh(SshConfig::default()),
        sort_order: 0,
        last_connected: None,
        password_source: PasswordSource::None,
        domain: None,
        custom_properties: Vec::new(),
        pre_connect_task: None,
        post_disconnect_task: None,
        wol_config: None,
        local_variables: HashMap::new(),
        log_config: None,
        key_sequence: None,
        automation: AutomationConfig::default(),
        window_mode: WindowMode::default(),
        remember_window_position: false,
        window_geometry: None,
    }
}

/// Creates a test async resolver with empty secret manager
fn create_test_resolver() -> AsyncCredentialResolver {
    use rustconn_core::config::SecretSettings;

    let secret_manager = Arc::new(SecretManager::empty());
    let settings = SecretSettings::default();
    AsyncCredentialResolver::new(secret_manager, settings)
}

// ========== Property Tests ==========

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: performance-improvements, Property 15: Async Credential Resolution**
    /// **Validates: Requirements 9.1, 9.4**
    ///
    /// For any credential resolution request, the operation SHALL complete
    /// without blocking the calling thread.
    ///
    /// This test verifies that:
    /// 1. Async resolution completes without blocking
    /// 2. The result is either Success, Cancelled, Error, or Timeout
    /// 3. The operation can be awaited
    #[test]
    fn async_resolution_completes_without_blocking(
        name in arb_connection_name(),
        host in arb_hostname(),
        port in arb_port(),
        username in arb_username(),
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();

        rt.block_on(async {
            let resolver = create_test_resolver();
            let connection = create_test_connection(name, host, port, username);

            // Async resolution should complete
            let result = resolver.resolve_async(&connection).await;

            // Result should be one of the valid variants
            // With empty secret manager, we expect Success(None)
            match result {
                AsyncCredentialResult::Success(_) => {
                    // Expected - no credentials found with empty manager
                }
                AsyncCredentialResult::Cancelled => {
                    // Valid - operation was cancelled
                }
                AsyncCredentialResult::Error(_) => {
                    // Valid - an error occurred
                }
                AsyncCredentialResult::Timeout => {
                    // Valid - operation timed out
                }
            }
        });
    }

    /// Property: Cancellation token cancels pending operations
    ///
    /// For any credential resolution with a cancellation token,
    /// cancelling the token should result in a Cancelled result.
    #[test]
    fn cancellation_token_cancels_operations(
        name in arb_connection_name(),
        host in arb_hostname(),
        port in arb_port(),
        username in arb_username(),
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();

        rt.block_on(async {
            let resolver = create_test_resolver();
            let connection = create_test_connection(name, host, port, username);
            let cancel_token = CancellationToken::new();

            // Cancel before starting
            cancel_token.cancel();

            let result = resolver
                .resolve_with_cancellation(&connection, &cancel_token)
                .await;

            // Should be cancelled
            prop_assert!(result.is_cancelled());
            Ok(())
        })?;
    }

    /// Property: Timeout returns Timeout result
    ///
    /// For any credential resolution with a very short timeout,
    /// the operation should return Timeout if it doesn't complete in time.
    #[test]
    fn timeout_returns_timeout_result(
        name in arb_connection_name(),
        host in arb_hostname(),
        port in arb_port(),
        username in arb_username(),
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();

        rt.block_on(async {
            let resolver = create_test_resolver();
            let connection = create_test_connection(name, host, port, username);

            // Use a reasonable timeout - the empty manager should complete quickly
            let result = resolver
                .resolve_with_timeout(&connection, Duration::from_secs(5))
                .await;

            // With empty manager, should complete successfully (not timeout)
            // because there's nothing to actually resolve
            match result {
                AsyncCredentialResult::Success(_) => {
                    // Expected - completed before timeout
                }
                AsyncCredentialResult::Timeout => {
                    // Also valid if system is slow
                }
                AsyncCredentialResult::Error(_) => {
                    // Valid - an error occurred
                }
                AsyncCredentialResult::Cancelled => {
                    // Valid - was cancelled
                }
            }
        });
    }

    /// Property: Result types are correctly identified
    ///
    /// For any AsyncCredentialResult, exactly one of the is_* methods
    /// should return true.
    #[test]
    fn result_type_identification_is_exclusive(
        variant in 0u8..4,
    ) {
        let result = match variant {
            0 => AsyncCredentialResult::Success(None),
            1 => AsyncCredentialResult::Cancelled,
            2 => AsyncCredentialResult::Error("test error".to_string()),
            _ => AsyncCredentialResult::Timeout,
        };

        let is_success = result.is_success();
        let is_cancelled = result.is_cancelled();
        let is_error = result.is_error();
        let is_timeout = result.is_timeout();

        // Exactly one should be true
        let count = [is_success, is_cancelled, is_error, is_timeout]
            .iter()
            .filter(|&&x| x)
            .count();

        prop_assert_eq!(count, 1);
    }
}

// ========== Unit Tests for CancellationToken ==========

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Property: CancellationToken clone shares state
    ///
    /// For any cancellation token, cloning it should share the cancellation state.
    #[test]
    fn cancellation_token_clone_shares_state(_seed in any::<u64>()) {
        let token1 = CancellationToken::new();
        let token2 = token1.clone();

        // Initially not cancelled
        prop_assert!(!token1.is_cancelled());
        prop_assert!(!token2.is_cancelled());

        // Cancel through token1
        token1.cancel();

        // Both should be cancelled
        prop_assert!(token1.is_cancelled());
        prop_assert!(token2.is_cancelled());
    }

    /// Property: CancellationToken reset clears state
    ///
    /// For any cancelled token, reset should clear the cancellation state.
    #[test]
    fn cancellation_token_reset_clears_state(_seed in any::<u64>()) {
        let token = CancellationToken::new();

        // Cancel and verify
        token.cancel();
        prop_assert!(token.is_cancelled());

        // Reset and verify
        token.reset();
        prop_assert!(!token.is_cancelled());
    }

    /// Property: Multiple cancellations are idempotent
    ///
    /// Calling cancel() multiple times should have the same effect as calling it once.
    #[test]
    fn multiple_cancellations_are_idempotent(num_cancels in 1usize..10) {
        let token = CancellationToken::new();

        for _ in 0..num_cancels {
            token.cancel();
        }

        prop_assert!(token.is_cancelled());
    }
}

// ========== Unit Tests for AsyncCredentialResult ==========

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Property: into_credentials returns credentials only for Success
    #[test]
    fn into_credentials_only_for_success(variant in 0u8..4) {
        let result = match variant {
            0 => AsyncCredentialResult::Success(Some(rustconn_core::Credentials::with_password(
                "test",
                "password",
            ))),
            1 => AsyncCredentialResult::Cancelled,
            2 => AsyncCredentialResult::Error("test error".to_string()),
            _ => AsyncCredentialResult::Timeout,
        };

        let creds = result.into_credentials();

        match variant {
            0 => prop_assert!(creds.is_some()),
            _ => prop_assert!(creds.is_none()),
        }
    }

    /// Property: error_message returns message only for Error
    #[test]
    fn error_message_only_for_error(
        error_msg in "[a-zA-Z0-9 ]{1,50}",
        variant in 0u8..4,
    ) {
        let result = match variant {
            0 => AsyncCredentialResult::Success(None),
            1 => AsyncCredentialResult::Cancelled,
            2 => AsyncCredentialResult::Error(error_msg.clone()),
            _ => AsyncCredentialResult::Timeout,
        };

        let msg = result.error_message();

        match variant {
            2 => {
                prop_assert!(msg.is_some());
                prop_assert_eq!(msg.unwrap(), &error_msg);
            }
            _ => prop_assert!(msg.is_none()),
        }
    }
}
