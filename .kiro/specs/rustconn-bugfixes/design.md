# Design Document: RustConn Bug Fixes and Improvements

## Overview

This design document describes the technical approach for implementing bug fixes, UI improvements, and new features for RustConn. The changes are organized into several categories: critical bug fixes, UX improvements, new features, and technical upgrades.

## Architecture

The changes integrate with the existing `rustconn-core` and `rustconn` crates following the established patterns:

```
┌─────────────────────────────────────────────────────────────────┐
│                        rustconn (GUI)                           │
├─────────────────────────────────────────────────────────────────┤
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐             │
│  │ Tray (fix)  │  │ Sidebar DnD │  │ Clipboard   │             │
│  └─────────────┘  └─────────────┘  └─────────────┘             │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐             │
│  │ RDP Widget  │  │ VNC Widget  │  │ Tree State  │             │
│  └─────────────┘  └─────────────┘  └─────────────┘             │
├─────────────────────────────────────────────────────────────────┤
│                      rustconn-core                              │
├─────────────────────────────────────────────────────────────────┤
│  ┌─────────────────────────────────────────────────────────┐   │
│  │                    Export/Import                         │   │
│  │  ┌───────────┐ ┌───────────┐ ┌───────────┐             │   │
│  │  │ Native RCN│ │ Schema    │ │ Migration │             │   │
│  │  └───────────┘ └───────────┘ └───────────┘             │   │
│  └─────────────────────────────────────────────────────────┘   │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │                    Protocol Enhancements                 │   │
│  │  ┌───────────┐ ┌───────────┐ ┌───────────┐             │   │
│  │  │SSH Options│ │CLI Output │ │ Icon Cache│             │   │
│  │  └───────────┘ └───────────┘ └───────────┘             │   │
│  └─────────────────────────────────────────────────────────┘   │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │                    Embedded RDP/VNC                      │   │
│  │  ┌───────────┐ ┌───────────┐ ┌───────────┐             │   │
│  │  │ Subsurface│ │ FreeRDP   │ │ Input Fwd │             │   │
│  │  └───────────┘ └───────────┘ └───────────┘             │   │
│  └─────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
```

## Components and Interfaces

### 1. Tray Icon Fix (`rustconn/src/tray.rs`)

The current implementation uses PNG fallback. We need to prioritize SVG loading:

```rust
impl Tray for RustConnTray {
    fn icon_name(&self) -> String {
        // Return the icon name for theme lookup
        "org.rustconn.RustConn".to_string()
    }

    fn icon_theme_path(&self) -> String {
        // Return path to scalable/apps directory containing SVG
        Self::find_icon_theme_path()
    }
}

impl RustConnTray {
    fn find_icon_theme_path() -> String {
        // Priority order:
        // 1. Development path: rustconn/assets/icons/hicolor
        // 2. Installed path: /usr/share/icons/hicolor
        // 3. XDG data dirs
    }
}
```

### 2. Drag-and-Drop Visual Feedback (`rustconn/src/sidebar.rs`)

Add drop indicator line using GTK4's drag-and-drop API:

```rust
/// Drop position indicator
pub struct DropIndicator {
    /// Position type: before, after, or into (for groups)
    position: DropPosition,
    /// Target row index
    target_index: u32,
}

pub enum DropPosition {
    Before,
    After,
    Into, // For dropping into groups
}

impl ConnectionSidebar {
    fn setup_drop_indicator(&self) {
        // Create a horizontal line widget for drop indication
        let indicator = gtk4::Separator::new(Orientation::Horizontal);
        indicator.add_css_class("drop-indicator");
        indicator.set_visible(false);
        
        // Add CSS for styling
        // .drop-indicator { background: @accent_color; min-height: 2px; }
    }
    
    fn update_drop_indicator(&self, y: f64, target_row: Option<&TreeListRow>) {
        // Calculate position relative to row
        // Show line above, below, or highlight group
    }
}
```

### 3. GTK PopoverMenu Fix

The warning occurs when popover is destroyed while still "active". Fix by properly managing lifecycle:

```rust
fn show_context_menu_for_item(widget: &impl IsA<Widget>, x: f64, y: f64, is_group: bool) {
    let popover = gtk4::Popover::new();
    popover.set_parent(widget);
    
    // Connect to closed signal for cleanup
    popover.connect_closed(|p| {
        // Ensure parent is unset before destruction
        p.unparent();
    });
    
    // Use popover.popup() instead of present()
    popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    popover.popup();
}
```

### 4. ZeroTrust CLI Provider Icons (`rustconn-core/src/protocol/icons.rs`)

```rust
/// Cloud provider icon cache
pub struct ProviderIconCache {
    cache_dir: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudProvider {
    Aws,
    Gcloud,
    Azure,
    Generic,
}

impl ProviderIconCache {
    pub fn new() -> Self {
        let cache_dir = dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("rustconn/icons");
        Self { cache_dir }
    }
    
    /// Get icon path for provider, caching if needed
    pub fn get_icon_path(&self, provider: CloudProvider) -> PathBuf {
        let icon_name = match provider {
            CloudProvider::Aws => "aws-logo.svg",
            CloudProvider::Gcloud => "gcloud-logo.svg",
            CloudProvider::Azure => "azure-logo.svg",
            CloudProvider::Generic => "cloud-symbolic.svg",
        };
        self.cache_dir.join(icon_name)
    }
    
    /// Detect provider from CLI command
    pub fn detect_provider(command: &str) -> CloudProvider {
        if command.contains("aws") { CloudProvider::Aws }
        else if command.contains("gcloud") { CloudProvider::Gcloud }
        else if command.contains("az ") || command.contains("azure") { CloudProvider::Azure }
        else { CloudProvider::Generic }
    }
}
```

### 5. CLI Connection Output Feedback (`rustconn-core/src/protocol/cli.rs`)

```rust
/// Format connection start message
pub fn format_connection_message(protocol: &str, host: &str) -> String {
    format!("🔗 Connecting via {} to {}...", protocol, host)
}

/// Format command execution message
pub fn format_command_message(command: &str) -> String {
    format!("⚡ Executing: {}", command)
}

/// CLI connection with output feedback
impl CliProtocol {
    pub async fn connect_with_feedback(&self, terminal: &Terminal) -> Result<()> {
        // Echo connection info
        terminal.feed(&format_connection_message(&self.protocol_name, &self.host));
        terminal.feed("\r\n");
        
        // Echo command
        terminal.feed(&format_command_message(&self.command));
        terminal.feed("\r\n\r\n");
        
        // Execute
        self.execute(terminal).await
    }
}
```

### 6. SSH IdentitiesOnly Option (`rustconn-core/src/models/protocol.rs`)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshConfig {
    // ... existing fields ...
    
    /// Use only the specified identity file (prevents "Too many authentication failures")
    #[serde(default)]
    pub identities_only: bool,
    
    /// SSH agent key fingerprint (for persistence)
    #[serde(default)]
    pub ssh_agent_key_fingerprint: Option<String>,
}

impl SshConfig {
    /// Build SSH command arguments
    pub fn build_command_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        
        if self.identities_only {
            args.push("-o".to_string());
            args.push("IdentitiesOnly=yes".to_string());
        }
        
        if let Some(ref key_path) = self.identity_file {
            args.push("-i".to_string());
            args.push(key_path.clone());
        }
        
        // ... other args
        args
    }
}
```

### 7. Connection Tree State Preservation (`rustconn/src/sidebar.rs`)

```rust
/// Tree state for preservation across refreshes
#[derive(Debug, Clone, Default)]
pub struct TreeState {
    /// Expanded group IDs
    pub expanded_groups: HashSet<Uuid>,
    /// Scroll position (vertical adjustment value)
    pub scroll_position: f64,
    /// Selected item ID
    pub selected_id: Option<Uuid>,
}

impl ConnectionSidebar {
    /// Save current tree state
    pub fn save_state(&self) -> TreeState {
        let mut state = TreeState::default();
        
        // Iterate tree model and record expanded rows
        // Save scroll position from ScrolledWindow
        // Save selected item ID
        
        state
    }
    
    /// Restore tree state after refresh
    pub fn restore_state(&self, state: &TreeState) {
        // Expand saved groups
        // Restore scroll position
        // Restore selection
    }
    
    /// Refresh tree while preserving state
    pub fn refresh_preserving_state(&self) {
        let state = self.save_state();
        self.refresh();
        self.restore_state(&state);
    }
}
```

### 8. Connection Clipboard (`rustconn/src/state.rs`)

```rust
/// Internal clipboard for connection copy/paste
#[derive(Debug, Clone, Default)]
pub struct ConnectionClipboard {
    /// Copied connection data
    connection: Option<Connection>,
    /// Source group ID
    source_group: Option<Uuid>,
}

impl ConnectionClipboard {
    pub fn copy(&mut self, connection: &Connection, group_id: Option<Uuid>) {
        self.connection = Some(connection.clone());
        self.source_group = group_id;
    }
    
    pub fn paste(&self) -> Option<Connection> {
        self.connection.as_ref().map(|conn| {
            let mut new_conn = conn.clone();
            new_conn.id = Uuid::new_v4();
            new_conn.name = format!("{} (Copy)", conn.name);
            new_conn
        })
    }
    
    pub fn has_content(&self) -> bool {
        self.connection.is_some()
    }
    
    pub fn source_group(&self) -> Option<Uuid> {
        self.source_group
    }
}
```

### 9. Native Export/Import Format (`rustconn-core/src/export/native.rs`)

```rust
/// RustConn native export format (.rcn)
pub const NATIVE_FORMAT_VERSION: u32 = 1;
pub const NATIVE_FILE_EXTENSION: &str = "rcn";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeExport {
    /// Format version for migrations
    pub version: u32,
    /// Export timestamp
    pub exported_at: DateTime<Utc>,
    /// Application version that created export
    pub app_version: String,
    /// All connections
    pub connections: Vec<Connection>,
    /// All groups with hierarchy
    pub groups: Vec<Group>,
    /// All templates
    pub templates: Vec<ConnectionTemplate>,
    /// All clusters
    pub clusters: Vec<Cluster>,
    /// Global variables
    pub variables: Vec<Variable>,
    /// Custom metadata
    pub metadata: HashMap<String, String>,
}

impl NativeExport {
    pub fn new() -> Self {
        Self {
            version: NATIVE_FORMAT_VERSION,
            exported_at: Utc::now(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            connections: Vec::new(),
            groups: Vec::new(),
            templates: Vec::new(),
            clusters: Vec::new(),
            variables: Vec::new(),
            metadata: HashMap::new(),
        }
    }
    
    /// Export to JSON string
    pub fn to_json(&self) -> Result<String, ExportError> {
        serde_json::to_string_pretty(self)
            .map_err(|e| ExportError::Serialization(e.to_string()))
    }
    
    /// Import from JSON string with version validation
    pub fn from_json(json: &str) -> Result<Self, ImportError> {
        let export: Self = serde_json::from_str(json)
            .map_err(|e| ImportError::Parse(e.to_string()))?;
        
        if export.version > NATIVE_FORMAT_VERSION {
            return Err(ImportError::UnsupportedVersion(export.version));
        }
        
        // Apply migrations if needed
        Self::migrate(export)
    }
    
    fn migrate(mut export: Self) -> Result<Self, ImportError> {
        // Version 1 is current, no migrations needed yet
        export.version = NATIVE_FORMAT_VERSION;
        Ok(export)
    }
}
```

### 10. Embedded RDP/VNC via Wayland Subsurface

This is a complex feature requiring Wayland and FreeRDP integration:

```rust
/// Embedded RDP widget using Wayland subsurface
pub struct EmbeddedRdpWidget {
    /// GTK drawing area for the subsurface
    drawing_area: gtk4::DrawingArea,
    /// Wayland surface handle
    wl_surface: Option<WlSurface>,
    /// Shared memory buffer for pixel data
    shm_buffer: Option<ShmBuffer>,
    /// FreeRDP context
    freerdp_context: Option<FreeRdpContext>,
    /// Frame buffer dimensions
    width: u32,
    height: u32,
}

impl EmbeddedRdpWidget {
    pub fn new() -> Self {
        let drawing_area = gtk4::DrawingArea::new();
        drawing_area.set_hexpand(true);
        drawing_area.set_vexpand(true);
        
        Self {
            drawing_area,
            wl_surface: None,
            shm_buffer: None,
            freerdp_context: None,
            width: 0,
            height: 0,
        }
    }
    
    /// Initialize Wayland subsurface
    fn init_subsurface(&mut self, parent_surface: &WlSurface) -> Result<()> {
        // Create wl_subsurface as child of parent
        // Set up shared memory buffer
        // Configure subsurface position
    }
    
    /// Connect to RDP server
    pub async fn connect(&mut self, config: &RdpConfig) -> Result<()> {
        // Initialize FreeRDP in software rendering mode
        // Set up BeginPaint/EndPaint callbacks
        // Start connection
    }
    
    /// FreeRDP EndPaint callback - blit frame to Wayland buffer
    fn on_end_paint(&mut self, x: i32, y: i32, w: i32, h: i32) {
        // Copy pixels from FreeRDP buffer to wl_buffer
        // Damage the updated region
        // Commit the surface
    }
    
    /// Forward keyboard input to RDP session
    pub fn send_key(&self, keyval: u32, pressed: bool) {
        // Convert GTK keyval to RDP scancode
        // Send to FreeRDP
    }
    
    /// Forward mouse input to RDP session
    pub fn send_mouse(&self, x: i32, y: i32, button: u32, pressed: bool) {
        // Send mouse event to FreeRDP
    }
}
```

## Data Models

### Extended SshConfig

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshConfig {
    pub port: u16,
    pub username: Option<String>,
    pub identity_file: Option<String>,
    pub proxy_command: Option<String>,
    pub forward_agent: bool,
    pub compression: bool,
    pub keep_alive_interval: Option<u32>,
    pub strict_host_key_checking: Option<bool>,
    // New fields
    pub identities_only: bool,
    pub ssh_agent_key_fingerprint: Option<String>,
}
```

### TreeState for Sidebar

```rust
#[derive(Debug, Clone, Default)]
pub struct TreeState {
    pub expanded_groups: HashSet<Uuid>,
    pub scroll_position: f64,
    pub selected_id: Option<Uuid>,
}
```

## Correctness Properties

*A property is a characteristic or behavior that should hold true across all valid executions of a system-essentially, a formal statement about what the system should do. Properties serve as the bridge between human-readable specifications and machine-verifiable correctness guarantees.*

### Property 1: Provider Icon Detection
*For any* CLI command string, the provider detection function should return a valid CloudProvider enum value.
**Validates: Requirements 4.2**

### Property 2: CLI Output Message Format
*For any* protocol name and host string, the connection message should contain both values and the 🔗 emoji.
**Validates: Requirements 5.1, 5.3**

### Property 3: CLI Command Echo Format
*For any* command string, the command echo message should contain the exact command and the ⚡ emoji.
**Validates: Requirements 5.2, 5.4**

### Property 4: SSH IdentitiesOnly Command Generation
*For any* SSH config with identities_only=true, the generated command should contain "-o IdentitiesOnly=yes".
**Validates: Requirements 6.2, 6.3**

### Property 5: SSH Config Serialization Round-Trip
*For any* valid SshConfig including identities_only and ssh_agent_key_fingerprint, serializing and deserializing should produce an equivalent config.
**Validates: Requirements 6.4, 8.4**

### Property 6: SSH Agent Key Fingerprint Persistence
*For any* connection with a saved ssh_agent_key_fingerprint, loading the connection should preserve the fingerprint value.
**Validates: Requirements 8.1, 8.2**

### Property 7: Session Logger File Creation
*For any* enabled LogConfig with valid path, creating a SessionLogger should result in a log file being created.
**Validates: Requirements 9.1, 9.2**

### Property 8: Cluster Serialization Round-Trip
*For any* valid Cluster, serializing and deserializing should produce an equivalent cluster.
**Validates: Requirements 10.1, 10.2**

### Property 9: Template Serialization Round-Trip
*For any* valid ConnectionTemplate, serializing and deserializing should produce an equivalent template.
**Validates: Requirements 11.3**

### Property 10: Connection Copy Creates Valid Duplicate
*For any* connection, copying and pasting should create a new connection with different ID and "(Copy)" suffix in name.
**Validates: Requirements 12.1, 12.2**

### Property 11: Connection Paste Preserves Group
*For any* copied connection with a source group, pasting should return the same source group ID.
**Validates: Requirements 12.3**

### Property 12: Native Export Contains All Data Types
*For any* NativeExport with connections, groups, templates, clusters, and variables, the JSON output should contain all data.
**Validates: Requirements 13.2**

### Property 13: Native Import Restores All Data
*For any* valid NativeExport JSON, importing should restore all connections, groups, templates, clusters, and variables.
**Validates: Requirements 13.3**

### Property 14: Native Format Round-Trip
*For any* valid NativeExport, serializing to JSON and deserializing should produce an equivalent export.
**Validates: Requirements 13.6**

### Property 15: Native Format Schema Version
*For any* NativeExport, the JSON output should contain a version field with value >= 1.
**Validates: Requirements 13.4**

### Property 16: Native Import Version Validation
*For any* NativeExport JSON with version > NATIVE_FORMAT_VERSION, importing should return an UnsupportedVersion error.
**Validates: Requirements 13.5**

## Error Handling

```rust
#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("Serialization failed: {0}")]
    Serialization(String),
    #[error("File write failed: {0}")]
    FileWrite(String),
}

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("Parse failed: {0}")]
    Parse(String),
    #[error("Unsupported format version: {0}")]
    UnsupportedVersion(u32),
    #[error("File read failed: {0}")]
    FileRead(String),
    #[error("Migration failed: {0}")]
    Migration(String),
}

#[derive(Debug, thiserror::Error)]
pub enum EmbeddedRdpError {
    #[error("Wayland subsurface creation failed: {0}")]
    SubsurfaceCreation(String),
    #[error("FreeRDP initialization failed: {0}")]
    FreeRdpInit(String),
    #[error("Connection failed: {0}")]
    Connection(String),
    #[error("wlfreerdp not available, falling back to external mode")]
    WlFreeRdpNotAvailable,
}
```

## Testing Strategy

### Dual Testing Approach

1. **Unit Tests**: Verify specific examples and edge cases
2. **Property-Based Tests**: Verify universal properties using `proptest`

### Property-Based Testing

Using `proptest` crate:

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn test_native_export_round_trip(
        connections in prop::collection::vec(arb_connection(), 0..10),
        groups in prop::collection::vec(arb_group(), 0..5),
    ) {
        let mut export = NativeExport::new();
        export.connections = connections.clone();
        export.groups = groups.clone();
        
        let json = export.to_json().unwrap();
        let imported = NativeExport::from_json(&json).unwrap();
        
        prop_assert_eq!(export.connections.len(), imported.connections.len());
        prop_assert_eq!(export.groups.len(), imported.groups.len());
    }
}
```

### Test Organization

- Property tests in `rustconn-core/tests/properties/`
- Unit tests co-located with source files
- Integration tests for GTK components (manual testing)

