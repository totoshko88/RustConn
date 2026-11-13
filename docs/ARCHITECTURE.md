# RustConn Architecture Guide

**Version 0.25.0** | Last updated: November 2026

This document describes the internal architecture of RustConn for contributors and maintainers.

## Crate Structure

RustConn is an eight-crate Cargo workspace (Rust 2024 edition) with strict separation of concerns:

```
rustconn/            # GTK4 GUI application
rustconn-core/       # Business logic library (GUI-free)
rustconn-cli/        # Command-line interface
rustconn-pty-sys/    # Isolated FFI helper (PTY + controlling terminal)
rustconn-locale-sys/ # Isolated FFI helper (startup setlocale)
rustconn-env-sys/    # Isolated FFI helper (startup GSK_RENDERER write)
rustconn-dock-sys/   # Isolated FFI helper (macOS Dock tile image)
rustconn-lock-sys/   # Isolated FFI helper (macOS screen lock + sleep notifications)
```

### Dependency Graph

```
┌─────────────┐        ┌─────────────────┐
│ rustconn    │───────▶│  rustconn-core  │
│ (GUI)       │        │   (Library)     │
└──────┬──────┘        └─────────────────┘
       │                        ▲
       │                        │
       │               ┌─────────────┐
       │               │ rustconn-cli│
       │               │   (CLI)     │
       │               └─────────────┘
       │
       │               ┌──────────────────────┐
       └──────────────▶│  rustconn-pty-sys    │
                       │  rustconn-locale-sys │
                       │  rustconn-env-sys    │
                       │  rustconn-dock-sys   │
                       │  rustconn-lock-sys   │
                       │    (FFI, no GUI)     │
                       └──────────────────────┘
```

Only `rustconn` depends on the `-sys` crates. `rustconn-cli` depends on
`rustconn-core` and nothing else in the workspace.

### Crate Boundaries

| Crate | Purpose | Dependency Boundary |
|-------|---------|---------------------|
| `rustconn-core` | Domain logic: models, config persistence, CRUD managers, protocol data, import/export, credential abstractions | Non-GUI Rust dependencies only; host keyring and embedded-client runtimes stay behind explicit features — NO GTK |
| `rustconn` | GTK4 UI, dialogs, terminal integration, embedded/external session presentation | Owns `gtk4`, `vte4`, `libadwaita`, and enables core integration features when needed |
| `rustconn-cli` | Headless management CLI: config, connections, import/export, list/show, simple operations | CLI/runtime dependencies plus `rustconn-core`; client launch and secret-management stay behind explicit features — NO GTK |
| `rustconn-pty-sys` | FFI helper: give a spawned child its PTY slave as a controlling terminal (`setsid` + `TIOCSCTTY`) — plus PTY creation, sizing and readiness — for every VTE-backed session on all platforms (first added for the macOS native PTY, [#175](https://github.com/totoshko88/RustConn/issues/175)) | `libc` + `nix` (`term`, for `openpty`) — NO GTK |
| `rustconn-locale-sys` | FFI helper: the startup `setlocale` call, refused once this program spawns a thread of its own, once a call arrives from a second thread, or once the startup window is sealed ([#267](https://github.com/totoshko88/RustConn/issues/267)) | `gettext-rs` only — NO GTK |
| `rustconn-env-sys` | FFI helper: the startup `GSK_RENDERER` and `LANGUAGE` writes, guarded the same way. Neither GTK nor gettext offers an API, so the environment is the only interface and it has to be written before `gtk_init` ([#274](https://github.com/totoshko88/RustConn/issues/274), [#158](https://github.com/totoshko88/RustConn/issues/158)) | No dependencies — NO GTK |
| `rustconn-dock-sys` | FFI helper: `-[NSApplication setApplicationIconImage:]`, so the macOS Dock shows RustConn's icon when no `.app` bundle is behind the process. GDK exposes no Dock API and `set_icon_name` is a no-op on its macOS backend | `objc2` + AppKit bindings, macOS-gated — NO GTK |
| `rustconn-lock-sys` | FFI helper: observes the macOS screen lock (`com.apple.screenIsLocked`, delivered immediately even in the background) and `NSWorkspaceWillSleepNotification`, which the GUI maps onto `rustconn-core::lock_events` so the SSH keys RustConn added are removed there as on Linux. `rustconn-core` does not depend on it | `objc2` + AppKit bindings, macOS-gated — NO GTK |

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
| `rustconn-core/web-embedded` | GUI | Embedded WebKitGTK browser as the default for new Web connections |
| `rustconn-core/bitwarden-serve` | GUI | Persistent `bw serve` transport for the Bitwarden backend |
| `rustconn-core/lock-events` | GUI (always on) | D-Bus screen-lock / suspend listener; not a GUI feature, so no build drops it |
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

No crate outside the `rustconn-*-sys` helpers may write `unsafe`. Each helper is a
sanctioned location for it, following the M-UNSAFE guideline (isolate FFI in a
small `-sys` crate with a documented safety contract) instead of relaxing the
lint in the main crates. New FFI gets a new `-sys` crate; it never earns an
exception where the caller lives.

The mechanism is `unsafe_code = "deny"` in `[workspace.lints.rust]`, re-opened by
a crate-level `#![expect(unsafe_code, reason = "…")]` in each helper; why `deny`
and not `forbid` is in `.kiro/steering/core-rules.md`. Each helper's safety
contract lives in one place — its `src/lib.rs` — and its rules in its `AGENTS.md`.
This section only says where to look:

| Crate | Called from | Contract and history |
|-------|-------------|----------------------|
| `rustconn-pty-sys` | `rustconn/src/terminal/pty_spawn.rs`, `pty_relay.rs` | `rustconn-pty-sys/src/lib.rs`; session data never passes through `unsafe` |
| `rustconn-locale-sys` | `rustconn/src/i18n.rs` (seals it) | `rustconn-locale-sys/src/lib.rs`; the thread-baseline guard, [#271](https://github.com/totoshko88/RustConn/issues/271) |
| `rustconn-env-sys` | `rustconn/src/i18n.rs` (`LANGUAGE`), then `rustconn/src/renderer.rs` (`GSK_RENDERER`, seals it) | `rustconn-env-sys/src/lib.rs`, `rustconn-env-sys/AGENTS.md` |
| `rustconn-dock-sys` | `rustconn/src/main.rs` (`apply_macos_dock_icon`, called from `app.rs`) | `rustconn-dock-sys/src/lib.rs` |
| `rustconn-lock-sys` | `rustconn/src/app.rs`, mapped onto `rustconn-core::lock_events::notify` | `rustconn-lock-sys/src/lib.rs` |

### Who Owns a Session's PTY

RustConn creates the pseudo-terminal for every VTE-backed session and keeps the
master descriptor; VTE is given none. The division of labour is:

| Concern | Owner |
|---------|-------|
| Rendering, scrollback, selection, key interpretation | VTE |
| PTY creation, child process, controlling terminal | `terminal::pty_spawn` |
| Reading output, writing input, window size | `terminal::pty_relay` |
| Wiring the two together per session | `TerminalNotebook` |

Output flows from a reader thread over a bounded channel to the GTK main thread,
which feeds it to VTE and to any registered observer — session logging is one.
Input flows the other way: VTE emits `commit` for every key, paste, mouse report
and terminal reply *even with no PTY attached* (guaranteed by VTE for
[vte#222](https://gitlab.gnome.org/GNOME/vte/-/issues/222)), and the notebook
forwards those bytes to a writer thread. Both directions use bounded or
off-thread I/O so that a flood of output or a paste into a process that is not
reading cannot block the window.

Owning the PTY also means taking over one thing VTE does for its own: it stops
reading while the user drags out a mouse selection, because any output that
arrives before the drag ends deselects. `terminal::selection_hold` restores that
pause on the main-thread side of the channel, so a remote that redraws on a timer
(a byobu or tmux status clock) does not cancel the selection.

This arrangement exists because a session transcript has to be a copy of what
the child wrote, and that cannot be recovered from the widget: VTE rewraps its
buffer on a width change and renumbers the rows underneath any reader, so a
scraped transcript both repeats and skips lines (issue
[#247](https://github.com/totoshko88/RustConn/issues/247)). The VTE behaviours the
design depends on are pinned by `terminal::vte_contract_tests`, which need a
display and are therefore `#[ignore]`d — run them when touching this area.

Two consequences worth knowing when editing session code:

- Input must keep going through `terminal.feed_child()`. That is what raises
  `commit`, which is what reaches the PTY. Writing to the relay directly from a
  call site would bypass input logging and keystroke broadcast.
- VTE never learns that a child exited, because it did not spawn one. A GLib
  child watch raises `child-exited` on the terminal instead, and the whole
  teardown path (reconnect banner, log flush, monitoring, tab state) still hangs
  off that signal.

### Keystroke broadcast — two independent models

Broadcast mirrors one session's typed input to others. Both models share the same
primitive: a per-session VTE `commit` handler reads the input and calls
`TerminalNotebook::send_text_to_session` on each target, which is just a
`feed_child` to that target's PTY — visibility-independent, so a background tab
receives it fine. A single shared re-entrancy guard stops the fed text's own
`commit` from re-broadcasting.

- **Split broadcast** targets the panels of one tab. State lives on that tab's
  `SplitViewBridge` (`broadcast_active` / `broadcast_wired_sessions` /
  `broadcast_busy`); targets are the split's active panes. Wired in
  `window/navigation_actions.rs`.
- **Group broadcast** (#329) targets sessions that each own their own tab, so its
  state cannot live on any one bridge — it lives on `MainWindow`'s
  `GroupBroadcast` (`window/group_broadcast.rs`). Membership is an explicit
  per-session opt-in set, deliberately not derived from the tab-group label.
  `resolve_broadcast_targets` is a pure function (unit-tested) that returns the
  members to feed only when the broadcast is active and the source is itself a
  member — a non-member tab types only into itself (the iTerm2 asymmetric rule).
  A persistent banner and a large-group confirmation exist because a group
  broadcast reaches tabs that are not all on screen.

The two are orthogonal and can be active at once without interfering.

### Session scripts (`.rsc`)

A script drives a live terminal session, so it cannot block the GTK main
thread while it waits for a prompt. It is split the same way as everything else:

| Piece | Location | Role |
|-------|----------|------|
| Parser + engine | `rustconn-core/src/automation/script.rs` | `parse_program` turns source into `ScriptStep`s; `ScriptEngine` is an event-driven state machine. `pump()` runs steps until one has to wait, and the caller resumes it with `on_output`, `on_timer`, `on_ask_result`, `on_run_result` or `on_session_started` |
| Host trait | `ScriptHost` in the same file | Every side effect (send keys, print, ask, start/split/close a session, run a local command). The engine never touches GTK; tests drive it with a recording host |
| Library | `rustconn-core/src/script/manager.rs` | `ScriptManager`: one `<id>.rsc` file per script under `<config_dir>/scripts/`; ids are restricted to `[A-Za-z0-9_-]` and a source that does not parse is never written |
| Auto-run inheritance | `rustconn-core/src/connection/automation_inheritance.rs` | `AutomationConfig.auto_scripts` / `ConnectionGroup.auto_scripts`; an empty list on a connection inherits the first non-empty list up the group chain |
| GUI host | `rustconn/src/script_runner.rs` | `ScriptRunner` implements `ScriptHost` over the notebook, split bridges and sidebar, feeds VTE output into the engine and owns the timers. Auto-run starts from `create_terminal_tab_with_settings`, the one place every terminal session is created |
| Manager dialog | `rustconn/src/dialogs/script.rs`, `script_reference.rs` | Tools → Scripts…: edit, validate, run, stop, delete (with confirmation), language reference |

Two rules keep a script from becoming an attack path. Variables substituted
into a `run` command go through `VariableManager::validate_command_value`, the
same check connection tasks use, because `capture`/`readline` values are text
the remote host chose; `run` also scrubs `SCRUBBED_ENV_VARS`, gets no stdin and
is killed on timeout. The engine's `Debug` prints variable names only and its
values are zeroized on replace and drop, since an `ask … hide` answer lives there.

### Why This Separation?

1. **Testability**: Core logic can be tested without a display server
2. **Reusability**: CLI shares all business logic with GUI
3. **Build times**: Changes to UI don't recompile core logic
4. **Future flexibility**: Could support alternative UIs (TUI, web)

## State Management

### SharedAppState Pattern

The GUI uses a shared mutable state pattern for GTK's single-threaded model:

```rust
// rustconn/src/state/mod.rs
pub type SharedAppState = Rc<RefCell<AppState>>;

pub struct AppState {
    connection_manager: ConnectionManager,
    session_manager: SessionManager,
    snippet_manager: SnippetManager,
    template_manager: TemplateManager,
    secret_manager: SecretManager,
    config_manager: ConfigManager,
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
| `SnippetManager` | Command snippets |
| `TemplateManager` | Connection template CRUD, search, import/export |
| `ClusterManager` | Connection clusters |
| `SyncManager` | Cross-device settings/connection sync |
| `WorkspaceProfileManager` | Save/restore sets of open sessions |
| `FolderConnectionTracker` | Which connections belong to which folder |

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

### Sharing the files with other processes

`connections.toml`, `groups.toml` and `trash.toml` are also written by `rustconn-cli` and by a second instance, so a save is never a plain overwrite. Each file has a *base*, the list this process last saw on disk (`config::merge::SharedBase`). A save takes the base, asks the debounce channel for the latest snapshot, reads the file under the write locks, merges the three lists by id (`merge_three_way`: an entry added elsewhere is kept, one removed elsewhere is removed, a one-sided edit is taken, a two-sided edit goes to the newer `updated_at`, a deletion made here is honoured) and writes the result unless it equals the file. The base then becomes the snapshot that was saved.

`ConnectionManager::reload_from_disk()` applies the same merge to memory. The GUI calls it from `config::watch::ConfigWatcher` (inotify, polled once a second from the GTK main loop, 700 ms of quiet) so a `rustconn-cli add` shows up in the sidebar without a restart. It takes the bases with `try_lock`, so it never waits for a disk write; a busy reload is retried on the next tick. The snapshot is read *after* the base is held, on both sides, because a snapshot taken earlier can predate a reload and would otherwise delete what the reload just added.

Not covered: `rustconn-cli` still reads, edits and writes the whole file itself, so two CLI commands racing each other, or a CLI command racing a GUI save in the few milliseconds between its read and its write, can still lose one change.

## Thread Safety Patterns

### Mutex Poisoning Recovery

When a thread panics while holding a mutex lock, the mutex becomes "poisoned" to signal that the protected data may be in an inconsistent state. By default, attempting to lock a poisoned mutex returns an error.

For simple state flags and process handles (like in `FreeRdpThread`), we can safely recover from poisoning by extracting the inner value:

```rust
// rustconn/src/embedded_rdp/thread.rs

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
// rustconn/src/async_utils.rs
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

// Run a tokio future on the shared runtime (async traits, channels)
let value = with_runtime(|rt| rt.block_on(async { fetch_value().await }))?;

// Blocking work on a worker thread, result handed back to the main loop
// (`rustconn/src/utils.rs`)
spawn_blocking_with_callback(
    || expensive_operation(),
    |result| handle_result(result),
);

// The same with a time limit; the callback gets `None` on timeout
spawn_blocking_with_timeout(
    || operation_that_might_hang(),
    Duration::from_secs(30),
    |result| handle_result(result),
);
```

**When to Use What:**
- `spawn_blocking_with_callback`: Simple blocking operations
- `spawn_blocking_with_timeout`: Operations that might hang
- `with_runtime`: When you need tokio features (async traits, channels)
- `spawn_async`: Non-blocking async on GTK main thread

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
- No `unwrap()`/`expect()` outside tests (`AGENTS.md`); clippy enforces `unwrap_used`
- Include context in error messages

### GUI Error Display

A `Display` string from a `rustconn-core` error is already meant for a user (the
`thiserror` messages are written that way), so the GUI shows it directly rather
than through a separate technical-to-friendly mapping layer. Which surface it
picks depends on the consequence of the failure — modal dialog for a lost action,
banner for a persistent problem, toast for a transient one (see the "Toasts vs
Banners vs Dialogs" table in the GNOME HIG steering).

The application-level helper is `show_error_dialog` in `rustconn/src/app.rs`. It
presents an `adw::AlertDialog` on the active window rather than parenting it on a
fresh `ApplicationWindow`, which would otherwise linger after the dialog is
dismissed:

```rust
// rustconn/src/app.rs
fn show_error_dialog(app: &adw::Application, title: &str, message: &str) {
    let dialog = adw::AlertDialog::new(Some(title), Some(message));
    dialog.add_response("ok", &crate::i18n::i18n("OK"));
    dialog.set_default_response(Some("ok"));

    // Present without a parent window — avoids creating an orphaned
    // ApplicationWindow that lingers after the dialog is dismissed.
    let parent = app.active_window();
    dialog.present(parent.as_ref());
}
```

Inside a window, the `alert` and toast helpers in `rustconn/src/dialogs/` carry
the same `title`/`message` shape; the title is the "what happened" and the
message is the "what to do", both wrapped in `i18n()`.

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
    pub password: Option<SecretString>, // Zeroed on drop
    pub domain: Option<String>,
}
```

**Never:**
- Store passwords as plain `String`
- Log credential values
- Include credentials in error messages
- Serialize passwords to config files

**Documented exceptions** (the rule list is `AGENTS.md`):
- *Passbolt resource password on argv.* `go-passbolt-cli` takes the resource
  password only as `--password` (no stdin, no environment), so it is visible in
  `/proc/PID/cmdline` while `create`/`update resource` runs. The child is
  `kill_on_drop` and bounded by a timeout; stderr is redacted. The account
  passphrase does not share this: it goes through `USERPASSWORD` in the
  environment, which only the owner can read.
- *RDP gateway password without `Zeroizing`.* `ironrdp-mstsgu` needs an owned
  `String`; it is moved (not copied) into `ZeroizingGatewayTarget`, whose `Drop`
  wipes that one buffer (`rdp_client/client/connection.rs`).

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
- Portable file: `store_portable_passphrase_in_keyring()` / `get_portable_passphrase_from_keyring()`

On settings load, backends with "Save to system keyring" enabled automatically restore credentials from the keyring (auto-unlock for Bitwarden, token/passphrase/password pre-fill for others). Pass uses GPG encryption natively and does not require keyring integration.

### Portable Encrypted File (KEK/DEK)

`portable_encrypted_file.rs` is the only backend whose key comes from the user
rather than from the machine or a vendor's vault, which is what makes the file
movable between machines (issue #293). It uses a two-level key hierarchy:

```
passphrase ──Argon2id(kdf_salt, kdf_params)──▶ KEK
                                                │
                         wrapped_key ──open──▶ DEK ──▶ every entry
```

One Argon2id derivation unlocks the whole file; each entry then costs a single
AES-256-GCM pass. Deriving per entry — the shape the machine-bound
`encrypted_file.rs` uses, where the key input is high-entropy and the cost is
therefore low — would make a single `retrieve` pay a full ~0.5 s KDF. The split
also makes the passphrase verifiable (unwrap the key) and a passphrase change
cheap (rewrap 32 bytes instead of re-encrypting every credential).

On-disk header: `format_version`, `kdf`, `kdf_params`, `kdf_salt`,
`wrapped_key`, `entries`. Because the file arrives from a shared folder, the
header is treated as untrusted input: `check_header` rejects an unknown format
version or KDF, and `check_kdf_cost` caps the Argon2 parameters so a hostile
file cannot demand gigabytes before anyone types a passphrase. Entry blobs and
the wrapped key are bound to their roles by AAD, so moving one into the other's
slot fails authentication rather than silently decrypting.

Unlock state lives in the backend behind a `std` lock (never held across an
`await`), which keeps `set_passphrase` synchronous — `SecretManager::build_from_settings`
is synchronous and has to be able to seed a passphrase it already holds.
`SecretManager` keeps a typed handle to this one backend alongside the erased
`Vec<Arc<dyn SecretBackend>>`, because it is the only backend that is unlocked
*after* construction: `rebuild_from_settings` fires only when `SecretSettings`
compares unequal, and the runtime passphrase is deliberately excluded from that
comparison, so `set_portable_passphrase()` is the only route in.

The DEK is cached per session, fingerprinted by `(kdf_salt, wrapped_key)`. Both
halves are needed: rewrapping produces a fresh DEK under the *same* salt, so a
salt-only cache would seal new entries with a superseded key and write them to a
file that can never open them again.

**Cloud-sync ceiling.** Every write is a read-modify-write of the whole JSON
file, so two machines writing while offline resolve as last-writer-wins. The
salt fingerprint means a file *replaced* by the sync client is noticed rather
than misdecrypted, but concurrent edits are not merged. Splitting the map into
one file per entry would let the sync client merge them; that is the documented
upgrade path if multi-writer use becomes real.

`prepare_portable_store()` performs the file's creation as an explicit step
rather than leaving it to the first `store`. It writes an empty store when the
file is absent and only *verifies* one that is present — never rewrites it, since
pointing a second machine at a synced file must not open a lost-update window on
a file the sync client is also holding.

### Credential Transfer Between Backends

`vault_ops::plan_credential_transfer` / `run_credential_transfer` implement bulk
copying between any two backends, behind Settings ▸ Secrets ▸ *Move between
stores*. Two constraints shape the design, and both are worth stating because
neither is obvious from the trait.

**It cannot be driven by the backends.** `SecretBackend` has no enumeration
method, and only the two file backends can list what they hold at all (their map
keys are stored in the clear, which is what lets `secret/migration.rs` copy them
verbatim). The transfer therefore derives its work list from the *connection
list*: every `Connection` and `ConnectionGroup` with `PasswordSource::Vault`,
plus the secret variables RustConn owns. A vault may hold entries RustConn never
created; those are out of reach by construction, because nothing can name them.
Variables carrying a custom `kdbx_entry_path` or `vault_entry_name` are skipped —
they reference an entry the user maintains elsewhere.

**Keys are regenerated per side, never copied.** The lookup key's shape belongs
to the backend, not to the credential: `RustConn/{group}/{name} ({protocol})` for
the system keyring, a hierarchical entry path for KDBX, flat `rustconn/{name}`
for the other six. `SecretManager::retrieve` walks the backend chain but always
with the *same* key string, so a shape mismatch is not rescued by the chain — a
credential copied verbatim from the keyring into the portable file would be
unreachable the moment that backend became preferred. Each
`CredentialTransferItem` therefore carries a *list* of source keys (current shape
first, then the legacy shapes `vault_keys_for_connection` knows about) and one
destination key generated for the destination backend.

KDBX is reached through `KeePassStatus`, not through `dispatch_vault_op_for`.
`SecretManager::build_from_settings` deliberately maps the KDBX backends onto the
system keyring, because KDBX proper goes through direct file access; routing a
transfer that way would silently substitute one store for the one the user named.
The KDBX read path returns a password and no username, which is why the transfer
item carries a username of its own.

The source is never modified. For a shared vault the entries may not be
RustConn's to delete, and for the machine-bound file the originals are this
machine's fallback — the same reason the portable-file wizard keeps them.
Deletion is not offered rather than defaulted off.

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
- Child connections can inherit group credentials via `PasswordSource::Inherit`

### Fallback Chain

`SecretManager::build_from_settings` builds a chain of at most two entries: the
backend the user selected, followed by the machine-bound encrypted file when
**Also read from the encrypted file** is on. `retrieve` walks it in order and
returns the first hit.

**Reads may fall back; writes may not.** The two sides are deliberately
asymmetric, because they answer different questions:

- A read falling through to the encrypted file is usually right. A password saved
  before the user switched backend still lives there, and refusing to look would
  strand it. `retrieve` warns with both backend ids when a non-primary entry
  answered, so a fall-through is diagnosable from a log rather than assumed.
- A write is a user action with an explicit destination, so `store_reported` is
  called with `allow_fallback = false` from the GUI save path and the selected
  backend's own error comes back untouched. Where the credential goes instead is
  then a question put to the user (`show_vault_store_failed_dialog`), and the
  encrypted-file destination is reached by naming it — `dispatch_vault_op_for`
  with `SecretBackendType::EncryptedFile`.

That asymmetry is the fix for a specific failure. The write side used to walk the
chain on any primary error, so a locked vault silently relocated the password into
`credentials.enc` — and the connect path does not read that file when another
backend is selected, because `resolve_credentials_blocking` queries the selected
backend alone. The password was saved and, from the connection's point of view,
missing at the same time; what the user saw was "Vault entry not found. You will
be prompted for a password" for a password that was on disk.

### The Vault password source does not use the chain

`resolve_credentials_blocking` is the connect-time path for
`PasswordSource::Vault`, and it goes through `dispatch_vault_op_for` —
`build_single_backend`, one backend, no chain. So the chain above is not what
makes **Also read from the encrypted file** true for a `Vault` connection; only
`PasswordSource::None`, `Inherit` and the variable paths reach `SecretManager` and
`CredentialResolver`.

That is why the setting is applied a second time, explicitly, at the end of both
Vault branches: `retrieve_from_encrypted_file_fallback` reads the encrypted file
under *the same lookup keys the selected backend was asked for*, which is also the
key the "Save to This Computer" response wrote under. `encrypted_file_fallback_enabled`
is the single predicate behind both, and applies the same test
`build_from_settings` applies before appending `EncryptedFileBackend`, so the two
routes to that store cannot disagree about whether it participates.

The read fallback is on the **miss** path only. The `Err` arms report
`BackendNotConfigured` and consult nothing: a store that could not be read has not
said the password is absent, and answering that with a password from elsewhere is
what was wrong with the old KeePassXC fall-through — a locked database answered
from libsecret with nothing said. The narrowing that survives is that a KeePassXC
miss now reaches the encrypted file only, not libsecret. Moving credentials
between stores is what **Copy Passwords…** in Settings ▸ Secrets is for.

## Protocol Architecture

### Protocol Trait

```rust
// rustconn-core/src/protocol/mod.rs
pub trait Protocol: Send + Sync {
    fn protocol_id(&self) -> &'static str;
    fn display_name(&self) -> &'static str;
    fn default_port(&self) -> u16;
    fn validate_connection(&self, connection: &Connection) -> ProtocolResult<()>;
    fn capabilities(&self) -> ProtocolCapabilities { ProtocolCapabilities::terminal() }
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
    pub multi_monitor: bool,
    pub usb_redirection: bool,
    pub port_forwarding: bool,
    pub wayland_forwarding: bool,
    pub x11_forwarding: bool,
    pub session_recording: bool,
    pub remote_monitoring: bool,
    pub command_snippets: bool,
    pub wake_on_lan: bool,
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
- `WebProtocol`: Web URLs in the embedded WebKitGTK browser (`web-embedded`) or the system browser via `UriLauncher`/`xdg-open` (capabilities: external_fallback)
- `WinRmProtocol`: PowerShell on Windows via external `evil-winrm` in a VTE terminal (capabilities: terminal, split_view)
- `ZeroTrustProtocol`: Zero Trust brokers via provider CLIs (aws, gcloud, az, oci, cloudflared, tsh, tailscale, boundary, hoop) in a VTE terminal (capabilities: terminal, split_view)

> **Split-view eligibility is not the `split_view` capability flag.** Since 0.18.1, whether a session
> can be placed in a split panel is decided at the *widget* level by
> `TerminalNotebook::split_eligibility()` (`rustconn/src/terminal/mod.rs`), keyed on the stored
> widget kind rather than the protocol's `ProtocolCapabilities.split_view` flag:
> - **Any in-process embedded widget is `Embeddable`** — a VTE terminal *or* an embedded viewer
>   (`EmbeddedRdp`, `Vnc`, `EmbeddedWeb`). Split view is no longer VTE-only; it works for every
>   embedded tab, including RDP/VNC remote desktops. SPICE has no embedded widget since 0.18.0.
> - An `ExternalProcess` session (xfreerdp/vncviewer/remote-viewer for SPICE) is `ExternalViewer` and
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

**RDP Fallback Strategy (3-step):**

When the embedded IronRDP client encounters issues, it follows a graduated fallback:

1. **GFX/H.264 pipeline** — default when `gfx-h264` feature is enabled and `graphics_mode` is Auto/GfxH264/GfxAvc444
2. **Retry without GFX** — if the GFX pipeline fails (decode errors or no first frame within 15s), the connection is retried with `GraphicsMode::Legacy` which skips EGFX DVC registration entirely. A 1-second delay between disconnect and retry avoids NLA rejection on single-session Windows Servers. This retry happens at most once per connection attempt (`gfx_retry_attempted` flag).
3. **External FreeRDP fallback** — if the Legacy retry also fails (or for non-GFX protocol errors like ServerDemandActive incompatibility), the session is handed off to an external FreeRDP process.

Authentication failures (wrong password, locked account) are excluded from fallback — they are reported immediately without retrying.

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

### Session Placement Model

A live session can sit in one of three places, and only one at a time:

| Placement | Where the widget lives | Tab in the main `adw::TabView`? |
|-----------|------------------------|---------------------------------|
| **Tab** | its own `TabPageContainer` | yes |
| **Split** | a panel of another tab's `SplitViewBridge` (the session is *parked*) | no (the guest tab is closed, state kept) |
| **Detached window** | the content area of an `adw::ApplicationWindow` hosting exactly one session | no (the tab is parked the same way) |

`TerminalNotebook` (`rustconn/src/terminal/`) remains the **single owner of session state** in every
placement — `sessions`, `terminals`, `session_widgets`, `session_info`, `tab_containers`, the park
sets, and the detached set. A split panel or a detached window only *borrows* the session's widget
subtree; it never owns session bookkeeping, and teardown always runs through the notebook's
`close-page` path. Both moves reuse the same primitives: `park_tab_page` removes the page while the
`close-page` handler skips teardown, `build_session_content` rewraps the live widget for its new
host, and `restore_session_tab` clears whichever park set the session was in.

`DetachedWindowRegistry` (`rustconn/src/detached_window.rs`) is the **window registry**: it owns the
`DetachedSessionWindow` values keyed by session id (`insert`, `take`, `contains`, `count`, `present`,
`with_window`, `close_all`), while the per-window operations — `present_fullscreen_on`,
`begin_attach`, `set_session_title`, `close` — sit on `DetachedSessionWindow`. `MainWindow` holds it as
`detached_windows: Rc<DetachedWindowRegistry>`; every callback back into the notebook or the registry
captures `Weak` handles only, so a closed window and its session drop cleanly (no `Rc` cycle).

The decision "may this session be detached" is GUI-free and lives in
`rustconn-core/src/session_placement.rs` — `detach_verdict(&DetachContext) -> DetachVerdict`, a pure
predicate over `renders_in_process`, `is_split_owner`, `is_split_guest`, `is_detached`. Every call
site (tab context menu, keyboard action, sidebar routing) goes through it, and `reason_key()` names
the translated explanation shown when a verdict is not `Allowed` (external viewer, split owner, split
guest, already detached). The notebook-side API is `rustconn/src/terminal/detach.rs`
(`take_session_content`, `attach_session`, `is_detached`, `detached_count`); the window actions
(`win.detach-session`, `win.detach-session-to-monitor`, `win.attach-session`, `win.toggle-detach`)
live in `rustconn/src/window/detach_actions.rs`.

Two consequences worth remembering when touching session code:
- `session_count()` counts tabbed sessions only, so any "open sessions" figure must add
  `detached_count()` (close confirmation, quit path).
- Wayland cannot position a toplevel, so a monitor choice is honoured with
  `present_fullscreen_on()` (fullscreen on the chosen `gdk::Monitor`), never with coordinates.

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
// rustconn/src/toast.rs (see plain_toast)
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
│   ├── detach_actions.rs  # win.detach-session / win.attach-session / win.toggle-detach
│   ├── group_broadcast.rs # Cross-tab keystroke broadcast (GroupBroadcast, #329)
│   └── ...                # Domain-specific window functionality
├── state.rs               # SharedAppState
├── async_utils.rs         # Async helpers (spawn_async, with_runtime)
├── sidebar/               # Connection tree (modular structure)
│   ├── mod.rs             # Module exports, Sidebar struct
│   ├── search.rs          # Search logic, predicates, history
│   ├── filter.rs          # Protocol filter buttons
│   ├── view.rs            # List item creation, binding, signals
│   └── drag_drop.rs       # Drag-and-drop logic with DragPayload
├── sidebar_types.rs       # Sidebar data types
├── sidebar_ui.rs          # Sidebar widget helpers
├── terminal/              # VTE terminal integration
│   ├── mod.rs             # TerminalNotebook — single owner of all session state
│   ├── detach.rs          # Detach/attach API (take_session_content, attach_session)
│   ├── tab_menu.rs        # Tab context menu (incl. Move to New Window)
│   └── ...                # Split view, monitoring, highlighting helpers
├── detached_window.rs     # DetachedSessionWindow + DetachedWindowRegistry (one session per window)
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
│   ├── portable_encrypted_file.rs # Passphrase-keyed KEK/DEK store (cloud-syncable)
│   ├── migration.rs       # Bulk transfer between the two file backends
│   ├── local_crypto.rs    # AES-256-GCM + Argon2id primitives
│   ├── detection.rs       # Password manager detection
│   ├── status.rs          # KeePass status detection
│   ├── async_resolver.rs  # Non-blocking credential resolution for the UI
│   ├── bitwarden_serve.rs # Persistent `bw serve` transport (feature bitwarden-serve)
│   ├── kdbx_backend.rs    # KDBX SecretBackend wrapper
│   ├── passphrase_strength.rs # Portable store passphrase strength estimate
│   ├── serde_helpers.rs   # Serde helpers for secret fields
│   ├── store_key.rs       # Vault keys a credential is stored under
│   ├── touch.rs           # Hardware-key touch notice for keepassxc-cli
│   ├── vault_identity.rs  # Account identity supplied with a vault password
│   ├── verification.rs    # Credential verification tracking
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
│   ├── quick_actions.rs   # Windows admin quick actions (key sequences)
│   └── ...
├── cli_download/          # Flatpak CLI download manager
├── dynamic_folder.rs      # Dynamic folder executor — script execution, JSON parsing, entry→Connection conversion
├── highlight.rs           # Text highlighting rules engine (CompiledHighlightRules, find_matches)
├── session_placement.rs   # detach_verdict() — GUI-free "can this session be detached" predicate
├── smart_folder.rs        # SmartFolderManager — dynamic connection grouping with filter evaluation
├── sftp.rs                # SFTP URI/command builders, ssh-add, mc FISH VFS
├── flatpak.rs             # Flatpak sandbox detection, portal key path resolution, stable key copy
├── snap.rs                # Snap environment detection and paths
├── performance/           # Search debouncer
├── tracing/               # Span name constants + the `rustconn::perf` timing target (PerfSpan)
└── ...                    # ssh_tunnel, ssh_agent, sync, split, tmux_control (experimental, unused by the GUI), cluster, lock_events,
                           # mc_ssh, socks_relay, jump_route, remote_script, … — full list: `pub mod` in lib.rs
```

## Performance Instrumentation (`rustconn::perf`)

0.24 is a performance release, so its targets are measured rather than felt. Every vault operation and every SSH-connect phase is timed by a `PerfSpan` (`rustconn-core/src/tracing/perf.rs`): construct one at the start of an operation, hold it for the operation's lifetime, and its `Drop` emits one structured line to the `rustconn::perf` tracing target with fields `op`, `transport`, `elapsed_ms` and `calls`. The line carries fields, never a formatted string, so a baseline table and a post-change table compare field by field.

The target is off by default. The application initialises its subscriber from `RUST_LOG`, so these lines appear only when you ask for them:

```bash
RUST_LOG=rustconn::perf=info ./target/debug/rustconn
```

A perf line never carries a secret, a credential, a vault item name, a hostname, or a username — only fixed `&'static str` labels. The timing of an operation is not sensitive; what it operated on is.

### Instrumented phases (W0)

| `op` | `transport` | Where | Measures |
|---|---|---|---|
| `vault.unlock` | `bitwarden` | `secret/bitwarden.rs::auto_unlock` | Start to "vault ready" (target ≤ 4 s) |
| `vault.retrieve` / `vault.store` / `vault.delete` | the backend (`bitwarden`, `keepassxc`, `libsecret`, …) | `vault_ops.rs::dispatch_vault_op_for` | One vault operation against the named backend |
| `connect.resolve` | `credentials` | `state/mod.rs::resolve_credentials_blocking` | The off-main-thread credential-resolve phase for every protocol, incl. cold-backend retries (`ssh.connect.resolve` until 0.24.8) |
| `ssh.connect.agent` | `ssh-agent` | `window/protocols_ssh.rs::ensure_identity_in_agent` | ssh-agent identity add; `calls` counts the agent round trips (A1 target) |
| `ssh.connect.args` | `ssh` | `window/protocols_ssh.rs::build_ssh_command_args` | Connect-args build: key inheritance, jump-host/ProxyCommand wiring, askpass helpers |

`calls` defaults to 1 and is raised with `PerfSpan::add_calls` when one logical operation fans out into several underlying calls (several `bw` spawns behind one store, several agent round trips behind one key lookup) — exactly the counts 0.24 is driving down.

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

Located in `rustconn-core/tests/properties/` (~240 `proptest!` blocks; `rustconn-core/tests/` holds 1500+ `#[test]` functions, on top of the unit tests in `src/`):

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
