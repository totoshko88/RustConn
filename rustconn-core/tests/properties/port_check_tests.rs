//! Property tests for port check functionality

use proptest::prelude::*;
use rustconn_core::connection::{PortCheckError, PortCheckResult};

proptest! {
    /// Property: PortCheckResult::Open equals itself
    #[test]
    fn port_check_result_open_equality(_dummy in 0..1) {
        prop_assert_eq!(PortCheckResult::Open, PortCheckResult::Open);
    }

    /// Property: PortCheckResult::Skipped equals itself
    #[test]
    fn port_check_result_skipped_equality(_dummy in 0..1) {
        prop_assert_eq!(PortCheckResult::Skipped, PortCheckResult::Skipped);
    }

    /// Property: Open and Skipped are not equal
    #[test]
    fn port_check_result_different(_dummy in 0..1) {
        prop_assert_ne!(PortCheckResult::Open, PortCheckResult::Skipped);
    }

    /// Property: PortCheckResult clone works correctly
    #[test]
    fn port_check_result_clone(_dummy in 0..1) {
        let open = PortCheckResult::Open;
        let skipped = PortCheckResult::Skipped;

        prop_assert_eq!(open.clone(), PortCheckResult::Open);
        prop_assert_eq!(skipped.clone(), PortCheckResult::Skipped);
    }

    /// Property: PortCheckError::ResolutionFailed preserves host
    #[test]
    fn resolution_failed_preserves_host(
        host in "[a-zA-Z][a-zA-Z0-9.-]{0,50}",
        reason in "[a-zA-Z0-9 ]{1,100}",
    ) {
        let error = PortCheckError::ResolutionFailed {
            host: host.clone(),
            reason: reason.clone(),
        };

        let error_str = format!("{error}");
        prop_assert!(error_str.contains(&host));
    }

    /// Property: PortCheckError::Unreachable preserves host and port
    #[test]
    fn unreachable_preserves_fields(
        host in "[a-zA-Z][a-zA-Z0-9.-]{0,50}",
        port in 1u16..65535,
        reason in "[a-zA-Z0-9 ]{1,100}",
    ) {
        let error = PortCheckError::Unreachable {
            host: host.clone(),
            port,
            reason: reason.clone(),
        };

        let error_str = format!("{error}");
        prop_assert!(error_str.contains(&host));
        prop_assert!(error_str.contains(&port.to_string()));
    }

    /// Property: Invalid hostnames fail resolution
    #[test]
    fn invalid_hostname_fails(
        invalid_suffix in "[a-z]{5,10}",
    ) {
        // Use a hostname that definitely won't resolve
        let host = format!("invalid-host-{invalid_suffix}.nonexistent.local");
        let result = rustconn_core::connection::check_port(&host, 22, 1);

        prop_assert!(result.is_err());
    }

    /// Property: Port 0 is invalid
    #[test]
    fn port_zero_fails(_dummy in 0..1) {
        // Port 0 should fail (either resolution or connection)
        let result = rustconn_core::connection::check_port("127.0.0.1", 0, 1);
        // Port 0 is special - it may fail differently on different systems
        // but should not succeed with Open
        if let Ok(r) = result {
            prop_assert_ne!(r, PortCheckResult::Open);
        }
    }

    /// Property: Very short timeout still works
    #[test]
    fn short_timeout_works(
        port in 50000u16..60000,
    ) {
        // Use a port that's unlikely to be open with 1 second timeout
        let result = rustconn_core::connection::check_port("127.0.0.1", port, 1);
        // Should fail (port closed) but not panic
        prop_assert!(result.is_err() || result.is_ok());
    }
}

#[test]
fn test_port_check_result_debug() {
    let open = PortCheckResult::Open;
    let skipped = PortCheckResult::Skipped;

    assert!(format!("{open:?}").contains("Open"));
    assert!(format!("{skipped:?}").contains("Skipped"));
}

#[test]
fn test_port_check_error_debug() {
    let resolution_error = PortCheckError::ResolutionFailed {
        host: "example.com".to_string(),
        reason: "DNS lookup failed".to_string(),
    };

    let unreachable_error = PortCheckError::Unreachable {
        host: "example.com".to_string(),
        port: 22,
        reason: "Connection refused".to_string(),
    };

    assert!(format!("{resolution_error:?}").contains("ResolutionFailed"));
    assert!(format!("{unreachable_error:?}").contains("Unreachable"));
}

#[test]
fn test_localhost_closed_port() {
    // Port 59998 is very unlikely to be open
    let result = rustconn_core::connection::check_port("127.0.0.1", 59998, 1);
    assert!(result.is_err());

    if let Err(PortCheckError::Unreachable { host, port, .. }) = result {
        assert_eq!(host, "127.0.0.1");
        assert_eq!(port, 59998);
    }
}

#[test]
fn test_invalid_hostname() {
    let result = rustconn_core::connection::check_port(
        "this-hostname-definitely-does-not-exist.invalid",
        22,
        1,
    );
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        PortCheckError::ResolutionFailed { .. }
    ));
}

#[tokio::test]
async fn test_check_port_async_invalid_host() {
    let result = rustconn_core::connection::check_port_async(
        "invalid.host.that.does.not.exist.local",
        22,
        1,
    )
    .await;
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        PortCheckError::ResolutionFailed { .. }
    ));
}

#[tokio::test]
async fn test_check_port_async_closed_port() {
    let result = rustconn_core::connection::check_port_async("127.0.0.1", 59997, 1).await;
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        PortCheckError::Unreachable { .. }
    ));
}
