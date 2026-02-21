//! Property-based tests for progress reporting functionality
//!
//! Tests correctness properties for progress reporting during long operations.

use proptest::prelude::*;
use rustconn_core::progress::{CallbackProgressReporter, NoOpProgressReporter, ProgressReporter};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Strategy for generating a reasonable number of items to process
fn arb_item_count() -> impl Strategy<Value = usize> {
    1usize..100
}

/// Strategy for generating progress messages
fn arb_message() -> impl Strategy<Value = String> {
    prop::string::string_regex("[A-Za-z][A-Za-z0-9 _-]{0,50}")
        .unwrap()
        .prop_filter("message must not be empty", |s| !s.is_empty())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn-enhancements, Property 5: Progress Reporter Invocation**
    /// **Validates: Requirements 3.1, 3.3**
    ///
    /// For any import operation with N items, the progress reporter should be called
    /// at least N times with monotonically increasing current values from 0 to N.
    #[test]
    fn prop_progress_reporter_invocation_count(
        item_count in arb_item_count(),
        messages in prop::collection::vec(arb_message(), 1..10)
    ) {
        let call_count = Arc::new(AtomicUsize::new(0));
        let last_current = Arc::new(AtomicUsize::new(0));
        let monotonic_violation = Arc::new(AtomicUsize::new(0));

        let call_count_clone = Arc::clone(&call_count);
        let last_current_clone = Arc::clone(&last_current);
        let monotonic_violation_clone = Arc::clone(&monotonic_violation);

        let reporter = CallbackProgressReporter::new(move |current, _total, _message| {
            let prev = last_current_clone.swap(current, Ordering::SeqCst);
            // Check monotonicity (current should be >= previous, allowing for reset to 0)
            if current < prev && current != 0 {
                monotonic_violation_clone.fetch_add(1, Ordering::SeqCst);
            }
            call_count_clone.fetch_add(1, Ordering::SeqCst);
        });

        // Simulate processing N items
        for i in 0..item_count {
            let msg_idx = i % messages.len();
            reporter.report(i, item_count, &messages[msg_idx]);
        }

        // Property: Reporter should be called exactly item_count times
        prop_assert_eq!(
            call_count.load(Ordering::SeqCst),
            item_count,
            "Expected {} calls, got {}",
            item_count,
            call_count.load(Ordering::SeqCst)
        );

        // Property: Current values should be monotonically increasing
        prop_assert_eq!(
            monotonic_violation.load(Ordering::SeqCst),
            0,
            "Monotonicity violated {} times",
            monotonic_violation.load(Ordering::SeqCst)
        );
    }

    /// **Feature: rustconn-enhancements, Property 5: Progress Reporter Invocation**
    /// **Validates: Requirements 3.1, 3.3**
    ///
    /// For any progress reporter, the total value passed should remain consistent
    /// throughout the operation.
    #[test]
    fn prop_progress_reporter_total_consistency(
        item_count in arb_item_count(),
    ) {
        let totals_seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let totals_clone = Arc::clone(&totals_seen);

        let reporter = CallbackProgressReporter::new(move |_current, total, _message| {
            totals_clone.lock().unwrap().push(total);
        });

        // Simulate processing N items
        for i in 0..item_count {
            reporter.report(i, item_count, "Processing");
        }

        let totals = totals_seen.lock().unwrap();

        // Property: All total values should be the same
        prop_assert!(
            totals.iter().all(|&t| t == item_count),
            "Total values should be consistent: expected all {}, got {:?}",
            item_count,
            *totals
        );
    }

    /// **Feature: rustconn-enhancements, Property 5: Progress Reporter Invocation**
    /// **Validates: Requirements 3.1, 3.3**
    ///
    /// For any progress reporter, current should always be less than or equal to total.
    #[test]
    fn prop_progress_reporter_current_bounded_by_total(
        item_count in arb_item_count(),
    ) {
        let violations = Arc::new(AtomicUsize::new(0));
        let violations_clone = Arc::clone(&violations);

        let reporter = CallbackProgressReporter::new(move |current, total, _message| {
            if current > total {
                violations_clone.fetch_add(1, Ordering::SeqCst);
            }
        });

        // Simulate processing N items (0 to N-1)
        for i in 0..item_count {
            reporter.report(i, item_count, "Processing");
        }

        // Property: current should never exceed total
        prop_assert_eq!(
            violations.load(Ordering::SeqCst),
            0,
            "Current exceeded total {} times",
            violations.load(Ordering::SeqCst)
        );
    }

    /// **Feature: rustconn-enhancements, Property 5: Progress Reporter Invocation**
    /// **Validates: Requirements 3.5**
    ///
    /// For any progress reporter, cancellation should be respected and is_cancelled
    /// should return true after cancel is called.
    #[test]
    fn prop_progress_reporter_cancellation(
        cancel_at in 0usize..50,
        item_count in 50usize..100,
    ) {
        let items_processed = Arc::new(AtomicUsize::new(0));
        let items_clone = Arc::clone(&items_processed);

        let reporter = CallbackProgressReporter::new(move |_current, _total, _message| {
            items_clone.fetch_add(1, Ordering::SeqCst);
        });

        // Property: is_cancelled should be false initially
        prop_assert!(!reporter.is_cancelled(), "Should not be cancelled initially");

        // Simulate processing with cancellation
        for i in 0..item_count {
            if reporter.is_cancelled() {
                break;
            }
            reporter.report(i, item_count, "Processing");

            if i == cancel_at {
                reporter.cancel();
            }
        }

        // Property: is_cancelled should be true after cancel
        prop_assert!(reporter.is_cancelled(), "Should be cancelled after cancel()");

        // Property: Processing should have stopped at or shortly after cancel_at
        let processed = items_processed.load(Ordering::SeqCst);
        prop_assert!(
            processed <= cancel_at + 2,
            "Should have stopped near cancel point. Processed: {}, cancel_at: {}",
            processed,
            cancel_at
        );
    }

    /// **Feature: rustconn-enhancements, Property 5: Progress Reporter Invocation**
    /// **Validates: Requirements 3.5**
    ///
    /// For any progress reporter, the cancel handle should work from a separate context.
    #[test]
    fn prop_progress_reporter_cancel_handle(
        _dummy in 0usize..10, // Proptest requires at least one input
    ) {
        let reporter = CallbackProgressReporter::new(|_current, _total, _message| {});
        let handle = reporter.cancel_handle();

        // Property: Both should report not cancelled initially
        prop_assert!(!reporter.is_cancelled());
        prop_assert!(!handle.is_cancelled());

        // Cancel via handle
        handle.cancel();

        // Property: Both should report cancelled
        prop_assert!(reporter.is_cancelled(), "Reporter should be cancelled via handle");
        prop_assert!(handle.is_cancelled(), "Handle should report cancelled");
    }

    /// **Feature: rustconn-enhancements, Property 5: Progress Reporter Invocation**
    /// **Validates: Requirements 3.1**
    ///
    /// NoOpProgressReporter should never report cancelled and should accept any input.
    #[test]
    fn prop_noop_reporter_never_cancelled(
        current in 0usize..1000,
        total in 1usize..1000,
        message in arb_message(),
    ) {
        let reporter = NoOpProgressReporter::new();

        // Should not panic with any valid input
        reporter.report(current, total, &message);

        // Property: Should never be cancelled
        prop_assert!(!reporter.is_cancelled(), "NoOp reporter should never be cancelled");
    }
}

// Remove the unused item_count warning by removing the unused test parameter
// The cancel_handle test doesn't need item_count
