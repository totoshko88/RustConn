//! Search result caching for improved performance
//!
//! This module provides a time-limited cache for search query results,
//! reducing redundant search operations for repeated queries.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::ConnectionSearchResult;

/// Default time-to-live for cached search results (30 seconds)
pub const DEFAULT_CACHE_TTL_SECS: u64 = 30;

/// Default maximum number of cached entries
pub const DEFAULT_MAX_CACHE_ENTRIES: usize = 100;

/// A cached search result with timestamp
#[derive(Clone)]
struct CachedEntry {
    /// The search results
    results: Vec<ConnectionSearchResult>,
    /// When the results were cached
    cached_at: Instant,
}

/// Search result cache with TTL and size limits
///
/// Caches search query results to avoid redundant search operations.
/// Entries expire after a configurable TTL and the cache has a maximum
/// size to prevent unbounded memory growth.
///
/// # Example
///
/// ```
/// use rustconn_core::search::cache::SearchCache;
/// use std::time::Duration;
///
/// let mut cache = SearchCache::new(100, Duration::from_secs(30));
///
/// // Insert results
/// cache.insert("server".to_string(), vec![]);
///
/// // Retrieve cached results
/// if let Some(results) = cache.get("server") {
///     // Use cached results
///     assert!(results.is_empty());
/// }
///
/// // Invalidate all entries when data changes
/// cache.invalidate_all();
/// ```
pub struct SearchCache {
    /// Cached entries keyed by query string
    cache: HashMap<String, CachedEntry>,
    /// Maximum number of entries to store
    max_entries: usize,
    /// Time-to-live for cached entries
    ttl: Duration,
}

impl SearchCache {
    /// Creates a new search cache with the specified limits
    ///
    /// # Arguments
    ///
    /// * `max_entries` - Maximum number of cached queries
    /// * `ttl` - Time-to-live for cached entries
    #[must_use]
    pub fn new(max_entries: usize, ttl: Duration) -> Self {
        Self {
            cache: HashMap::with_capacity(max_entries.min(64)),
            max_entries,
            ttl,
        }
    }

    /// Creates a new search cache with default settings
    ///
    /// Uses 100 max entries and 30 second TTL.
    #[must_use]
    pub fn with_defaults() -> Self {
        Self::new(
            DEFAULT_MAX_CACHE_ENTRIES,
            Duration::from_secs(DEFAULT_CACHE_TTL_SECS),
        )
    }

    /// Gets cached results for a query if they exist and haven't expired
    ///
    /// Returns `None` if the query is not cached or has expired.
    #[must_use]
    pub fn get(&self, query: &str) -> Option<&[ConnectionSearchResult]> {
        self.cache.get(query).and_then(|entry| {
            if entry.cached_at.elapsed() < self.ttl {
                Some(entry.results.as_slice())
            } else {
                None
            }
        })
    }

    /// Inserts search results into the cache
    ///
    /// If the cache is at capacity, stale entries are evicted first,
    /// then the oldest entry is evicted if still at capacity.
    pub fn insert(&mut self, query: String, results: Vec<ConnectionSearchResult>) {
        // First try to evict stale entries
        self.evict_stale();

        // If still at capacity, evict the oldest entry
        if self.cache.len() >= self.max_entries {
            self.evict_oldest();
        }

        self.cache.insert(
            query,
            CachedEntry {
                results,
                cached_at: Instant::now(),
            },
        );
    }

    /// Invalidates all cached entries
    ///
    /// Should be called when the underlying data changes (connection
    /// added, modified, or deleted).
    pub fn invalidate_all(&mut self) {
        self.cache.clear();
    }

    /// Evicts all entries that have exceeded their TTL
    ///
    /// Returns the number of entries evicted.
    pub fn evict_stale(&mut self) -> usize {
        let before = self.cache.len();
        self.cache
            .retain(|_, entry| entry.cached_at.elapsed() < self.ttl);
        before - self.cache.len()
    }

    /// Evicts the oldest entry from the cache
    ///
    /// Returns `true` if an entry was evicted, `false` if the cache was empty.
    pub fn evict_oldest(&mut self) -> bool {
        if self.cache.is_empty() {
            return false;
        }

        // Find the oldest entry
        let oldest_key = self
            .cache
            .iter()
            .min_by_key(|(_, entry)| entry.cached_at)
            .map(|(key, _)| key.clone());

        if let Some(key) = oldest_key {
            self.cache.remove(&key);
            true
        } else {
            false
        }
    }

    /// Returns the number of cached entries
    #[must_use]
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// Returns `true` if the cache is empty
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }

    /// Returns the maximum number of entries
    #[must_use]
    pub const fn max_entries(&self) -> usize {
        self.max_entries
    }

    /// Returns the TTL for cached entries
    #[must_use]
    pub const fn ttl(&self) -> Duration {
        self.ttl
    }

    /// Returns the number of valid (non-expired) entries
    #[must_use]
    pub fn valid_count(&self) -> usize {
        self.cache
            .values()
            .filter(|entry| entry.cached_at.elapsed() < self.ttl)
            .count()
    }
}

impl Default for SearchCache {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn create_test_result(score: f32) -> ConnectionSearchResult {
        ConnectionSearchResult::new(Uuid::new_v4(), score)
    }

    #[test]
    fn test_cache_insert_and_get() {
        let mut cache = SearchCache::with_defaults();
        let results = vec![create_test_result(0.9), create_test_result(0.8)];

        cache.insert("test".to_string(), results);

        let cached = cache.get("test");
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().len(), 2);
    }

    #[test]
    fn test_cache_miss_for_unknown_query() {
        let cache = SearchCache::with_defaults();
        assert!(cache.get("unknown").is_none());
    }

    #[test]
    fn test_cache_invalidate_all() {
        let mut cache = SearchCache::with_defaults();
        cache.insert("query1".to_string(), vec![create_test_result(0.9)]);
        cache.insert("query2".to_string(), vec![create_test_result(0.8)]);

        assert_eq!(cache.len(), 2);

        cache.invalidate_all();

        assert!(cache.is_empty());
        assert!(cache.get("query1").is_none());
        assert!(cache.get("query2").is_none());
    }

    #[test]
    fn test_cache_ttl_expiration() {
        let mut cache = SearchCache::new(100, Duration::from_millis(10));
        cache.insert("test".to_string(), vec![create_test_result(0.9)]);

        // Should be available immediately
        assert!(cache.get("test").is_some());

        // Wait for TTL to expire
        std::thread::sleep(Duration::from_millis(20));

        // Should be expired now
        assert!(cache.get("test").is_none());
    }

    #[test]
    fn test_cache_evict_stale() {
        let mut cache = SearchCache::new(100, Duration::from_millis(10));
        cache.insert("test1".to_string(), vec![create_test_result(0.9)]);
        cache.insert("test2".to_string(), vec![create_test_result(0.8)]);

        assert_eq!(cache.len(), 2);

        // Wait for TTL to expire
        std::thread::sleep(Duration::from_millis(20));

        let evicted = cache.evict_stale();
        assert_eq!(evicted, 2);
        assert!(cache.is_empty());
    }

    #[test]
    fn test_cache_size_limit() {
        let mut cache = SearchCache::new(3, Duration::from_secs(60));

        cache.insert("query1".to_string(), vec![]);
        cache.insert("query2".to_string(), vec![]);
        cache.insert("query3".to_string(), vec![]);

        assert_eq!(cache.len(), 3);

        // Adding a 4th entry should evict the oldest
        cache.insert("query4".to_string(), vec![]);

        assert_eq!(cache.len(), 3);
    }

    #[test]
    fn test_cache_evict_oldest() {
        let mut cache = SearchCache::new(100, Duration::from_secs(60));

        cache.insert("first".to_string(), vec![]);
        std::thread::sleep(Duration::from_millis(5));
        cache.insert("second".to_string(), vec![]);

        assert!(cache.evict_oldest());
        assert_eq!(cache.len(), 1);
        assert!(cache.get("first").is_none());
        assert!(cache.get("second").is_some());
    }

    #[test]
    fn test_cache_evict_oldest_empty() {
        let mut cache = SearchCache::with_defaults();
        assert!(!cache.evict_oldest());
    }

    #[test]
    fn test_cache_valid_count() {
        let mut cache = SearchCache::new(100, Duration::from_millis(50));

        cache.insert("test1".to_string(), vec![]);
        cache.insert("test2".to_string(), vec![]);

        assert_eq!(cache.valid_count(), 2);

        // Wait for TTL to expire
        std::thread::sleep(Duration::from_millis(60));

        // Entries are still in cache but not valid
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.valid_count(), 0);
    }
}
