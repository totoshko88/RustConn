//! Performance utilities for `RustConn`.
//!
//! - [`Debouncer`] — rate-limits rapid operations (e.g. search input).
//! - [`StringInterner`] — deduplicates repeated connection strings. The
//!   connection manager feeds it protocol names, hostnames and usernames, but
//!   the models keep their own `String`s, so today it saves no memory.

use std::sync::{Mutex, MutexGuard, OnceLock, RwLock, RwLockReadGuard, RwLockWriteGuard};

mod debouncer;
pub mod interner;

pub use debouncer::Debouncer;
pub use interner::{InternerStats, StringInterner};

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

/// Global string interner instance.
static INTERNER: OnceLock<StringInterner> = OnceLock::new();

/// Returns the global string interner used to deduplicate connection strings.
#[must_use]
pub fn interner() -> &'static StringInterner {
    INTERNER.get_or_init(StringInterner::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_global_interner() {
        interner().intern("global_test");
        assert!(!interner().is_empty());
    }
}
