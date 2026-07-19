//! Busy-state tracking with RAII guards.
//!
//! [`BusyStack`] is a thread-safe counter of in-flight operations. Each call
//! to [`BusyStack::busy()`] increments the counter and returns a [`BusyGuard`]
//! that decrements it on drop. The first increment triggers a "busy" callback;
//! the last decrement triggers an "idle" callback.
//!
//! This is useful for showing/hiding a spinner in the GUI: the GUI layer
//! connects the `on_change` callback to show/hide an `AdwSpinner` or
//! `GtkSpinner`, while the core layer simply acquires guards around
//! long-running operations.
//!
//! # Example
//!
//! ```rust
//! use rustconn_core::busy::BusyStack;
//!
//! let stack = BusyStack::new(|busy| {
//!     if busy { println!("show spinner"); }
//!     else    { println!("hide spinner"); }
//! });
//!
//! {
//!     let _guard1 = stack.busy();
//!     // spinner shown (count: 1)
//!     {
//!         let _guard2 = stack.busy();
//!         // still shown (count: 2)
//!     }
//!     // still shown (count: 1)
//! }
//! // spinner hidden (count: 0)
//! ```

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Thread-safe counter of in-flight operations with change notification.
///
/// When the counter transitions from 0→1, the callback is invoked with `true`.
/// When it transitions from 1→0, the callback is invoked with `false`.
/// Nested operations (count > 1) do not trigger additional callbacks.
pub struct BusyStack {
    inner: Arc<BusyInner>,
}

struct BusyInner {
    count: AtomicUsize,
    on_change: Box<dyn Fn(bool) + Send + Sync>,
}

impl std::fmt::Debug for BusyStack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BusyStack")
            .field("count", &self.inner.count.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

/// RAII guard that decrements the busy counter on drop.
///
/// Created by [`BusyStack::busy()`]. When the last guard is dropped,
/// the "idle" callback fires.
pub struct BusyGuard {
    inner: Arc<BusyInner>,
}

impl std::fmt::Debug for BusyGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BusyGuard")
            .field("count", &self.inner.count.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl BusyStack {
    /// Creates a new `BusyStack` with the given change callback.
    ///
    /// The callback receives `true` when the stack becomes busy (0→1)
    /// and `false` when it becomes idle (1→0).
    pub fn new(on_change: impl Fn(bool) + Send + Sync + 'static) -> Self {
        Self {
            inner: Arc::new(BusyInner {
                count: AtomicUsize::new(0),
                on_change: Box::new(on_change),
            }),
        }
    }

    /// Marks the stack as busy and returns a guard.
    ///
    /// The guard decrements the counter when dropped. If this is the
    /// first active guard, the `on_change(true)` callback fires.
    pub fn busy(&self) -> BusyGuard {
        let prev = self.inner.count.fetch_add(1, Ordering::SeqCst);
        if prev == 0 {
            (self.inner.on_change)(true);
        }
        BusyGuard {
            inner: Arc::clone(&self.inner),
        }
    }

    /// Returns the current number of active operations.
    pub fn count(&self) -> usize {
        self.inner.count.load(Ordering::SeqCst)
    }

    /// Returns `true` if there are any active operations.
    pub fn is_busy(&self) -> bool {
        self.count() > 0
    }
}

impl Clone for BusyStack {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        let prev = self.inner.count.fetch_sub(1, Ordering::SeqCst);
        if prev == 1 {
            (self.inner.on_change)(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use super::*;

    #[test]
    fn test_single_guard() {
        let is_busy = Arc::new(AtomicBool::new(false));
        let is_busy_clone = Arc::clone(&is_busy);

        let stack = BusyStack::new(move |busy| {
            is_busy_clone.store(busy, Ordering::SeqCst);
        });

        assert!(!stack.is_busy());
        assert!(!is_busy.load(Ordering::SeqCst));

        {
            let _guard = stack.busy();
            assert!(stack.is_busy());
            assert_eq!(stack.count(), 1);
            assert!(is_busy.load(Ordering::SeqCst));
        }

        assert!(!stack.is_busy());
        assert_eq!(stack.count(), 0);
        assert!(!is_busy.load(Ordering::SeqCst));
    }

    #[test]
    fn test_nested_guards() {
        let change_count = Arc::new(AtomicUsize::new(0));
        let change_count_clone = Arc::clone(&change_count);

        let stack = BusyStack::new(move |_busy| {
            change_count_clone.fetch_add(1, Ordering::SeqCst);
        });

        {
            let _g1 = stack.busy();
            assert_eq!(stack.count(), 1);
            assert_eq!(change_count.load(Ordering::SeqCst), 1); // 0→1 transition

            {
                let _g2 = stack.busy();
                assert_eq!(stack.count(), 2);
                // No additional callback for 1→2
                assert_eq!(change_count.load(Ordering::SeqCst), 1);

                {
                    let _g3 = stack.busy();
                    assert_eq!(stack.count(), 3);
                    assert_eq!(change_count.load(Ordering::SeqCst), 1);
                }

                assert_eq!(stack.count(), 2);
                assert_eq!(change_count.load(Ordering::SeqCst), 1);
            }

            assert_eq!(stack.count(), 1);
            assert_eq!(change_count.load(Ordering::SeqCst), 1);
        }

        assert_eq!(stack.count(), 0);
        assert_eq!(change_count.load(Ordering::SeqCst), 2); // 1→0 transition
    }

    #[test]
    fn test_clone_shares_state() {
        let is_busy = Arc::new(AtomicBool::new(false));
        let is_busy_clone = is_busy;

        let stack1 = BusyStack::new(move |busy| {
            is_busy_clone.store(busy, Ordering::SeqCst);
        });
        let stack2 = stack1.clone();

        let _g1 = stack1.busy();
        assert_eq!(stack2.count(), 1);
        assert!(stack2.is_busy());

        let _g2 = stack2.busy();
        assert_eq!(stack1.count(), 2);
    }

    #[test]
    fn test_guard_drop_order_independent() {
        let is_busy = Arc::new(AtomicBool::new(false));
        let is_busy_clone = Arc::clone(&is_busy);

        let stack = BusyStack::new(move |busy| {
            is_busy_clone.store(busy, Ordering::SeqCst);
        });

        let g1 = stack.busy();
        let g2 = stack.busy();
        assert_eq!(stack.count(), 2);

        // Drop g1 first (not g2)
        drop(g1);
        assert_eq!(stack.count(), 1);
        assert!(is_busy.load(Ordering::SeqCst)); // Still busy

        drop(g2);
        assert_eq!(stack.count(), 0);
        assert!(!is_busy.load(Ordering::SeqCst)); // Now idle
    }
}
