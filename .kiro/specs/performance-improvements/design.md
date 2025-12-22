# Design Document: Performance Improvements

## Overview

This design document describes the architecture and implementation approach for performance improvements in RustConn, including native SPICE protocol embedding, search caching, lazy loading, tracing integration, and optimization of existing performance utilities.

The design follows RustConn's three-crate architecture:
- `rustconn-core`: Business logic, caching, batch processing (GUI-free)
- `rustconn`: GTK4 GUI integration, virtual scrolling, debounced search UI
- `rustconn-cli`: CLI operations with batch processing

## Architecture

### High-Level Component Diagram

```mermaid
graph TB
    subgraph "rustconn (GUI)"
        UI[GTK4 UI]
        Sidebar[Sidebar with Virtual Scrolling]
        SearchBox[Debounced Search Box]
        SpiceWidget[SPICE Embedded Widget]
    end
    
    subgraph "rustconn-core (Library)"
        SearchCache[Search Cache]
        LazyLoader[Lazy Group Loader]
        BatchProc[Batch Processor]
        StringInt[String Interner]
        Tracing[Tracing Integration]
        SpiceClient[SPICE Client]
        CredResolver[Async Credential Resolver]
    end
    
    UI --> Sidebar
    UI --> SearchBox
    UI --> SpiceWidget
    
    Sidebar --> LazyLoader
    Sidebar --> StringInt
    SearchBox --> SearchCache
    SpiceWidget --> SpiceClient
    
    SearchCache --> BatchProc
    LazyLoader --> BatchProc
    CredResolver --> Tracing
    SpiceClient --> Tracing
end
```

## Components and Interfaces

### 1. Native SPICE Protocol Embedding

The existing `SpiceClient` in `rustconn-core/src/spice_client/` will be enhanced to use the `spice-client` crate when the `spice-embedded` feature is enabled.

```rust
// rustconn-core/src/spice_client/client.rs
#[cfg(feature = "spice-embedded")]
use spice_client::{SpiceSession, SpiceChannel, SpiceDisplay};

pub struct SpiceClient {
    config: SpiceClientConfig,
    event_tx: mpsc::Sender<SpiceClientEvent>,
    command_rx: mpsc::Receiver<SpiceClientCommand>,
    #[cfg(feature = "spice-embedded")]
    session: Option<SpiceSession>,
}

impl SpiceClient {
    /// Connects to SPICE server using native protocol
    #[cfg(feature = "spice-embedded")]
    pub async fn connect_native(&mut self) -> Result<(), SpiceClientError> {
        let session = SpiceSession::new();
        session.set_host(&self.config.host);
        session.set_port(self.config.port);
        
        if let Some(ref password) = self.config.password {
            session.set_password(password.expose_secret());
        }
        
        session.connect().await?;
        self.session = Some(session);
        Ok(())
    }
    
    /// Falls back to external viewer if native fails
    pub fn fallback_to_viewer(&self) -> SpiceViewerLaunchResult {
        launch_spice_viewer(&self.config)
    }
}
```

### 2. Search Result Caching

New `SearchCache` component in `rustconn-core/src/search/`:

```rust
// rustconn-core/src/search/cache.rs
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub struct SearchCache {
    cache: HashMap<String, CachedResult>,
    max_entries: usize,
    ttl: Duration,
}

struct CachedResult {
    results: Vec<ConnectionSearchResult>,
    cached_at: Instant,
}

impl SearchCache {
    pub fn new(max_entries: usize, ttl: Duration) -> Self {
        Self {
            cache: HashMap::with_capacity(max_entries),
            max_entries,
            ttl,
        }
    }
    
    pub fn get(&self, query: &str) -> Option<&[ConnectionSearchResult]> {
        self.cache.get(query).and_then(|cached| {
            if cached.cached_at.elapsed() < self.ttl {
                Some(cached.results.as_slice())
            } else {
                None
            }
        })
    }
    
    pub fn insert(&mut self, query: String, results: Vec<ConnectionSearchResult>) {
        self.evict_stale();
        if self.cache.len() >= self.max_entries {
            self.evict_oldest();
        }
        self.cache.insert(query, CachedResult {
            results,
            cached_at: Instant::now(),
        });
    }
    
    pub fn invalidate_all(&mut self) {
        self.cache.clear();
    }
}
```

### 3. Lazy Loading for Connection Groups

New `LazyGroupLoader` in `rustconn-core/src/connection/`:

```rust
// rustconn-core/src/connection/lazy_loader.rs
use std::collections::HashSet;
use uuid::Uuid;

pub struct LazyGroupLoader {
    loaded_groups: HashSet<Uuid>,
    root_loaded: bool,
}

impl LazyGroupLoader {
    pub fn new() -> Self {
        Self {
            loaded_groups: HashSet::new(),
            root_loaded: false,
        }
    }
    
    pub fn is_group_loaded(&self, group_id: Uuid) -> bool {
        self.loaded_groups.contains(&group_id)
    }
    
    pub fn mark_group_loaded(&mut self, group_id: Uuid) {
        self.loaded_groups.insert(group_id);
    }
    
    pub fn get_children_to_load(
        &self,
        group_id: Uuid,
        all_groups: &[ConnectionGroup],
        all_connections: &[Connection],
    ) -> (Vec<ConnectionGroup>, Vec<Connection>) {
        let child_groups: Vec<_> = all_groups
            .iter()
            .filter(|g| g.parent_id == Some(group_id))
            .cloned()
            .collect();
            
        let child_connections: Vec<_> = all_connections
            .iter()
            .filter(|c| c.group_id == Some(group_id))
            .cloned()
            .collect();
            
        (child_groups, child_connections)
    }
}
```

### 4. Tracing Integration

New tracing module in `rustconn-core/src/tracing/`:

```rust
// rustconn-core/src/tracing/mod.rs
use tracing::{info_span, instrument, Span};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

pub struct TracingConfig {
    pub level: tracing::Level,
    pub output: TracingOutput,
    pub profiling_enabled: bool,
}

pub enum TracingOutput {
    Stdout,
    File(PathBuf),
    OpenTelemetry { endpoint: String },
}

pub fn init_tracing(config: &TracingConfig) -> Result<(), TracingError> {
    let subscriber = tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer()
            .with_level(true)
            .with_target(true));
    
    subscriber.init();
    Ok(())
}

/// Macro for creating operation spans with standard fields
#[macro_export]
macro_rules! trace_operation {
    ($name:expr, $($field:tt)*) => {
        tracing::info_span!($name, $($field)*)
    };
}
```

### 5. String Interning Integration

The existing `StringInterner` in `rustconn-core/src/performance/mod.rs` will be integrated into connection loading:

```rust
// Integration in rustconn-core/src/connection/manager.rs
use crate::performance::memory_optimizer;

impl ConnectionManager {
    pub fn load_connections(&mut self) -> Result<Vec<Connection>, ConfigError> {
        let connections = self.load_from_storage()?;
        
        // Intern protocol names for memory efficiency
        let interner = memory_optimizer().interner();
        for conn in &mut connections {
            // Protocol names are frequently repeated
            let protocol_name = conn.protocol.to_string();
            let _interned = interner.intern(&protocol_name);
        }
        
        Ok(connections)
    }
}
```

### 6. Virtual Scrolling Integration

The existing `VirtualScroller` will be integrated into the sidebar:

```rust
// rustconn/src/sidebar.rs
use rustconn_core::performance::VirtualScroller;

impl Sidebar {
    fn setup_virtual_scrolling(&mut self, connection_count: usize) {
        if connection_count > 100 {
            self.scroller = Some(VirtualScroller::new(
                connection_count,
                ROW_HEIGHT,
                self.viewport_height(),
            ).with_overscan(5));
        }
    }
    
    fn on_scroll(&mut self, offset: f64) {
        if let Some(ref mut scroller) = self.scroller {
            scroller.set_scroll_offset(offset);
            let (start, end) = scroller.visible_range();
            self.render_visible_items(start, end);
        }
    }
}
```

### 7. Debounced Search Integration

The existing `Debouncer` will be integrated into the search UI:

```rust
// rustconn/src/sidebar.rs
use rustconn_core::performance::Debouncer;

impl Sidebar {
    fn setup_search(&mut self) {
        self.search_debouncer = Debouncer::for_search(); // 100ms delay
        
        self.search_entry.connect_changed(|entry| {
            let text = entry.text();
            if self.search_debouncer.should_proceed() {
                self.execute_search(&text);
            } else {
                self.show_search_pending_indicator();
            }
        });
    }
}
```

### 8. Async Credential Resolution

Refactor `state.rs` to use async credential resolution:

```rust
// rustconn/src/state.rs
impl AppState {
    /// Async credential resolution without blocking
    pub async fn resolve_credentials_async(
        &self,
        connection: &Connection,
    ) -> Result<Option<Credentials>, String> {
        self.secret_manager
            .retrieve(&connection.id.to_string())
            .await
            .map_err(|e| e.to_string())
    }
    
    /// Spawns async credential resolution with callback
    pub fn resolve_credentials_with_callback<F>(
        &self,
        connection: Connection,
        callback: F,
    ) where
        F: FnOnce(Result<Option<Credentials>, String>) + Send + 'static,
    {
        let manager = self.secret_manager.clone();
        let conn_id = connection.id.to_string();
        
        glib::spawn_future_local(async move {
            let result = manager.retrieve(&conn_id).await.map_err(|e| e.to_string());
            callback(result);
        });
    }
}
```

### 9. Batch Processing Integration

The existing `BatchProcessor` will be integrated into import/export:

```rust
// rustconn-core/src/import/mod.rs
use crate::performance::BatchProcessor;

pub struct BatchImporter {
    processor: BatchProcessor<Connection>,
    progress_callback: Option<Box<dyn Fn(usize, usize)>>,
}

impl BatchImporter {
    pub fn new(batch_size: usize) -> Self {
        Self {
            processor: BatchProcessor::new(batch_size, Duration::from_millis(50)),
            progress_callback: None,
        }
    }
    
    pub fn import_batch(&mut self, connections: Vec<Connection>) -> ImportResult {
        let total = connections.len();
        let mut imported = 0;
        
        for conn in connections {
            if let Some(batch) = self.processor.add(conn) {
                self.process_batch(batch)?;
                imported += batch.len();
                if let Some(ref cb) = self.progress_callback {
                    cb(imported, total);
                }
            }
        }
        
        // Process remaining
        let remaining = self.processor.flush();
        self.process_batch(remaining)?;
        
        Ok(ImportSummary { imported: total })
    }
}
```

## Data Models

### Search Cache Entry

```rust
pub struct CachedSearchResult {
    pub query: String,
    pub results: Vec<ConnectionSearchResult>,
    pub cached_at: Instant,
    pub hit_count: usize,
}
```

### Tracing Configuration

```rust
pub struct TracingConfig {
    pub level: TracingLevel,
    pub output: TracingOutput,
    pub profiling_enabled: bool,
    pub include_connection_ids: bool,
    pub include_timing: bool,
}

pub enum TracingLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

pub enum TracingOutput {
    Stdout,
    File { path: PathBuf, rotate: bool },
    OpenTelemetry { endpoint: String },
}
```

## Correctness Properties

*A property is a characteristic or behavior that should hold true across all valid executions of a system—essentially, a formal statement about what the system should do. Properties serve as the bridge between human-readable specifications and machine-verifiable correctness guarantees.*

### Property 1: Search Cache Round-Trip

*For any* search query and connection set, caching the results and retrieving them within TTL SHALL return identical results.

**Validates: Requirements 2.1, 2.2**

### Property 2: Search Cache Invalidation

*For any* cached search results, modifying the connection set (add/update/delete) SHALL invalidate all cached entries.

**Validates: Requirements 2.3**

### Property 3: Search Cache TTL Expiration

*For any* cached search result, after TTL expiration, the cache SHALL return None for that query.

**Validates: Requirements 2.4**

### Property 4: Search Cache Size Limit

*For any* sequence of cache insertions, the cache size SHALL never exceed the configured maximum entries.

**Validates: Requirements 2.5**

### Property 5: Lazy Loading Initial State

*For any* connection database, initial sidebar load SHALL only include root-level groups and ungrouped connections.

**Validates: Requirements 3.1**

### Property 6: Lazy Loading Expansion

*For any* group expansion, the loaded children SHALL match the actual children in the database.

**Validates: Requirements 3.2, 3.3**

### Property 7: Search Ignores Lazy Loading

*For any* search query, results SHALL include matches from all connections regardless of lazy loading state.

**Validates: Requirements 3.5**

### Property 8: Tracing Span Creation

*For any* traced operation (connection, search, import/export, credential resolution), a tracing span SHALL be created with required fields.

**Validates: Requirements 4.2, 4.5**

### Property 9: String Interning Deduplication

*For any* set of connections with repeated protocol names, interning SHALL reduce memory usage compared to non-interned storage.

**Validates: Requirements 5.1, 5.2**

### Property 10: String Interning Statistics

*For any* sequence of intern operations, statistics (hit rate, bytes saved) SHALL be accurately tracked.

**Validates: Requirements 5.3, 5.4**

### Property 11: Virtual Scrolling Visible Range

*For any* scroll position and viewport size, the visible range SHALL include exactly the items that would be visible plus overscan buffer.

**Validates: Requirements 6.1, 6.2**

### Property 12: Virtual Scrolling Selection Preservation

*For any* selection state and scroll operation, the selection SHALL be preserved when items scroll in and out of view.

**Validates: Requirements 6.4**

### Property 13: Debounce Behavior

*For any* sequence of rapid inputs, only the final input after debounce delay SHALL trigger execution.

**Validates: Requirements 7.1, 7.2, 7.3**

### Property 14: SPICE Fallback on Failure

*For any* failed native SPICE connection attempt, the system SHALL attempt fallback to external viewer.

**Validates: Requirements 1.5**

### Property 15: Async Credential Resolution

*For any* credential resolution request, the operation SHALL complete without blocking the calling thread.

**Validates: Requirements 9.1, 9.4**

### Property 16: Batch Processing Size

*For any* batch operation, items SHALL be processed in batches not exceeding the configured batch size.

**Validates: Requirements 10.1, 10.2, 10.3**

### Property 17: Batch Processing Cancellation

*For any* cancelled batch operation, processing SHALL stop and partial results SHALL be reported.

**Validates: Requirements 10.5**

## Error Handling

### SPICE Connection Errors

```rust
#[derive(Debug, thiserror::Error)]
pub enum SpiceClientError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
    
    #[error("Authentication failed")]
    AuthenticationFailed,
    
    #[error("Native client unavailable, fallback required")]
    NativeUnavailable,
    
    #[error("Fallback viewer not found")]
    FallbackNotFound,
}
```

### Cache Errors

```rust
#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("Cache capacity exceeded")]
    CapacityExceeded,
    
    #[error("Invalid TTL configuration: {0}")]
    InvalidTtl(String),
}
```

### Tracing Errors

```rust
#[derive(Debug, thiserror::Error)]
pub enum TracingError {
    #[error("Failed to initialize tracing: {0}")]
    InitializationFailed(String),
    
    #[error("Invalid output configuration: {0}")]
    InvalidOutput(String),
}
```

## Testing Strategy

### Unit Tests

Unit tests will verify specific examples and edge cases:

- Search cache with empty queries
- Lazy loading with no groups
- Virtual scrolling with zero items
- Debouncer with immediate execution
- Batch processor with single item

### Property-Based Tests

Property-based tests using `proptest` will verify universal properties:

- **Search Cache Tests**: Round-trip, invalidation, TTL, size limits
- **Lazy Loading Tests**: Initial state, expansion, search coverage
- **Virtual Scrolling Tests**: Visible range calculation, selection preservation
- **Debounce Tests**: Timing behavior, reset behavior
- **Batch Processing Tests**: Size limits, cancellation

Each property test will run minimum 100 iterations and be tagged with:
```rust
// Feature: performance-improvements, Property N: [property description]
```

### Integration Tests

Integration tests will verify component interactions:

- Search with caching and debouncing
- Import with batch processing and progress reporting
- SPICE connection with fallback behavior

## Dependencies

### New Dependencies (rustconn-core/Cargo.toml)

```toml
[dependencies]
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

[dependencies.spice-client]
version = "0.2.0"
optional = true

[features]
spice-embedded = ["spice-client"]
```

### Existing Dependencies Used

- `tokio` - async runtime for credential resolution
- `proptest` - property-based testing
- `chrono` - timestamps for cache TTL
