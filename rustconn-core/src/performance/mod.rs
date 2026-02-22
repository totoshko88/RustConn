//! Performance optimization utilities for `RustConn`
//!
//! This module provides utilities for measuring and optimizing application performance,
//! including startup time profiling, lazy loading, debouncing, and memory optimization.

// Allow pedantic lints for this module - performance code uses many Mutex locks
// and the panics documentation would be excessive for internal metrics code.
// Cast warnings are acceptable here as we're dealing with metrics/statistics
// where precision loss is not critical.
// cast_possible_truncation, cast_precision_loss, unused_self allowed at workspace level
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::significant_drop_in_scrutinee)]
#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::format_push_string)]
#![allow(clippy::incompatible_msrv)]
#![allow(clippy::option_if_let_else)]
#![allow(clippy::len_zero)]

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

/// Global performance metrics instance
static METRICS: OnceLock<PerformanceMetrics> = OnceLock::new();

/// Gets the global performance metrics instance
#[must_use]
pub fn metrics() -> &'static PerformanceMetrics {
    METRICS.get_or_init(PerformanceMetrics::new)
}

/// Performance metrics collector for tracking application performance
///
/// Provides timing measurements for various operations including startup,
/// rendering, and search operations.
pub struct PerformanceMetrics {
    /// Startup timing measurements
    startup_timings: Mutex<HashMap<String, Duration>>,
    /// Operation timing measurements
    operation_timings: Mutex<HashMap<String, Vec<Duration>>>,
    /// Startup start time
    startup_start: Mutex<Option<Instant>>,
    /// Whether profiling is enabled
    profiling_enabled: AtomicBool,
}

impl PerformanceMetrics {
    /// Creates a new performance metrics instance
    #[must_use]
    pub fn new() -> Self {
        Self {
            startup_timings: Mutex::new(HashMap::new()),
            operation_timings: Mutex::new(HashMap::new()),
            startup_start: Mutex::new(None),
            profiling_enabled: AtomicBool::new(cfg!(debug_assertions)),
        }
    }

    /// Enables or disables profiling
    pub fn set_profiling_enabled(&self, enabled: bool) {
        self.profiling_enabled.store(enabled, Ordering::SeqCst);
    }

    /// Returns whether profiling is enabled
    #[must_use]
    pub fn is_profiling_enabled(&self) -> bool {
        self.profiling_enabled.load(Ordering::SeqCst)
    }

    /// Marks the start of application startup
    pub fn start_startup(&self) {
        if self.is_profiling_enabled() {
            *self.startup_start.lock().unwrap() = Some(Instant::now());
        }
    }

    /// Records a startup phase timing
    pub fn record_startup_phase(&self, phase: &str) {
        if !self.is_profiling_enabled() {
            return;
        }

        if let Some(start) = *self.startup_start.lock().unwrap() {
            let elapsed = start.elapsed();
            self.startup_timings
                .lock()
                .unwrap()
                .insert(phase.to_string(), elapsed);
        }
    }

    /// Records an operation timing
    pub fn record_operation(&self, operation: &str, duration: Duration) {
        if !self.is_profiling_enabled() {
            return;
        }

        self.operation_timings
            .lock()
            .unwrap()
            .entry(operation.to_string())
            .or_default()
            .push(duration);
    }

    /// Gets the total startup time
    #[must_use]
    pub fn total_startup_time(&self) -> Option<Duration> {
        self.startup_timings
            .lock()
            .unwrap()
            .get("complete")
            .copied()
    }

    /// Gets all startup phase timings
    #[must_use]
    pub fn startup_phases(&self) -> HashMap<String, Duration> {
        self.startup_timings.lock().unwrap().clone()
    }

    /// Gets average duration for an operation
    #[must_use]
    pub fn average_operation_time(&self, operation: &str) -> Option<Duration> {
        let timings = self.operation_timings.lock().unwrap();
        timings.get(operation).and_then(|durations| {
            if durations.is_empty() {
                None
            } else {
                let total: Duration = durations.iter().sum();
                Some(total / durations.len() as u32)
            }
        })
    }

    /// Gets operation statistics
    #[must_use]
    pub fn operation_stats(&self, operation: &str) -> Option<OperationStats> {
        let timings = self.operation_timings.lock().unwrap();
        timings.get(operation).and_then(|durations| {
            if durations.is_empty() {
                return None;
            }

            let mut sorted: Vec<_> = durations.clone();
            sorted.sort();

            let count = sorted.len();
            let total: Duration = sorted.iter().sum();
            let avg = total / count as u32;
            let min = sorted[0];
            let max = sorted[count - 1];
            let median = sorted[count / 2];
            let p95 = sorted[(count as f64 * 0.95) as usize];

            Some(OperationStats {
                count,
                total,
                average: avg,
                min,
                max,
                median,
                p95,
            })
        })
    }

    /// Clears all recorded metrics
    pub fn clear(&self) {
        self.startup_timings.lock().unwrap().clear();
        self.operation_timings.lock().unwrap().clear();
        *self.startup_start.lock().unwrap() = None;
    }

    /// Creates a timing guard for measuring operation duration
    #[must_use]
    pub fn time_operation(&self, operation: &str) -> TimingGuard<'_> {
        TimingGuard::new(self, operation)
    }
}

impl Default for PerformanceMetrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics for an operation
#[derive(Debug, Clone)]
pub struct OperationStats {
    /// Number of measurements
    pub count: usize,
    /// Total time across all measurements
    pub total: Duration,
    /// Average time per operation
    pub average: Duration,
    /// Minimum time
    pub min: Duration,
    /// Maximum time
    pub max: Duration,
    /// Median time
    pub median: Duration,
    /// 95th percentile time
    pub p95: Duration,
}

/// RAII guard for timing operations
pub struct TimingGuard<'a> {
    metrics: &'a PerformanceMetrics,
    operation: String,
    start: Instant,
}

impl<'a> TimingGuard<'a> {
    fn new(metrics: &'a PerformanceMetrics, operation: &str) -> Self {
        Self {
            metrics,
            operation: operation.to_string(),
            start: Instant::now(),
        }
    }
}

impl Drop for TimingGuard<'_> {
    fn drop(&mut self) {
        let duration = self.start.elapsed();
        self.metrics.record_operation(&self.operation, duration);
    }
}

/// Debouncer for rate-limiting operations
///
/// Useful for search input and other high-frequency events where
/// we want to wait for user input to settle before processing.
pub struct Debouncer {
    /// Minimum delay between operations
    delay: Duration,
    /// Last operation instant
    last_operation: Mutex<Option<Instant>>,
    /// Pending operation flag
    pending: AtomicBool,
}

impl Debouncer {
    /// Creates a new debouncer with the specified delay
    #[must_use]
    pub const fn new(delay: Duration) -> Self {
        Self {
            delay,
            last_operation: Mutex::new(None),
            pending: AtomicBool::new(false),
        }
    }

    /// Creates a debouncer with a 100ms delay (good for search input)
    #[must_use]
    pub const fn for_search() -> Self {
        Self::new(Duration::from_millis(100))
    }

    /// Creates a debouncer with a 16ms delay (good for rendering at 60fps)
    #[must_use]
    pub const fn for_render() -> Self {
        Self::new(Duration::from_millis(16))
    }

    /// Checks if enough time has passed since the last operation
    ///
    /// Returns `true` if the operation should proceed, `false` if it should be skipped.
    #[must_use]
    pub fn should_proceed(&self) -> bool {
        let now = Instant::now();
        let mut last = self.last_operation.lock().unwrap();

        match *last {
            None => {
                *last = Some(now);
                self.pending.store(false, Ordering::SeqCst);
                true
            }
            Some(last_time) if now.duration_since(last_time) >= self.delay => {
                *last = Some(now);
                self.pending.store(false, Ordering::SeqCst);
                true
            }
            _ => {
                self.pending.store(true, Ordering::SeqCst);
                false
            }
        }
    }

    /// Marks that there's a pending operation
    pub fn mark_pending(&self) {
        self.pending.store(true, Ordering::SeqCst);
    }

    /// Checks if there's a pending operation
    #[must_use]
    pub fn has_pending(&self) -> bool {
        self.pending.load(Ordering::SeqCst)
    }

    /// Resets the debouncer state
    pub fn reset(&self) {
        *self.last_operation.lock().unwrap() = None;
        self.pending.store(false, Ordering::SeqCst);
    }

    /// Gets the delay duration
    #[must_use]
    pub const fn delay(&self) -> Duration {
        self.delay
    }
}

/// Lazy initializer for deferred loading
///
/// Wraps a value that is initialized on first access, useful for
/// deferring expensive initialization until actually needed.
pub struct LazyInit<T, F = fn() -> T> {
    /// The initialized value
    value: OnceLock<T>,
    /// The initialization function
    init: F,
}

impl<T, F: Fn() -> T> LazyInit<T, F> {
    /// Creates a new lazy initializer with the given initialization function
    pub const fn new(init: F) -> Self {
        Self {
            value: OnceLock::new(),
            init,
        }
    }

    /// Gets the value, initializing it if necessary
    pub fn get(&self) -> &T {
        self.value.get_or_init(&self.init)
    }

    /// Checks if the value has been initialized
    #[must_use]
    pub fn is_initialized(&self) -> bool {
        self.value.get().is_some()
    }
}

/// Memory usage tracker
///
/// Provides utilities for tracking and optimizing memory usage.
pub struct MemoryTracker {
    /// Tracked allocations by category
    allocations: Mutex<HashMap<String, usize>>,
    /// Peak memory usage by category
    peak_usage: Mutex<HashMap<String, usize>>,
}

impl MemoryTracker {
    /// Creates a new memory tracker
    #[must_use]
    pub fn new() -> Self {
        Self {
            allocations: Mutex::new(HashMap::new()),
            peak_usage: Mutex::new(HashMap::new()),
        }
    }

    /// Records an allocation
    pub fn record_allocation(&self, category: &str, size: usize) {
        let mut allocations = self.allocations.lock().unwrap();
        let current = allocations.entry(category.to_string()).or_insert(0);
        *current += size;

        let mut peak = self.peak_usage.lock().unwrap();
        let peak_val = peak.entry(category.to_string()).or_insert(0);
        if *current > *peak_val {
            *peak_val = *current;
        }
    }

    /// Records a deallocation
    pub fn record_deallocation(&self, category: &str, size: usize) {
        let mut allocations = self.allocations.lock().unwrap();
        if let Some(current) = allocations.get_mut(category) {
            *current = current.saturating_sub(size);
        }
    }

    /// Gets current allocation for a category
    #[must_use]
    pub fn current_allocation(&self, category: &str) -> usize {
        self.allocations
            .lock()
            .unwrap()
            .get(category)
            .copied()
            .unwrap_or(0)
    }

    /// Gets peak allocation for a category
    #[must_use]
    pub fn peak_allocation(&self, category: &str) -> usize {
        self.peak_usage
            .lock()
            .unwrap()
            .get(category)
            .copied()
            .unwrap_or(0)
    }

    /// Gets total current allocation across all categories
    #[must_use]
    pub fn total_allocation(&self) -> usize {
        self.allocations.lock().unwrap().values().sum()
    }

    /// Gets all allocation statistics
    #[must_use]
    pub fn all_stats(&self) -> HashMap<String, AllocationStats> {
        let allocations = self.allocations.lock().unwrap();
        let peak = self.peak_usage.lock().unwrap();

        allocations
            .iter()
            .map(|(k, &current)| {
                let peak_val = peak.get(k).copied().unwrap_or(current);
                (
                    k.clone(),
                    AllocationStats {
                        current,
                        peak: peak_val,
                    },
                )
            })
            .collect()
    }

    /// Clears all tracking data
    pub fn clear(&self) {
        self.allocations.lock().unwrap().clear();
        self.peak_usage.lock().unwrap().clear();
    }
}

impl Default for MemoryTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Allocation statistics for a category
#[derive(Debug, Clone, Copy)]
pub struct AllocationStats {
    /// Current allocation in bytes
    pub current: usize,
    /// Peak allocation in bytes
    pub peak: usize,
}

/// Batch processor for optimizing bulk operations
///
/// Collects items and processes them in batches to reduce overhead.
pub struct BatchProcessor<T> {
    /// Items waiting to be processed
    items: Arc<Mutex<Vec<T>>>,
    /// Maximum batch size
    max_batch_size: usize,
    /// Maximum wait time before processing
    max_wait: Duration,
    /// Last flush time
    last_flush: Arc<Mutex<Instant>>,
}

impl<T> BatchProcessor<T> {
    /// Creates a new batch processor
    #[must_use]
    pub fn new(max_batch_size: usize, max_wait: Duration) -> Self {
        Self {
            items: Arc::new(Mutex::new(Vec::with_capacity(max_batch_size))),
            max_batch_size,
            max_wait,
            last_flush: Arc::new(Mutex::new(Instant::now())),
        }
    }

    /// Adds an item to the batch
    ///
    /// Returns `Some(Vec<T>)` if the batch should be processed now.
    pub fn add(&self, item: T) -> Option<Vec<T>> {
        let mut items = self.items.lock().unwrap();
        items.push(item);

        let should_flush = items.len() >= self.max_batch_size
            || self.last_flush.lock().unwrap().elapsed() >= self.max_wait;

        if should_flush {
            let batch = std::mem::take(&mut *items);
            *self.last_flush.lock().unwrap() = Instant::now();
            Some(batch)
        } else {
            None
        }
    }

    /// Forces a flush of all pending items
    #[must_use]
    pub fn flush(&self) -> Vec<T> {
        let mut items = self.items.lock().unwrap();
        let batch = std::mem::take(&mut *items);
        *self.last_flush.lock().unwrap() = Instant::now();
        batch
    }

    /// Returns the number of pending items
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.items.lock().unwrap().len()
    }

    /// Checks if there are pending items
    #[must_use]
    pub fn has_pending(&self) -> bool {
        !self.items.lock().unwrap().is_empty()
    }
}

impl<T> Default for BatchProcessor<T> {
    fn default() -> Self {
        Self::new(100, Duration::from_millis(50))
    }
}

/// Virtual scrolling helper for large lists
///
/// Calculates which items should be visible based on scroll position
/// and viewport size, enabling efficient rendering of large lists.
#[derive(Debug, Clone)]
pub struct VirtualScroller {
    /// Total number of items
    total_items: usize,
    /// Height of each item in pixels
    item_height: f64,
    /// Height of the viewport in pixels
    viewport_height: f64,
    /// Current scroll offset in pixels
    scroll_offset: f64,
    /// Number of items to render above/below visible area (buffer)
    overscan: usize,
}

impl VirtualScroller {
    /// Creates a new virtual scroller
    #[must_use]
    pub const fn new(total_items: usize, item_height: f64, viewport_height: f64) -> Self {
        Self {
            total_items,
            item_height,
            viewport_height,
            scroll_offset: 0.0,
            overscan: 5,
        }
    }

    /// Sets the overscan (buffer) count
    #[must_use]
    pub const fn with_overscan(mut self, overscan: usize) -> Self {
        self.overscan = overscan;
        self
    }

    /// Updates the scroll offset
    pub const fn set_scroll_offset(&mut self, offset: f64) {
        self.scroll_offset = offset.max(0.0);
    }

    /// Updates the viewport height
    pub const fn set_viewport_height(&mut self, height: f64) {
        self.viewport_height = height.max(0.0);
    }

    /// Updates the total item count
    pub const fn set_total_items(&mut self, count: usize) {
        self.total_items = count;
    }

    /// Gets the range of visible items (`start_index`, `end_index`)
    #[must_use]
    #[allow(clippy::cast_sign_loss)]
    pub fn visible_range(&self) -> (usize, usize) {
        if self.total_items == 0 || self.item_height <= 0.0 {
            return (0, 0);
        }

        let first_visible = (self.scroll_offset / self.item_height).floor() as usize;
        let visible_count = (self.viewport_height / self.item_height).ceil() as usize + 1;

        let start = first_visible.saturating_sub(self.overscan);
        let end = (first_visible + visible_count + self.overscan).min(self.total_items);

        (start, end)
    }

    /// Gets the total scrollable height
    #[must_use]
    pub fn total_height(&self) -> f64 {
        self.total_items as f64 * self.item_height
    }

    /// Gets the offset for a specific item index
    #[must_use]
    pub fn item_offset(&self, index: usize) -> f64 {
        index as f64 * self.item_height
    }

    /// Checks if an item is currently visible
    #[must_use]
    pub fn is_visible(&self, index: usize) -> bool {
        let (start, end) = self.visible_range();
        index >= start && index < end
    }
}

/// String interner for deduplicating repeated strings
///
/// Reduces memory usage when the same strings appear multiple times
/// (e.g., protocol names, common hostnames, usernames).
pub struct StringInterner {
    /// Interned strings storage
    strings: RwLock<HashMap<u64, Arc<str>>>,
    /// Statistics
    stats: InternerStats,
}

/// Statistics for the string interner
#[derive(Debug, Default)]
pub struct InternerStats {
    /// Number of intern requests
    pub intern_count: AtomicUsize,
    /// Number of cache hits
    pub hit_count: AtomicUsize,
    /// Number of unique strings stored
    pub unique_count: AtomicUsize,
    /// Estimated bytes saved through deduplication
    pub bytes_saved: AtomicUsize,
}

impl StringInterner {
    /// Creates a new string interner
    #[must_use]
    pub fn new() -> Self {
        Self {
            strings: RwLock::new(HashMap::new()),
            stats: InternerStats::default(),
        }
    }

    /// Interns a string, returning a reference-counted pointer
    ///
    /// If the string was already interned, returns the existing Arc.
    /// Otherwise, creates a new Arc and stores it.
    pub fn intern(&self, s: &str) -> Arc<str> {
        self.stats.intern_count.fetch_add(1, Ordering::Relaxed);

        let hash = self.hash_string(s);

        // Try read lock first for cache hit
        {
            let strings = self.strings.read().unwrap();
            if let Some(existing) = strings.get(&hash)
                && &**existing == s
            {
                self.stats.hit_count.fetch_add(1, Ordering::Relaxed);
                self.stats.bytes_saved.fetch_add(s.len(), Ordering::Relaxed);
                return Arc::clone(existing);
            }
        }

        // Need write lock to insert
        let mut strings = self.strings.write().unwrap();

        // Double-check after acquiring write lock
        if let Some(existing) = strings.get(&hash)
            && &**existing == s
        {
            self.stats.hit_count.fetch_add(1, Ordering::Relaxed);
            self.stats.bytes_saved.fetch_add(s.len(), Ordering::Relaxed);
            return Arc::clone(existing);
        }

        // Insert new string
        let arc: Arc<str> = Arc::from(s);
        strings.insert(hash, Arc::clone(&arc));
        self.stats.unique_count.fetch_add(1, Ordering::Relaxed);
        arc
    }

    /// Gets the interner statistics
    #[must_use]
    pub const fn stats(&self) -> &InternerStats {
        &self.stats
    }

    /// Returns the number of unique strings stored
    #[must_use]
    pub fn len(&self) -> usize {
        self.strings.read().unwrap().len()
    }

    /// Returns true if no strings are interned
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.strings.read().unwrap().is_empty()
    }

    /// Clears all interned strings
    pub fn clear(&self) {
        self.strings.write().unwrap().clear();
        self.stats.unique_count.store(0, Ordering::Relaxed);
    }

    fn hash_string(&self, s: &str) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        let mut hasher = DefaultHasher::new();
        s.hash(&mut hasher);
        hasher.finish()
    }
}

impl Default for StringInterner {
    fn default() -> Self {
        Self::new()
    }
}

/// Memory optimizer providing utilities for reducing memory usage
///
/// Provides methods for estimating memory usage, identifying optimization
/// opportunities, and implementing memory-efficient patterns.
pub struct MemoryOptimizer {
    /// String interner for deduplication
    interner: StringInterner,
    /// Memory usage snapshots
    snapshots: Mutex<Vec<MemorySnapshot>>,
    /// Maximum snapshots to retain
    max_snapshots: usize,
}

/// A snapshot of memory usage at a point in time
#[derive(Debug, Clone)]
pub struct MemorySnapshot {
    /// Timestamp of the snapshot
    pub timestamp: Instant,
    /// Label for the snapshot
    pub label: String,
    /// Estimated heap usage in bytes
    pub heap_estimate: usize,
    /// Number of connections
    pub connection_count: usize,
    /// Number of groups
    pub group_count: usize,
    /// Number of sessions
    pub session_count: usize,
}

/// Memory usage estimate for a data structure
#[derive(Debug, Clone, Default)]
pub struct MemoryEstimate {
    /// Stack size in bytes
    pub stack_size: usize,
    /// Heap size in bytes (estimated)
    pub heap_size: usize,
    /// Total size in bytes
    pub total_size: usize,
    /// Number of heap allocations (estimated)
    pub allocation_count: usize,
}

impl MemoryEstimate {
    /// Creates a new memory estimate
    #[must_use]
    pub const fn new(stack_size: usize, heap_size: usize, allocation_count: usize) -> Self {
        Self {
            stack_size,
            heap_size,
            total_size: stack_size + heap_size,
            allocation_count,
        }
    }

    /// Adds another estimate to this one
    #[must_use]
    pub const fn add(&self, other: &Self) -> Self {
        Self {
            stack_size: self.stack_size + other.stack_size,
            heap_size: self.heap_size + other.heap_size,
            total_size: self.total_size + other.total_size,
            allocation_count: self.allocation_count + other.allocation_count,
        }
    }

    /// Multiplies the estimate by a count
    #[must_use]
    pub const fn multiply(&self, count: usize) -> Self {
        Self {
            stack_size: self.stack_size * count,
            heap_size: self.heap_size * count,
            total_size: self.total_size * count,
            allocation_count: self.allocation_count * count,
        }
    }

    /// Formats the estimate as a human-readable string
    #[must_use]
    pub fn format(&self) -> String {
        format!(
            "stack: {}, heap: {}, total: {}, allocations: {}",
            format_bytes(self.stack_size),
            format_bytes(self.heap_size),
            format_bytes(self.total_size),
            self.allocation_count
        )
    }
}

/// Memory optimization recommendation
#[derive(Debug, Clone)]
pub struct OptimizationRecommendation {
    /// Category of the recommendation
    pub category: OptimizationCategory,
    /// Description of the issue
    pub description: String,
    /// Estimated memory savings in bytes
    pub estimated_savings: usize,
    /// Priority level (1-5, 5 being highest)
    pub priority: u8,
}

/// Categories of memory optimization
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptimizationCategory {
    /// String deduplication opportunities
    StringDeduplication,
    /// Collection capacity optimization
    CollectionCapacity,
    /// Unused data cleanup
    UnusedData,
    /// Data structure choice
    DataStructure,
    /// Cache management
    CacheManagement,
}

impl MemoryOptimizer {
    /// Creates a new memory optimizer
    #[must_use]
    pub fn new() -> Self {
        Self {
            interner: StringInterner::new(),
            snapshots: Mutex::new(Vec::new()),
            max_snapshots: 100,
        }
    }

    /// Gets the string interner
    #[must_use]
    pub const fn interner(&self) -> &StringInterner {
        &self.interner
    }

    /// Takes a memory snapshot
    pub fn take_snapshot(
        &self,
        label: &str,
        connection_count: usize,
        group_count: usize,
        session_count: usize,
    ) {
        let snapshot = MemorySnapshot {
            timestamp: Instant::now(),
            label: label.to_string(),
            heap_estimate: self.estimate_current_heap(connection_count, group_count, session_count),
            connection_count,
            group_count,
            session_count,
        };

        let mut snapshots = self.snapshots.lock().unwrap();
        if snapshots.len() >= self.max_snapshots {
            snapshots.remove(0);
        }
        snapshots.push(snapshot);
    }

    /// Gets all memory snapshots
    #[must_use]
    pub fn snapshots(&self) -> Vec<MemorySnapshot> {
        self.snapshots.lock().unwrap().clone()
    }

    /// Clears all snapshots
    pub fn clear_snapshots(&self) {
        self.snapshots.lock().unwrap().clear();
    }

    /// Estimates current heap usage based on data counts
    #[must_use]
    pub const fn estimate_current_heap(
        &self,
        connection_count: usize,
        group_count: usize,
        session_count: usize,
    ) -> usize {
        // Rough estimates based on typical data structure sizes
        const CONNECTION_SIZE: usize = 2048; // ~2KB per connection with all fields
        const GROUP_SIZE: usize = 256; // ~256 bytes per group
        const SESSION_SIZE: usize = 512; // ~512 bytes per session

        connection_count * CONNECTION_SIZE + group_count * GROUP_SIZE + session_count * SESSION_SIZE
    }

    /// Estimates memory usage for a string
    #[must_use]
    pub const fn estimate_string(s: &str) -> MemoryEstimate {
        let stack_size = std::mem::size_of::<String>();
        let heap_size = s.len() + std::mem::size_of::<usize>(); // capacity overhead
        MemoryEstimate::new(stack_size, heap_size, 1)
    }

    /// Estimates memory usage for a Vec
    #[must_use]
    pub const fn estimate_vec<T>(vec: &[T]) -> MemoryEstimate {
        let stack_size = std::mem::size_of::<Vec<T>>();
        let heap_size = std::mem::size_of_val(vec);
        MemoryEstimate::new(stack_size, heap_size, 1)
    }

    /// Estimates memory usage for a `HashMap`
    #[must_use]
    pub fn estimate_hashmap<K, V>(map: &HashMap<K, V>) -> MemoryEstimate {
        let stack_size = std::mem::size_of::<HashMap<K, V>>();
        // HashMap has ~1.5x overhead for hash table
        let entry_size = std::mem::size_of::<K>() + std::mem::size_of::<V>() + 8; // 8 bytes for hash
        let heap_size = (map.len() as f64 * entry_size as f64 * 1.5) as usize;
        MemoryEstimate::new(stack_size, heap_size, 1)
    }

    /// Analyzes memory usage and provides optimization recommendations
    #[must_use]
    pub fn analyze(
        &self,
        connection_count: usize,
        _group_count: usize,
        session_count: usize,
    ) -> Vec<OptimizationRecommendation> {
        let mut recommendations = Vec::new();

        // Check string interner effectiveness
        let interner_stats = self.interner.stats();
        let hit_rate = if interner_stats.intern_count.load(Ordering::Relaxed) > 0 {
            interner_stats.hit_count.load(Ordering::Relaxed) as f64
                / interner_stats.intern_count.load(Ordering::Relaxed) as f64
        } else {
            0.0
        };

        if hit_rate < 0.3 && interner_stats.intern_count.load(Ordering::Relaxed) > 100 {
            recommendations.push(OptimizationRecommendation {
                category: OptimizationCategory::StringDeduplication,
                description: format!(
                    "Low string interner hit rate ({:.1}%). Consider interning more repeated strings.",
                    hit_rate * 100.0
                ),
                estimated_savings: interner_stats.bytes_saved.load(Ordering::Relaxed) * 2,
                priority: 3,
            });
        }

        // Check for large connection counts
        if connection_count > 500 {
            recommendations.push(OptimizationRecommendation {
                category: OptimizationCategory::DataStructure,
                description: format!(
                    "Large connection count ({connection_count}). Consider using virtual scrolling for UI."
                ),
                estimated_savings: connection_count * 100, // UI widget overhead
                priority: 4,
            });
        }

        // Check for many active sessions
        if session_count > 20 {
            recommendations.push(OptimizationRecommendation {
                category: OptimizationCategory::CacheManagement,
                description: format!(
                    "Many active sessions ({session_count}). Consider session cleanup for idle connections."
                ),
                estimated_savings: session_count * 512,
                priority: 3,
            });
        }

        // Check snapshots for memory growth
        let snapshots = self.snapshots.lock().unwrap();
        if snapshots.len() >= 2 {
            let first = &snapshots[0];
            let last = &snapshots[snapshots.len() - 1];
            let growth = last.heap_estimate.saturating_sub(first.heap_estimate);
            let growth_rate = if first.heap_estimate > 0 {
                growth as f64 / first.heap_estimate as f64
            } else {
                0.0
            };

            if growth_rate > 0.5 && growth > 1024 * 1024 {
                recommendations.push(OptimizationRecommendation {
                    category: OptimizationCategory::UnusedData,
                    description: format!(
                        "Memory grew by {:.1}% ({}) since first snapshot. Check for memory leaks.",
                        growth_rate * 100.0,
                        format_bytes(growth)
                    ),
                    estimated_savings: growth / 2,
                    priority: 5,
                });
            }
        }

        // Sort by priority (highest first)
        recommendations.sort_by(|a, b| b.priority.cmp(&a.priority));
        recommendations
    }

    /// Generates a memory usage report
    #[must_use]
    pub fn generate_report(
        &self,
        connection_count: usize,
        group_count: usize,
        session_count: usize,
    ) -> String {
        let mut report = String::new();
        report.push_str("=== Memory Usage Report ===\n\n");

        // Current estimates
        let heap_estimate =
            self.estimate_current_heap(connection_count, group_count, session_count);
        report.push_str(&format!(
            "Estimated heap usage: {}\n",
            format_bytes(heap_estimate)
        ));
        report.push_str(&format!(
            "  Connections: {} ({} each)\n",
            connection_count,
            format_bytes(2048)
        ));
        report.push_str(&format!(
            "  Groups: {} ({} each)\n",
            group_count,
            format_bytes(256)
        ));
        report.push_str(&format!(
            "  Sessions: {} ({} each)\n\n",
            session_count,
            format_bytes(512)
        ));

        // String interner stats
        let stats = self.interner.stats();
        report.push_str("String Interner:\n");
        report.push_str(&format!(
            "  Unique strings: {}\n",
            stats.unique_count.load(Ordering::Relaxed)
        ));
        report.push_str(&format!(
            "  Intern requests: {}\n",
            stats.intern_count.load(Ordering::Relaxed)
        ));
        report.push_str(&format!(
            "  Cache hits: {}\n",
            stats.hit_count.load(Ordering::Relaxed)
        ));
        report.push_str(&format!(
            "  Bytes saved: {}\n\n",
            format_bytes(stats.bytes_saved.load(Ordering::Relaxed))
        ));

        // Recommendations
        let recommendations = self.analyze(connection_count, group_count, session_count);
        if recommendations.is_empty() {
            report.push_str("No optimization recommendations at this time.\n");
        } else {
            report.push_str("Optimization Recommendations:\n");
            for rec in &recommendations {
                report.push_str(&format!(
                    "  [P{}] {:?}: {}\n       Estimated savings: {}\n",
                    rec.priority,
                    rec.category,
                    rec.description,
                    format_bytes(rec.estimated_savings)
                ));
            }
        }

        report
    }
}

impl Default for MemoryOptimizer {
    fn default() -> Self {
        Self::new()
    }
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

/// Compact string storage for frequently repeated short strings
///
/// Uses a small inline buffer for short strings to avoid heap allocation.
/// Strings longer than the inline capacity fall back to heap allocation.
#[derive(Clone)]
pub struct CompactString {
    /// Storage: either inline bytes or heap-allocated String
    storage: CompactStringStorage,
}

#[derive(Clone)]
enum CompactStringStorage {
    /// Inline storage for short strings (up to 23 bytes on 64-bit)
    Inline {
        /// Length of the string
        len: u8,
        /// Inline buffer storing valid UTF-8
        buf: [u8; 23],
    },
    /// Heap-allocated string for longer strings
    Heap(String),
}

impl CompactString {
    /// Maximum length for inline storage
    pub const INLINE_CAPACITY: usize = 23;

    /// Creates a new compact string
    #[must_use]
    pub fn new(s: &str) -> Self {
        if s.len() <= Self::INLINE_CAPACITY {
            let mut buf = [0u8; 23];
            buf[..s.len()].copy_from_slice(s.as_bytes());
            Self {
                storage: CompactStringStorage::Inline {
                    len: s.len() as u8,
                    buf,
                },
            }
        } else {
            Self {
                storage: CompactStringStorage::Heap(s.to_string()),
            }
        }
    }

    /// Returns the string as a slice
    #[must_use]
    pub fn as_str(&self) -> &str {
        match &self.storage {
            CompactStringStorage::Inline { len, buf } => {
                // We only store valid UTF-8 in the buffer (from str input)
                // This unwrap is safe because we only copy from valid UTF-8 strings
                std::str::from_utf8(&buf[..*len as usize])
                    .expect("CompactString buffer should contain valid UTF-8")
            }
            CompactStringStorage::Heap(s) => s,
        }
    }

    /// Returns true if the string is stored inline
    #[must_use]
    pub const fn is_inline(&self) -> bool {
        matches!(self.storage, CompactStringStorage::Inline { .. })
    }

    /// Returns the length of the string
    #[must_use]
    pub fn len(&self) -> usize {
        match &self.storage {
            CompactStringStorage::Inline { len, .. } => *len as usize,
            CompactStringStorage::Heap(s) => s.len(),
        }
    }

    /// Returns true if the string is empty
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl std::fmt::Debug for CompactString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompactString")
            .field("value", &self.as_str())
            .field("inline", &self.is_inline())
            .finish()
    }
}

impl std::fmt::Display for CompactString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl PartialEq for CompactString {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for CompactString {}

impl Hash for CompactString {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

impl From<&str> for CompactString {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for CompactString {
    fn from(s: String) -> Self {
        Self::new(&s)
    }
}

impl AsRef<str> for CompactString {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

/// Global memory optimizer instance
static MEMORY_OPTIMIZER: OnceLock<MemoryOptimizer> = OnceLock::new();

/// Gets the global memory optimizer instance
#[must_use]
pub fn memory_optimizer() -> &'static MemoryOptimizer {
    MEMORY_OPTIMIZER.get_or_init(MemoryOptimizer::new)
}

/// Memory-efficient pool for reusing allocations
///
/// Reduces allocation overhead by reusing previously allocated objects.
pub struct ObjectPool<T> {
    /// Pool of available objects
    pool: Mutex<Vec<T>>,
    /// Maximum pool size
    max_size: usize,
    /// Statistics
    stats: PoolStats,
}

/// Statistics for the object pool
#[derive(Debug, Default)]
pub struct PoolStats {
    /// Number of objects acquired
    pub acquired: AtomicUsize,
    /// Number of objects returned
    pub returned: AtomicUsize,
    /// Number of objects created (not from pool)
    pub created: AtomicUsize,
    /// Number of objects dropped (pool full)
    pub dropped: AtomicUsize,
}

impl<T: Default> ObjectPool<T> {
    /// Creates a new object pool with the specified maximum size
    #[must_use]
    pub fn new(max_size: usize) -> Self {
        Self {
            pool: Mutex::new(Vec::with_capacity(max_size)),
            max_size,
            stats: PoolStats::default(),
        }
    }

    /// Acquires an object from the pool, or creates a new one if empty
    pub fn acquire(&self) -> T {
        self.stats.acquired.fetch_add(1, Ordering::Relaxed);

        let mut pool = self.pool.lock().unwrap();
        if let Some(obj) = pool.pop() {
            obj
        } else {
            self.stats.created.fetch_add(1, Ordering::Relaxed);
            T::default()
        }
    }

    /// Returns an object to the pool for reuse
    pub fn release(&self, obj: T) {
        self.stats.returned.fetch_add(1, Ordering::Relaxed);

        let mut pool = self.pool.lock().unwrap();
        if pool.len() < self.max_size {
            pool.push(obj);
        } else {
            self.stats.dropped.fetch_add(1, Ordering::Relaxed);
            // Object is dropped here
        }
    }

    /// Returns the current pool size
    #[must_use]
    pub fn size(&self) -> usize {
        self.pool.lock().unwrap().len()
    }

    /// Returns pool statistics
    #[must_use]
    pub const fn stats(&self) -> &PoolStats {
        &self.stats
    }

    /// Clears the pool
    pub fn clear(&self) {
        self.pool.lock().unwrap().clear();
    }

    /// Pre-populates the pool with objects
    pub fn warm(&self, count: usize) {
        let mut pool = self.pool.lock().unwrap();
        let to_add = count.min(self.max_size).saturating_sub(pool.len());
        for _ in 0..to_add {
            pool.push(T::default());
        }
    }
}

/// Shrinkable wrapper for collections that can release excess capacity
///
/// Wraps a Vec and periodically shrinks it to reduce memory usage.
pub struct ShrinkableVec<T> {
    /// Inner vector (pub for testing)
    pub inner: Vec<T>,
    /// Shrink threshold (shrink when capacity > len * threshold)
    shrink_threshold: f64,
    /// Minimum capacity to maintain
    min_capacity: usize,
}

impl<T> ShrinkableVec<T> {
    /// Creates a new shrinkable vector
    #[must_use]
    pub const fn new() -> Self {
        Self {
            inner: Vec::new(),
            shrink_threshold: 2.0,
            min_capacity: 16,
        }
    }

    /// Creates with custom shrink settings
    #[must_use]
    pub const fn with_settings(shrink_threshold: f64, min_capacity: usize) -> Self {
        Self {
            inner: Vec::new(),
            shrink_threshold,
            min_capacity,
        }
    }

    /// Pushes an item
    pub fn push(&mut self, item: T) {
        self.inner.push(item);
    }

    /// Removes and returns the last item
    pub fn pop(&mut self) -> Option<T> {
        let result = self.inner.pop();
        self.maybe_shrink();
        result
    }

    /// Removes an item at index
    pub fn remove(&mut self, index: usize) -> T {
        let result = self.inner.remove(index);
        self.maybe_shrink();
        result
    }

    /// Clears the vector
    pub fn clear(&mut self) {
        self.inner.clear();
        self.maybe_shrink();
    }

    /// Returns the length
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns true if empty
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns the capacity
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    /// Returns a slice
    #[must_use]
    pub fn as_slice(&self) -> &[T] {
        &self.inner
    }

    /// Returns a mutable slice
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.inner
    }

    /// Forces a shrink operation
    pub fn shrink(&mut self) {
        let target = self.inner.len().max(self.min_capacity);
        self.inner.shrink_to(target);
    }

    /// Shrinks if capacity exceeds threshold
    fn maybe_shrink(&mut self) {
        let len = self.inner.len();
        let capacity = self.inner.capacity();

        if capacity > self.min_capacity && capacity as f64 > len as f64 * self.shrink_threshold {
            self.shrink();
        }
    }

    /// Returns the inner vector
    #[must_use]
    pub fn into_inner(self) -> Vec<T> {
        self.inner
    }

    /// Returns estimated memory savings from shrinking
    #[must_use]
    pub fn potential_savings(&self) -> usize {
        let excess = self
            .inner
            .capacity()
            .saturating_sub(self.inner.len().max(self.min_capacity));
        excess * std::mem::size_of::<T>()
    }
}

impl<T> Default for ShrinkableVec<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> std::ops::Deref for ShrinkableVec<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T> std::ops::DerefMut for ShrinkableVec<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

/// Memory pressure levels for adaptive behavior
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MemoryPressure {
    /// Low memory usage, normal operation
    Low,
    /// Moderate memory usage, consider optimization
    Moderate,
    /// High memory usage, aggressive optimization needed
    High,
    /// Critical memory usage, emergency measures
    Critical,
}

impl MemoryPressure {
    /// Determines memory pressure based on estimated usage and threshold
    #[must_use]
    pub fn from_usage(current_bytes: usize, threshold_bytes: usize) -> Self {
        let ratio = current_bytes as f64 / threshold_bytes as f64;

        if ratio < 0.5 {
            Self::Low
        } else if ratio < 0.75 {
            Self::Moderate
        } else if ratio < 0.9 {
            Self::High
        } else {
            Self::Critical
        }
    }

    /// Returns recommended actions for this pressure level
    #[must_use]
    pub const fn recommended_actions(&self) -> &'static [&'static str] {
        match self {
            Self::Low => &[],
            Self::Moderate => &[
                "Consider clearing unused caches",
                "Shrink over-allocated collections",
            ],
            Self::High => &[
                "Clear all caches",
                "Shrink all collections",
                "Disable non-essential features",
            ],
            Self::Critical => &[
                "Emergency cache clear",
                "Close idle sessions",
                "Reduce connection list to visible items only",
            ],
        }
    }
}

/// Detailed memory breakdown by category
#[derive(Debug, Clone, Default)]
pub struct MemoryBreakdown {
    /// Memory used by connections
    pub connections: usize,
    /// Memory used by groups
    pub groups: usize,
    /// Memory used by sessions
    pub sessions: usize,
    /// Memory used by snippets
    pub snippets: usize,
    /// Memory used by templates
    pub templates: usize,
    /// Memory used by clusters
    pub clusters: usize,
    /// Memory used by caches
    pub caches: usize,
    /// Memory used by UI state
    pub ui_state: usize,
    /// Other/overhead memory
    pub overhead: usize,
}

impl MemoryBreakdown {
    /// Creates a new memory breakdown with estimated sizes
    #[must_use]
    pub const fn estimate(
        connection_count: usize,
        group_count: usize,
        session_count: usize,
        snippet_count: usize,
        template_count: usize,
        cluster_count: usize,
    ) -> Self {
        // Size estimates based on typical struct sizes
        const CONNECTION_SIZE: usize = 2048;
        const GROUP_SIZE: usize = 256;
        const SESSION_SIZE: usize = 512;
        const SNIPPET_SIZE: usize = 512;
        const TEMPLATE_SIZE: usize = 1024;
        const CLUSTER_SIZE: usize = 384;
        const CACHE_OVERHEAD: usize = 4096;
        const UI_STATE_SIZE: usize = 8192;
        const BASE_OVERHEAD: usize = 16384;

        Self {
            connections: connection_count * CONNECTION_SIZE,
            groups: group_count * GROUP_SIZE,
            sessions: session_count * SESSION_SIZE,
            snippets: snippet_count * SNIPPET_SIZE,
            templates: template_count * TEMPLATE_SIZE,
            clusters: cluster_count * CLUSTER_SIZE,
            caches: CACHE_OVERHEAD,
            ui_state: UI_STATE_SIZE,
            overhead: BASE_OVERHEAD,
        }
    }

    /// Returns total estimated memory usage
    #[must_use]
    pub const fn total(&self) -> usize {
        self.connections
            + self.groups
            + self.sessions
            + self.snippets
            + self.templates
            + self.clusters
            + self.caches
            + self.ui_state
            + self.overhead
    }

    /// Returns the largest memory consumer
    #[must_use]
    pub fn largest_consumer(&self) -> (&'static str, usize) {
        let categories = [
            ("connections", self.connections),
            ("groups", self.groups),
            ("sessions", self.sessions),
            ("snippets", self.snippets),
            ("templates", self.templates),
            ("clusters", self.clusters),
            ("caches", self.caches),
            ("ui_state", self.ui_state),
            ("overhead", self.overhead),
        ];

        categories
            .into_iter()
            .max_by_key(|(_, size)| *size)
            .unwrap_or(("unknown", 0))
    }

    /// Formats the breakdown as a report
    #[must_use]
    pub fn format_report(&self) -> String {
        let mut report = String::new();
        report.push_str("Memory Breakdown:\n");
        report.push_str(&format!(
            "  Connections: {}\n",
            format_bytes(self.connections)
        ));
        report.push_str(&format!("  Groups: {}\n", format_bytes(self.groups)));
        report.push_str(&format!("  Sessions: {}\n", format_bytes(self.sessions)));
        report.push_str(&format!("  Snippets: {}\n", format_bytes(self.snippets)));
        report.push_str(&format!("  Templates: {}\n", format_bytes(self.templates)));
        report.push_str(&format!("  Clusters: {}\n", format_bytes(self.clusters)));
        report.push_str(&format!("  Caches: {}\n", format_bytes(self.caches)));
        report.push_str(&format!("  UI State: {}\n", format_bytes(self.ui_state)));
        report.push_str(&format!("  Overhead: {}\n", format_bytes(self.overhead)));
        report.push_str("  ─────────────────\n");
        report.push_str(&format!("  Total: {}\n", format_bytes(self.total())));
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_performance_metrics_startup() {
        let metrics = PerformanceMetrics::new();
        metrics.set_profiling_enabled(true);
        metrics.start_startup();

        std::thread::sleep(Duration::from_millis(10));
        metrics.record_startup_phase("config_load");

        std::thread::sleep(Duration::from_millis(10));
        metrics.record_startup_phase("complete");

        let phases = metrics.startup_phases();
        assert!(phases.contains_key("config_load"));
        assert!(phases.contains_key("complete"));
        assert!(phases["complete"] > phases["config_load"]);
    }

    #[test]
    fn test_performance_metrics_operations() {
        let metrics = PerformanceMetrics::new();
        metrics.set_profiling_enabled(true);

        metrics.record_operation("search", Duration::from_millis(10));
        metrics.record_operation("search", Duration::from_millis(20));
        metrics.record_operation("search", Duration::from_millis(15));

        let avg = metrics.average_operation_time("search").unwrap();
        assert!(avg >= Duration::from_millis(14) && avg <= Duration::from_millis(16));

        let stats = metrics.operation_stats("search").unwrap();
        assert_eq!(stats.count, 3);
        assert_eq!(stats.min, Duration::from_millis(10));
        assert_eq!(stats.max, Duration::from_millis(20));
    }

    #[test]
    fn test_timing_guard() {
        let metrics = PerformanceMetrics::new();
        metrics.set_profiling_enabled(true);

        {
            let _guard = metrics.time_operation("test_op");
            std::thread::sleep(Duration::from_millis(5));
        }

        let stats = metrics.operation_stats("test_op");
        assert!(stats.is_some());
        assert_eq!(stats.unwrap().count, 1);
    }

    #[test]
    fn test_debouncer() {
        let debouncer = Debouncer::new(Duration::from_millis(50));

        // First call should proceed
        assert!(debouncer.should_proceed());

        // Immediate second call should not proceed
        assert!(!debouncer.should_proceed());
        assert!(debouncer.has_pending());

        // After delay, should proceed again
        std::thread::sleep(Duration::from_millis(60));
        assert!(debouncer.should_proceed());
    }

    #[test]
    fn test_lazy_init() {
        let counter = std::sync::atomic::AtomicUsize::new(0);
        let lazy = LazyInit::new(|| {
            counter.fetch_add(1, Ordering::SeqCst);
            42
        });

        assert!(!lazy.is_initialized());
        assert_eq!(*lazy.get(), 42);
        assert!(lazy.is_initialized());
        assert_eq!(*lazy.get(), 42);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_memory_tracker() {
        let tracker = MemoryTracker::new();

        tracker.record_allocation("connections", 1000);
        tracker.record_allocation("connections", 500);
        assert_eq!(tracker.current_allocation("connections"), 1500);

        tracker.record_deallocation("connections", 300);
        assert_eq!(tracker.current_allocation("connections"), 1200);
        assert_eq!(tracker.peak_allocation("connections"), 1500);
    }

    #[test]
    fn test_batch_processor() {
        let processor: BatchProcessor<i32> = BatchProcessor::new(3, Duration::from_secs(10));

        assert!(processor.add(1).is_none());
        assert!(processor.add(2).is_none());

        let batch = processor.add(3);
        assert!(batch.is_some());
        assert_eq!(batch.unwrap(), vec![1, 2, 3]);

        processor.add(4);
        let remaining = processor.flush();
        assert_eq!(remaining, vec![4]);
    }

    #[test]
    fn test_virtual_scroller() {
        let mut scroller = VirtualScroller::new(100, 30.0, 300.0);

        // At top, should show items 0-15 (10 visible + 1 extra + 5 overscan below)
        let (start, end) = scroller.visible_range();
        assert_eq!(start, 0);
        assert!(end <= 16);
        assert!(end > 0);

        // Scroll down significantly (past the overscan buffer)
        scroller.set_scroll_offset(300.0); // 10 items down
        let (start, end) = scroller.visible_range();
        // start should be 10 - 5 (overscan) = 5
        assert!(start >= 5);
        assert!(end > start);

        // Check visibility
        assert!(scroller.is_visible(start));
        assert!(scroller.is_visible(end - 1));
        assert!(!scroller.is_visible(end + 10));
    }

    #[test]
    fn test_virtual_scroller_empty() {
        let scroller = VirtualScroller::new(0, 30.0, 300.0);
        let (start, end) = scroller.visible_range();
        assert_eq!(start, 0);
        assert_eq!(end, 0);
    }

    #[test]
    fn test_virtual_scroller_total_height() {
        let scroller = VirtualScroller::new(100, 30.0, 300.0);
        assert!((scroller.total_height() - 3000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_string_interner_basic() {
        let interner = StringInterner::new();

        let s1 = interner.intern("hello");
        let s2 = interner.intern("hello");
        let s3 = interner.intern("world");

        // Same string should return same Arc
        assert!(Arc::ptr_eq(&s1, &s2));
        // Different strings should be different
        assert!(!Arc::ptr_eq(&s1, &s3));

        assert_eq!(interner.len(), 2);
        assert_eq!(interner.stats().hit_count.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_string_interner_stats() {
        let interner = StringInterner::new();

        interner.intern("test");
        interner.intern("test");
        interner.intern("test");
        interner.intern("other");

        let stats = interner.stats();
        assert_eq!(stats.intern_count.load(Ordering::Relaxed), 4);
        assert_eq!(stats.hit_count.load(Ordering::Relaxed), 2);
        assert_eq!(stats.unique_count.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn test_memory_estimate() {
        let est1 = MemoryEstimate::new(8, 100, 1);
        let est2 = MemoryEstimate::new(8, 50, 1);

        let combined = est1.add(&est2);
        assert_eq!(combined.stack_size, 16);
        assert_eq!(combined.heap_size, 150);
        assert_eq!(combined.total_size, 166);
        assert_eq!(combined.allocation_count, 2);

        let multiplied = est1.multiply(3);
        assert_eq!(multiplied.stack_size, 24);
        assert_eq!(multiplied.heap_size, 300);
    }

    #[test]
    fn test_memory_optimizer_snapshot() {
        let optimizer = MemoryOptimizer::new();

        optimizer.take_snapshot("initial", 10, 2, 1);
        optimizer.take_snapshot("after_load", 50, 5, 3);

        let snapshots = optimizer.snapshots();
        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[0].label, "initial");
        assert_eq!(snapshots[1].label, "after_load");
        assert_eq!(snapshots[0].connection_count, 10);
        assert_eq!(snapshots[1].connection_count, 50);
    }

    #[test]
    fn test_memory_optimizer_estimate() {
        let optimizer = MemoryOptimizer::new();

        let estimate = optimizer.estimate_current_heap(10, 2, 1);
        // 10 * 2048 + 2 * 256 + 1 * 512 = 20480 + 512 + 512 = 21504
        assert_eq!(estimate, 21504);
    }

    #[test]
    fn test_memory_optimizer_report() {
        let optimizer = MemoryOptimizer::new();
        optimizer.interner().intern("test");
        optimizer.interner().intern("test");

        let report = optimizer.generate_report(10, 2, 1);
        assert!(report.contains("Memory Usage Report"));
        assert!(report.contains("Connections: 10"));
        assert!(report.contains("String Interner"));
    }

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
    fn test_compact_string_inline() {
        let short = CompactString::new("hello");
        assert!(short.is_inline());
        assert_eq!(short.as_str(), "hello");
        assert_eq!(short.len(), 5);
    }

    #[test]
    fn test_compact_string_heap() {
        let long = CompactString::new("this is a very long string that exceeds inline capacity");
        assert!(!long.is_inline());
        assert_eq!(
            long.as_str(),
            "this is a very long string that exceeds inline capacity"
        );
    }

    #[test]
    fn test_compact_string_equality() {
        let s1 = CompactString::new("test");
        let s2 = CompactString::new("test");
        let s3 = CompactString::new("other");

        assert_eq!(s1, s2);
        assert_ne!(s1, s3);
    }

    #[test]
    fn test_compact_string_empty() {
        let empty = CompactString::new("");
        assert!(empty.is_empty());
        assert!(empty.is_inline());
        assert_eq!(empty.len(), 0);
    }

    #[test]
    fn test_compact_string_max_inline() {
        // Test string at exactly inline capacity
        let max_inline = "a".repeat(CompactString::INLINE_CAPACITY);
        let s = CompactString::new(&max_inline);
        assert!(s.is_inline());
        assert_eq!(s.len(), CompactString::INLINE_CAPACITY);

        // Test string just over inline capacity
        let over_inline = "a".repeat(CompactString::INLINE_CAPACITY + 1);
        let s2 = CompactString::new(&over_inline);
        assert!(!s2.is_inline());
    }

    #[test]
    fn test_global_memory_optimizer() {
        let optimizer = memory_optimizer();
        optimizer.interner().intern("global_test");
        assert!(optimizer.interner().len() >= 1);
    }

    #[test]
    fn test_optimization_recommendations() {
        let optimizer = MemoryOptimizer::new();

        // Test with large connection count
        let recommendations = optimizer.analyze(600, 10, 5);
        assert!(
            recommendations
                .iter()
                .any(|r| r.category == OptimizationCategory::DataStructure)
        );

        // Test with many sessions
        let recommendations = optimizer.analyze(10, 5, 25);
        assert!(
            recommendations
                .iter()
                .any(|r| r.category == OptimizationCategory::CacheManagement)
        );
    }

    #[test]
    fn test_object_pool_basic() {
        let pool: ObjectPool<Vec<u8>> = ObjectPool::new(10);

        // Acquire creates new object when pool is empty
        let obj1 = pool.acquire();
        assert!(obj1.is_empty());
        assert_eq!(pool.stats().created.load(Ordering::Relaxed), 1);

        // Release returns object to pool
        pool.release(obj1);
        assert_eq!(pool.size(), 1);

        // Acquire reuses pooled object
        let _obj2 = pool.acquire();
        assert_eq!(pool.size(), 0);
        assert_eq!(pool.stats().acquired.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn test_object_pool_max_size() {
        let pool: ObjectPool<Vec<u8>> = ObjectPool::new(2);

        pool.release(Vec::new());
        pool.release(Vec::new());
        pool.release(Vec::new()); // Should be dropped

        assert_eq!(pool.size(), 2);
        assert_eq!(pool.stats().dropped.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_object_pool_warm() {
        let pool: ObjectPool<Vec<u8>> = ObjectPool::new(10);
        pool.warm(5);
        assert_eq!(pool.size(), 5);
    }

    #[test]
    fn test_shrinkable_vec_basic() {
        let mut vec: ShrinkableVec<i32> = ShrinkableVec::new();

        vec.push(1);
        vec.push(2);
        vec.push(3);

        assert_eq!(vec.len(), 3);
        assert_eq!(vec.as_slice(), &[1, 2, 3]);

        assert_eq!(vec.pop(), Some(3));
        assert_eq!(vec.len(), 2);
    }

    #[test]
    fn test_shrinkable_vec_shrink() {
        let mut vec: ShrinkableVec<i32> = ShrinkableVec::with_settings(1.5, 4);

        // Add many items
        for i in 0..100 {
            vec.push(i);
        }

        let initial_capacity = vec.capacity();

        // Clear and check shrinking
        vec.clear();
        vec.shrink();

        assert!(vec.capacity() < initial_capacity);
    }

    #[test]
    fn test_shrinkable_vec_potential_savings() {
        // Use settings that won't auto-shrink on clear
        let mut vec: ShrinkableVec<i32> = ShrinkableVec::with_settings(10.0, 4);

        for i in 0..100 {
            vec.push(i);
        }

        // Don't clear - just check potential savings with excess capacity
        // After pushing 100 items, capacity is likely > 100
        // Potential savings = (capacity - max(len, min_capacity)) * size_of::<i32>()
        // With 100 items and min_capacity=4, savings should be based on capacity - 100

        // Instead, let's test with a partially filled vec
        let mut vec2: ShrinkableVec<i32> = ShrinkableVec::with_settings(10.0, 4);
        vec2.inner.reserve(200); // Force large capacity
        vec2.push(1);
        vec2.push(2);

        let savings = vec2.potential_savings();
        // capacity is ~200, len is 2, min_capacity is 4
        // savings = (200 - 4) * 4 bytes = 784 bytes
        assert!(savings > 0, "Expected savings > 0, got {savings}");
    }

    #[test]
    fn test_memory_pressure_levels() {
        assert_eq!(MemoryPressure::from_usage(40, 100), MemoryPressure::Low);
        assert_eq!(
            MemoryPressure::from_usage(60, 100),
            MemoryPressure::Moderate
        );
        assert_eq!(MemoryPressure::from_usage(80, 100), MemoryPressure::High);
        assert_eq!(
            MemoryPressure::from_usage(95, 100),
            MemoryPressure::Critical
        );
    }

    #[test]
    fn test_memory_pressure_recommendations() {
        assert!(MemoryPressure::Low.recommended_actions().is_empty());
        assert!(!MemoryPressure::Moderate.recommended_actions().is_empty());
        assert!(!MemoryPressure::High.recommended_actions().is_empty());
        assert!(!MemoryPressure::Critical.recommended_actions().is_empty());
    }

    #[test]
    fn test_memory_breakdown_estimate() {
        let breakdown = MemoryBreakdown::estimate(10, 5, 2, 3, 1, 2);

        assert!(breakdown.connections > 0);
        assert!(breakdown.groups > 0);
        assert!(breakdown.sessions > 0);
        assert!(breakdown.total() > 0);
    }

    #[test]
    fn test_memory_breakdown_largest_consumer() {
        let breakdown = MemoryBreakdown::estimate(100, 5, 2, 3, 1, 2);
        let (name, _size) = breakdown.largest_consumer();
        assert_eq!(name, "connections");
    }

    #[test]
    fn test_memory_breakdown_report() {
        let breakdown = MemoryBreakdown::estimate(10, 5, 2, 3, 1, 2);
        let report = breakdown.format_report();

        assert!(report.contains("Memory Breakdown"));
        assert!(report.contains("Connections"));
        assert!(report.contains("Total"));
    }
}
