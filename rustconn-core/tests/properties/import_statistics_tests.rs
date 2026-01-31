//! Property tests for import statistics and skipped field tracking

use proptest::prelude::*;
use rustconn_core::import::{ImportStatistics, SkippedField, SkippedFieldReason};

proptest! {
    /// Property: ImportStatistics starts empty
    #[test]
    fn statistics_starts_empty(_dummy in 0..1) {
        let stats = ImportStatistics::new();
        prop_assert_eq!(stats.total_connections, 0);
        prop_assert_eq!(stats.imported_connections, 0);
        prop_assert_eq!(stats.failed_connections, 0);
        prop_assert_eq!(stats.total_groups, 0);
        prop_assert_eq!(stats.imported_groups, 0);
        prop_assert!(stats.skipped_fields.is_empty());
        prop_assert!(stats.warnings.is_empty());
    }

    /// Property: record_connection_success increments both counters
    #[test]
    fn record_success_increments(count in 1usize..100) {
        let mut stats = ImportStatistics::new();
        for _ in 0..count {
            stats.record_connection_success();
        }
        prop_assert_eq!(stats.total_connections, count);
        prop_assert_eq!(stats.imported_connections, count);
        prop_assert_eq!(stats.failed_connections, 0);
    }

    /// Property: record_connection_failure increments both counters
    #[test]
    fn record_failure_increments(count in 1usize..100) {
        let mut stats = ImportStatistics::new();
        for _ in 0..count {
            stats.record_connection_failure();
        }
        prop_assert_eq!(stats.total_connections, count);
        prop_assert_eq!(stats.imported_connections, 0);
        prop_assert_eq!(stats.failed_connections, count);
    }

    /// Property: record_group_success increments group counters
    #[test]
    fn record_group_increments(count in 1usize..100) {
        let mut stats = ImportStatistics::new();
        for _ in 0..count {
            stats.record_group_success();
        }
        prop_assert_eq!(stats.total_groups, count);
        prop_assert_eq!(stats.imported_groups, count);
    }

    /// Property: success_rate is 100% when no failures
    #[test]
    fn success_rate_100_when_no_failures(count in 1usize..100) {
        let mut stats = ImportStatistics::new();
        for _ in 0..count {
            stats.record_connection_success();
        }
        #[allow(clippy::float_cmp)]
        let is_100 = stats.success_rate() == 100.0;
        prop_assert!(is_100);
    }

    /// Property: success_rate is 0% when all failures
    #[test]
    fn success_rate_0_when_all_failures(count in 1usize..100) {
        let mut stats = ImportStatistics::new();
        for _ in 0..count {
            stats.record_connection_failure();
        }
        #[allow(clippy::float_cmp)]
        let is_0 = stats.success_rate() == 0.0;
        prop_assert!(is_0);
    }

    /// Property: success_rate is 100% when empty
    #[test]
    fn success_rate_100_when_empty(_dummy in 0..1) {
        let stats = ImportStatistics::new();
        #[allow(clippy::float_cmp)]
        let is_100 = stats.success_rate() == 100.0;
        prop_assert!(is_100);
    }

    /// Property: success_rate is between 0 and 100
    #[test]
    fn success_rate_in_range(success in 0usize..100, failure in 0usize..100) {
        let mut stats = ImportStatistics::new();
        for _ in 0..success {
            stats.record_connection_success();
        }
        for _ in 0..failure {
            stats.record_connection_failure();
        }
        let rate = stats.success_rate();
        prop_assert!(rate >= 0.0);
        prop_assert!(rate <= 100.0);
    }

    /// Property: has_skipped_fields is false when empty
    #[test]
    fn no_skipped_fields_initially(_dummy in 0..1) {
        let stats = ImportStatistics::new();
        prop_assert!(!stats.has_skipped_fields());
    }

    /// Property: has_skipped_fields is true after recording
    #[test]
    fn has_skipped_fields_after_record(_dummy in 0..1) {
        let mut stats = ImportStatistics::new();
        stats.record_skipped_field(SkippedField::new("conn", "field", SkippedFieldReason::NotSupported));
        prop_assert!(stats.has_skipped_fields());
    }

    /// Property: has_warnings is false when empty
    #[test]
    fn no_warnings_initially(_dummy in 0..1) {
        let stats = ImportStatistics::new();
        prop_assert!(!stats.has_warnings());
    }

    /// Property: has_warnings is true after recording
    #[test]
    fn has_warnings_after_record(warning in "[a-z ]{10,50}") {
        let mut stats = ImportStatistics::new();
        stats.record_warning(&warning);
        prop_assert!(stats.has_warnings());
        prop_assert_eq!(stats.warnings.len(), 1);
        prop_assert_eq!(&stats.warnings[0], &warning);
    }

    /// Property: skipped_fields_summary groups by reason
    #[test]
    fn skipped_fields_summary_groups(_dummy in 0..1) {
        let mut stats = ImportStatistics::new();
        stats.record_skipped_field(SkippedField::new("c1", "f1", SkippedFieldReason::NotSupported));
        stats.record_skipped_field(SkippedField::new("c2", "f2", SkippedFieldReason::NotSupported));
        stats.record_skipped_field(SkippedField::new("c3", "f3", SkippedFieldReason::InvalidValue));

        let summary = stats.skipped_fields_summary();
        prop_assert_eq!(summary.get(&SkippedFieldReason::NotSupported), Some(&2));
        prop_assert_eq!(summary.get(&SkippedFieldReason::InvalidValue), Some(&1));
    }

    /// Property: skipped_fields_for_connection filters correctly
    #[test]
    fn skipped_fields_for_connection_filters(_dummy in 0..1) {
        let mut stats = ImportStatistics::new();
        stats.record_skipped_field(SkippedField::new("conn1", "f1", SkippedFieldReason::NotSupported));
        stats.record_skipped_field(SkippedField::new("conn1", "f2", SkippedFieldReason::InvalidValue));
        stats.record_skipped_field(SkippedField::new("conn2", "f3", SkippedFieldReason::Deprecated));

        let conn1_fields = stats.skipped_fields_for_connection("conn1");
        prop_assert_eq!(conn1_fields.len(), 2);

        let conn2_fields = stats.skipped_fields_for_connection("conn2");
        prop_assert_eq!(conn2_fields.len(), 1);

        let conn3_fields = stats.skipped_fields_for_connection("conn3");
        prop_assert!(conn3_fields.is_empty());
    }

    /// Property: detailed_report contains key information
    #[test]
    fn detailed_report_contains_info(_dummy in 0..1) {
        let mut stats = ImportStatistics::new();
        stats.record_connection_success();
        stats.record_connection_failure();
        stats.record_group_success();
        stats.record_skipped_field(SkippedField::new("c", "f", SkippedFieldReason::NotSupported));
        stats.record_warning("Test warning");

        let report = stats.detailed_report();
        prop_assert!(report.contains("Import Statistics"));
        prop_assert!(report.contains("Connections"));
        prop_assert!(report.contains("Groups"));
        prop_assert!(report.contains("Skipped Fields"));
        prop_assert!(report.contains("Warnings"));
    }

    /// Property: SkippedFieldReason has description
    #[test]
    fn skipped_field_reason_has_description(_dummy in 0..1) {
        let reasons = [
            SkippedFieldReason::NotSupported,
            SkippedFieldReason::InvalidValue,
            SkippedFieldReason::Deprecated,
            SkippedFieldReason::FeatureUnavailable,
            SkippedFieldReason::Ignored,
            SkippedFieldReason::Unknown,
        ];

        for reason in reasons {
            let desc = reason.description();
            prop_assert!(!desc.is_empty());
        }
    }

    /// Property: SkippedField::new creates correct struct
    #[test]
    fn skipped_field_new_creates_struct(
        conn in "[a-z]{3,10}",
        field in "[a-z]{3,10}",
    ) {
        let sf = SkippedField::new(&conn, &field, SkippedFieldReason::NotSupported);
        prop_assert_eq!(sf.connection_name, conn);
        prop_assert_eq!(sf.field_name, field);
        prop_assert!(sf.original_value.is_none());
        prop_assert_eq!(sf.reason, SkippedFieldReason::NotSupported);
    }

    /// Property: SkippedField::with_value stores value
    #[test]
    fn skipped_field_with_value_stores(
        conn in "[a-z]{3,10}",
        field in "[a-z]{3,10}",
        value in "[a-zA-Z0-9]{5,20}",
    ) {
        let sf = SkippedField::with_value(&conn, &field, &value, SkippedFieldReason::InvalidValue);
        prop_assert_eq!(sf.connection_name, conn);
        prop_assert_eq!(sf.field_name, field);
        prop_assert_eq!(sf.original_value.as_deref(), Some(value.as_str()));
        prop_assert_eq!(sf.reason, SkippedFieldReason::InvalidValue);
    }
}

#[test]
fn test_statistics_default() {
    let stats = ImportStatistics::default();
    assert_eq!(stats.total_connections, 0);
    assert!(!stats.has_skipped_fields());
    assert!(!stats.has_warnings());
}

#[test]
fn test_mixed_success_failure() {
    let mut stats = ImportStatistics::new();
    stats.record_connection_success();
    stats.record_connection_success();
    stats.record_connection_failure();

    assert_eq!(stats.total_connections, 3);
    assert_eq!(stats.imported_connections, 2);
    assert_eq!(stats.failed_connections, 1);

    let rate = stats.success_rate();
    assert!((rate - 66.666_666_666_666_66).abs() < 0.01);
}

#[test]
fn test_all_skipped_field_reasons() {
    let mut stats = ImportStatistics::new();

    stats.record_skipped_field(SkippedField::new(
        "c",
        "f1",
        SkippedFieldReason::NotSupported,
    ));
    stats.record_skipped_field(SkippedField::new(
        "c",
        "f2",
        SkippedFieldReason::InvalidValue,
    ));
    stats.record_skipped_field(SkippedField::new("c", "f3", SkippedFieldReason::Deprecated));
    stats.record_skipped_field(SkippedField::new(
        "c",
        "f4",
        SkippedFieldReason::FeatureUnavailable,
    ));
    stats.record_skipped_field(SkippedField::new("c", "f5", SkippedFieldReason::Ignored));
    stats.record_skipped_field(SkippedField::new("c", "f6", SkippedFieldReason::Unknown));

    assert_eq!(stats.skipped_fields.len(), 6);

    let summary = stats.skipped_fields_summary();
    assert_eq!(summary.len(), 6);
}

#[test]
fn test_detailed_report_empty() {
    let stats = ImportStatistics::new();
    let report = stats.detailed_report();
    assert!(report.contains("100.0%")); // 100% success when empty
    assert!(!report.contains("Skipped Fields:\n")); // No skipped fields section
    assert!(!report.contains("Warnings:\n")); // No warnings section
}
