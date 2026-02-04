# Code Quality Refactoring - Design

## Overview

This document describes the technical design for addressing code quality issues identified in the RustConn v0.7.4 audit.

## Architecture

### Component Diagram

```
rustconn-core/
├── src/
│   ├── import/
│   │   ├── traits.rs          # Add read_import_file() helper
│   │   ├── ssh_config.rs      # Use helper
│   │   ├── ansible.rs         # Use helper
│   │   ├── remmina.rs         # Use helper
│   │   ├── asbru.rs           # Use helper
│   │   └── royalts.rs         # Use helper
│   ├── embedded_client_error.rs  # NEW: Unified error type
│   ├── rdp_client/
│   │   ├── error.rs           # Keep for backward compat, re-export alias
│   │   └── mod.rs             # Re-export EmbeddedClientError as RdpClientError
│   ├── vnc_client/
│   │   ├── error.rs           # Keep for backward compat, re-export alias
│   │   └── mod.rs             # Re-export EmbeddedClientError as VncClientError
│   ├── spice_client/
│   │   ├── error.rs           # Keep for backward compat, re-export alias
│   │   └── mod.rs             # Re-export EmbeddedClientError as SpiceClientError
│   └── config/
│       └── manager.rs         # Atomic writes
│
rustconn/
└── src/
    ├── lib.rs                 # GTK lifecycle documentation
    └── terminal/
        └── types.rs           # Remove legacy types
```

## Detailed Design

### 1. Import File I/O Helper

**Location:** `rustconn-core/src/import/traits.rs`

```rust
use std::fs;
use std::path::Path;
use crate::error::ImportError;

/// Reads a file for import operations with consistent error handling.
///
/// # Arguments
/// * `path` - Path to the file to read
/// * `source_name` - Human-readable name of the import source (e.g., "SSH config")
///
/// # Errors
/// Returns `ImportError::ParseError` if the file cannot be read.
///
/// # Example
/// ```ignore
/// let content = read_import_file(path, "SSH config")?;
/// ```
pub fn read_import_file(path: &Path, source_name: &str) -> Result<String, ImportError> {
    fs::read_to_string(path).map_err(|e| ImportError::ParseError {
        source_name: source_name.to_string(),
        reason: format!("Failed to read {}: {}", path.display(), e),
    })
}

/// Async variant of `read_import_file` for future use.
///
/// Uses tokio's async file I/O for non-blocking reads.
pub async fn read_import_file_async(
    path: &Path,
    source_name: &str,
) -> Result<String, ImportError> {
    tokio::fs::read_to_string(path)
        .await
        .map_err(|e| ImportError::ParseError {
            source_name: source_name.to_string(),
            reason: format!("Failed to read {}: {}", path.display(), e),
        })
}
```

**Usage in importers:**
```rust
// Before (5 lines):
let content = fs::read_to_string(path).map_err(|e| ImportError::ParseError {
    source_name: "SSH config".to_string(),
    reason: format!("Failed to read {}: {}", path.display(), e),
})?;

// After (1 line):
let content = read_import_file(path, "SSH config")?;
```

### 2. Unified Protocol Error Type

**Location:** `rustconn-core/src/embedded_client_error.rs`

```rust
//! Unified error types for embedded protocol clients (RDP, VNC, SPICE)
//!
//! This module provides a single error enum that covers all embedded client
//! operations, reducing code duplication across protocol implementations.

use thiserror::Error;

/// Generic error type for embedded protocol clients.
///
/// This enum consolidates error variants from RDP, VNC, and SPICE clients
/// into a single type. Protocol-specific variants are included for cases
/// that only apply to certain protocols.
#[derive(Debug, Error, Clone)]
pub enum EmbeddedClientError {
    // === Common variants (all protocols) ===
    
    /// Connection to server failed
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    /// Authentication failed
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),

    /// Protocol error during communication
    #[error("Protocol error: {0}")]
    ProtocolError(String),

    /// IO error during network operations
    #[error("IO error: {0}")]
    IoError(String),

    /// Client is not connected
    #[error("Not connected")]
    NotConnected,

    /// Client is already connected
    #[error("Already connected")]
    AlreadyConnected,

    /// Invalid configuration
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    /// Channel communication error
    #[error("Channel error: {0}")]
    ChannelError(String),

    /// Timeout waiting for operation
    #[error("Operation timed out")]
    Timeout,

    /// Server disconnected
    #[error("Server disconnected: {0}")]
    ServerDisconnected(String),

    /// Unsupported feature
    #[error("Unsupported feature: {0}")]
    Unsupported(String),

    // === TLS (RDP, SPICE) ===
    
    /// TLS/SSL error
    #[error("TLS error: {0}")]
    TlsError(String),

    // === SPICE-specific ===
    
    /// USB redirection error
    #[error("USB redirection error: {0}")]
    UsbRedirectionError(String),

    /// Shared folder error
    #[error("Shared folder error: {0}")]
    SharedFolderError(String),

    /// Native SPICE client not available, fallback required
    #[error("Native SPICE client not available, falling back to virt-viewer")]
    NativeClientNotAvailable,
}

impl From<std::io::Error> for EmbeddedClientError {
    fn from(err: std::io::Error) -> Self {
        Self::IoError(err.to_string())
    }
}

// Type aliases for backward compatibility
/// RDP client error type (alias for `EmbeddedClientError`)
pub type RdpClientError = EmbeddedClientError;

/// VNC client error type (alias for `EmbeddedClientError`)
pub type VncClientError = EmbeddedClientError;

/// SPICE client error type (alias for `EmbeddedClientError`)
pub type SpiceClientError = EmbeddedClientError;
```

**Migration strategy:**
1. Create new `embedded_client_error.rs` module
2. Update `rdp_client/error.rs` to re-export from new module
3. Update `vnc_client/error.rs` to re-export from new module
4. Update `spice_client/error.rs` to re-export from new module
5. Update `lib.rs` to export unified type

### 3. Atomic Config Writes

**Location:** `rustconn-core/src/config/manager.rs`

```rust
/// Saves data to a TOML file asynchronously with atomic write.
///
/// Uses a temp file + rename pattern to prevent data corruption
/// if the process crashes during write.
#[allow(clippy::future_not_send)]
async fn save_toml_file_async<T>(path: &Path, data: &T) -> ConfigResult<()>
where
    T: serde::Serialize,
{
    let content = toml::to_string_pretty(data)
        .map_err(|e| ConfigError::Serialize(format!("Failed to serialize: {e}")))?;

    // Use temp file for atomic write
    let temp_path = path.with_extension("tmp");

    // Write to temp file
    tokio::fs::write(&temp_path, content.as_bytes())
        .await
        .map_err(|e| ConfigError::Write(
            format!("Failed to write {}: {}", temp_path.display(), e)
        ))?;

    // Atomic rename
    tokio::fs::rename(&temp_path, path)
        .await
        .map_err(|e| ConfigError::Write(
            format!("Failed to finalize {}: {}", path.display(), e)
        ))?;

    Ok(())
}
```

### 4. Legacy Code Removal

**Files to modify:** `rustconn/src/terminal/types.rs`

**Types to remove after verification:**
- `TabDisplayMode` - Legacy enum for tab display modes (now handled by `adw::TabView`)
- `SessionWidgetStorage` - Legacy enum for storing session widgets
- `TabLabelWidgets` - Legacy struct for tab label components

**Verification command:**
```bash
grep -r "TabDisplayMode\|SessionWidgetStorage\|TabLabelWidgets" rustconn/src/ --include="*.rs" | grep -v "types.rs"
```

### 5. GTK Lifecycle Documentation

**Location:** `rustconn/src/lib.rs` (add at top of file)

```rust
//! # RustConn GTK4 Application
//!
//! ## GTK Widget Lifecycle Pattern
//!
//! Throughout this crate, you'll see struct fields marked with `#[allow(dead_code)]`.
//! These are **intentionally kept alive** for GTK widget lifecycle management:
//!
//! - **Signal handlers**: `connect_clicked()`, `connect_changed()`, etc. hold references
//! - **Event controllers**: Motion, key, and scroll controllers need widget references
//! - **Widget tree ownership**: Parent-child relationships require keeping references
//!
//! **⚠️ WARNING**: Removing these "unused" fields will cause **segmentation faults**
//! when GTK signals fire, because the signal handler closures capture these references.
//!
//! ### Example
//!
//! ```ignore
//! pub struct MyDialog {
//!     window: adw::Window,
//!     #[allow(dead_code)] // Kept alive for connect_clicked() handler
//!     save_button: gtk4::Button,
//! }
//! ```
//!
//! The `save_button` field appears unused, but removing it would cause the button's
//! click handler to crash when invoked.
```

## Testing Strategy

### Unit Tests

1. **Import helper tests** - Verify `read_import_file()` returns correct errors
2. **Error type tests** - Verify all variants serialize/deserialize correctly
3. **Atomic write tests** - Verify temp file is created and renamed

### Property Tests

**Location:** `rustconn-core/tests/properties/`

1. Update `import_tests.rs` to use new helper
2. Create `embedded_client_error_tests.rs` for unified error type
3. Update `config_tests.rs` for atomic writes

### Integration Tests

1. Run full import test suite after refactoring
2. Verify all protocol clients work with unified error type

## Rollout Plan

1. **Phase 1**: Create `read_import_file()` helper and update importers
2. **Phase 2**: Create `EmbeddedClientError` and update protocol clients
3. **Phase 3**: Implement atomic config writes
4. **Phase 4**: Remove legacy types and add documentation
5. **Phase 5**: Run full test suite and verify clippy/fmt compliance

## Correctness Properties

### Property 1: Import File Reading Consistency
For any valid file path and source name, `read_import_file(path, source_name)` SHALL return the same content as `fs::read_to_string(path)` when successful.

### Property 2: Error Type Backward Compatibility
For any code using `RdpClientError`, `VncClientError`, or `SpiceClientError`, the code SHALL compile without modification after migration to type aliases.

### Property 3: Atomic Write Integrity
For any configuration save operation, if the process crashes during `save_toml_file_async()`, the original file SHALL remain intact (either old content or new content, never partial).

### Property 4: Legacy Type Removal Safety
After removing legacy types, `cargo build -p rustconn` SHALL succeed with zero errors.
