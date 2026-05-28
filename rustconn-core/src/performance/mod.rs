//! Performance optimization utilities for `RustConn`
//!
//! This module provides utilities for measuring and optimizing application performance,
//! including startup time profiling, lazy loading, debouncing, and memory optimization.

// Allow pedantic lints for this module - performance code uses many Mutex locks
// and the panics documentation would be excessive for internal metrics code.
// Cast warnings are acceptable here as we're dealing with metrics/statistics
// where precision loss is not critical.
// cast_possible_truncation, cast_precision_loss, unused_self allowed at workspace level
#![allow(
    clippy::missing_panics_doc,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::significant_drop_in_scrutinee,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::significant_drop_tightening,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::cast_sign_loss,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::format_push_string,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::incompatible_msrv,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::option_if_let_else,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::len_zero,
    reason = "module-wide override for legacy code; refactored case by case"
)]

mod batch;
mod compact_string;
mod debouncer;
pub mod interner;
mod lazy;
mod memory;
mod metrics;
mod pool;
mod scroller;
mod shrinkable_vec;

use std::sync::{Mutex, MutexGuard, OnceLock, RwLock, RwLockReadGuard, RwLockWriteGuard};

pub use batch::BatchProcessor;
pub use compact_string::CompactString;
pub use debouncer::Debouncer;
pub use interner::{InternerStats, StringInterner};
pub use lazy::LazyInit;
pub use memory::{
    AllocationStats, MemoryBreakdown, MemoryEstimate, MemoryOptimizer, MemoryPressure,
    MemorySnapshot, MemoryTracker, OptimizationCategory, OptimizationRecommendation,
};
pub use metrics::{OperationStats, PerformanceMetrics, TimingGuard};
pub use pool::{ObjectPool, PoolStats};
pub use scroller::VirtualScroller;
pub use shrinkable_vec::ShrinkableVec;

/// Acquires a `Mutex` lock, logging and returning `None` on poison.
pub(crate) fn lock_mutex<'a, T>(mutex: &'a Mutex<T>, name: &str) -> Option<MutexGuard<'a, T>> {
    mutex
        .lock()
        .map_err(|e| {
            tracing::error!(mutex = name, "Mutex poisoned: {e}");
        })
        .ok()
}

/// Acquires a `RwLock` read lock, logging and returning `None` on poison.
pub(crate) fn read_rwlock<'a, T>(
    lock: &'a RwLock<T>,
    name: &str,
) -> Option<RwLockReadGuard<'a, T>> {
    lock.read()
        .map_err(|e| {
            tracing::error!(rwlock = name, "RwLock poisoned (read): {e}");
        })
        .ok()
}

/// Acquires a `RwLock` write lock, logging and returning `None` on poison.
pub(crate) fn write_rwlock<'a, T>(
    lock: &'a RwLock<T>,
    name: &str,
) -> Option<RwLockWriteGuard<'a, T>> {
    lock.write()
        .map_err(|e| {
            tracing::error!(rwlock = name, "RwLock poisoned (write): {e}");
        })
        .ok()
}

/// Global performance metrics instance
static METRICS: OnceLock<PerformanceMetrics> = OnceLock::new();

/// Gets the global performance metrics instance
#[must_use]
pub fn metrics() -> &'static PerformanceMetrics {
    METRICS.get_or_init(PerformanceMetrics::new)
}

/// Global memory optimizer instance
static MEMORY_OPTIMIZER: OnceLock<MemoryOptimizer> = OnceLock::new();

/// Gets the global memory optimizer instance
#[must_use]
pub fn memory_optimizer() -> &'static MemoryOptimizer {
    MEMORY_OPTIMIZER.get_or_init(MemoryOptimizer::new)
}

/// Formats bytes as a human-readable string
#[must_use]
pub fn format_bytes(bytes: usize) -> String {
    const KB: usize = 1024;
    const MB: usize = KB * 1024;
    const GB: usize = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.00 KB");
        assert_eq!(format_bytes(1536), "1.50 KB");
        assert_eq!(format_bytes(1_048_576), "1.00 MB");
        assert_eq!(format_bytes(1_073_741_824), "1.00 GB");
    }

    #[test]
    fn test_global_memory_optimizer() {
        let optimizer = memory_optimizer();
        optimizer.interner().intern("global_test");
        assert!(optimizer.interner().len() >= 1);
    }
}
