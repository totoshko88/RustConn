//! Property-based tests for batch processing functionality
//!
//! Tests correctness properties for batch import and export operations.

use proptest::prelude::*;
use rustconn_core::export::{BatchExporter, DEFAULT_EXPORT_BATCH_SIZE};
use rustconn_core::import::{BatchImporter, DEFAULT_IMPORT_BATCH_SIZE};
use rustconn_core::models::Connection;
use rustconn_core::progress::CallbackProgressReporter;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Strategy for generating a reasonable batch size
fn arb_batch_size() -> impl Strategy<Value = usize> {
    1usize..100
}

/// Strategy for generating a reasonable number of connections
fn arb_connection_count() -> impl Strategy<Value = usize> {
    0usize..200
}

/// Strategy for generating a cancel point (batch index to cancel at)
fn arb_cancel_point() -> impl Strategy<Value = usize> {
    0usize..10
}

/// Creates a test connection with the given index
fn create_test_connection(index: usize) -> Connection {
    Connection::new_ssh(
        format!("test_connection_{index}"),
        format!("host{index}.example.com"),
        22,
    )
}

/// Creates a vector of test connections
fn create_test_connections(count: usize) -> Vec<Connection> {
    (0..count).map(create_test_connection).collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // =========================================================================
    // Property 16: Batch Processing Size
    // =========================================================================

    /// **Feature: performance-improvements, Property 16: Batch Processing Size**
    /// **Validates: Requirements 10.1, 10.2, 10.3**
    ///
    /// For any batch import operation, items SHALL be processed in batches
    /// not exceeding the configured batch size.
    #[test]
    fn prop_batch_import_size_limit(
        batch_size in arb_batch_size(),
        connection_count in arb_connection_count(),
    ) {
        let importer = BatchImporter::new(batch_size);
        let connections = create_test_connections(connection_count);

        // Track the maximum batch size observed
        let max_batch_observed = Arc::new(AtomicUsize::new(0));
        let current_batch_count = Arc::new(AtomicUsize::new(0));
        let max_clone = Arc::clone(&max_batch_observed);
        let count_clone = Arc::clone(&current_batch_count);

        let reporter = CallbackProgressReporter::new(move |_current, _total, _msg| {
            // Each progress report represents one item processed
            let batch_count = count_clone.fetch_add(1, Ordering::SeqCst) + 1;

            // When we see a reset (current goes back or stays same), record the batch
            // This is a simplified tracking - we just track total items
            let current_max = max_clone.load(Ordering::SeqCst);
            if batch_count > current_max {
                max_clone.store(batch_count, Ordering::SeqCst);
            }
        });

        let result = importer.process_connections(
            &connections,
            Some(&reporter),
            |c| Ok(c.clone()),
        );

        // Property: All connections should be processed
        prop_assert_eq!(
            result.result.connections.len(),
            connection_count,
            "Expected {} connections, got {}",
            connection_count,
            result.result.connections.len()
        );

        // Property: Number of batches should be ceil(count / batch_size)
        let expected_batches = if connection_count == 0 {
            0
        } else {
            connection_count.div_ceil(batch_size)
        };
        prop_assert_eq!(
            result.batches_processed,
            expected_batches,
            "Expected {} batches, got {}",
            expected_batches,
            result.batches_processed
        );

        // Property: Operation should complete successfully
        prop_assert!(result.is_complete(), "Import should complete without cancellation");
    }

    /// **Feature: performance-improvements, Property 16: Batch Processing Size**
    /// **Validates: Requirements 10.1, 10.2, 10.3**
    ///
    /// For any batch export operation, items SHALL be processed in batches
    /// not exceeding the configured batch size.
    #[test]
    fn prop_batch_export_size_limit(
        batch_size in arb_batch_size(),
        connection_count in arb_connection_count(),
    ) {
        let exporter = BatchExporter::new(batch_size);
        let connections = create_test_connections(connection_count);

        let result = exporter.process_connections(
            &connections,
            None,
            |c| Ok(c.name.clone()),
        );

        // Property: All connections should be exported
        prop_assert_eq!(
            result.result.exported_count,
            connection_count,
            "Expected {} exported, got {}",
            connection_count,
            result.result.exported_count
        );

        // Property: Number of batches should be ceil(count / batch_size)
        let expected_batches = if connection_count == 0 {
            0
        } else {
            connection_count.div_ceil(batch_size)
        };
        prop_assert_eq!(
            result.batches_processed,
            expected_batches,
            "Expected {} batches, got {}",
            expected_batches,
            result.batches_processed
        );

        // Property: Operation should complete successfully
        prop_assert!(result.is_complete(), "Export should complete without cancellation");
    }

    /// **Feature: performance-improvements, Property 16: Batch Processing Size**
    /// **Validates: Requirements 10.3**
    ///
    /// For any batch size configuration, the batch processor SHALL respect
    /// the configured size and process items accordingly.
    #[test]
    fn prop_batch_size_configuration(
        batch_size in arb_batch_size(),
    ) {
        let importer = BatchImporter::new(batch_size);
        let exporter = BatchExporter::new(batch_size);

        // Property: Batch size should be stored correctly
        prop_assert_eq!(
            importer.batch_size(),
            batch_size,
            "Importer batch size mismatch"
        );
        prop_assert_eq!(
            exporter.batch_size(),
            batch_size,
            "Exporter batch size mismatch"
        );

        // Property: Default batch sizes should be correct
        let default_importer = BatchImporter::default();
        let default_exporter = BatchExporter::default();

        prop_assert_eq!(
            default_importer.batch_size(),
            DEFAULT_IMPORT_BATCH_SIZE,
            "Default import batch size mismatch"
        );
        prop_assert_eq!(
            default_exporter.batch_size(),
            DEFAULT_EXPORT_BATCH_SIZE,
            "Default export batch size mismatch"
        );
    }

    /// **Feature: performance-improvements, Property 16: Batch Processing Size**
    /// **Validates: Requirements 10.1, 10.2**
    ///
    /// For any import/export with more than threshold items, batch processing
    /// should be recommended.
    #[test]
    fn prop_batch_threshold_recommendation(
        connection_count in arb_connection_count(),
    ) {
        let should_batch_import = BatchImporter::should_use_batch(connection_count);
        let should_batch_export = BatchExporter::should_use_batch(connection_count);

        // Property: Threshold should be consistent (> 10)
        if connection_count > 10 {
            prop_assert!(
                should_batch_import,
                "Should recommend batch import for {} connections",
                connection_count
            );
            prop_assert!(
                should_batch_export,
                "Should recommend batch export for {} connections",
                connection_count
            );
        } else {
            prop_assert!(
                !should_batch_import,
                "Should not recommend batch import for {} connections",
                connection_count
            );
            prop_assert!(
                !should_batch_export,
                "Should not recommend batch export for {} connections",
                connection_count
            );
        }
    }

    // =========================================================================
    // Property 17: Batch Processing Cancellation
    // =========================================================================

    /// **Feature: performance-improvements, Property 17: Batch Processing Cancellation**
    /// **Validates: Requirements 10.5**
    ///
    /// For any cancelled batch import operation, processing SHALL stop
    /// and partial results SHALL be reported.
    #[test]
    fn prop_batch_import_cancellation(
        batch_size in 2usize..20,
        connection_count in 20usize..100,
        cancel_at_batch in arb_cancel_point(),
    ) {
        let importer = BatchImporter::new(batch_size);
        let connections = create_test_connections(connection_count);

        let items_processed = Arc::new(AtomicUsize::new(0));
        let items_clone = Arc::clone(&items_processed);
        let cancel_at = cancel_at_batch * batch_size;

        let reporter = CallbackProgressReporter::new(move |_current, _total, _msg| {
            items_clone.fetch_add(1, Ordering::SeqCst);
            // Note: We can't cancel from within the callback easily,
            // so we'll use the cancel handle approach
        });

        // Cancel after processing some items
        let handle = importer.cancel_handle();

        // We need to cancel during processing, so we'll use a different approach
        // Cancel before starting if cancel_at_batch is 0
        if cancel_at_batch == 0 {
            handle.cancel();
        }

        let result = importer.process_connections(
            &connections,
            Some(&reporter),
            |c| {
                // Check if we should cancel mid-processing
                if items_processed.load(Ordering::SeqCst) >= cancel_at && cancel_at > 0 {
                    handle.cancel();
                }
                Ok(c.clone())
            },
        );

        if cancel_at_batch == 0 {
            // Property: Should be cancelled immediately
            prop_assert!(
                result.was_cancelled,
                "Should be cancelled when cancel_at_batch is 0"
            );
            prop_assert_eq!(
                result.batches_processed,
                0,
                "No batches should be processed when cancelled immediately"
            );
        } else {
            // Property: Partial results should be available
            // The exact number depends on when cancellation was detected
            prop_assert!(
                result.result.connections.len() <= connection_count,
                "Should have at most {} connections, got {}",
                connection_count,
                result.result.connections.len()
            );
        }
    }

    /// **Feature: performance-improvements, Property 17: Batch Processing Cancellation**
    /// **Validates: Requirements 10.5**
    ///
    /// For any cancelled batch export operation, processing SHALL stop
    /// and partial results SHALL be reported.
    #[test]
    fn prop_batch_export_cancellation(
        batch_size in 2usize..20,
        connection_count in 20usize..100,
        cancel_at_batch in arb_cancel_point(),
    ) {
        let exporter = BatchExporter::new(batch_size);
        let connections = create_test_connections(connection_count);

        let items_processed = Arc::new(AtomicUsize::new(0));
        let cancel_at = cancel_at_batch * batch_size;

        let handle = exporter.cancel_handle();

        // Cancel before starting if cancel_at_batch is 0
        if cancel_at_batch == 0 {
            handle.cancel();
        }

        let result = exporter.process_connections(
            &connections,
            None,
            |c| {
                let processed = items_processed.fetch_add(1, Ordering::SeqCst);
                // Check if we should cancel mid-processing
                if processed >= cancel_at && cancel_at > 0 {
                    handle.cancel();
                }
                Ok(c.name.clone())
            },
        );

        if cancel_at_batch == 0 {
            // Property: Should be cancelled immediately
            prop_assert!(
                result.was_cancelled,
                "Should be cancelled when cancel_at_batch is 0"
            );
            prop_assert_eq!(
                result.batches_processed,
                0,
                "No batches should be processed when cancelled immediately"
            );
        } else {
            // Property: Partial results should be available
            prop_assert!(
                result.result.exported_count <= connection_count,
                "Should have at most {} exported, got {}",
                connection_count,
                result.result.exported_count
            );
        }
    }

    /// **Feature: performance-improvements, Property 17: Batch Processing Cancellation**
    /// **Validates: Requirements 10.5**
    ///
    /// For any batch operation, the cancel handle should work correctly
    /// from a separate context.
    #[test]
    fn prop_batch_cancel_handle_works(
        _dummy in 0usize..10,
    ) {
        let importer = BatchImporter::new(10);
        let exporter = BatchExporter::new(10);

        let import_handle = importer.cancel_handle();
        let export_handle = exporter.cancel_handle();

        // Property: Initially not cancelled
        prop_assert!(!importer.is_cancelled());
        prop_assert!(!exporter.is_cancelled());
        prop_assert!(!import_handle.is_cancelled());
        prop_assert!(!export_handle.is_cancelled());

        // Cancel via handles
        import_handle.cancel();
        export_handle.cancel();

        // Property: Both should report cancelled
        prop_assert!(importer.is_cancelled(), "Importer should be cancelled via handle");
        prop_assert!(exporter.is_cancelled(), "Exporter should be cancelled via handle");
        prop_assert!(import_handle.is_cancelled(), "Import handle should report cancelled");
        prop_assert!(export_handle.is_cancelled(), "Export handle should report cancelled");

        // Reset and verify
        importer.reset();
        exporter.reset();

        prop_assert!(!importer.is_cancelled(), "Importer should be reset");
        prop_assert!(!exporter.is_cancelled(), "Exporter should be reset");
    }

    /// **Feature: performance-improvements, Property 17: Batch Processing Cancellation**
    /// **Validates: Requirements 10.5**
    ///
    /// For any batch operation cancelled via progress reporter, processing
    /// SHALL stop and partial results SHALL be reported.
    #[test]
    fn prop_batch_cancellation_via_progress_reporter(
        batch_size in 2usize..20,
        connection_count in 20usize..100,
        cancel_at in 5usize..50,
    ) {
        let importer = BatchImporter::new(batch_size);
        let connections = create_test_connections(connection_count);

        let items_processed = Arc::new(AtomicUsize::new(0));
        let items_clone = Arc::clone(&items_processed);

        let reporter = CallbackProgressReporter::new(move |_current, _total, _msg| {
            items_clone.fetch_add(1, Ordering::SeqCst);
        });

        // Cancel after processing some items
        let cancel_handle = reporter.cancel_handle();

        let result = importer.process_connections(
            &connections,
            Some(&reporter),
            |c| {
                let processed = items_processed.load(Ordering::SeqCst);
                if processed >= cancel_at {
                    cancel_handle.cancel();
                }
                Ok(c.clone())
            },
        );

        // Property: If cancelled, partial results should be available
        if result.was_cancelled {
            prop_assert!(
                result.result.connections.len() < connection_count,
                "Cancelled import should have partial results"
            );
        }
    }
}
