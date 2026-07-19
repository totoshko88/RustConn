//! Generic async cache with TTL and automatic refresh.
//!
//! Provides [`Cached<T>`] — a thread-safe, async-aware cache that stores a single
//! value of type `T` with a configurable time-to-live (TTL). When the cached value
//! expires, it is transparently refreshed via the [`LoadCacheObject`] trait.
//!
//! # Design
//!
//! Inspired by Field Monitor's `Cached<T>` pattern, adapted for RustConn's
//! Tokio-based async architecture. Uses `RwLock` for concurrent read access
//! with exclusive write access during refresh.
//!
//! # Example
//!
//! ```rust,ignore
//! use rustconn_core::cache::{Cached, LoadCacheObject};
//! use std::time::Duration;
//!
//! struct MyData { value: String }
//!
//! impl LoadCacheObject for MyData {
//!     type Params = ();
//!     type Error = std::convert::Infallible;
//!
//!     async fn construct(
//!         _previous: Option<Self>,
//!         _params: &Self::Params,
//!     ) -> Result<Self, Self::Error> {
//!         Ok(MyData { value: "hello".into() })
//!     }
//! }
//!
//! let cache: Cached<MyData> = Cached::with_ttl((), Duration::from_secs(30));
//! let data = cache.get().await.unwrap();
//! ```

use std::fmt;
use std::ops::Deref;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;

/// Default TTL for cached values (60 seconds).
pub const DEFAULT_CACHE_TTL_SECS: u64 = 60;

/// Trait for objects that can be loaded into a [`Cached<T>`].
///
/// Implementors define how to construct (or refresh) the cached value.
/// The `previous_value` parameter enables incremental updates — if the
/// previous value is still partially valid, the implementation can reuse
/// parts of it instead of rebuilding from scratch.
pub trait LoadCacheObject: Send + Sync {
    /// Parameters needed to construct the value (e.g., an API client, DB pool).
    type Params: Send + Sync;

    /// Error type returned when construction fails.
    type Error: fmt::Display + Send + Sync;

    /// Construct a new value, optionally reusing the `previous_value`
    /// for incremental updates.
    ///
    /// # Errors
    ///
    /// Returns `Self::Error` if the value cannot be constructed.
    fn construct(
        previous_value: Option<Self>,
        params: &Self::Params,
    ) -> impl std::future::Future<Output = Result<Self, Self::Error>> + Send
    where
        Self: Sized;
}

/// Generic async cache with TTL and automatic refresh.
///
/// Stores a single value of type `T` and refreshes it when the TTL expires.
/// Thread-safe via `tokio::sync::RwLock` — multiple readers can access the
/// cached value concurrently, while refresh acquires an exclusive write lock.
///
/// Uses double-checked locking to avoid redundant refreshes when multiple
/// tasks race to refresh an expired value.
pub struct Cached<T: LoadCacheObject> {
    /// The cached value with its creation timestamp.
    value: RwLock<Option<CacheEntry<T>>>,
    /// Parameters passed to `LoadCacheObject::construct`.
    params: T::Params,
    /// How long a cached value remains valid.
    valid_for: Duration,
}

/// Internal entry storing the value and when it was created.
struct CacheEntry<T> {
    value: T,
    created_at: Instant,
}

impl<T: LoadCacheObject> Cached<T> {
    /// Creates a new cache with the default TTL (60 seconds).
    pub fn new(params: T::Params) -> Self {
        Self::with_ttl(params, Duration::from_secs(DEFAULT_CACHE_TTL_SECS))
    }

    /// Creates a new cache with a custom TTL.
    pub fn with_ttl(params: T::Params, valid_for: Duration) -> Self {
        Self {
            value: RwLock::new(None),
            params,
            valid_for,
        }
    }

    /// Returns the cached value, refreshing it if the TTL has expired.
    ///
    /// Uses double-checked locking:
    /// 1. Acquire read lock — if value is fresh, return it.
    /// 2. Acquire write lock — double-check freshness (another task may
    ///    have refreshed while we waited), then reconstruct if needed.
    ///
    /// # Errors
    ///
    /// Returns `T::Error` if `LoadCacheObject::construct` fails.
    pub async fn get(&self) -> Result<CacheRef<'_, T>, T::Error> {
        // Fast path: read lock, check if value is still valid
        {
            let read = self.value.read().await;
            if let Some(entry) = &*read
                && entry.created_at.elapsed() < self.valid_for
            {
                return Ok(CacheRef(read));
            }
        }

        // Slow path: write lock, refresh
        let mut write = self.value.write().await;

        // Double-check: another task may have refreshed while we waited
        if let Some(entry) = &*write
            && entry.created_at.elapsed() < self.valid_for
        {
            drop(write);
            return Ok(CacheRef(self.value.read().await));
        }

        let old_value = write.take().map(|entry| entry.value);
        let new_value = T::construct(old_value, &self.params).await?;
        *write = Some(CacheEntry {
            value: new_value,
            created_at: Instant::now(),
        });
        drop(write);

        Ok(CacheRef(self.value.read().await))
    }

    /// Invalidates the cached value, forcing a refresh on the next `get()`.
    pub async fn invalidate(&self) {
        *self.value.write().await = None;
    }

    /// Returns `true` if the cache currently holds a valid (non-expired) value.
    pub async fn is_valid(&self) -> bool {
        let read = self.value.read().await;
        read.as_ref()
            .is_some_and(|entry| entry.created_at.elapsed() < self.valid_for)
    }

    /// Returns the configured TTL duration.
    pub const fn ttl(&self) -> Duration {
        self.valid_for
    }
}

/// RAII guard providing read access to the cached value.
///
/// Dereferences to `&T` while holding the read lock. The lock is
/// released when this guard is dropped.
pub struct CacheRef<'a, T: LoadCacheObject>(
    tokio::sync::RwLockReadGuard<'a, Option<CacheEntry<T>>>,
);

impl<T: LoadCacheObject> Deref for CacheRef<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // CacheRef is only created after confirming the value exists.
        // The write lock in `get()` always sets Some before dropping to read.
        &self
            .0
            .as_ref()
            .expect("CacheRef created with None value — this is a bug")
            .value
    }
}

impl<T: LoadCacheObject + fmt::Debug> fmt::Debug for Cached<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Cached")
            .field("valid_for", &self.valid_for)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    /// Simple counter that tracks how many times construct() was called.
    struct Counter {
        value: u32,
    }

    impl LoadCacheObject for Counter {
        type Params = Arc<AtomicU32>;
        type Error = std::convert::Infallible;

        async fn construct(
            previous: Option<Self>,
            params: &Self::Params,
        ) -> Result<Self, Self::Error> {
            let call_count = params.fetch_add(1, Ordering::SeqCst);
            Ok(Self {
                value: previous.map_or(0, |p| p.value) + call_count + 1,
            })
        }
    }

    /// Value that always fails to construct.
    struct Failing;

    impl LoadCacheObject for Failing {
        type Params = ();
        type Error = String;

        async fn construct(
            _previous: Option<Self>,
            _params: &Self::Params,
        ) -> Result<Self, Self::Error> {
            Err("construction failed".to_string())
        }
    }

    #[tokio::test]
    async fn test_cache_returns_value() {
        let call_count = Arc::new(AtomicU32::new(0));
        let cache: Cached<Counter> =
            Cached::with_ttl(Arc::clone(&call_count), Duration::from_mins(1));

        let val = cache.get().await.unwrap();
        assert_eq!(val.value, 1);
        drop(val);
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_cache_reuses_within_ttl() {
        let call_count = Arc::new(AtomicU32::new(0));
        let cache: Cached<Counter> =
            Cached::with_ttl(Arc::clone(&call_count), Duration::from_mins(1));

        let val1 = cache.get().await.unwrap();
        assert_eq!(val1.value, 1);
        drop(val1);

        // Second call should reuse cached value
        let val2 = cache.get().await.unwrap();
        assert_eq!(val2.value, 1);
        drop(val2);
        assert_eq!(
            call_count.load(Ordering::SeqCst),
            1,
            "construct called only once"
        );
    }

    #[tokio::test]
    async fn test_cache_refreshes_after_ttl() {
        let call_count = Arc::new(AtomicU32::new(0));
        let cache: Cached<Counter> =
            Cached::with_ttl(Arc::clone(&call_count), Duration::from_millis(1));

        let val1 = cache.get().await.unwrap();
        assert_eq!(val1.value, 1);
        drop(val1);

        // Wait for TTL to expire
        tokio::time::sleep(Duration::from_millis(10)).await;

        let val2 = cache.get().await.unwrap();
        // Previous value (1) + call_count (1) + 1 = 3
        assert_eq!(val2.value, 3);
        drop(val2);
        assert_eq!(
            call_count.load(Ordering::SeqCst),
            2,
            "construct called twice"
        );
    }

    #[tokio::test]
    async fn test_cache_invalidate() {
        let call_count = Arc::new(AtomicU32::new(0));
        let cache: Cached<Counter> =
            Cached::with_ttl(Arc::clone(&call_count), Duration::from_mins(1));

        let val1 = cache.get().await.unwrap();
        assert_eq!(val1.value, 1);
        drop(val1);

        assert!(cache.is_valid().await);
        cache.invalidate().await;
        assert!(!cache.is_valid().await);

        // Next get() should reconstruct (no previous value after invalidate)
        let val2 = cache.get().await.unwrap();
        // No previous (invalidated), call_count=1, so 0 + 1 + 1 = 2
        assert_eq!(val2.value, 2);
        drop(val2);
        assert_eq!(call_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_cache_construction_error() {
        let cache: Cached<Failing> = Cached::with_ttl((), Duration::from_mins(1));

        let err = {
            let result = cache.get().await;
            assert!(result.is_err());
            result.err().expect("expected error")
        };
        assert_eq!(err, "construction failed");
        assert!(!cache.is_valid().await);
    }

    #[tokio::test]
    async fn test_cache_ttl_getter() {
        let call_count = Arc::new(AtomicU32::new(0));
        let cache: Cached<Counter> = Cached::with_ttl(call_count, Duration::from_secs(42));
        assert_eq!(cache.ttl(), Duration::from_secs(42));
    }
}
