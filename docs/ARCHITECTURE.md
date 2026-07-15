# RustConn Architecture Guide

**Version 0.18.9** | Last updated: July 2026

This document describes the internal architecture of RustConn for contributors and maintainers.

## Crate Structure

RustConn is a four-crate Cargo workspace (Rust 2024 edition) with strict separation of concerns:

```
rustconn/           # GTK4 GUI application
rustconn-core/      # Business logic library (GUI-free)
rustconn-cli/       # Command-line interface
rustconn-pty-sys/   # Isolated FFI helper (macOS PTY controlling terminal)
```

### Dependency Graph

```
┌─────────────┐     ┌─────────────────┐     ┌────────────────────┐
│ rustconn    │────▶│  rustconn-core  │     │  rustconn-pty-sys  │
│ (GUI)       │──┐  │  (Library)      │     │  (FFI, libc only)  │
└─────────────┘  │  └─────────────────┘     └────────────────────┘
                 │          ▲                        ▲
┌─────────────┐  │          │                        │
│ rustconn-cli│──┼──────────┘                        │
│ (CLI)       │  └───────────────────────────────────┘
└─────────────┘   (rustconn → rustconn-pty-sys, macOS PTY only)
```

### Crate Boundaries

| Crate | Purpose | Dependency Boundary |
|-------|---------|---------------------|
| `rustconn-core` | Domain logic: models, config persistence, CRUD managers, protocol data, import/export, credential abstractions | Non-GUI Rust dependencies only; host keyring and embedded-client runtimes stay behind explicit features — NO GTK |
| `rustconn` | GTK4 UI, dialogs, terminal integration, embedded/external session presentation | Owns `gtk4`, `vte4`, `libadwaita`, and enables core integration features when needed |
| `rustconn-cli` | Headless management CLI: config, connections, import/export, list/show, simple operations | CLI/runtime dependencies plus `rustconn-core`; client launch and secret-management stay behind explicit features — NO GTK |
| `rustconn-pty-sys` | FFI helper: give a spawned child its PTY slave as a controlling terminal (`setsid` + `TIOCSCTTY`) for the macOS native PTY ([#175](https://github.com/totoshko88/RustConn/issues/175)) | `libc` only — NO GTK |

**Decision Rule:** "Does this code need GTK widgets?" → No → `rustconn-core` / Yes → `rustconn`

### Headless Boundary

`rustconn-core` defaults to an empty feature set. A minimal build is the shared
domain kernel and must not pull GUI, DBus/keyring, embedded-client, GFX, or RD
Gateway runtime dependencies by default.

Optional integration features:

| Feature | Owner | Purpose |
|---------|-------|---------|
| `rustconn-core/system-keyring` | GUI / full CLI | Host keyring integration (`oo7` or macOS Keychain) |
| `rustconn-core/vnc-embedded` | GUI | Native VNC client runtime |
| `rustconn-core/rdp-embedded` | GUI | Native IronRDP client runtime |
| `rustconn-core/gfx-h264` | GUI | RDP EGFX/H.264 pipeline |
| `rustconn-core/rd-gateway` | GUI | Native RD Gateway tunneling for embedded RDP |
| `rustconn-cli/client-launch` | Full CLI | Launch external clients or desktop file managers |
| `rustconn-cli/secret-management` | Full CLI | Secret backend commands and system keyring support |

Default edit targets:

| Change | Start here | Do not start here |
|--------|------------|-------------------|
| Connection fields, validation, serialization | `rustconn-core/src/models`, `rustconn-core/src/connection`, `rustconn-core/src/protocol` | `rustconn/src/dialogs` |
| Headless CRUD/list/import/export behavior | `rustconn-cli/src/commands`, `rustconn-core/src/config`, `rustconn-core/src/import`, `rustconn-core/src/export` | `rustconn/src/window` |
| Embedded RDP/VNC runtime behavior | `rustconn-core/src/rdp_client`, `rustconn-core/src/vnc_client`, then `rustconn/src/embedded_*` | Generic core managers |
| GUI dialogs, session widgets, toasts | `rustconn/src/dialogs`, `rustconn/src/window`, `rustconn/src/embedded_*` | `rustconn-core` |
| Secret storage model | `rustconn-core/src/secret` | GUI settings pages, unless only presentation changes |

Keep desktop/client integration behind explicit features or in `rustconn/`.
Core may expose data types and pure builders for those integrations, but should
not require their runtime dependencies in the default feature set.

### The `unsafe` Exception

The workspace sets `unsafe_code = "forbid"` in every crate **except** `rustconn-pty-sys`.
That crate is the single sanctioned location for `unsafe`, following the M-UNSAFE
guideline (isolate FFI in a small `-sys` crate with a documented safety contract)
instead of relaxing the lint in the main crates. It exposes one safe function,
`set_controlling_terminal()`, which registers a `pre_exec` hook calling only
async-signal-safe `libc` functions. See `rustconn-pty-sys/src/lib.rs` and its
usage in `rustconn/src/macos_pty.rs`.

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
    template_manager: TemplateManager,
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
| `TemplateManager` | Connection template CRUD, search, import/export |
| `ClusterManager` | Connection clusters |

### Connection Retry

The `retry` module (`rustconn-core/src/connection/retry.rs`) provides automatic retry with exponential backoff:

```rust
// Configure retry behavior per connection
let config = RetryConfig::default()
    .with_max_attempts(5)
    .with_initial_delay_ms(1000)
    .with_max_delay_ms(30_000)
    .with_backoff_multiplier(2.0)
    .with_enabled(true);

// Or use presets
let aggressive = RetryConfig::aggressive();     // 5 attempts, 500ms initial, 1.5× multiplier
let conservative = RetryConfig::conservative(); // 2 attempts, 2000ms initial, 3× multiplier
let no_retry = RetryConfig::no_retry();         // Disabled

// Track retry state during reconnection
let mut state = RetryState::new(config);
loop {
    if let Some(delay) = state.next_delay() {
        tokio::time::sleep(delay).await;
    } else {
        break; // All retries exhausted
    }
    if check_host_online(&host, port).await? {
        state.record_success();
        return Ok(true);
    }
    if !state.record_failure("Host offline") {
        return Ok(false); // Exhausted
    }
}
```

**Per-connection configuration:** Each connection stores an optional `retry_config: Option<RetryConfig>` field (serialized with `#[serde(default)]`). When `None`, the default config (3 attempts, 1s initial, 2× multiplier) is used. The "Automatic Reconnection" section in the connection dialog Advanced tab allows users to configure retry behavior.

**Auto-reconnect flow:**
1. Session terminates unexpectedly (not SSH auth failure, not rapid crash <5s)
2. `RetryConfig` is read from the connection (or default)
3. `poll_until_online_with_backoff()` probes the host with exponential delays
4. On success → triggers reconnect callback to reuse the existing tab
5. On exhaustion → stops polling, marks session as failed

**Validation:** `delay_for_attempt()` enforces a minimum of 100ms for `initial_delay_ms` and ensures `max_delay_ms >= initial_delay_ms` to prevent degenerate configurations from deserialized data.

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

### Network Change Monitor

The GUI crate (`rustconn/src/window/network_monitor.rs`) subscribes to `gio::NetworkMonitor::network_changed` and orchestrates the reaction to interface switches:

```
NetworkMonitor::network_changed signal
        │
        ▼
┌───────────────────────────────────────────────┐
│ Rate-limit check (>3 events / 60s → quiet)    │
├───────────────────────────────────────────────┤
│ Connectivity check (< Full → skip reconnect)  │
├───────────────────────────────────────────────┤
│ 1. close_all_control_sockets (ssh -O exit "_")│
│ 2. Toast: "Network changed"                   │
│ 3. glib::timeout_add_local_once(500ms) {      │
│       reconnect eligible sessions             │
│    }                                          │
└───────────────────────────────────────────────┘
```

**Design decisions:**
- Lives in `rustconn/` (GUI crate) because it uses `gio::NetworkMonitor`, GTK toasts, and GLib timeouts.
- Socket cleanup (`ssh -O exit`) runs in a background thread; the 500 ms delay before reconnect gives it time to finish.
- Embedded RDP/VNC sessions expose a `reconnect()` method checked directly (they have no VTE overlay).
- The rate-limiter is a simple counter + timestamp, not a sliding window — good enough for the "VPN flapping" scenario and avoids extra state.

**SSH keepalive defaults** (`ServerAliveInterval=15`, `ServerAliveCountMax=3`) are injected by the SSH command builder in `rustconn-core` (protocol-level, no GUI dependency). They ensure the SSH client notices a dead link within ~45 seconds so the exit triggers the auto-reconnect flow promptly.

### Debounced Persistence

The `ConnectionManager` uses `tokio::sync::watch` channels for debounced persistence to reduce disk I/O during rapid modifications:

```rust
// Changes are sent via watch channels and saved after 2 seconds of inactivity
connection_manager.add_connection(conn);  // Sends via conn_tx
connection_manager.update_connection(conn);  // Resets debounce timer

// Force immediate save (e.g., on application exit)
connection_manager.flush_persistence();  // Uses send_replace(None) for atomic take-and-save
```

A generic `debounce_worker()` async function handles all three channels (connections, groups, trash) with the same debounce logic, eliminating code duplication.

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
    glib::timeout_add_local(Duration::from_millis(16), move || {
        match receiver.try_recv() {
            Ok(result) => {
                callback(result);
                glib::ControlFlow::Break
            }
            Err(TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(TryRecvError::Disconnected) => glib::ControlFlow::Break,
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

### Deferred Secret Backend Initialization

Secret backends (Bitwarden vault unlock, KDBX password decryption) are initialized asynchronously after the window is presented, not during `AppState::new()`. This prevents the UI from blocking on slow operations like vault unlock or password prompts at startup.

```rust
// In build_ui():
window.present();  // Show window immediately

// Phase 1: Decrypt stored credentials (fast, main thread)
glib::idle_add_local_once(move || {
    state.borrow_mut().settings_mut().secrets.decrypt_bitwarden_password();

    // Phase 2: Slow Bitwarden unlock in background thread
    let secret_settings = state.borrow().settings().secrets.clone();
    let (tx, rx) = std::sync::mpsc::channel();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(auto_unlock(&secret_settings));
        let _ = tx.send(result.is_ok());
    });

    // Poll result on GTK main thread (non-blocking)
    glib::timeout_add_local(Duration::from_millis(100), move || {
        match rx.try_recv() {
            Ok(_) => { refresh_sidebar(); glib::ControlFlow::Break }
            Err(TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(TryRecvError::Disconnected) => glib::ControlFlow::Break,
        }
    });
});
```

This ensures the application window appears instantly while credential backends initialize in the background without triggering "application not responding" dialogs.

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

### Stored Credential Encryption

Backend passwords stored in settings (KeePassXC, Bitwarden, 1Password, Passbolt master passwords) are encrypted with AES-256-GCM + Argon2id key derivation, tied to a machine-specific key. Legacy XOR-obfuscated values are transparently migrated on first save.

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
- `LibsecretBackend`: Secret Service via `oo7` when `system-keyring` is enabled on non-macOS
- `MacOsKeychainBackend`: macOS Keychain when `system-keyring` is enabled on macOS
- `KdbxExporter` + `kdbx_keyring`: KeePassXC/KeePass via direct `.kdbx` file access
- `BitwardenBackend`: Bitwarden via CLI
- `OnePasswordBackend`: 1Password via CLI
- `PassboltBackend`: Passbolt via CLI (`go-passbolt-cli`)
- `PassBackend`: Pass (passwordstore.org) via `pass` CLI

### Optional System Keyring Integration

The `keyring` module (`rustconn-core/src/secret/keyring.rs`) provides shared
keyring storage for backends that need host keyring integration. It is active
only when `rustconn-core/system-keyring` is enabled; otherwise the same public
async functions compile as unavailable stubs so headless builds do not pull
desktop/keyring dependencies.

On Linux/BSD this talks to the Secret Service in process via `oo7`. On macOS,
credentials route to the native Keychain backend.

```rust
// Check if host keyring integration is available
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

On settings load, backends with "Save to system keyring" enabled automatically restore credentials from the keyring (auto-unlock for Bitwarden, token/passphrase/password pre-fill for others). Pass uses GPG encryption natively and does not require keyring integration.

#### Flatpak Compatibility

Flatpak keyring access uses the same Secret Service D-Bus permission
(`--talk-name=org.freedesktop.secrets`). No `secret-tool` runtime binary is
required for the Linux/BSD path because the `oo7` client runs in process.

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
    fn capabilities(&self) -> ProtocolCapabilities { ProtocolCapabilities::default() }
    fn build_command(&self, connection: &Connection) -> Option<Vec<String>> { None }
}

/// Describes what a protocol supports at runtime
pub struct ProtocolCapabilities {
    pub embedded: bool,
    pub external_fallback: bool,
    pub file_transfer: bool,
    pub audio: bool,
    pub clipboard: bool,
    pub split_view: bool,
    pub terminal_based: bool,
}
```

**Implementations:**
- `SshProtocol`: SSH via VTE terminal (capabilities: embedded, terminal, split_view, port forwarding)
- `RdpProtocol`: RDP via IronRDP/FreeRDP (capabilities: embedded, external_fallback, file_transfer, audio, clipboard)
- `VncProtocol`: VNC via vnc-rs/TigerVNC (capabilities: embedded, external_fallback, clipboard)
- `SpiceProtocol`: SPICE via remote-viewer (capabilities: external_fallback, clipboard)
- `TelnetProtocol`: Telnet via external `telnet` client (capabilities: terminal, split_view)
- `SerialProtocol`: Serial via external `picocom` client (capabilities: terminal, split_view)
- `KubernetesProtocol`: Kubernetes via external `kubectl exec` (capabilities: terminal, split_view)
- `SftpProtocol`: SFTP file transfer via file manager/mc (capabilities: file_transfer, external_fallback, split_view when mc mode is active)
- `MoshProtocol`: MOSH mobile shell via external `mosh` client (capabilities: terminal, split_view)
- `WebProtocol`: Web URLs opened in the system browser via `UriLauncher`/`xdg-open` (capabilities: external_fallback)

> **Split-view eligibility is not the `split_view` capability flag.** Since 0.18.1, whether a session
> can be placed in a split panel is decided at the *widget* level by
> `TerminalNotebook::split_eligibility()` (`rustconn/src/terminal/mod.rs`), keyed on the stored
> widget kind rather than the protocol's `ProtocolCapabilities.split_view` flag:
> - **Any in-process embedded widget is `Embeddable`** — a VTE terminal *or* an embedded viewer
>   (`EmbeddedRdp`, `Vnc`, `EmbeddedSpice`). Split view is no longer VTE-only; it works for every
>   embedded tab, including RDP/VNC/SPICE remote desktops.
> - An `ExternalProcess` session (xfreerdp/vncviewer/external SPICE viewer) is `ExternalViewer` and
>   is declined — it has no in-process widget to reparent into a panel.
> - A session with no live widget is `None`.
>
> The `ProtocolCapabilities.split_view` flag therefore remains `true` only for the terminal-based
> protocols above and is no longer the gate for embedded remote desktops.

### Adding a New Protocol

1. Create `rustconn-core/src/protocol/myprotocol.rs`
2. Implement `Protocol` trait (including `capabilities()` and optionally `build_command()`)
3. Add protocol config to `ProtocolConfig` enum
4. Register in `ProtocolRegistry`
5. Add UI fields in `rustconn/src/dialogs/connection/{protocol}.rs` (e.g., `rdp.rs`, `vnc.rs`)

See `TelnetProtocol`, `SerialProtocol`, or `KubernetesProtocol` for minimal reference implementations using external clients.

### SSH Port Forwarding

The `PortForward` model (`rustconn-core/src/models/protocol.rs`) supports local (`-L`), remote (`-R`), and dynamic (`-D`) SSH port forwarding:

```rust
pub enum PortForwardDirection {
    Local,   // -L local_port:remote_host:remote_port
    Remote,  // -R remote_port:local_host:local_port
    Dynamic, // -D local_port (SOCKS proxy)
}

pub struct PortForward {
    pub direction: PortForwardDirection,
    pub local_port: u16,
    pub remote_host: String,
    pub remote_port: u16,
}
```

Rules are stored in `SshConfig::port_forwards: Vec<PortForward>` and converted to SSH arguments via `PortForward::to_ssh_arg()`. The GUI provides an inline editor in the SSH tab for adding/removing rules. Import from SSH config (`LocalForward`, `RemoteForward`, `DynamicForward`), Remmina, Asbru-CM, MobaXterm, and SecureCRT is supported.

**Waypipe Integration:** SSH connections optionally support Wayland application forwarding via `waypipe`. When enabled in the connection config (`SshConfig.waypipe`) and the `waypipe` binary is detected on PATH, the SSH command is wrapped as `waypipe ssh ...` (with automatic password injection for vault-authenticated connections). Detection is handled by `detect_waypipe()` in `rustconn-core/src/protocol/detection.rs`.

### Zero Trust Integration

Zero Trust connections (AWS SSM, GCP IAP, Teleport, Tailscale, Cloudflare, Boundary) have provider-specific validation and CLI detection:

- `ZeroTrustConfig::validate()` checks required fields per provider before save
- CLI tool availability (`aws`, `gcloud`, `tsh`, `tailscale`, etc.) is verified before connection launch
- Missing tools show a toast and log a warning via `tracing`
- All connection attempts and failures are logged in both GUI and CLI paths

### RDP Backend Selection

The `detect` module (`rustconn/src/embedded_rdp/detect.rs`) provides unified FreeRDP detection with Wayland-first candidate ordering:

```rust
// Single detection function with Wayland-first priority
let best = detect_best_freerdp();
// Tries: wlfreerdp3 → wlfreerdp → sdl-freerdp3 → sdl-freerdp → xfreerdp3 → xfreerdp

// All detection paths delegate to detect_best_freerdp()
// No more separate Wayland/X11 detection functions
```

**Backend Priority:**
- **Embedded:** IronRDP (native Rust, always preferred)
- **External Wayland-first:** wlfreerdp3 → wlfreerdp → sdl-freerdp3 → sdl-freerdp → xfreerdp3 → xfreerdp

**Security:** FreeRDP passwords are passed via `/from-stdin` instead of `/p:{password}` command-line argument, preventing exposure via `/proc/PID/cmdline`.

**HiDPI:** IronRDP sends `desktop_scale_factor` to the Windows server (e.g. 200 for 2× display), and mouse coordinates use CSS pixels matching GTK event coordinates.

### RDP Clipboard Integration

Bidirectional clipboard sync between local desktop and remote RDP session via the CLIPRDR virtual channel (MS-RDPECLIP).

**Architecture:**

```
┌─────────────────────────────────────────────────────────────┐
│ rustconn-core/src/rdp_client/                               │
│                                                             │
│  clipboard.rs                                               │
│    RustConnClipboardBackend (implements CliprdrBackend)      │
│      on_remote_copy()  ──▶  ClipboardText event             │
│      on_format_data_request()  ──▶  ClipboardDataReady      │
│      on_format_data_response() ──▶  ClipboardText event     │
│                                                             │
│  client/commands.rs                                         │
│    ClipboardText cmd  ──▶  set_pending_copy_data()          │
│                       ──▶  handle_clipboard_copy()          │
└─────────────────────────────────────────────────────────────┘
                        │
                        ▼
┌─────────────────────────────────────────────────────────────┐
│ rustconn/src/embedded_rdp/                                  │
│                                                             │
│  clipboard.rs + connection.rs (polling handler)             │
│  Phase 1: Paste via cliprdr                                 │
│    Paste button → ClipboardText cmd → cliprdr announce      │
│                                                             │
│  Phase 2: Auto-sync server→client                           │
│    ClipboardText event → clipboard.set_text()               │
│    (suppression flag prevents feedback loop)                │
│                                                             │
│  Phase 3: Local clipboard monitoring                        │
│    gdk::Clipboard::connect_changed() → ClipboardText cmd    │
│    (handler disconnected on session end/error)              │
│                                                             │
│  Autotype (autotype.rs):                                    │
│    Type Clipboard btn → read clipboard → AutotypeText cmd   │
│    Type Text… btn → dialog → AutotypeText cmd               │
│    AutotypeText → grapheme iteration → UnicodeKeyboardEvent │
│    (inter-char delay configurable per connection)           │
└─────────────────────────────────────────────────────────────┘
```

**Data Flow — Client→Server (Paste):**
1. User copies text locally (or clicks Paste button)
2. `connect_changed` handler fires → sends `ClipboardText` command
3. Command handler encodes text as UTF-16LE, stores in backend via `set_pending_copy_data()`
4. `handle_clipboard_copy()` announces `CF_UNICODETEXT` format to server
5. Server requests data via `FormatDataRequest` → backend serves pending data via `ClipboardDataReady` event

**Data Flow — Server→Client (Copy):**
1. Server copies text → `on_remote_copy()` fires with format list
2. Backend auto-requests `CF_UNICODETEXT` via `initiate_paste()`
3. Server responds → `on_format_data_response()` decodes UTF-16LE → `ClipboardText` event
4. GUI handler sets local GTK clipboard via `clipboard.set_text()` (with suppression flag)

**Feedback Loop Prevention:**
A `clipboard_sync_suppressed` flag is set before `clipboard.set_text()` in Phase 2 and cleared after 100ms. The Phase 3 `connect_changed` handler checks this flag and skips announcing when suppressed.

**Cleanup:**
The clipboard `connect_changed` handler is disconnected on: normal disconnect, protocol error, stale generation, and embedded mode exit (via `cleanup_embedded_mode()`).

### RDP Quick Actions

The `quick_actions` module (`rustconn-core/src/rdp_client/quick_actions.rs`) defines predefined Windows admin key sequences that can be sent through the embedded RDP session.

**Architecture:**

```
rustconn-core/src/rdp_client/
  quick_actions.rs          # QuickAction definitions + key sequence builders
  event.rs                  # SendKeySequence(Vec<(u16, bool, bool)>) command variant
  client/commands.rs        # Handler: sends scancodes with 30ms inter-key delay

rustconn/src/embedded_rdp/
  mod.rs                    # MenuButton dropdown + GIO action group on toolbar
```

**Data Flow:**
1. `QUICK_ACTIONS` static array defines the actions with id, label, tooltip, icon
2. Hotkey actions → `build_hotkey_sequence(id)` returns `Vec<(scancode, pressed, extended)>`; Run-dialog actions → `run_command_for(id)` returns the command string
3. GUI creates a `MenuButton` with `gio::Menu` items, each mapped to a GIO action
4. Hotkey actions send `RdpClientCommand::SendKeySequence`; Run-dialog actions send Win+R (`build_open_run_dialog`) → `AutotypeText` (Unicode, layout-independent) → Enter (`build_enter_sequence`)
5. The command loop drains these in FIFO order, awaiting each before the next

**Key Sequence Patterns:**
- Direct hotkey: Task Manager (`Ctrl+Shift+Esc`), Settings (`Win+I`) — scancodes (virtual-key resolved, layout-safe)
- Win+R launch: Event Viewer, Services, etc. — opens Run dialog with a scancode hotkey, types the command via Unicode keyboard events so it is correct on any remote keyboard layout (issue #184), then presses Enter

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
├── window/                # Main window (modular structure)
│   ├── mod.rs             # Module exports, MainWindow struct
│   └── ...                # Domain-specific window functionality
├── state.rs               # SharedAppState
├── async_utils.rs         # Async helpers (spawn_async, block_on_async_with_timeout)
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
│   │   ├── dialog/        # ConnectionDialog (split from the old ~7000-line dialog.rs)
│   │   │   ├── mod.rs         # ConnectionDialog struct + public API
│   │   │   ├── construction.rs # Widget construction / wiring
│   │   │   ├── build.rs       # build_* methods (assemble Connection from UI)
│   │   │   ├── populate.rs    # populate_* methods (fill UI from Connection)
│   │   │   ├── rows.rs        # Reusable row builders
│   │   │   ├── passwords.rs   # Credential/password row handling
│   │   │   ├── save.rs        # Save / validation flow
│   │   │   └── agent_variables.rs # SSH agent + variable rows
│   │   ├── builders.rs    # Shared field/section builders for tabs
│   │   ├── general_tab.rs # General tab: name, host, port, group, credentials
│   │   ├── data_tab.rs    # Data tab: variables, custom properties
│   │   ├── automation_tab.rs # Automation tab: expect rules, pre/post tasks
│   │   ├── advanced_tab.rs   # Advanced tab: window mode, Wake-on-LAN
│   │   ├── logging_tab.rs # LoggingTab struct (extracted from dialog)
│   │   ├── protocol_layout.rs # ProtocolLayoutBuilder for consistent UI
│   │   ├── shared_folders.rs  # Shared folders UI (RDP/SPICE)
│   │   ├── widgets.rs     # Re-exports from parent dialogs/widgets.rs
│   │   ├── ssh.rs         # SSH options
│   │   ├── rdp.rs         # RDP options
│   │   ├── vnc.rs         # VNC options
│   │   ├── spice.rs       # SPICE options
│   │   ├── telnet.rs      # Telnet options
│   │   ├── serial.rs      # Serial options
│   │   ├── kubernetes.rs  # Kubernetes options
│   │   ├── web.rs         # Web (browser) options
│   │   └── zerotrust.rs   # Zero Trust provider options
│   ├── keyboard.rs        # Keyboard navigation helpers
│   ├── command_palette.rs # Command palette dialog (Ctrl+P)
│   ├── wol.rs             # Wake On LAN dialog (standalone + manual entry)
│   ├── flatpak_components.rs  # Flatpak CLI download dialog
│   ├── settings/          # Settings tabs (incl. keybindings_tab.rs)
│   └── ...
├── embedded_rdp/          # Embedded RDP viewer (modular structure)
│   ├── mod.rs             # EmbeddedRdpWidget struct, signals, public API (~860 lines)
│   ├── autotype.rs        # Autotype: send text as keystrokes (Type Clipboard / Type Text dialog)
│   ├── clipboard.rs       # Copy/Paste and Ctrl+Alt+Del button handlers
│   ├── connection.rs      # connect/disconnect/reconnect, IronRDP polling, external fallback
│   ├── drawing.rs         # DrawingArea draw function, framebuffer rendering, status overlay
│   ├── input.rs           # Keyboard/mouse input handlers (cfg-gated for rdp-embedded)
│   ├── resize.rs          # Debounced resize with resolution change (cfg-gated)
│   ├── buffer.rs          # Frame buffer management
│   ├── detect.rs          # Backend detection
│   ├── launcher.rs        # FreeRDP launcher
│   ├── thread.rs          # FreeRDP thread with consolidated mutex
│   ├── types.rs           # Shared types
│   └── ui.rs              # Status overlay rendering
├── monitoring.rs           # MonitoringBar widget, MonitoringCoordinator
├── smart_folder_ui.rs     # Smart Folders sidebar section and dialogs
└── utils.rs               # Async helpers, utilities

rustconn-core/src/
├── lib.rs                 # Public API re-exports
├── error.rs               # Error types
├── models/                # Data models (incl. smart_folder.rs, highlight.rs, dynamic_folder.rs)
├── config/                # Settings persistence, keybindings
├── connection/            # Connection management
│   ├── mod.rs             # Module exports
│   ├── manager.rs         # ConnectionManager with debounced persistence
│   ├── retry.rs           # RetryConfig, RetryState, exponential backoff
│   ├── port_check.rs      # TCP port reachability check
│   ├── virtual_scroll.rs  # Virtual scrolling helpers
│   └── ...
├── protocol/              # Protocol implementations
├── secret/                # Credential backends
│   ├── mod.rs             # Module exports
│   ├── backend.rs         # SecretBackend trait
│   ├── manager.rs         # SecretManager with bulk operations
│   ├── resolver.rs        # CredentialResolver (Vault/Variable/Inherit/Script resolution)
│   ├── script_resolver.rs # Script credential resolver (shell-words, 30s timeout)
│   ├── hierarchy.rs       # KeePass hierarchical paths
│   ├── keyring.rs         # Shared system keyring via secret-tool
│   ├── libsecret.rs       # GNOME Keyring backend
│   ├── kdbx.rs            # KDBX file backend (KeePass-compatible)
│   ├── kdbx_keyring.rs    # KDBX database keyring helpers
│   ├── bitwarden.rs       # Bitwarden backend (with keyring storage)
│   ├── onepassword.rs     # 1Password backend (with keyring storage)
│   ├── passbolt.rs        # Passbolt backend (with keyring storage)
│   ├── pass.rs            # Pass (passwordstore.org) backend
│   ├── macos_keychain.rs  # macOS Keychain backend (Security.framework)
│   ├── encrypted_file.rs  # App-managed AES-256-GCM file (no keyring needed)
│   ├── local_crypto.rs    # AES-256-GCM + Argon2id primitives
│   ├── detection.rs       # Password manager detection
│   ├── status.rs          # KeePass status detection
│   └── ...
├── session/               # Session management
│   ├── mod.rs             # Module exports
│   ├── manager.rs         # SessionManager with health checks
│   ├── logger.rs          # Session logging with sanitization
│   ├── recording.rs       # Session recording (scriptreplay-compatible format)
│   ├── restore.rs         # Session state persistence
│   └── ...
├── monitoring/            # Remote host metrics (agentless)
│   ├── mod.rs             # Module exports, re-exports
│   ├── metrics.rs         # Data models (RemoteMetrics, SystemInfo, LoadAverage)
│   ├── parser.rs          # Shell command output parsing
│   ├── collector.rs       # MetricsComputer, CollectorHandle, async polling
│   ├── settings.rs        # MonitoringSettings, MonitoringConfig
│   └── ssh_exec.rs        # SSH command execution factory
├── import/                # Format importers
│   ├── mod.rs             # Module exports
│   ├── traits.rs          # ImportSource trait, ImportStatistics
│   ├── csv_import.rs      # CSV importer (RFC 4180, auto column mapping)
│   ├── securecrt.rs       # SecureCRT .ini session importer
│   └── ...
├── export/                # Format exporters (incl. csv_export.rs, securecrt.rs)
├── search/                # Search engine, command palette
├── rdp_client/            # RDP client implementation
│   ├── mod.rs             # Module exports
│   ├── backend.rs         # RdpBackendSelector
│   ├── quick_actions.rs   # Windows admin quick actions (key sequences)
│   └── ...
├── cli_download/          # Flatpak CLI download manager
├── dynamic_folder.rs      # Dynamic folder executor — script execution, JSON parsing, entry→Connection conversion
├── highlight.rs           # Text highlighting rules engine (CompiledHighlightRules, find_matches)
├── smart_folder.rs        # SmartFolderManager — dynamic connection grouping with filter evaluation
├── sftp.rs                # SFTP URI/command builders, ssh-add, mc FISH VFS
├── flatpak.rs             # Flatpak sandbox detection, portal key path resolution, stable key copy
├── snap.rs                # Snap environment detection and paths
├── performance/           # String interner (connection-string dedup) + search debouncer
├── tracing/               # Span name constants for structured tracing
└── ...
```

## Remote Monitoring Architecture

Agentless system metrics collection for SSH, Telnet, and Kubernetes sessions. Parses `/proc/*` and `df` output from remote Linux hosts without installing any agent.

### Data Flow

```
┌──────────────────────────────────────────────────────────────────┐
│ rustconn-core/src/monitoring/                                    │
│                                                                  │
│  METRICS_COMMAND (shell)  ──▶  MetricsParser::parse_metrics()    │
│  SYSTEM_INFO_COMMAND      ──▶  MetricsParser::parse_system_info()│
│                                                                  │
│  CollectorHandle ◀── start_collector() ──▶ MetricsComputer       │
│       │                                        │                 │
│       │  MetricsEvent::Metrics(RemoteMetrics)  │                 │
│       │  MetricsEvent::SystemInfo(SystemInfo)  │                 │
│       ▼                                        │                 │
│  tokio::sync::mpsc channel                     │                 │
└──────────────────────────────────────────────────────────────────┘
                        │
                        ▼
┌──────────────────────────────────────────────────────────────────┐
│ rustconn/src/monitoring.rs                                       │
│                                                                  │
│  MonitoringCoordinator                                           │
│       │  manages per-session MonitoringBar instances              │
│       │  starts/stops collectors per session                     │
│       ▼                                                          │
│  MonitoringBar (GTK widget)                                      │
│       [CPU ██░░ 45%] [RAM ██░░ 62%] [Disk ██░░ 78%]            │
│       [1.23 0.98 0.76] [↓ 1.2 MB/s ↑ 0.3 MB/s]                │
│       [Ubuntu 24.04 (6.8.0) · x86_64 · 15.6 GiB · 8C/16T]    │
└──────────────────────────────────────────────────────────────────┘
```

### Core Layer (`rustconn-core/src/monitoring/`)

| File | Purpose |
|------|---------|
| `metrics.rs` | Data models: `RemoteMetrics`, `MemoryMetrics`, `DiskMetrics`, `NetworkMetrics`, `LoadAverage`, `SystemInfo`, `CpuSnapshot`, `NetworkSnapshot` |
| `parser.rs` | `MetricsParser` — parses shell output into metric structs; `METRICS_COMMAND` and `SYSTEM_INFO_COMMAND` shell one-liners |
| `collector.rs` | `MetricsComputer` — computes deltas between snapshots (CPU%, network throughput); `CollectorHandle` — async polling loop; `MetricsEvent` enum |
| `settings.rs` | `MonitoringSettings` — global toggles (enabled, interval, show_cpu/memory/disk/network/load/system_info); `MonitoringConfig` — per-connection override |
| `ssh_exec.rs` | Factory for executing shell commands over the existing session |

### GUI Layer (`rustconn/src/monitoring.rs`)

| Type | Purpose |
|------|---------|
| `MonitoringBar` | GTK widget with `LevelBar` + `Label` for each metric; `update()` for periodic metrics, `update_system_info()` for one-time static info |
| `MonitoringCoordinator` | Manages per-session `MonitoringBar` instances; starts/stops collectors; applies settings changes to all active bars |

### Shell Commands

Two shell one-liners are sent to the remote host:

- `METRICS_COMMAND` — runs every polling interval; reads `/proc/stat`, `/proc/meminfo`, `/proc/net/dev`, `/proc/loadavg`, and `df /`
- `SYSTEM_INFO_COMMAND` — runs once at monitoring start; reads `/etc/os-release`, `uname -r`, `/proc/uptime`, `/proc/meminfo` (total RAM), `/proc/cpuinfo` (cores/threads), and `uname -m` (architecture)

### Settings

Global settings in `MonitoringSettings` (stored in `config.toml` under `[monitoring]`):
- `enabled` — global toggle (default: false)
- `interval_secs` — polling interval 1–60s (default: 3)
- `show_cpu`, `show_memory`, `show_disk`, `show_network`, `show_load`, `show_system_info` — per-metric visibility toggles

Per-connection override via `MonitoringConfig` on the `Connection` model:
- `enabled: Option<bool>` — override global toggle
- `interval_secs: Option<u8>` — override polling interval

## Testing

### Property Tests

Located in `rustconn-core/tests/properties/` (1300+ tests):

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
