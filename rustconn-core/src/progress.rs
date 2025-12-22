//! Progress reporting for long-running operations.
//!
//! This module provides traits and implementations for reporting progress
//! during operations like imports, exports, and bulk operations.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Trait for reporting progress during long operations.
///
/// Implementations of this trait can be passed to long-running operations
/// to receive progress updates and allow cancellation.
pub trait ProgressReporter: Send + Sync {
    /// Report progress update.
    ///
    /// # Arguments
    ///
    /// * `current` - Current item number (0-indexed)
    /// * `total` - Total number of items to process
    /// * `message` - Human-readable status message
    fn report(&self, current: usize, total: usize, message: &str);

    /// Check if the operation was cancelled.
    ///
    /// Long-running operations should check this periodically and
    /// stop processing if it returns true.
    fn is_cancelled(&self) -> bool;
}

/// A progress reporter that invokes callbacks for progress updates.
///
/// This implementation is useful for connecting progress reporting
/// to GUI elements or logging systems.
pub struct CallbackProgressReporter<F>
where
    F: Fn(usize, usize, &str) + Send + Sync,
{
    callback: F,
    cancelled: Arc<AtomicBool>,
}

impl<F> CallbackProgressReporter<F>
where
    F: Fn(usize, usize, &str) + Send + Sync,
{
    /// Creates a new callback-based progress reporter.
    ///
    /// # Arguments
    ///
    /// * `callback` - Function called for each progress update
    #[must_use]
    pub fn new(callback: F) -> Self {
        Self {
            callback,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Returns a handle that can be used to cancel the operation.
    #[must_use]
    pub fn cancel_handle(&self) -> CancelHandle {
        CancelHandle {
            cancelled: Arc::clone(&self.cancelled),
        }
    }

    /// Cancels the operation.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
}

impl<F> ProgressReporter for CallbackProgressReporter<F>
where
    F: Fn(usize, usize, &str) + Send + Sync,
{
    fn report(&self, current: usize, total: usize, message: &str) {
        (self.callback)(current, total, message);
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// A handle for cancelling an operation from another thread or context.
#[derive(Clone)]
pub struct CancelHandle {
    cancelled: Arc<AtomicBool>,
}

impl CancelHandle {
    /// Signals cancellation to the associated progress reporter.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Returns true if cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// A no-op progress reporter that ignores all updates.
///
/// Useful as a default when no progress reporting is needed.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoOpProgressReporter;

impl NoOpProgressReporter {
    /// Creates a new no-op progress reporter.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl ProgressReporter for NoOpProgressReporter {
    fn report(&self, _current: usize, _total: usize, _message: &str) {
        // No-op
    }

    fn is_cancelled(&self) -> bool {
        false
    }
}

/// A single-threaded progress reporter for GUI contexts.
///
/// This implementation is useful for GTK applications where widgets
/// cannot be shared across threads. It uses `Rc<RefCell>` instead of
/// `Arc<AtomicBool>` for the cancellation flag.
///
/// Note: This type is NOT `Send` or `Sync` and should only be used
/// in single-threaded contexts like GTK main loops.
pub struct LocalProgressReporter<F>
where
    F: Fn(usize, usize, &str),
{
    callback: F,
    cancelled: std::rc::Rc<std::cell::Cell<bool>>,
}

impl<F> LocalProgressReporter<F>
where
    F: Fn(usize, usize, &str),
{
    /// Creates a new local progress reporter.
    ///
    /// # Arguments
    ///
    /// * `callback` - Function called for each progress update
    #[must_use]
    pub fn new(callback: F) -> Self {
        Self {
            callback,
            cancelled: std::rc::Rc::new(std::cell::Cell::new(false)),
        }
    }

    /// Creates a new local progress reporter with a shared cancellation flag.
    ///
    /// # Arguments
    ///
    /// * `callback` - Function called for each progress update
    /// * `cancelled` - Shared cancellation flag
    #[must_use]
    pub const fn with_cancel_flag(
        callback: F,
        cancelled: std::rc::Rc<std::cell::Cell<bool>>,
    ) -> Self {
        Self {
            callback,
            cancelled,
        }
    }

    /// Cancels the operation.
    pub fn cancel(&self) {
        self.cancelled.set(true);
    }

    /// Returns a clone of the cancellation flag.
    #[must_use]
    pub fn cancel_flag(&self) -> std::rc::Rc<std::cell::Cell<bool>> {
        std::rc::Rc::clone(&self.cancelled)
    }

    /// Reports progress update.
    pub fn report(&self, current: usize, total: usize, message: &str) {
        (self.callback)(current, total, message);
    }

    /// Returns true if the operation was cancelled.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn test_callback_reporter_invokes_callback() {
        let call_count = Arc::new(AtomicUsize::new(0));
        let count_clone = Arc::clone(&call_count);

        let reporter = CallbackProgressReporter::new(move |_, _, _| {
            count_clone.fetch_add(1, Ordering::SeqCst);
        });

        reporter.report(0, 10, "Processing item 1");
        reporter.report(1, 10, "Processing item 2");

        assert_eq!(call_count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn test_callback_reporter_cancellation() {
        let reporter = CallbackProgressReporter::new(|_, _, _| {});

        assert!(!reporter.is_cancelled());

        reporter.cancel();

        assert!(reporter.is_cancelled());
    }

    #[test]
    fn test_cancel_handle() {
        let reporter = CallbackProgressReporter::new(|_, _, _| {});
        let handle = reporter.cancel_handle();

        assert!(!reporter.is_cancelled());
        assert!(!handle.is_cancelled());

        handle.cancel();

        assert!(reporter.is_cancelled());
        assert!(handle.is_cancelled());
    }

    #[test]
    fn test_noop_reporter() {
        let reporter = NoOpProgressReporter::new();

        // Should not panic
        reporter.report(0, 100, "test");
        reporter.report(50, 100, "test");

        // Should always return false
        assert!(!reporter.is_cancelled());
    }
}
