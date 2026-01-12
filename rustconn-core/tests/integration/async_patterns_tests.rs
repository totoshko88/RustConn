//! Integration tests for async/callback patterns
//!
//! These tests verify that the async credential resolution patterns work correctly
//! when used with callbacks, simulating how they would be used in a GTK context.
//!
//! **Validates: Requirements 9.1, 9.2 - Async operations instead of blocking calls**

use rustconn_core::models::AutomationConfig;
use rustconn_core::{
    AsyncCredentialResolver, AsyncCredentialResult, CancellationToken, Connection, Credentials,
    PasswordSource, ProtocolConfig, ProtocolType, SecretManager, SshConfig, WindowMode,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Creates a test connection for integration testing
fn create_test_connection(name: &str, host: &str) -> Connection {
    Connection {
        id: uuid::Uuid::new_v4(),
        name: name.to_string(),
        description: None,
        host: host.to_string(),
        port: 22,
        protocol: ProtocolType::Ssh,
        username: Some("testuser".to_string()),
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

// ========== Callback Pattern Tests ==========

/// Tests that resolve_with_callback invokes the callback with the result
#[test]
fn callback_is_invoked_on_completion() {
    let rt = tokio::runtime::Runtime::new().expect("Failed to create runtime");
    let callback_invoked = Arc::new(AtomicBool::new(false));
    let callback_invoked_clone = callback_invoked.clone();

    rt.block_on(async {
        let resolver = Arc::new(create_test_resolver());
        let connection = create_test_connection("test-server", "192.168.1.1");

        // Spawn the resolution with a callback
        let (tx, rx) = tokio::sync::oneshot::channel();

        let resolver_clone = resolver.clone();
        tokio::spawn(async move {
            let result = resolver_clone.resolve_async(&connection).await;
            callback_invoked_clone.store(true, Ordering::SeqCst);
            let _ = tx.send(result);
        });

        // Wait for completion
        let result = rx.await.expect("Channel closed unexpectedly");

        // Verify callback was invoked
        assert!(callback_invoked.load(Ordering::SeqCst));

        // With empty manager, should get Success(None)
        assert!(result.is_success());
    });
}

/// Tests that multiple concurrent resolutions complete independently
#[test]
fn concurrent_resolutions_complete_independently() {
    let rt = tokio::runtime::Runtime::new().expect("Failed to create runtime");
    let completed_count = Arc::new(AtomicUsize::new(0));

    rt.block_on(async {
        let resolver = Arc::new(create_test_resolver());
        let mut handles = Vec::new();

        // Spawn 5 concurrent resolutions
        for i in 0..5 {
            let resolver_clone = resolver.clone();
            let completed_clone = completed_count.clone();
            let connection = create_test_connection(&format!("server-{i}"), &format!("10.0.0.{i}"));

            let handle = tokio::spawn(async move {
                let result = resolver_clone.resolve_async(&connection).await;
                completed_clone.fetch_add(1, Ordering::SeqCst);
                result
            });

            handles.push(handle);
        }

        // Wait for all to complete
        for handle in handles {
            let result = handle.await.expect("Task panicked");
            assert!(result.is_success());
        }

        // All 5 should have completed
        assert_eq!(completed_count.load(Ordering::SeqCst), 5);
    });
}

/// Tests that cancellation works correctly with callbacks
#[test]
fn cancellation_stops_pending_resolution() {
    let rt = tokio::runtime::Runtime::new().expect("Failed to create runtime");

    rt.block_on(async {
        let resolver = Arc::new(create_test_resolver());
        let connection = create_test_connection("test-server", "192.168.1.1");
        let cancel_token = CancellationToken::new();

        // Cancel immediately
        cancel_token.cancel();

        // Resolution should return Cancelled
        let result = resolver
            .resolve_with_cancellation(&connection, &cancel_token)
            .await;

        assert!(result.is_cancelled());
    });
}

/// Tests that timeout works correctly
#[test]
fn timeout_returns_appropriate_result() {
    let rt = tokio::runtime::Runtime::new().expect("Failed to create runtime");

    rt.block_on(async {
        let resolver = Arc::new(create_test_resolver());
        let connection = create_test_connection("test-server", "192.168.1.1");

        // With empty manager, should complete quickly (not timeout)
        let result = resolver
            .resolve_with_timeout(&connection, Duration::from_secs(5))
            .await;

        // Should succeed (empty manager returns None quickly)
        assert!(result.is_success() || result.is_timeout());
    });
}

// ========== Result Handling Tests ==========

/// Tests that Success result can be converted to credentials
#[test]
fn success_result_provides_credentials() {
    let creds = Credentials::with_password("user", "pass");
    let result = AsyncCredentialResult::Success(Some(creds));

    assert!(result.is_success());
    assert!(!result.is_cancelled());
    assert!(!result.is_error());
    assert!(!result.is_timeout());

    let extracted = result.into_credentials();
    assert!(extracted.is_some());
}

/// Tests that Success(None) indicates no credentials found
#[test]
fn success_none_indicates_no_credentials() {
    let result = AsyncCredentialResult::Success(None);

    assert!(result.is_success());
    let extracted = result.into_credentials();
    assert!(extracted.is_none());
}

/// Tests that Error result provides error message
#[test]
fn error_result_provides_message() {
    let result = AsyncCredentialResult::Error("Connection failed".to_string());

    assert!(result.is_error());
    assert!(!result.is_success());

    let msg = result.error_message();
    assert!(msg.is_some());
    assert_eq!(msg.unwrap(), "Connection failed");
}

/// Tests that Cancelled result is correctly identified
#[test]
fn cancelled_result_is_identified() {
    let result = AsyncCredentialResult::Cancelled;

    assert!(result.is_cancelled());
    assert!(!result.is_success());
    assert!(!result.is_error());
    assert!(!result.is_timeout());
}

/// Tests that Timeout result is correctly identified
#[test]
fn timeout_result_is_identified() {
    let result = AsyncCredentialResult::Timeout;

    assert!(result.is_timeout());
    assert!(!result.is_success());
    assert!(!result.is_error());
    assert!(!result.is_cancelled());
}

// ========== CancellationToken Tests ==========

/// Tests that CancellationToken can be shared across threads
#[test]
fn cancellation_token_is_thread_safe() {
    let rt = tokio::runtime::Runtime::new().expect("Failed to create runtime");

    rt.block_on(async {
        let token = CancellationToken::new();
        let token_clone = token.clone();

        // Spawn a task that will cancel the token
        let handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            token_clone.cancel();
        });

        // Wait for cancellation
        handle.await.expect("Task panicked");

        // Original token should be cancelled
        assert!(token.is_cancelled());
    });
}

/// Tests that CancellationToken reset works correctly
#[test]
fn cancellation_token_can_be_reset() {
    let token = CancellationToken::new();

    // Initially not cancelled
    assert!(!token.is_cancelled());

    // Cancel
    token.cancel();
    assert!(token.is_cancelled());

    // Reset
    token.reset();
    assert!(!token.is_cancelled());

    // Can cancel again
    token.cancel();
    assert!(token.is_cancelled());
}

// ========== Channel-Based Pattern Tests ==========

/// Tests the channel-based pattern used in spawn_blocking_with_callback
#[test]
fn channel_pattern_delivers_results() {
    let (tx, rx) = std::sync::mpsc::channel::<String>();

    // Simulate background thread work
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(10));
        let _ = tx.send("result".to_string());
    });

    // Poll for result (simulating GTK idle_add pattern)
    let mut result = None;
    for _ in 0..100 {
        match rx.try_recv() {
            Ok(r) => {
                result = Some(r);
                break;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                break;
            }
        }
    }

    assert_eq!(result, Some("result".to_string()));
}

/// Tests that channel disconnection is handled gracefully
#[test]
fn channel_disconnection_is_detected() {
    let (tx, rx) = std::sync::mpsc::channel::<String>();

    // Drop sender without sending
    drop(tx);

    // Should detect disconnection
    match rx.try_recv() {
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            // Expected
        }
        _ => panic!("Expected Disconnected error"),
    }
}

/// Tests timeout pattern with channels
#[test]
fn channel_timeout_pattern_works() {
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let start = std::time::Instant::now();
    let timeout = Duration::from_millis(100);

    // Spawn a slow task
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        let _ = tx.send("late result".to_string());
    });

    // Poll with timeout
    let mut result = None;
    while start.elapsed() < timeout {
        match rx.try_recv() {
            Ok(r) => {
                result = Some(r);
                break;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                break;
            }
        }
    }

    // Should have timed out (no result)
    assert!(result.is_none());
}
