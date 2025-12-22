//! Property-based tests for connection testing module
//!
//! These tests validate the correctness properties defined in the design document
//! for the connection testing functionality.

// Allow precision loss for percentage calculations in tests
#![allow(clippy::cast_precision_loss)]

use proptest::prelude::*;
use rustconn_core::testing::{TestError, TestResult, TestSummary};
use uuid::Uuid;

/// Strategy for generating test errors
fn test_error_strategy() -> impl Strategy<Value = TestError> {
    prop_oneof![
        (1u64..=3600u64).prop_map(TestError::Timeout),
        Just(TestError::ConnectionRefused),
        "[a-z0-9.-]{1,50}".prop_map(|s| TestError::HostUnreachable(s)),
        "[a-z0-9.-]{1,50}".prop_map(|s| TestError::DnsResolutionFailed(s)),
        "[a-zA-Z0-9 ]{1,100}".prop_map(|s| TestError::ProtocolError(s)),
        "[a-zA-Z0-9 ]{1,100}".prop_map(|s| TestError::IoError(s)),
        "[a-zA-Z0-9 ]{1,100}".prop_map(|s| TestError::InvalidConfig(s)),
    ]
}

/// Strategy for generating connection names
fn connection_name_strategy() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9_-]{0,30}".prop_map(|s| s)
}

/// Strategy for generating test results
fn test_result_strategy() -> impl Strategy<Value = TestResult> {
    prop_oneof![
        // Successful results
        (any::<u128>(), connection_name_strategy(), 0u64..10000u64).prop_map(
            |(id_bits, name, latency)| {
                let id = Uuid::from_u128(id_bits);
                TestResult::success(id, name, latency)
            }
        ),
        // Failed results
        (
            any::<u128>(),
            connection_name_strategy(),
            test_error_strategy()
        )
            .prop_map(|(id_bits, name, error)| {
                let id = Uuid::from_u128(id_bits);
                TestResult::from_error(id, name, &error)
            }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: ssh-agent-cli, Property 16: Test Result Error Details**
    /// **Validates: Requirements 8.5**
    ///
    /// For any failed connection test, the result should include a descriptive
    /// error message explaining the failure.
    #[test]
    fn test_failed_result_contains_error_details(
        id_bits in any::<u128>(),
        name in connection_name_strategy(),
        error in test_error_strategy(),
    ) {
        let id = Uuid::from_u128(id_bits);
        let result = TestResult::from_error(id, name.clone(), &error);

        // Property: Failed results must have an error message
        prop_assert!(!result.success, "Result should be marked as failed");
        prop_assert!(result.error.is_some(), "Failed result must have an error message");

        // Property: Error message must be non-empty and descriptive
        let error_msg = result.error.as_ref().unwrap();
        prop_assert!(!error_msg.is_empty(), "Error message must not be empty");

        // Property: Error message should match the original error
        let expected_msg = error.to_string();
        prop_assert_eq!(
            error_msg,
            &expected_msg,
            "Error message should match the TestError display"
        );

        // Property: Connection ID and name should be preserved
        prop_assert_eq!(result.connection_id, id);
        prop_assert_eq!(result.connection_name, name);

        // Property: Latency should be None for failed results
        prop_assert!(
            result.latency_ms.is_none(),
            "Failed results should not have latency"
        );
    }

    /// Additional property: Successful results should have latency but no error
    #[test]
    fn test_successful_result_has_latency_no_error(
        id_bits in any::<u128>(),
        name in connection_name_strategy(),
        latency in 0u64..10000u64,
    ) {
        let id = Uuid::from_u128(id_bits);
        let result = TestResult::success(id, name.clone(), latency);

        // Property: Successful results must have latency
        prop_assert!(result.success, "Result should be marked as successful");
        prop_assert!(result.latency_ms.is_some(), "Successful result must have latency");
        prop_assert_eq!(result.latency_ms.unwrap(), latency);

        // Property: Successful results should not have an error
        prop_assert!(result.error.is_none(), "Successful result should not have error");

        // Property: Connection ID and name should be preserved
        prop_assert_eq!(result.connection_id, id);
        prop_assert_eq!(result.connection_name, name);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: ssh-agent-cli, Property 17: Batch Test Summary Accuracy**
    /// **Validates: Requirements 8.6**
    ///
    /// For any batch test of N connections, the summary should report exactly N
    /// total tests with passed + failed = N.
    #[test]
    fn test_batch_summary_accuracy(
        results in prop::collection::vec(test_result_strategy(), 0..50),
    ) {
        let n = results.len();
        let expected_passed = results.iter().filter(|r| r.success).count();
        let expected_failed = n - expected_passed;

        let summary = TestSummary::from_results(results);

        // Property: Total must equal the number of input results
        prop_assert_eq!(
            summary.total, n,
            "Total should equal number of input results"
        );

        // Property: passed + failed must equal total
        prop_assert_eq!(
            summary.passed + summary.failed, summary.total,
            "passed + failed must equal total"
        );

        // Property: passed count must match actual successful results
        prop_assert_eq!(
            summary.passed, expected_passed,
            "Passed count should match actual successful results"
        );

        // Property: failed count must match actual failed results
        prop_assert_eq!(
            summary.failed, expected_failed,
            "Failed count should match actual failed results"
        );

        // Property: results vector should contain all input results
        prop_assert_eq!(
            summary.results.len(), n,
            "Results vector should contain all input results"
        );
    }

    /// Property: Adding results one by one should produce same summary as from_results
    #[test]
    fn test_summary_add_result_consistency(
        results in prop::collection::vec(test_result_strategy(), 0..20),
    ) {
        // Create summary using from_results
        let summary_batch = TestSummary::from_results(results.clone());

        // Create summary by adding results one by one
        let mut summary_incremental = TestSummary::new();
        for result in results {
            summary_incremental.add_result(result);
        }

        // Property: Both methods should produce identical counts
        prop_assert_eq!(
            summary_batch.total, summary_incremental.total,
            "Total should match between batch and incremental"
        );
        prop_assert_eq!(
            summary_batch.passed, summary_incremental.passed,
            "Passed should match between batch and incremental"
        );
        prop_assert_eq!(
            summary_batch.failed, summary_incremental.failed,
            "Failed should match between batch and incremental"
        );
    }

    /// Property: Pass rate calculation should be accurate
    #[test]
    fn test_summary_pass_rate_accuracy(
        results in prop::collection::vec(test_result_strategy(), 1..50),
    ) {
        let summary = TestSummary::from_results(results);

        // Calculate expected pass rate
        let expected_rate = if summary.total == 0 {
            100.0
        } else {
            (summary.passed as f64 / summary.total as f64) * 100.0
        };

        let actual_rate = summary.pass_rate();

        // Property: Pass rate should be accurate within floating point tolerance
        prop_assert!(
            (actual_rate - expected_rate).abs() < 0.0001,
            "Pass rate should be accurate: expected {}, got {}",
            expected_rate,
            actual_rate
        );

        // Property: Pass rate should be between 0 and 100
        prop_assert!(
            (0.0..=100.0).contains(&actual_rate),
            "Pass rate should be between 0 and 100"
        );
    }

    /// Property: all_passed and has_failures should be consistent
    #[test]
    fn test_summary_status_consistency(
        results in prop::collection::vec(test_result_strategy(), 0..20),
    ) {
        let summary = TestSummary::from_results(results);

        // Property: all_passed should be true iff failed == 0
        prop_assert_eq!(
            summary.all_passed(),
            summary.failed == 0,
            "all_passed should be true iff failed == 0"
        );

        // Property: has_failures should be true iff failed > 0
        prop_assert_eq!(
            summary.has_failures(),
            summary.failed > 0,
            "has_failures should be true iff failed > 0"
        );

        // Property: all_passed and has_failures should be mutually exclusive
        // (except when total == 0, where all_passed is true and has_failures is false)
        if summary.total > 0 {
            prop_assert_ne!(
                summary.all_passed(),
                summary.has_failures(),
                "all_passed and has_failures should be mutually exclusive when total > 0"
            );
        }
    }

    /// Property: failed_results and successful_results should partition results
    #[test]
    fn test_summary_result_partitioning(
        results in prop::collection::vec(test_result_strategy(), 0..20),
    ) {
        let summary = TestSummary::from_results(results);

        let failed = summary.failed_results();
        let successful = summary.successful_results();

        // Property: failed + successful should equal total results
        prop_assert_eq!(
            failed.len() + successful.len(),
            summary.results.len(),
            "failed + successful should equal total results"
        );

        // Property: All failed results should have success == false
        for result in &failed {
            prop_assert!(!result.success, "Failed results should have success == false");
        }

        // Property: All successful results should have success == true
        for result in &successful {
            prop_assert!(result.success, "Successful results should have success == true");
        }
    }
}
