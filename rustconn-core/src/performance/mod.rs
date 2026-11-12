//! Performance utilities for `RustConn`.
//!
//! [`Debouncer`] rate-limits rapid operations (e.g. search input).

use std::sync::{Mutex, MutexGuard};

mod debouncer;

pub use debouncer::Debouncer;

/// Acquires a `Mutex` lock, logging and returning `None` on poison.
pub(crate) fn lock_mutex<'a, T>(mutex: &'a Mutex<T>, name: &str) -> Option<MutexGuard<'a, T>> {
    mutex
        .lock()
        .map_err(|e| {
            tracing::error!(mutex = name, "Mutex poisoned: {e}");
        })
        .ok()
}
