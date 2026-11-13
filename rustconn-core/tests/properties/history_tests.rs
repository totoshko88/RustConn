//! Property tests for connection history and statistics models

use proptest::prelude::*;
use rustconn_core::models::{ConnectionHistoryEntry, ConnectionStatistics, HistorySettings};
use uuid::Uuid;

/// Strategy for generating valid protocol names
fn protocol_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("ssh".to_string()),
        Just("rdp".to_string()),
        Just("vnc".to_string()),
        Just("spice".to_string()),
    ]
}

/// Strategy for generating valid port numbers
fn port_strategy() -> impl Strategy<Value = u16> {
    prop_oneof![
        Just(22_u16),   // SSH
        Just(3389_u16), // RDP
        Just(5900_u16), // VNC
        Just(5930_u16), // SPICE
        1..=65535_u16,  // Any valid port
    ]
}

/// Strategy for generating valid hostnames
fn host_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("localhost".to_string()),
        Just("127.0.0.1".to_string()),
        "[a-z]{3,10}\\.[a-z]{2,4}",
        "[a-z]{3,10}-[0-9]{1,3}\\.[a-z]{2,4}",
    ]
}

/// Strategy for generating optional usernames
fn username_strategy() -> impl Strategy<Value = Option<String>> {
    prop_oneof![
        Just(None),
        "[a-z]{3,12}".prop_map(Some),
        Just(Some("root".to_string())),
        Just(Some("admin".to_string())),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// History entry creation always produces valid entry
    #[test]
    fn prop_history_entry_creation_valid(
        host in host_strategy(),
        port in port_strategy(),
        protocol in protocol_strategy(),
        username in username_strategy(),
    ) {
        let connection_id = Uuid::new_v4();
        let entry = ConnectionHistoryEntry::new(
            connection_id,
            "Test Connection".to_string(),
            host.clone(),
            port,
            protocol.clone(),
            username.clone(),
        );

        prop_assert_eq!(entry.connection_id, connection_id);
        prop_assert_eq!(&entry.host, &host);
        prop_assert_eq!(entry.port, port);
        prop_assert_eq!(&entry.protocol, &protocol);
        prop_assert_eq!(&entry.username, &username);
        prop_assert!(entry.successful);
        prop_assert!(entry.ended_at.is_none());
        prop_assert!(entry.error_message.is_none());
        prop_assert!(!entry.is_quick_connect());
    }

    /// Quick connect entries have nil connection_id
    #[test]
    fn prop_quick_connect_has_nil_id(
        host in host_strategy(),
        port in port_strategy(),
        protocol in protocol_strategy(),
        username in username_strategy(),
    ) {
        let entry = ConnectionHistoryEntry::new_quick_connect(
            host,
            port,
            protocol,
            username,
        );

        prop_assert!(entry.is_quick_connect());
        prop_assert!(entry.connection_id.is_nil());
        prop_assert!(entry.connection_name.starts_with("Quick:"));
    }

    /// Entry end() sets ended_at and calculates duration
    #[test]
    fn prop_entry_end_sets_duration(
        host in host_strategy(),
        port in port_strategy(),
        protocol in protocol_strategy(),
    ) {
        let mut entry = ConnectionHistoryEntry::new(
            Uuid::new_v4(),
            "Test".to_string(),
            host,
            port,
            protocol,
            None,
        );

        entry.end();

        prop_assert!(entry.ended_at.is_some());
        prop_assert!(entry.duration_seconds.is_some());
        prop_assert!(entry.duration_seconds.unwrap() >= 0);
        prop_assert!(entry.successful);
    }

    /// Entry fail() marks as unsuccessful and sets error
    #[test]
    fn prop_entry_fail_sets_error(
        host in host_strategy(),
        error_msg in "[a-zA-Z ]{5,50}",
    ) {
        let mut entry = ConnectionHistoryEntry::new(
            Uuid::new_v4(),
            "Test".to_string(),
            host,
            22,
            "ssh".to_string(),
            None,
        );

        entry.fail(&error_msg);

        prop_assert!(!entry.successful);
        prop_assert!(entry.error_message.is_some());
        prop_assert_eq!(entry.error_message.as_ref().unwrap(), &error_msg);
        prop_assert!(entry.ended_at.is_some());
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Statistics update maintains consistency
    #[test]
    fn prop_statistics_update_consistent(
        successful_count in 1..20_u32,
        failed_count in 0..10_u32,
    ) {
        let connection_id = Uuid::new_v4();
        let mut stats = ConnectionStatistics::new(connection_id);

        // Add successful entries
        for i in 0..successful_count {
            let mut entry = ConnectionHistoryEntry::new(
                connection_id,
                "Test".to_string(),
                "host".to_string(),
                22,
                "ssh".to_string(),
                None,
            );
            entry.duration_seconds = Some(i64::from(i + 1) * 60); // 1-N minutes
            entry.ended_at = Some(chrono::Utc::now());
            stats.update_from_entry(&entry);
        }

        // Add failed entries
        for _ in 0..failed_count {
            let mut entry = ConnectionHistoryEntry::new(
                connection_id,
                "Test".to_string(),
                "host".to_string(),
                22,
                "ssh".to_string(),
                None,
            );
            entry.fail("Connection refused");
            stats.update_from_entry(&entry);
        }

        let total = successful_count + failed_count;
        prop_assert_eq!(stats.total_connections, total);
        prop_assert_eq!(stats.successful_connections, successful_count);
        prop_assert_eq!(stats.failed_connections, failed_count);
        prop_assert!(stats.last_connected.is_some());
        prop_assert!(stats.first_connected.is_some());
    }

    /// Success rate is always between 0 and 100
    #[test]
    fn prop_success_rate_bounded(
        successful in 0..100_u32,
        failed in 0..100_u32,
    ) {
        let mut stats = ConnectionStatistics::new(Uuid::new_v4());
        stats.total_connections = successful + failed;
        stats.successful_connections = successful;
        stats.failed_connections = failed;

        let rate = stats.success_rate();
        prop_assert!(rate >= 0.0);
        prop_assert!(rate <= 100.0);

        if stats.total_connections > 0 {
            let expected = f64::from(successful) / f64::from(successful + failed) * 100.0;
            prop_assert!((rate - expected).abs() < 0.001);
        }
    }

    /// Duration formatting produces valid output
    #[test]
    fn prop_format_duration_valid(seconds in 0..86400_i64 * 7) {
        let formatted = ConnectionStatistics::format_duration(seconds);

        prop_assert!(!formatted.is_empty());

        // Should contain time unit
        prop_assert!(
            formatted.contains('s') || formatted.contains('m') || formatted.contains('h'),
            "Duration '{}' should contain time unit", formatted
        );
    }

    /// History settings have valid defaults
    #[test]
    fn prop_history_settings_defaults_valid(
        max_entries in 100..10000_usize,
        retention_days in 7..365_u32,
    ) {
        let settings = HistorySettings {
            enabled: true,
            max_entries,
            retention_days,
        };

        prop_assert!(settings.max_entries > 0);
        prop_assert!(settings.retention_days > 0);
    }

    /// Serialization round-trip preserves history entry
    #[test]
    fn prop_history_entry_serialization_roundtrip(
        host in host_strategy(),
        port in port_strategy(),
        protocol in protocol_strategy(),
    ) {
        let entry = ConnectionHistoryEntry::new(
            Uuid::new_v4(),
            "Test Connection".to_string(),
            host,
            port,
            protocol,
            Some("user".to_string()),
        );

        let json = serde_json::to_string(&entry).unwrap();
        let restored: ConnectionHistoryEntry = serde_json::from_str(&json).unwrap();

        prop_assert_eq!(entry.id, restored.id);
        prop_assert_eq!(entry.connection_id, restored.connection_id);
        prop_assert_eq!(entry.host, restored.host);
        prop_assert_eq!(entry.port, restored.port);
        prop_assert_eq!(entry.protocol, restored.protocol);
        prop_assert_eq!(entry.username, restored.username);
    }

    /// Serialization round-trip preserves statistics
    #[test]
    fn prop_statistics_serialization_roundtrip(
        total in 0..1000_u32,
        successful in 0..1000_u32,
        duration in 0..86400_i64 * 30,
    ) {
        let successful = successful.min(total);
        let mut stats = ConnectionStatistics::new(Uuid::new_v4());
        stats.total_connections = total;
        stats.successful_connections = successful;
        stats.failed_connections = total - successful;
        stats.total_duration_seconds = duration;

        let json = serde_json::to_string(&stats).unwrap();
        let restored: ConnectionStatistics = serde_json::from_str(&json).unwrap();

        prop_assert_eq!(stats.connection_id, restored.connection_id);
        prop_assert_eq!(stats.total_connections, restored.total_connections);
        prop_assert_eq!(stats.successful_connections, restored.successful_connections);
        prop_assert_eq!(stats.total_duration_seconds, restored.total_duration_seconds);
    }

    /// Serialization round-trip preserves history settings
    #[test]
    fn prop_history_settings_serialization_roundtrip(
        enabled in proptest::bool::ANY,
        max_entries in 100..10000_usize,
        retention_days in 7..365_u32,
    ) {
        let settings = HistorySettings {
            enabled,
            max_entries,
            retention_days,
        };

        let json = serde_json::to_string(&settings).unwrap();
        let restored: HistorySettings = serde_json::from_str(&json).unwrap();

        prop_assert_eq!(settings, restored);
    }
}
