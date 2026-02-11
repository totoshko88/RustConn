# RustConn Architecture Guide

**Version 0.8.2** | Last updated: February 2026

This document describes the internal architecture of RustConn for contributors and maintainers.

## Crate Structure

RustConn is a three-crate Cargo workspace with strict separation of concerns:

```
rustconn/           # GTK4 GUI application
rustconn-core/      # Business logic library (GUI-free)
rustconn-cli/       # Command-line interface
```

### Dependency Graph

```
┌─────────────┐     ┌─────────────────┐
│ rustconn    │────▶│  rustconn-core  │
│ (GUI)       │     │  (Library)      │
└─────────────┘     └─────────────────┘
                            ▲
┌─────────────┐             │
│ rustconn-cli│─────────────┘
│ (CLI)       │
└─────────────┘
```

### Crate Boundaries

| Crate | Purpose | Allowed Dependencies |
|-------|---------|---------------------|
| `rustconn-core` | Business logic, protocols, credentials, import/export | `tokio`, `serde`, `secrecy`, `thiserror` — NO GTK |
| `rustconn` | GTK4 UI, dialogs, terminal integration | `gtk4`, `vte4`, `libadwaita`, `rustconn-core` |
| `rustconn-cli` | CLI interface | `clap`, `rustconn-core` — NO GTK |

**Decision Rule:** "Does this code need GTK widgets?" → No → `rustconn-core` / Yes → `rustconn`

### Why This Separation?

1. **Testability**: Core logic can be tested without a display server
2. **Reusability**: CLI shares all business logic with GUI
3. **Build times**: Changes to UI don't recompile core logic
4. **Future flexibility**: Could support alternative UIs (TUI, web)

## State Management

### SharedAppState Pattern

The GUI uses a shared mutable state pattern for GTK's single-threaded model:

```rust
// rustconn/src/state.rs
pub type SharedAppState = Rc<RefCell<AppState>>;

pub struct AppState {
    connection_manager: ConnectionManager,
    session_manager: SessionManager,
    snippet_manager: SnippetManager,
    secret_manager: SecretManager,
    config_manager: ConfigManager,
    document_manager: DocumentManager,
    cluster_manager: ClusterManager,
    // ... cached credentials, clipboard, etc.
}
```

**Usage Pattern:**
```rust
fn do_something(state: &SharedAppState) {
    let state_ref = state.borrow();
    let connections = state_ref.connection_manager().connections();
    // Use data...
} // borrow released here

// For mutations:
fn update_something(state: &SharedAppState) {
    let mut state_ref = state.borrow_mut();
    state_ref.connection_manager_mut().add_connection(conn);
}
```

**Safe State Access Helpers:**

To reduce RefCell borrow panics, use the helper functions:

```rust
// Safe read access
with_state(&state, |s| {
    let connections = s.connection_manager().connections();
    // Use data...
});

// Safe read with error handling
let result = try_with_state(&state, |s| {
    s.connection_manager().get_connection(id)
});

// Safe write access
with_state_mut(&state, |s| {
    s.connection_manager_mut().add_connection(conn);
});

// Safe write with error handling
let result = try_with_state_mut(&state, |s| {
    s.connection_manager_mut().update_connection(conn)
});
```

**Rules:**
- Never hold a borrow across an async boundary
- Never hold a borrow when calling GTK methods that might trigger callbacks
- Prefer short-lived borrows over storing references
- Use `with_state`/`with_state_mut` helpers for safer access

### Manager Pattern

Each domain has a dedicated manager in `rustconn-core`:

| Manager | Responsibility |
|---------|---------------|
| `ConnectionManager` | CRUD for connections and groups |
| `SessionManager` | Active session tracking, logging |
| `SecretManager` | Credential storage with backend fallback |
| `ConfigManager` | Settings persistence |
| `DocumentManager` | Multi-document support |
| `SnippetManager` | Command snippets |
| `ClusterManager` | Connection clusters |

### Connection Retry

The `retry` module (`rustconn-core/src/connection/retry.rs`) provides automatic retry with exponential backoff:

```rust
// Configure retry behavior
let config = RetryConfig::default()
    .with_max_attempts(5)
    .with_base_delay(Duration::from_secs(1))
    .with_max_delay(Duration::from_secs(30))
    .with_jitter(true);

// Or use presets
let aggressive = RetryConfig::aggressive();   // 10 attempts, 500ms base
let conservative = RetryConfig::conservative(); // 3 attempts, 2s base
let no_retry = RetryConfig::no_retry();       // Single attempt

// Track retry state
let mut state = RetryState::new(&config);
while state.should_retry() {
    match attempt_connection().await {
        Ok(conn) => return Ok(conn),
        Err(e) if e.is_retryable() => {
            let delay = state.next_delay();
            tokio::time::sleep(delay).await;
        }
        Err(e) => return Err(e),
    }
}
```

### Session Health Monitoring

The `SessionManager` includes health check capabilities:

```rust
// Configure health checks
let config = HealthCheckConfig::default()
    .with_interval(Duration::from_secs(30))
    .with_auto_cleanup(true);

// Check session health
let status = session_manager.get_session_health(session_id);
match status {
    HealthStatus::Healthy => { /* Session is active */ }
    HealthStatus::Unhealthy(reason) => { /* Connection issues */ }
    HealthStatus::Unknown => { /* Status not determined */ }
    HealthStatus::Terminated => { /* Session ended */ }
}

// Get all unhealthy sessions
let problems = session_manager.unhealthy_sessions();
```

### Session State Persistence

The `restore` module (`rustconn-core/src/session/restore.rs`) handles session persistence:

```rust
// Save session state
let restore_data = SessionRestoreData {
    connection_id: conn.id,
    protocol: conn.protocol.clone(),
    started_at: session.started_at,
    split_layout: Some(SplitLayoutRestoreData { ... }),
};

let state = SessionRestoreState::new();
state.add_session(restore_data);
state.save_to_file(&config_dir.join("sessions.json"))?;

// Restore on startup
let state = SessionRestoreState::load_from_file(&path)?;
for session in state.sessions_within_age(max_age) {
    restore_session(session);
}
```

Managers own their data and handle I/O. They don't know about GTK.

### Debounced Persistence

The `ConnectionManager` uses debounced persistence to reduce disk I/O during rapid modifications:

```rust
// Changes are batched and saved after 2 seconds of inactivity
connection_manager.add_connection(conn);  // Triggers debounced save
connection_manager.update_connection(conn);  // Resets debounce timer

// Force immediate save (e.g., on application exit)
connection_manager.flush_persistence();
```

This is particularly useful during:
- Drag-and-drop reordering of multiple items
- Bulk import operations
- Rapid edits to connection properties

## Thread Safety Patterns

### Mutex Poisoning Recovery

When a thread panics while holding a mutex lock, the mutex becomes "poisoned" to signal that the protected data may be in an inconsistent state. By default, attempting to lock a poisoned mutex returns an error.

For simple state flags and process handles (like in `FreeRdpThread`), we can safely recover from poisoning by extracting the inner value:

```rust
// rustconn/src/embedded_rdp_thread.rs

/// Safely locks a mutex, recovering from poisoning by extracting the inner value.
fn lock_or_recover<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!("Mutex was poisoned, recovering inner value");
            poisoned.into_inner()
        }
    }
}

// Helper functions for common operations
fn set_state(mutex: &Mutex<FreeRdpThreadState>, state: FreeRdpThreadState) {
    *lock_or_recover(mutex) = state;
}

fn get_state(mutex: &Mutex<FreeRdpThreadState>) -> FreeRdpThreadState {
    *lock_or_recover(mutex)
}
```

**When to Use Poisoning Recovery:**
- Simple state flags (enums, booleans)
- Process handles that can be safely reset
- Data that doesn't have complex invariants

**When NOT to Use:**
- Complex data structures with invariants
- Financial or security-critical data
- Data where partial updates could cause corruption

**Rules:**
- Always log when recovering from poisoning
- Set an error state after recovery when appropriate
- Document why recovery is safe for the specific data type

## Async Patterns

### The Challenge

GTK4 runs on a single-threaded main loop. Blocking operations (network, disk, KeePass) would freeze the UI. We need to run async code without blocking GTK.

### Solution: Background Threads with Callbacks

```rust
// rustconn/src/utils.rs
pub fn spawn_blocking_with_callback<T, F, C>(operation: F, callback: C)
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
    C: FnOnce(T) + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    
    // Run operation in background thread
    std::thread::spawn(move || {
        let result = operation();
        let _ = tx.send(result);
    });
    
    // Poll for result on GTK main thread
    poll_for_result(rx, callback);
}

fn poll_for_result<T, C>(rx: Receiver<T>, callback: C)
where
    T: Send + 'static,
    C: FnOnce(T) + 'static,
{
    glib::idle_add_local_once(move || {
        match rx.try_recv() {
            Ok(result) => callback(result),
            Err(TryRecvError::Empty) => poll_for_result(rx, callback),
            Err(TryRecvError::Disconnected) => {
                tracing::error!("Background thread disconnected");
            }
        }
    });
}
```

**Usage:**
```rust
spawn_blocking_with_callback(
    move || {
        // Runs in background thread
        check_port(&host, port, timeout)
    },
    move |result| {
        // Runs on GTK main thread
        match result {
            Ok(open) => update_ui(open),
            Err(e) => show_error(e),
        }
    },
);
```

### Thread-Local Tokio Runtime

For async operations that need tokio (credential backends, etc.):

```rust
// rustconn/src/state.rs
thread_local! {
    static TOKIO_RUNTIME: RefCell<Option<tokio::runtime::Runtime>> = 
        const { RefCell::new(None) };
}

fn with_runtime<F, R>(f: F) -> Result<R, String>
where
    F: FnOnce(&tokio::runtime::Runtime) -> R,
{
    TOKIO_RUNTIME.with(|rt| {
        let mut rt_ref = rt.borrow_mut();
        if rt_ref.is_none() {
            *rt_ref = Some(tokio::runtime::Runtime::new()?);
        }
        Ok(f(rt_ref.as_ref().unwrap()))
    })
}
```

### Async Utilities Module

The `async_utils` module (`rustconn/src/async_utils.rs`) provides helpers for async operations in GTK:

```rust
// Non-blocking async on GLib main context
spawn_async(async move {
    let result = fetch_data().await;
    update_ui(result);
});

// Async with callback for result handling
spawn_async_with_callback(
    async move { expensive_operation().await },
    |result| handle_result(result),
);

// Blocking async with timeout (for operations that must complete)
let result = block_on_async_with_timeout(
    async move { critical_operation().await },
    Duration::from_secs(30),
)?;

// Thread safety checks
if is_main_thread() {
    update_widget();
}
ensure_main_thread(|| update_widget());
```

**When to Use What:**
- `spawn_blocking_with_callback`: Simple blocking operations
- `spawn_blocking_with_timeout`: Operations that might hang
- `with_runtime`: When you need tokio features (async traits, channels)
- `spawn_async`: Non-blocking async on GTK main thread
- `spawn_async_with_callback`: Async with result callback
- `block_on_async_with_timeout`: Bounded blocking for critical operations

## Error Handling

### Core Library Errors

All errors in `rustconn-core` use `thiserror`:

```rust
// rustconn-core/src/error.rs
#[derive(Debug, Error)]
pub enum RustConnError {
    #[error("Configuration error: {0}")]
    Config(#[from] ConfigError),
    
    #[error("Protocol error: {0}")]
    Protocol(#[from] ProtocolError),
    
    #[error("Secret storage error: {0}")]
    Secret(#[from] SecretError),
    // ...
}

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
    
    #[error("Client not found: {0}")]
    ClientNotFound(PathBuf),
    // ...
}
```

**Rules:**
- Every fallible function returns `Result<T, E>`
- Use `?` for propagation
- No `unwrap()` except for provably impossible states
- Include context in error messages

### GUI Error Display

The GUI converts technical errors to user-friendly messages:

```rust
// rustconn/src/error_display.rs
pub fn user_friendly_message(error: &AppStateError) -> String {
    match error {
        AppStateError::ConnectionNotFound(_) => 
            "The connection could not be found. It may have been deleted.".to_string(),
        AppStateError::CredentialError(_) => 
            "Could not access credentials. Check your secret storage settings.".to_string(),
        // ...
    }
}

pub fn show_error_dialog(parent: &impl IsA<gtk4::Window>, error: &AppStateError) {
    let dialog = adw::AlertDialog::new(
        Some("Error"),
        Some(&user_friendly_message(error)),
    );
    // Technical details in expandable section...
}
```

### Log Sanitization

The `logger` module (`rustconn-core/src/session/logger.rs`) automatically removes sensitive data from logs:

```rust
// Configure sanitization
let config = SanitizeConfig::default()
    .with_password_patterns(true)
    .with_api_key_patterns(true)
    .with_aws_credentials(true)
    .with_private_keys(true);

// Sanitize output before logging
let safe_output = sanitize_output(&raw_output, &config);
// "password=secret123" → "password=[REDACTED]"
// "AWS_SECRET_ACCESS_KEY=..." → "AWS_SECRET_ACCESS_KEY=[REDACTED]"

// Check if output contains sensitive prompts
if contains_sensitive_prompt(&output) {
    // Don't log this line
}
```

**Detected Patterns:**
- Passwords: `password=`, `passwd:`, `Password:` prompts
- API Keys: `api_key=`, `apikey=`, `api-key=`
- Tokens: `Bearer `, `token=`, `auth_token=`
- AWS: `AWS_SECRET_ACCESS_KEY`, `aws_secret_access_key`
- Private Keys: `-----BEGIN.*PRIVATE KEY-----`

## Credential Security

### SecretString Usage

All passwords and keys use `secrecy::SecretString`:

```rust
// rustconn-core/src/models/credentials.rs
pub struct Credentials {
    pub username: Option<String>,
    pub password: Option<SecretString>,      // Zeroed on drop
    pub key_passphrase: Option<SecretString>, // Zeroed on drop
    pub domain: Option<String>,
}
```

**Never:**
- Store passwords as plain `String`
- Log credential values
- Include credentials in error messages
- Serialize passwords to config files

### Secret Backend Abstraction

```rust
// rustconn-core/src/secret/backend.rs
#[async_trait]
pub trait SecretBackend: Send + Sync {
    async fn store(&self, connection_id: &str, credentials: &Credentials) -> SecretResult<()>;
    async fn retrieve(&self, connection_id: &str) -> SecretResult<Option<Credentials>>;
    async fn delete(&self, connection_id: &str) -> SecretResult<()>;
    async fn is_available(&self) -> bool;
    fn backend_id(&self) -> &'static str;
}
```

**Implementations:**
- `LibsecretBackend`: GNOME Keyring (default)
- `KeePassXcBackend`: KeePassXC via CLI
- `BitwardenBackend`: Bitwarden via CLI
- `OnePasswordBackend`: 1Password via CLI
- `PassboltBackend`: Passbolt via CLI (`go-passbolt-cli`)

### System Keyring Integration

The `keyring` module (`rustconn-core/src/secret/keyring.rs`) provides shared keyring storage via `secret-tool` (libsecret Secret Service API) for all backends that need system keyring integration:

```rust
// Check if secret-tool is available
if keyring::is_secret_tool_available().await {
    // Store a credential
    keyring::store("bitwarden-master", &password, "Bitwarden Master Password").await?;

    // Retrieve a credential
    if let Some(value) = keyring::lookup("bitwarden-master").await? {
        // Use value...
    }

    // Delete a credential
    keyring::clear("bitwarden-master").await?;
}
```

Each backend wraps these generic functions with typed helpers:
- Bitwarden: `store_master_password_in_keyring()` / `get_master_password_from_keyring()`
- 1Password: `store_token_in_keyring()` / `get_token_from_keyring()`
- Passbolt: `store_passphrase_in_keyring()` / `get_passphrase_from_keyring()`
- KeePassXC: `store_kdbx_password_in_keyring()` / `get_kdbx_password_from_keyring()`

On settings load, backends with "Save to system keyring" enabled automatically restore credentials from the keyring (auto-unlock for Bitwarden, token/passphrase/password pre-fill for others).

#### Flatpak Compatibility

The `secret-tool` binary is not included in the GNOME Flatpak runtime (`org.gnome.Platform`). To ensure keyring operations work inside the Flatpak sandbox, `libsecret` 0.21.7 is built as a Flatpak module in all manifests. This provides the `secret-tool` binary at `/app/bin/secret-tool`. The D-Bus permission `--talk-name=org.freedesktop.secrets` is already present in `finish-args`, allowing `secret-tool` to communicate with GNOME Keyring / KDE Wallet from within the sandbox.

### KeePass Hierarchical Storage

The `hierarchy` module (`rustconn-core/src/secret/hierarchy.rs`) manages hierarchical password storage in KeePass databases, mirroring RustConn's group structure:

```
KeePass Database
└── RustConn/                          # Root group for all RustConn entries
    ├── Groups/                        # Group-level credentials
    │   ├── Production/                # Mirrors RustConn group hierarchy
    │   │   └── Web Servers            # Group password entry
    │   └── Development/
    │       └── Local                  # Nested group password
    ├── server-01 (ssh)                # Connection credentials
    ├── Production/                    # Connections inherit group path
    │   └── web-server (rdp)
    └── Development/
        └── db-server (ssh)
```

**Key Functions:**

```rust
// Build entry path for a connection
let path = KeePassHierarchy::build_entry_path(&connection, &groups);
// Returns: "RustConn/Production/Web Servers/nginx-01"

// Build entry path for group credentials
let path = KeePassHierarchy::build_group_entry_path(&group, &groups);
// Returns: "RustConn/Groups/Production/Web Servers"

// Build lookup key for non-hierarchical backends (libsecret)
let key = KeePassHierarchy::build_group_lookup_key(&group, &groups, true);
// Returns: "group:Production-Web Servers"
```

**Group Credentials:**
- Groups can store shared credentials (username/password)
- Stored in `RustConn/Groups/{path}` to separate from connection entries
- Child connections can inherit group credentials via `PasswordSource::Group`

### Fallback Chain

`SecretManager` tries backends in priority order:

```rust
pub struct SecretManager {
    backends: Vec<Arc<dyn SecretBackend>>,
    cache: Arc<RwLock<HashMap<String, Credentials>>>,
}

impl SecretManager {
    async fn get_available_backend(&self) -> SecretResult<&Arc<dyn SecretBackend>> {
        for backend in &self.backends {
            if backend.is_available().await {
                return Ok(backend);
            }
        }
        Err(SecretError::BackendUnavailable("No backend available".into()))
    }
}
```

## Protocol Architecture

### Protocol Trait

```rust
// rustconn-core/src/protocol/mod.rs
pub trait Protocol: Send + Sync {
    fn protocol_id(&self) -> &'static str;
    fn display_name(&self) -> &'static str;
    fn default_port(&self) -> u16;
    fn validate_connection(&self, connection: &Connection) -> ProtocolResult<()>;
}
```

**Implementations:**
- `SshProtocol`: SSH via VTE terminal
- `RdpProtocol`: RDP via FreeRDP
- `VncProtocol`: VNC via TigerVNC
- `SpiceProtocol`: SPICE via remote-viewer
- `TelnetProtocol`: Telnet via external `telnet` client

### Adding a New Protocol

1. Create `rustconn-core/src/protocol/myprotocol.rs`
2. Implement `Protocol` trait
3. Add protocol config to `ProtocolConfig` enum
4. Register in `ProtocolRegistry`
5. Add UI fields in `rustconn/src/dialogs/connection/dialog.rs`

See `TelnetProtocol` for a minimal reference implementation using an external client.

### RDP Backend Selection

The `backend` module (`rustconn-core/src/rdp_client/backend.rs`) centralizes RDP backend selection:

```rust
// Detect available backends
let selector = RdpBackendSelector::new();
let backends = selector.detect_all();

// Select best backend for embedded mode (IronRDP > wlfreerdp)
let embedded = selector.select_embedded();

// Select best backend for external mode (xfreerdp3 > xfreerdp > freerdp)
let external = selector.select_external();

// Auto-select based on context
let best = selector.select_best();

// Check embedded support
if selector.has_embedded_support() {
    // Can use native RDP rendering
}
```

**Backend Priority:**
- **Embedded:** IronRDP (native Rust) → wlfreerdp (Wayland)
- **External:** xfreerdp3 → xfreerdp → freerdp (legacy)

## GTK4/Libadwaita Patterns

### Sidebar Module Structure

The sidebar is decomposed into focused submodules for maintainability:

```rust
// rustconn/src/sidebar/mod.rs - Main Sidebar struct and initialization
// rustconn/src/sidebar/search.rs - Search logic, predicates, history
// rustconn/src/sidebar/filter.rs - Protocol filter buttons
// rustconn/src/sidebar/view.rs - List item creation, binding, signals
// rustconn/src/sidebar/drag_drop.rs - Drag-and-drop with DragPayload
```

**Drag-and-Drop Payload:**
```rust
// Strongly typed drag payload (replaces string-based parsing)
#[derive(Serialize, Deserialize)]
pub enum DragPayload {
    Connection { id: Uuid },
    Group { id: Uuid },
}

// Serialize for drag data
let json = serde_json::to_string(&DragPayload::Connection { id })?;

// Deserialize on drop
let payload: DragPayload = serde_json::from_str(&data)?;
```

### Widget Hierarchy

```rust
// Correct libadwaita structure
let window = adw::ApplicationWindow::builder()
    .application(app)
    .build();

let toolbar_view = adw::ToolbarView::new();
toolbar_view.add_top_bar(&adw::HeaderBar::new());
toolbar_view.set_content(Some(&content));

window.set_content(Some(&toolbar_view));
```

### Toast Notifications

```rust
// rustconn/src/dialogs/adw_dialogs.rs
pub fn show_toast(overlay: &adw::ToastOverlay, message: &str) {
    let toast = adw::Toast::builder()
        .title(message)
        .timeout(3)
        .build();
    overlay.add_toast(toast);
}
```

### Signal Connections with State

```rust
button.connect_clicked(glib::clone!(
    #[weak] state,
    #[weak] window,
    move |_| {
        let state_ref = state.borrow();
        // Use state...
    }
));
```

## Directory Structure

```
rustconn/src/
├── app.rs                 # Application setup, CSS, actions
├── window.rs              # Main window layout
├── window_*.rs            # Window functionality by domain
├── state.rs               # SharedAppState
├── async_utils.rs         # Async helpers (spawn_async, block_on_async_with_timeout)
├── loading.rs             # LoadingOverlay, LoadingDialog components
├── sidebar/               # Connection tree (modular structure)
│   ├── mod.rs             # Module exports, Sidebar struct
│   ├── search.rs          # Search logic, predicates, history
│   ├── filter.rs          # Protocol filter buttons
│   ├── view.rs            # List item creation, binding, signals
│   └── drag_drop.rs       # Drag-and-drop logic with DragPayload
├── sidebar_types.rs       # Sidebar data types
├── sidebar_ui.rs          # Sidebar widget helpers
├── terminal/              # VTE terminal integration
├── dialogs/               # Modal dialogs
│   ├── widgets.rs         # Shared widget builders (CheckboxRow, EntryRow, SwitchRow, etc.)
│   ├── connection/        # Connection dialog (modular)
│   │   ├── mod.rs         # Module exports
│   │   ├── dialog.rs      # Main ConnectionDialog
│   │   ├── protocol_layout.rs # ProtocolLayoutBuilder for consistent UI
│   │   ├── shared_folders.rs  # Shared folders UI (RDP/SPICE)
│   │   ├── widgets.rs     # Re-exports from parent dialogs/widgets.rs
│   │   ├── ssh.rs         # SSH options
│   │   ├── rdp.rs         # RDP options
│   │   ├── vnc.rs         # VNC options
│   │   ├── spice.rs       # SPICE options
│   │   ├── telnet.rs      # Telnet options
│   │   └── zerotrust.rs   # Zero Trust provider options
│   ├── keyboard.rs        # Keyboard navigation helpers
│   ├── flatpak_components.rs  # Flatpak CLI download dialog
│   ├── settings/          # Settings tabs
│   └── ...
├── embedded_*.rs          # Embedded protocol viewers
└── utils.rs               # Async helpers, utilities

rustconn-core/src/
├── lib.rs                 # Public API re-exports
├── error.rs               # Error types
├── models/                # Data models
├── config/                # Settings persistence
├── connection/            # Connection management
│   ├── mod.rs             # Module exports
│   ├── manager.rs         # ConnectionManager with debounced persistence
│   ├── retry.rs           # RetryConfig, RetryState, exponential backoff
│   ├── port_check.rs      # TCP port reachability check
│   └── ...
├── protocol/              # Protocol implementations
├── secret/                # Credential backends
│   ├── mod.rs             # Module exports
│   ├── backend.rs         # SecretBackend trait
│   ├── manager.rs         # SecretManager with bulk operations
│   ├── resolver.rs        # CredentialResolver (Vault/Variable/Inherit resolution)
│   ├── hierarchy.rs       # KeePass hierarchical paths
│   ├── keyring.rs         # Shared system keyring via secret-tool
│   ├── libsecret.rs       # GNOME Keyring backend
│   ├── keepassxc.rs       # KeePassXC backend
│   ├── bitwarden.rs       # Bitwarden backend (with keyring storage)
│   ├── onepassword.rs     # 1Password backend (with keyring storage)
│   ├── passbolt.rs        # Passbolt backend (with keyring storage)
│   ├── detection.rs       # Password manager detection
│   ├── status.rs          # KeePass status detection
│   └── ...
├── session/               # Session management
│   ├── mod.rs             # Module exports
│   ├── manager.rs         # SessionManager with health checks
│   ├── logger.rs          # Session logging with sanitization
│   ├── restore.rs         # Session state persistence
│   └── ...
├── import/                # Format importers
│   ├── mod.rs             # Module exports
│   ├── traits.rs          # ImportSource trait, ImportStatistics
│   └── ...
├── export/                # Format exporters
├── rdp_client/            # RDP client implementation
│   ├── mod.rs             # Module exports
│   ├── backend.rs         # RdpBackendSelector
│   └── ...
├── cli_download.rs        # Flatpak CLI download manager
├── snap.rs                # Snap environment detection and paths
└── ...
```

## Testing

### Property Tests

Located in `rustconn-core/tests/properties/` (1250+ tests):

```rust
proptest! {
    #[test]
    fn connection_roundtrip(conn in arb_connection()) {
        let json = serde_json::to_string(&conn)?;
        let parsed: Connection = serde_json::from_str(&json)?;
        prop_assert_eq!(conn.id, parsed.id);
    }
}
```

**Test Modules:**
- `connection_tests.rs` — Connection CRUD operations
- `retry_tests.rs` — Retry logic with exponential backoff
- `session_restore_tests.rs` — Session persistence
- `health_check_tests.rs` — Session health monitoring
- `log_sanitization_tests.rs` — Sensitive data removal
- `rdp_backend_tests.rs` — RDP backend selection
- `vnc_client_tests.rs` — VNC client configuration
- `bulk_credential_tests.rs` — Bulk credential operations
- And 60+ more modules...

### Running Tests

```bash
cargo test                                    # All tests
cargo test -p rustconn-core                   # Core only
cargo test -p rustconn-core --test property_tests  # Property tests
```

## Build Commands

```bash
cargo build                    # Debug build
cargo build --release          # Release build
cargo run -p rustconn          # Run GUI
cargo run -p rustconn-cli      # Run CLI
cargo clippy --all-targets     # Lint (must pass)
cargo fmt --check              # Format check
```

## Contributing

1. **Check crate placement**: Business logic → `rustconn-core`; UI → `rustconn`
2. **Use SecretString**: For any credential data
3. **Return Result**: From all fallible functions
4. **Run clippy**: Must pass with no warnings
5. **Add tests**: Property tests for new core functionality
