//! Batch import processing for efficient bulk connection imports.
//!
//! This module provides `BatchImporter` for processing large numbers of connections
//! efficiently using configurable batch sizes and progress reporting.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::ImportError;
use crate::models::Connection;
use crate::progress::ProgressReporter;

use super::traits::ImportResult;

/// Default batch size for import operations.
pub const DEFAULT_IMPORT_BATCH_SIZE: usize = 50;

/// Threshold for using batch processing (connections count).
pub const BATCH_IMPORT_THRESHOLD: usize = 10;

/// Result of a batch import operation.
#[derive(Debug)]
pub struct BatchImportResult {
    /// The import result containing connections, groups, and any issues.
    pub result: ImportResult,
    /// Whether the operation was cancelled.
    pub was_cancelled: bool,
    /// Number of batches processed.
    pub batches_processed: usize,
}

impl BatchImportResult {
    /// Creates a new batch import result.
    #[must_use]
    pub const fn new(result: ImportResult, was_cancelled: bool, batches_processed: usize) -> Self {
        Self {
            result,
            was_cancelled,
            batches_processed,
        }
    }

    /// Returns true if the import completed without cancellation.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        !self.was_cancelled
    }
}

/// Batch importer for efficient bulk connection imports.
///
/// Processes connections in configurable batch sizes with progress reporting
/// and cancellation support.
pub struct BatchImporter {
    /// Maximum number of connections to process per batch.
    batch_size: usize,
    /// Cancellation flag.
    cancelled: Arc<AtomicBool>,
}

impl BatchImporter {
    /// Creates a new batch importer with the specified batch size.
    #[must_use]
    pub fn new(batch_size: usize) -> Self {
        Self {
            batch_size: batch_size.max(1),
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Creates a new batch importer with the default batch size.
    #[must_use]
    pub fn with_default_batch_size() -> Self {
        Self::new(DEFAULT_IMPORT_BATCH_SIZE)
    }

    /// Returns the configured batch size.
    #[must_use]
    pub const fn batch_size(&self) -> usize {
        self.batch_size
    }

    /// Returns a handle for cancelling the import operation.
    #[must_use]
    pub fn cancel_handle(&self) -> BatchCancelHandle {
        BatchCancelHandle {
            cancelled: Arc::clone(&self.cancelled),
        }
    }

    /// Cancels the import operation.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Returns true if the import has been cancelled.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Resets the cancellation flag.
    pub fn reset(&self) {
        self.cancelled.store(false, Ordering::SeqCst);
    }

    /// Processes connections in batches with progress reporting.
    ///
    /// # Arguments
    ///
    /// * `connections` - The connections to process
    /// * `progress` - Optional progress reporter for receiving updates
    /// * `processor` - Function to process each connection
    ///
    /// # Returns
    ///
    /// Returns a `BatchImportResult` containing the processed connections
    /// and information about whether the operation was cancelled.
    pub fn process_connections<F>(
        &self,
        connections: &[Connection],
        progress: Option<&dyn ProgressReporter>,
        processor: F,
    ) -> BatchImportResult
    where
        F: Fn(&Connection) -> Result<Connection, ImportError>,
    {
        let total = connections.len();
        let mut result = ImportResult::new();
        let mut batches_processed = 0;

        // Check for cancellation from progress reporter or internal flag
        let check_cancelled =
            || self.is_cancelled() || progress.is_some_and(ProgressReporter::is_cancelled);

        if check_cancelled() {
            return BatchImportResult::new(result, true, 0);
        }

        // Process in batches
        for (batch_idx, chunk) in connections.chunks(self.batch_size).enumerate() {
            // Check cancellation between batches
            if check_cancelled() {
                return BatchImportResult::new(result, true, batches_processed);
            }

            let batch_start = batch_idx * self.batch_size;

            // Process each connection in the batch
            for (idx, conn) in chunk.iter().enumerate() {
                let current = batch_start + idx;

                // Report progress
                if let Some(reporter) = progress {
                    reporter.report(
                        current,
                        total,
                        &format!("Processing connection {} of {}", current + 1, total),
                    );
                }

                // Process the connection
                match processor(conn) {
                    Ok(processed) => result.add_connection(processed),
                    Err(e) => result.add_error(e),
                }
            }

            batches_processed += 1;
        }

        // Report completion
        if let Some(reporter) = progress {
            reporter.report(total, total, "Import complete");
        }

        BatchImportResult::new(result, false, batches_processed)
    }

    /// Imports connections from an import result in batches.
    ///
    /// This is useful when you have an `ImportResult` from another source
    /// and want to process it in batches.
    ///
    /// # Arguments
    ///
    /// * `import_result` - The import result to process
    /// * `progress` - Optional progress reporter
    ///
    /// # Returns
    ///
    /// Returns a `BatchImportResult` with the processed data.
    #[must_use]
    pub fn process_import_result(
        &self,
        import_result: ImportResult,
        progress: Option<&dyn ProgressReporter>,
    ) -> BatchImportResult {
        let total = import_result.connections.len();
        let mut result = ImportResult::new();
        let mut batches_processed = 0;

        // Copy over groups, skipped entries, and errors
        result.groups = import_result.groups;
        result.skipped = import_result.skipped;
        result.errors = import_result.errors;

        // Check for cancellation
        let check_cancelled =
            || self.is_cancelled() || progress.is_some_and(ProgressReporter::is_cancelled);

        if check_cancelled() {
            return BatchImportResult::new(result, true, 0);
        }

        // Process connections in batches
        for (batch_idx, chunk) in import_result
            .connections
            .chunks(self.batch_size)
            .enumerate()
        {
            if check_cancelled() {
                return BatchImportResult::new(result, true, batches_processed);
            }

            let batch_start = batch_idx * self.batch_size;

            for (idx, conn) in chunk.iter().enumerate() {
                let current = batch_start + idx;

                if let Some(reporter) = progress {
                    reporter.report(
                        current,
                        total,
                        &format!("Processing connection {} of {}", current + 1, total),
                    );
                }

                result.add_connection(conn.clone());
            }

            batches_processed += 1;
        }

        if let Some(reporter) = progress {
            reporter.report(total, total, "Import complete");
        }

        BatchImportResult::new(result, false, batches_processed)
    }

    /// Returns true if batch processing should be used for the given count.
    #[must_use]
    pub const fn should_use_batch(count: usize) -> bool {
        count > BATCH_IMPORT_THRESHOLD
    }
}

impl Default for BatchImporter {
    fn default() -> Self {
        Self::with_default_batch_size()
    }
}

/// Handle for cancelling a batch import operation from another context.
#[derive(Clone)]
pub struct BatchCancelHandle {
    cancelled: Arc<AtomicBool>,
}

impl BatchCancelHandle {
    /// Signals cancellation to the batch importer.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Returns true if cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_connection(name: &str) -> Connection {
        Connection::new_ssh(name.to_string(), "localhost".to_string(), 22)
    }

    #[test]
    fn test_batch_importer_creation() {
        let importer = BatchImporter::new(25);
        assert_eq!(importer.batch_size(), 25);
        assert!(!importer.is_cancelled());
    }

    #[test]
    fn test_batch_importer_default() {
        let importer = BatchImporter::default();
        assert_eq!(importer.batch_size(), DEFAULT_IMPORT_BATCH_SIZE);
    }

    #[test]
    fn test_batch_importer_min_batch_size() {
        let importer = BatchImporter::new(0);
        assert_eq!(importer.batch_size(), 1);
    }

    #[test]
    fn test_batch_importer_cancellation() {
        let importer = BatchImporter::new(10);
        assert!(!importer.is_cancelled());

        importer.cancel();
        assert!(importer.is_cancelled());

        importer.reset();
        assert!(!importer.is_cancelled());
    }

    #[test]
    fn test_batch_cancel_handle() {
        let importer = BatchImporter::new(10);
        let handle = importer.cancel_handle();

        assert!(!importer.is_cancelled());
        assert!(!handle.is_cancelled());

        handle.cancel();

        assert!(importer.is_cancelled());
        assert!(handle.is_cancelled());
    }

    #[test]
    fn test_should_use_batch() {
        assert!(!BatchImporter::should_use_batch(5));
        assert!(!BatchImporter::should_use_batch(10));
        assert!(BatchImporter::should_use_batch(11));
        assert!(BatchImporter::should_use_batch(100));
    }

    #[test]
    fn test_process_connections_empty() {
        let importer = BatchImporter::new(10);
        let result = importer.process_connections(&[], None, |c| Ok(c.clone()));

        assert!(result.is_complete());
        assert_eq!(result.result.connections.len(), 0);
        assert_eq!(result.batches_processed, 0);
    }

    #[test]
    fn test_process_connections_single_batch() {
        let importer = BatchImporter::new(10);
        let connections: Vec<_> = (0..5)
            .map(|i| create_test_connection(&format!("conn{i}")))
            .collect();

        let result = importer.process_connections(&connections, None, |c| Ok(c.clone()));

        assert!(result.is_complete());
        assert_eq!(result.result.connections.len(), 5);
        assert_eq!(result.batches_processed, 1);
    }

    #[test]
    fn test_process_connections_multiple_batches() {
        let importer = BatchImporter::new(3);
        let connections: Vec<_> = (0..10)
            .map(|i| create_test_connection(&format!("conn{i}")))
            .collect();

        let result = importer.process_connections(&connections, None, |c| Ok(c.clone()));

        assert!(result.is_complete());
        assert_eq!(result.result.connections.len(), 10);
        assert_eq!(result.batches_processed, 4); // 3 + 3 + 3 + 1
    }

    #[test]
    fn test_process_connections_with_errors() {
        let importer = BatchImporter::new(10);
        let connections: Vec<_> = (0..5)
            .map(|i| create_test_connection(&format!("conn{i}")))
            .collect();

        let result = importer.process_connections(&connections, None, |c| {
            if c.name.contains('2') {
                Err(ImportError::InvalidEntry {
                    source_name: "test".to_string(),
                    reason: "test error".to_string(),
                })
            } else {
                Ok(c.clone())
            }
        });

        assert!(result.is_complete());
        assert_eq!(result.result.connections.len(), 4);
        assert_eq!(result.result.errors.len(), 1);
    }

    #[test]
    fn test_process_connections_cancelled() {
        let importer = BatchImporter::new(2);
        let connections: Vec<_> = (0..10)
            .map(|i| create_test_connection(&format!("conn{i}")))
            .collect();

        // Cancel after first batch
        importer.cancel();

        let result = importer.process_connections(&connections, None, |c| Ok(c.clone()));

        assert!(!result.is_complete());
        assert!(result.was_cancelled);
        assert_eq!(result.batches_processed, 0);
    }
}
