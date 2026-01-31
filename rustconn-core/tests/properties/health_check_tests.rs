//! Property tests for session health check functionality

use proptest::prelude::*;
use rustconn_core::session::{HealthCheckConfig, HealthCheckEvent, HealthStatus};
use std::time::Duration;
use uuid::Uuid;

proptest! {
    /// Property: HealthCheckConfig builder preserves interval
    #[test]
    fn health_check_config_preserves_interval(
        secs in 1u64..3600,
    ) {
        let config = HealthCheckConfig::new()
            .with_interval_secs(secs);

        prop_assert_eq!(config.interval, Duration::from_secs(secs));
        prop_assert!(config.enabled);
    }

    /// Property: HealthCheckConfig with_interval matches with_interval_secs
    #[test]
    fn interval_methods_equivalent(
        secs in 1u64..3600,
    ) {
        let config1 = HealthCheckConfig::new()
            .with_interval_secs(secs);
        let config2 = HealthCheckConfig::new()
            .with_interval(Duration::from_secs(secs));

        prop_assert_eq!(config1.interval, config2.interval);
    }

    /// Property: Disabled config has enabled = false
    #[test]
    fn disabled_config_not_enabled(_dummy in 0..1) {
        let config = HealthCheckConfig::disabled();
        prop_assert!(!config.enabled);
    }

    /// Property: Auto-cleanup setting is preserved
    #[test]
    fn auto_cleanup_preserved(
        enabled in any::<bool>(),
    ) {
        let config = HealthCheckConfig::new()
            .with_auto_cleanup(enabled);

        prop_assert_eq!(config.auto_cleanup, enabled);
    }

    /// Property: HealthCheckEvent preserves all fields
    #[test]
    fn health_check_event_preserves_fields(
        session_name in "[a-zA-Z][a-zA-Z0-9 _-]{0,30}",
    ) {
        let session_id = Uuid::new_v4();
        let connection_id = Uuid::new_v4();
        let previous = HealthStatus::Healthy;
        let current = HealthStatus::Unhealthy("test error".to_string());
        let checked_at = chrono::Utc::now();

        let event = HealthCheckEvent {
            session_id,
            connection_id,
            session_name: session_name.clone(),
            previous_status: previous.clone(),
            current_status: current.clone(),
            checked_at,
        };

        prop_assert_eq!(event.session_id, session_id);
        prop_assert_eq!(event.connection_id, connection_id);
        prop_assert_eq!(event.session_name, session_name);
        prop_assert_eq!(event.previous_status, previous);
        prop_assert_eq!(event.current_status, current);
        prop_assert_eq!(event.checked_at, checked_at);
    }

    /// Property: HealthStatus equality works correctly
    #[test]
    fn health_status_equality(
        error_msg in "[a-zA-Z0-9 ]{0,50}",
    ) {
        // Healthy equals Healthy
        prop_assert_eq!(HealthStatus::Healthy, HealthStatus::Healthy);

        // Unknown equals Unknown
        prop_assert_eq!(HealthStatus::Unknown, HealthStatus::Unknown);

        // Terminated equals Terminated
        prop_assert_eq!(HealthStatus::Terminated, HealthStatus::Terminated);

        // Unhealthy with same message equals
        let unhealthy1 = HealthStatus::Unhealthy(error_msg.clone());
        let unhealthy2 = HealthStatus::Unhealthy(error_msg);
        prop_assert_eq!(unhealthy1, unhealthy2);

        // Different statuses are not equal
        prop_assert_ne!(HealthStatus::Healthy, HealthStatus::Unknown);
        prop_assert_ne!(HealthStatus::Healthy, HealthStatus::Terminated);
    }

    /// Property: HealthStatus clone works correctly
    #[test]
    fn health_status_clone(
        error_msg in "[a-zA-Z0-9 ]{0,50}",
    ) {
        let statuses = [
            HealthStatus::Healthy,
            HealthStatus::Unknown,
            HealthStatus::Terminated,
            HealthStatus::Unhealthy(error_msg),
        ];

        for status in &statuses {
            let cloned = status.clone();
            prop_assert_eq!(status, &cloned);
        }
    }
}

#[test]
fn test_health_check_config_default() {
    let config = HealthCheckConfig::default();
    assert!(config.enabled);
    assert!(!config.auto_cleanup);
    assert!(config.interval.as_secs() > 0);
}

#[test]
fn test_health_check_config_new_equals_default() {
    let new_config = HealthCheckConfig::new();
    let default_config = HealthCheckConfig::default();

    assert_eq!(new_config.enabled, default_config.enabled);
    assert_eq!(new_config.interval, default_config.interval);
    assert_eq!(new_config.auto_cleanup, default_config.auto_cleanup);
}

#[test]
fn test_health_status_debug() {
    // Verify Debug trait is implemented
    let healthy = HealthStatus::Healthy;
    let debug_str = format!("{healthy:?}");
    assert!(debug_str.contains("Healthy"));

    let unhealthy = HealthStatus::Unhealthy("connection lost".to_string());
    let debug_str = format!("{unhealthy:?}");
    assert!(debug_str.contains("Unhealthy"));
    assert!(debug_str.contains("connection lost"));
}

#[test]
fn test_health_check_event_debug() {
    let event = HealthCheckEvent {
        session_id: Uuid::new_v4(),
        connection_id: Uuid::new_v4(),
        session_name: "test-session".to_string(),
        previous_status: HealthStatus::Healthy,
        current_status: HealthStatus::Terminated,
        checked_at: chrono::Utc::now(),
    };

    let debug_str = format!("{event:?}");
    assert!(debug_str.contains("HealthCheckEvent"));
    assert!(debug_str.contains("test-session"));
}

#[test]
fn test_health_status_unhealthy_different_messages() {
    let status1 = HealthStatus::Unhealthy("error 1".to_string());
    let status2 = HealthStatus::Unhealthy("error 2".to_string());

    assert_ne!(status1, status2);
}

#[test]
fn test_config_chaining() {
    let config = HealthCheckConfig::new()
        .with_interval_secs(60)
        .with_auto_cleanup(true);

    assert!(config.enabled);
    assert_eq!(config.interval, Duration::from_secs(60));
    assert!(config.auto_cleanup);
}
