//! Property-based tests for dashboard functionality
//!
//! These tests validate the correctness properties for session statistics
//! and dashboard filtering as defined in the design document.
//!
//! **Feature: rustconn-enhancements**

use proptest::prelude::*;
use uuid::Uuid;

use rustconn_core::dashboard::{DashboardFilter, SessionStats};
use rustconn_core::session::SessionState;

// ============================================================================
// Strategies for generating test data
// ============================================================================

/// Strategy for generating valid protocol names
fn arb_protocol() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("ssh".to_string()),
        Just("rdp".to_string()),
        Just("vnc".to_string()),
        Just("spice".to_string()),
    ]
}

/// Strategy for generating session states
fn arb_session_state() -> impl Strategy<Value = SessionState> {
    prop_oneof![
        Just(SessionState::Starting),
        Just(SessionState::Active),
        Just(SessionState::Disconnecting),
        Just(SessionState::Terminated),
        Just(SessionState::Error),
    ]
}

/// Strategy for generating connection names
fn arb_connection_name() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9_-]{0,30}".prop_map(|s| s.to_string())
}

/// Strategy for generating host addresses
fn arb_host() -> impl Strategy<Value = String> {
    prop_oneof![
        "[a-z0-9]([a-z0-9-]{0,15}[a-z0-9])?(\\.[a-z0-9]([a-z0-9-]{0,15}[a-z0-9])?)*",
        "192\\.168\\.[0-9]{1,3}\\.[0-9]{1,3}",
        Just("localhost".to_string()),
    ]
}

/// Strategy for generating byte counts
fn arb_bytes() -> impl Strategy<Value = u64> {
    0u64..10_000_000_000u64 // Up to 10 GB
}

/// Strategy for generating session stats
fn arb_session_stats() -> impl Strategy<Value = SessionStats> {
    (
        arb_connection_name(),
        arb_protocol(),
        arb_host(),
        arb_session_state(),
        arb_bytes(),
        arb_bytes(),
        any::<bool>(), // has group
    )
        .prop_map(
            |(name, protocol, host, state, bytes_sent, bytes_received, has_group)| {
                let mut stats =
                    SessionStats::new(Uuid::new_v4(), Uuid::new_v4(), name, protocol, host);
                stats.state = state;
                stats.bytes_sent = bytes_sent;
                stats.bytes_received = bytes_received;
                if has_group {
                    stats.group_id = Some(Uuid::new_v4());
                }
                stats
            },
        )
}

/// Strategy for generating dashboard filters
fn arb_filter() -> impl Strategy<Value = DashboardFilter> {
    (
        prop::option::of(arb_protocol()),
        prop::option::of(any::<[u8; 16]>().prop_map(|b| Uuid::from_bytes(b))),
        prop::option::of(arb_session_state()),
    )
        .prop_map(|(protocol, group_id, status)| {
            DashboardFilter::new()
                .with_protocol(protocol)
                .with_group(group_id)
                .with_status(status)
        })
}

// ============================================================================
// Property 31: Dashboard Session Statistics
// **Validates: Requirements 13.2**
//
// For any active session, the dashboard should display accurate duration
// and byte counts.
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn-enhancements, Property 31: Dashboard Session Statistics**
    /// **Validates: Requirements 13.2**
    ///
    /// For any session stats, duration should be non-negative.
    #[test]
    fn prop_session_duration_is_non_negative(
        name in arb_connection_name(),
        protocol in arb_protocol(),
        host in arb_host()
    ) {
        let stats = SessionStats::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            name,
            protocol,
            host,
        );

        // Duration should be non-negative (session just started)
        prop_assert!(
            stats.duration_seconds() >= 0,
            "Duration should be non-negative, got {}",
            stats.duration_seconds()
        );
    }

    /// **Feature: rustconn-enhancements, Property 31: Dashboard Session Statistics**
    /// **Validates: Requirements 13.2**
    ///
    /// For any byte count, format_bytes should produce a valid human-readable string.
    #[test]
    fn prop_format_bytes_produces_valid_string(bytes in arb_bytes()) {
        let formatted = SessionStats::format_bytes(bytes);

        // Should not be empty
        prop_assert!(!formatted.is_empty(), "Formatted bytes should not be empty");

        // Should end with a unit
        prop_assert!(
            formatted.ends_with(" B") ||
            formatted.ends_with(" KB") ||
            formatted.ends_with(" MB") ||
            formatted.ends_with(" GB"),
            "Formatted bytes '{}' should end with a unit", formatted
        );

        // Should contain a number
        let has_number = formatted.chars().any(|c| c.is_ascii_digit());
        prop_assert!(has_number, "Formatted bytes '{}' should contain a number", formatted);
    }

    /// **Feature: rustconn-enhancements, Property 31: Dashboard Session Statistics**
    /// **Validates: Requirements 13.2**
    ///
    /// For any session stats, byte counts should be accurately tracked.
    #[test]
    fn prop_byte_counts_are_accurate(
        name in arb_connection_name(),
        protocol in arb_protocol(),
        host in arb_host(),
        initial_sent in 0u64..1_000_000u64,
        initial_received in 0u64..1_000_000u64,
        add_sent in 0u64..1_000_000u64,
        add_received in 0u64..1_000_000u64
    ) {
        let mut stats = SessionStats::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            name,
            protocol,
            host,
        );

        stats.bytes_sent = initial_sent;
        stats.bytes_received = initial_received;

        stats.add_bytes_sent(add_sent);
        stats.add_bytes_received(add_received);

        prop_assert_eq!(
            stats.bytes_sent,
            initial_sent.saturating_add(add_sent),
            "Bytes sent should be accurately tracked"
        );

        prop_assert_eq!(
            stats.bytes_received,
            initial_received.saturating_add(add_received),
            "Bytes received should be accurately tracked"
        );
    }

    /// **Feature: rustconn-enhancements, Property 31: Dashboard Session Statistics**
    /// **Validates: Requirements 13.2**
    ///
    /// For any session state, state_display should return a non-empty string.
    #[test]
    fn prop_state_display_is_valid(state in arb_session_state()) {
        let mut stats = SessionStats::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            "Test".to_string(),
            "ssh".to_string(),
            "localhost".to_string(),
        );
        stats.state = state;

        let display = stats.state_display();

        prop_assert!(!display.is_empty(), "State display should not be empty");
        prop_assert!(
            ["Starting", "Connected", "Disconnecting", "Disconnected", "Error"].contains(&display),
            "State display '{}' should be a valid state name", display
        );
    }

    /// **Feature: rustconn-enhancements, Property 31: Dashboard Session Statistics**
    /// **Validates: Requirements 13.2**
    ///
    /// Duration formatting should produce valid human-readable strings.
    #[test]
    fn prop_format_duration_is_valid(
        name in arb_connection_name(),
        protocol in arb_protocol(),
        host in arb_host()
    ) {
        let stats = SessionStats::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            name,
            protocol,
            host,
        );

        let formatted = stats.format_duration();

        // Should not be empty
        prop_assert!(!formatted.is_empty(), "Formatted duration should not be empty");

        // Should contain time unit
        prop_assert!(
            formatted.contains('s') || formatted.contains('m') || formatted.contains('h'),
            "Formatted duration '{}' should contain a time unit", formatted
        );
    }
}

// ============================================================================
// Property 32: Dashboard Filter Application
// **Validates: Requirements 13.5**
//
// For any dashboard filter, only sessions matching the filter criteria
// should be displayed.
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn-enhancements, Property 32: Dashboard Filter Application**
    /// **Validates: Requirements 13.5**
    ///
    /// An empty filter should match all sessions.
    #[test]
    fn prop_empty_filter_matches_all(stats in arb_session_stats()) {
        let filter = DashboardFilter::new();

        prop_assert!(
            filter.matches(&stats),
            "Empty filter should match all sessions"
        );
    }

    /// **Feature: rustconn-enhancements, Property 32: Dashboard Filter Application**
    /// **Validates: Requirements 13.5**
    ///
    /// Protocol filter should only match sessions with the specified protocol.
    #[test]
    fn prop_protocol_filter_matches_correctly(
        stats in arb_session_stats(),
        filter_protocol in arb_protocol()
    ) {
        let filter = DashboardFilter::new().with_protocol(Some(filter_protocol.clone()));

        let matches = filter.matches(&stats);
        let should_match = stats.protocol == filter_protocol;

        prop_assert_eq!(
            matches, should_match,
            "Protocol filter for '{}' should {} session with protocol '{}'",
            filter_protocol,
            if should_match { "match" } else { "not match" },
            stats.protocol
        );
    }

    /// **Feature: rustconn-enhancements, Property 32: Dashboard Filter Application**
    /// **Validates: Requirements 13.5**
    ///
    /// Status filter should only match sessions with the specified state.
    #[test]
    fn prop_status_filter_matches_correctly(
        stats in arb_session_stats(),
        filter_status in arb_session_state()
    ) {
        let filter = DashboardFilter::new().with_status(Some(filter_status));

        let matches = filter.matches(&stats);
        let should_match = stats.state == filter_status;

        prop_assert_eq!(
            matches, should_match,
            "Status filter for {:?} should {} session with state {:?}",
            filter_status,
            if should_match { "match" } else { "not match" },
            stats.state
        );
    }

    /// **Feature: rustconn-enhancements, Property 32: Dashboard Filter Application**
    /// **Validates: Requirements 13.5**
    ///
    /// Group filter should only match sessions in the specified group.
    #[test]
    fn prop_group_filter_matches_correctly(
        stats in arb_session_stats()
    ) {
        let filter_group_id = Uuid::new_v4();
        let filter = DashboardFilter::new().with_group(Some(filter_group_id));

        let matches = filter.matches(&stats);
        let should_match = stats.group_id == Some(filter_group_id);

        prop_assert_eq!(
            matches, should_match,
            "Group filter should {} session",
            if should_match { "match" } else { "not match" }
        );
    }

    /// **Feature: rustconn-enhancements, Property 32: Dashboard Filter Application**
    /// **Validates: Requirements 13.5**
    ///
    /// Combined filters should require all conditions to match.
    #[test]
    fn prop_combined_filters_require_all_conditions(
        stats in arb_session_stats(),
        filter_protocol in arb_protocol(),
        filter_status in arb_session_state()
    ) {
        let filter = DashboardFilter::new()
            .with_protocol(Some(filter_protocol.clone()))
            .with_status(Some(filter_status));

        let matches = filter.matches(&stats);
        let should_match = stats.protocol == filter_protocol && stats.state == filter_status;

        prop_assert_eq!(
            matches, should_match,
            "Combined filter should require all conditions to match"
        );
    }

    /// **Feature: rustconn-enhancements, Property 32: Dashboard Filter Application**
    /// **Validates: Requirements 13.5**
    ///
    /// Filter apply should return only matching sessions.
    #[test]
    fn prop_filter_apply_returns_only_matches(
        sessions in prop::collection::vec(arb_session_stats(), 0..20),
        filter in arb_filter()
    ) {
        let filtered = filter.apply(&sessions);

        // All filtered sessions should match the filter
        for session in &filtered {
            prop_assert!(
                filter.matches(session),
                "Filtered session should match the filter"
            );
        }

        // Count should match
        let expected_count = sessions.iter().filter(|s| filter.matches(s)).count();
        prop_assert_eq!(
            filtered.len(),
            expected_count,
            "Filtered count should match expected count"
        );
    }

    /// **Feature: rustconn-enhancements, Property 32: Dashboard Filter Application**
    /// **Validates: Requirements 13.5**
    ///
    /// count_matches should equal the length of apply result.
    #[test]
    fn prop_count_matches_equals_apply_length(
        sessions in prop::collection::vec(arb_session_stats(), 0..20),
        filter in arb_filter()
    ) {
        let count = filter.count_matches(&sessions);
        let filtered = filter.apply(&sessions);

        prop_assert_eq!(
            count,
            filtered.len(),
            "count_matches should equal apply().len()"
        );
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[test]
fn test_session_stats_creation() {
    let stats = SessionStats::new(
        Uuid::new_v4(),
        Uuid::new_v4(),
        "Test Connection".to_string(),
        "ssh".to_string(),
        "192.168.1.1".to_string(),
    );

    assert_eq!(stats.connection_name, "Test Connection");
    assert_eq!(stats.protocol, "ssh");
    assert_eq!(stats.host, "192.168.1.1");
    assert_eq!(stats.state, SessionState::Active);
    assert_eq!(stats.bytes_sent, 0);
    assert_eq!(stats.bytes_received, 0);
    assert!(stats.group_id.is_none());
}

#[test]
fn test_format_bytes_boundaries() {
    // Test boundary values
    assert_eq!(SessionStats::format_bytes(0), "0 B");
    assert_eq!(SessionStats::format_bytes(1023), "1023 B");
    assert_eq!(SessionStats::format_bytes(1024), "1.00 KB");
    assert_eq!(SessionStats::format_bytes(1048575), "1024.00 KB");
    assert_eq!(SessionStats::format_bytes(1048576), "1.00 MB");
    assert_eq!(SessionStats::format_bytes(1073741823), "1024.00 MB");
    assert_eq!(SessionStats::format_bytes(1073741824), "1.00 GB");
}

#[test]
fn test_filter_builder_pattern() {
    let filter = DashboardFilter::new()
        .with_protocol(Some("ssh".to_string()))
        .with_status(Some(SessionState::Active));

    assert_eq!(filter.protocol, Some("ssh".to_string()));
    assert_eq!(filter.status, Some(SessionState::Active));
    assert!(filter.group_id.is_none());
}

#[test]
fn test_filter_apply_empty_list() {
    let filter = DashboardFilter::new().with_protocol(Some("ssh".to_string()));
    let sessions: Vec<SessionStats> = vec![];

    let filtered = filter.apply(&sessions);
    assert!(filtered.is_empty());
}
