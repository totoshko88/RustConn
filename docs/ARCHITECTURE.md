# RustConn Architecture Guide

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

**Rules:**
- Never hold a borrow across an async boundary
- Never hold a borrow when calling GTK methods that might trigger callbacks
- Prefer short-lived borrows over storing references

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

Managers own their data and handle I/O. They don't know about GTK.

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

**When to Use What:**
- `spawn_blocking_with_callback`: Simple blocking operations
- `spawn_blocking_with_timeout`: Operations that might hang
- `with_runtime`: When you need tokio features (async traits, channels)

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

### Adding a New Protocol

1. Create `rustconn-core/src/protocol/myprotocol.rs`
2. Implement `Protocol` trait
3. Add protocol config to `ProtocolConfig` enum
4. Register in `ProtocolRegistry`
5. Add UI fields in `rustconn/src/dialogs/connection.rs`

## GTK4/Libadwaita Patterns

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
├── sidebar.rs             # Connection tree
├── sidebar_types.rs       # Sidebar data types
├── sidebar_ui.rs          # Sidebar widget helpers
├── terminal/              # VTE terminal integration
├── dialogs/               # Modal dialogs
│   ├── connection.rs      # Connection editor
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
├── protocol/              # Protocol implementations
├── secret/                # Credential backends
├── session/               # Session management
├── import/                # Format importers
├── export/                # Format exporters
└── ...
```

## Testing

### Property Tests

Located in `rustconn-core/tests/properties/`:

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
