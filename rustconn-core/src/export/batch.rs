//! Batch export processing for efficient bulk connection exports.
//!
//! This module provides `BatchExporter` for processing large numbers of connections
//! efficiently using configurable batch sizes and progress reporting.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::models::Connection;
use crate::progress::ProgressReporter;

use super::{ExportError, ExportResult};

/// Default batch size for export operations.
pub const DEFAULT_EXPORT_BATCH_SIZE: usize = 50;

/// Threshold for using batch processing (connections count).
pub const BATCH_EXPORT_THRESHOLD: usize = 10;

/// Result of a batch export operation.
#[derive(Debug)]
pub struct BatchExportResult {
    /// The export result containing counts and any issues.
    pub result: ExportResult,
    /// Whether the operation was cancelled.
    pub was_cancelled: bool,
    /// Number of batches processed.
    pub batches_processed: usize,
}

impl BatchExportResult {
    /// Creates a new batch export result.
    #[must_use]
    pub const fn new(result: ExportResult, was_cancelled: bool, batches_processed: usize) -> Self {
        Self {
            result,
            was_cancelled,
            batches_processed,
        }
    }

    /// Returns true if the export completed without cancellation.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        !self.was_cancelled
    }
}

/// Batch exporter for efficient bulk connection exports.
///
/// Processes connections in configurable batch sizes with progress reporting
/// and cancellation support.
pub struct BatchExporter {
    /// Maximum number of connections to process per batch.
    batch_size: usize,
    /// Cancellation flag.
    cancelled: Arc<AtomicBool>,
}

impl BatchExporter {
    /// Creates a new batch exporter with the specified batch size.
    #[must_use]
    pub fn new(batch_size: usize) -> Self {
        Self {
            batch_size: batch_size.max(1),
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Creates a new batch exporter with the default batch size.
    #[must_use]
    pub fn with_default_batch_size() -> Self {
        Self::new(DEFAULT_EXPORT_BATCH_SIZE)
    }

    /// Returns the configured batch size.
    #[must_use]
    pub const fn batch_size(&self) -> usize {
        self.batch_size
    }

    /// Returns a handle for cancelling the export operation.
    #[must_use]
    pub fn cancel_handle(&self) -> BatchExportCancelHandle {
        BatchExportCancelHandle {
            cancelled: Arc::clone(&self.cancelled),
        }
    }

    /// Cancels the export operation.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Returns true if the export has been cancelled.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Resets the cancellation flag.
    pub fn reset(&self) {
        self.cancelled.store(false, Ordering::SeqCst);
    }

    /// Processes connections for export in batches with progress reporting.
    ///
    /// # Arguments
    ///
    /// * `connections` - The connections to export
    /// * `progress` - Optional progress reporter for receiving updates
    /// * `processor` - Function to process each connection for export
    ///
    /// # Returns
    ///
    /// Returns a `BatchExportResult` containing the export results
    /// and information about whether the operation was cancelled.
    pub fn process_connections<F>(
        &self,
        connections: &[Connection],
        progress: Option<&dyn ProgressReporter>,
        processor: F,
    ) -> BatchExportResult
    where
        F: Fn(&Connection) -> Result<String, ExportError>,
    {
        let total = connections.len();
        let mut result = ExportResult::new();
        let mut batches_processed = 0;
        let mut exported_data = Vec::with_capacity(total);

        // Check for cancellation from progress reporter or internal flag
        let check_cancelled =
            || self.is_cancelled() || progress.is_some_and(ProgressReporter::is_cancelled);

        if check_cancelled() {
            return BatchExportResult::new(result, true, 0);
        }

        // Process in batches
        for (batch_idx, chunk) in connections.chunks(self.batch_size).enumerate() {
            // Check cancellation between batches
            if check_cancelled() {
                // Return partial results
                result.exported_count = exported_data.len();
                return BatchExportResult::new(result, true, batches_processed);
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
                        &format!("Exporting connection {} of {}", current + 1, total),
                    );
                }

                // Process the connection
                match processor(conn) {
                    Ok(data) => {
                        exported_data.push(data);
                        result.increment_exported();
                    }
                    Err(e) => {
                        result.add_warning(format!("Failed to export '{}': {}", conn.name, e));
                        result.increment_skipped();
                    }
                }
            }

            batches_processed += 1;
        }

        // Report completion
        if let Some(reporter) = progress {
            reporter.report(total, total, "Export complete");
        }

        BatchExportResult::new(result, false, batches_processed)
    }

    /// Processes connections for export in batches, collecting the exported data.
    ///
    /// # Arguments
    ///
    /// * `connections` - The connections to export
    /// * `progress` - Optional progress reporter for receiving updates
    /// * `processor` - Function to process each connection for export
    ///
    /// # Returns
    ///
    /// Returns a tuple of (`BatchExportResult`, `Vec<String>`) containing the export
    /// results and the collected exported data strings.
    pub fn process_connections_with_data<F>(
        &self,
        connections: &[Connection],
        progress: Option<&dyn ProgressReporter>,
        processor: F,
    ) -> (BatchExportResult, Vec<String>)
    where
        F: Fn(&Connection) -> Result<String, ExportError>,
    {
        let total = connections.len();
        let mut result = ExportResult::new();
        let mut batches_processed = 0;
        let mut exported_data = Vec::with_capacity(total);

        // Check for cancellation from progress reporter or internal flag
        let check_cancelled =
            || self.is_cancelled() || progress.is_some_and(ProgressReporter::is_cancelled);

        if check_cancelled() {
            return (BatchExportResult::new(result, true, 0), exported_data);
        }

        // Process in batches
        for (batch_idx, chunk) in connections.chunks(self.batch_size).enumerate() {
            // Check cancellation between batches
            if check_cancelled() {
                result.exported_count = exported_data.len();
                return (
                    BatchExportResult::new(result, true, batches_processed),
                    exported_data,
                );
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
                        &format!("Exporting connection {} of {}", current + 1, total),
                    );
                }

                // Process the connection
                match processor(conn) {
                    Ok(data) => {
                        exported_data.push(data);
                        result.increment_exported();
                    }
                    Err(e) => {
                        result.add_warning(format!("Failed to export '{}': {}", conn.name, e));
                        result.increment_skipped();
                    }
                }
            }

            batches_processed += 1;
        }

        // Report completion
        if let Some(reporter) = progress {
            reporter.report(total, total, "Export complete");
        }

        (
            BatchExportResult::new(result, false, batches_processed),
            exported_data,
        )
    }

    /// Returns true if batch processing should be used for the given count.
    #[must_use]
    pub const fn should_use_batch(count: usize) -> bool {
        count > BATCH_EXPORT_THRESHOLD
    }
}

impl Default for BatchExporter {
    fn default() -> Self {
        Self::with_default_batch_size()
    }
}

/// Handle for cancelling a batch export operation from another context.
#[derive(Clone)]
pub struct BatchExportCancelHandle {
    cancelled: Arc<AtomicBool>,
}

impl BatchExportCancelHandle {
    /// Signals cancellation to the batch exporter.
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
    fn test_batch_exporter_creation() {
        let exporter = BatchExporter::new(25);
        assert_eq!(exporter.batch_size(), 25);
        assert!(!exporter.is_cancelled());
    }

    #[test]
    fn test_batch_exporter_default() {
        let exporter = BatchExporter::default();
        assert_eq!(exporter.batch_size(), DEFAULT_EXPORT_BATCH_SIZE);
    }

    #[test]
    fn test_batch_exporter_min_batch_size() {
        let exporter = BatchExporter::new(0);
        assert_eq!(exporter.batch_size(), 1);
    }

    #[test]
    fn test_batch_exporter_cancellation() {
        let exporter = BatchExporter::new(10);
        assert!(!exporter.is_cancelled());

        exporter.cancel();
        assert!(exporter.is_cancelled());

        exporter.reset();
        assert!(!exporter.is_cancelled());
    }

    #[test]
    fn test_batch_export_cancel_handle() {
        let exporter = BatchExporter::new(10);
        let handle = exporter.cancel_handle();

        assert!(!exporter.is_cancelled());
        assert!(!handle.is_cancelled());

        handle.cancel();

        assert!(exporter.is_cancelled());
        assert!(handle.is_cancelled());
    }

    #[test]
    fn test_should_use_batch() {
        assert!(!BatchExporter::should_use_batch(5));
        assert!(!BatchExporter::should_use_batch(10));
        assert!(BatchExporter::should_use_batch(11));
        assert!(BatchExporter::should_use_batch(100));
    }

    #[test]
    fn test_process_connections_empty() {
        let exporter = BatchExporter::new(10);
        let result = exporter.process_connections(&[], None, |c| Ok(c.name.clone()));

        assert!(result.is_complete());
        assert_eq!(result.result.exported_count, 0);
        assert_eq!(result.batches_processed, 0);
    }

    #[test]
    fn test_process_connections_single_batch() {
        let exporter = BatchExporter::new(10);
        let connections: Vec<_> = (0..5)
            .map(|i| create_test_connection(&format!("conn{i}")))
            .collect();

        let result = exporter.process_connections(&connections, None, |c| Ok(c.name.clone()));

        assert!(result.is_complete());
        assert_eq!(result.result.exported_count, 5);
        assert_eq!(result.batches_processed, 1);
    }

    #[test]
    fn test_process_connections_multiple_batches() {
        let exporter = BatchExporter::new(3);
        let connections: Vec<_> = (0..10)
            .map(|i| create_test_connection(&format!("conn{i}")))
            .collect();

        let result = exporter.process_connections(&connections, None, |c| Ok(c.name.clone()));

        assert!(result.is_complete());
        assert_eq!(result.result.exported_count, 10);
        assert_eq!(result.batches_processed, 4); // 3 + 3 + 3 + 1
    }

    #[test]
    fn test_process_connections_with_errors() {
        let exporter = BatchExporter::new(10);
        let connections: Vec<_> = (0..5)
            .map(|i| create_test_connection(&format!("conn{i}")))
            .collect();

        let result = exporter.process_connections(&connections, None, |c| {
            if c.name.contains('2') {
                Err(ExportError::InvalidData("test error".to_string()))
            } else {
                Ok(c.name.clone())
            }
        });

        assert!(result.is_complete());
        assert_eq!(result.result.exported_count, 4);
        assert_eq!(result.result.skipped_count, 1);
        assert_eq!(result.result.warnings.len(), 1);
    }

    #[test]
    fn test_process_connections_cancelled() {
        let exporter = BatchExporter::new(2);
        let connections: Vec<_> = (0..10)
            .map(|i| create_test_connection(&format!("conn{i}")))
            .collect();

        // Cancel before processing
        exporter.cancel();

        let result = exporter.process_connections(&connections, None, |c| Ok(c.name.clone()));

        assert!(!result.is_complete());
        assert!(result.was_cancelled);
        assert_eq!(result.batches_processed, 0);
    }

    #[test]
    fn test_process_connections_with_data() {
        let exporter = BatchExporter::new(10);
        let connections: Vec<_> = (0..5)
            .map(|i| create_test_connection(&format!("conn{i}")))
            .collect();

        let (result, data) =
            exporter.process_connections_with_data(&connections, None, |c| Ok(c.name.clone()));

        assert!(result.is_complete());
        assert_eq!(result.result.exported_count, 5);
        assert_eq!(data.len(), 5);
        assert!(data.contains(&"conn0".to_string()));
        assert!(data.contains(&"conn4".to_string()));
    }
}
