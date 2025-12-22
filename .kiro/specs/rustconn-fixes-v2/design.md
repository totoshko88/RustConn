# Design Document: RustConn Fixes V2

## Overview

This design document describes the technical approach for fixing critical bugs discovered during user testing: embedded RDP/VNC not working, KeePass password storage without hierarchy, cluster dialog not refreshing, and ZeroTrust provider icons not displaying correctly.

## Architecture

The fixes integrate with existing modules while addressing specific integration issues:

```
┌─────────────────────────────────────────────────────────────────┐
│                        rustconn (GUI)                           │
├─────────────────────────────────────────────────────────────────┤
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐ │
│  │ EmbeddedRdp     │  │ EmbeddedVnc     │  │ ClusterDialog   │ │
│  │ (GTK4 native)   │  │ (GTK4 native)   │  │ (auto-refresh)  │ │
│  └─────────────────┘  └─────────────────┘  └─────────────────┘ │
│  ┌─────────────────┐  ┌─────────────────┐                      │
│  │ Sidebar Icons   │  │ FreeRDP Thread  │                      │
│  │ (provider fix)  │  │ (Qt isolation)  │                      │
│  └─────────────────┘  └─────────────────┘                      │
├─────────────────────────────────────────────────────────────────┤
│                      rustconn-core                              │
├─────────────────────────────────────────────────────────────────┤
│  ┌─────────────────────────────────────────────────────────┐   │
│  │                    Secret/KeePass                        │   │
│  │  ┌───────────┐ ┌───────────┐ ┌───────────┐             │   │
│  │  │ Hierarchy │ │ KDBX Write│ │ Entry Mgmt│             │   │
│  │  └───────────┘ └───────────┘ └───────────┘             │   │
│  └─────────────────────────────────────────────────────────┘   │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │                    Protocol/Icons                        │   │
│  │  ┌───────────┐ ┌───────────┐                           │   │
│  │  │ Detection │ │ Persistence│                           │   │
│  │  └───────────┘ └───────────┘                           │   │
│  └─────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
```

## Components and Interfaces

### 1. Embedded RDP via GTK4 Native Rendering

The current implementation attempts to use wlfreerdp but encounters Qt/Wayland threading issues. The fix involves:

```rust
/// Thread-safe FreeRDP wrapper that isolates Qt from GTK main thread
pub struct FreeRdpThread {
    /// Handle to the FreeRDP process
    process: Option<Child>,
    /// Shared memory buffer for frame data
    frame_buffer: Arc<Mutex<PixelBuffer>>,
    /// Channel for sending commands to FreeRDP thread
    command_tx: mpsc::Sender<RdpCommand>,
    /// Channel for receiving events from FreeRDP thread
    event_rx: mpsc::Receiver<RdpEvent>,
}

impl FreeRdpThread {
    /// Spawns FreeRDP in a dedicated thread to avoid Qt/GTK conflicts
    pub fn spawn(config: &RdpConfig) -> Result<Self, EmbeddedRdpError> {
        // Run FreeRDP in separate thread to isolate QSocketNotifier issues
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (evt_tx, evt_rx) = mpsc::channel();
        
        std::thread::spawn(move || {
            // FreeRDP operations run here, isolated from GTK main thread
            Self::run_freerdp_loop(cmd_rx, evt_tx);
        });
        
        Ok(Self { ... })
    }
}

/// Alternative: Use xfreerdp with X11 embedding via GtkSocket (X11 only)
/// or render to shared memory and blit to Cairo surface
pub struct SoftwareRdpRenderer {
    /// Cairo surface for rendering
    surface: cairo::ImageSurface,
    /// Frame buffer from FreeRDP
    buffer: Arc<Mutex<Vec<u8>>>,
}
```

### 2. KeePass Password Storage with Hierarchy

The current implementation stores entries flat. We need to create folder hierarchy:

```rust
/// KeePass entry manager with hierarchical storage
pub struct KeePassManager {
    /// Path to KDBX database
    database_path: PathBuf,
    /// Database password (encrypted in memory)
    password: SecretString,
    /// Cached group structure
    groups: HashMap<String, KeePassGroup>,
}

impl KeePassManager {
    /// Builds the group path for a connection based on its hierarchy
    /// 
    /// # Example
    /// Connection in "Servers/Production/Web" group becomes:
    /// "RustConn/Servers/Production/Web/connection_name"
    pub fn build_entry_path(
        &self,
        connection: &Connection,
        groups: &[ConnectionGroup],
    ) -> String {
        let mut path_parts = vec!["RustConn".to_string()];
        
        // Build path from connection's group hierarchy
        if let Some(group_id) = connection.group_id {
            let group_path = self.resolve_group_path(group_id, groups);
            path_parts.extend(group_path);
        }
        
        path_parts.push(connection.name.clone());
        path_parts.join("/")
    }
    
    /// Resolves full group path from group ID
    fn resolve_group_path(
        &self,
        group_id: Uuid,
        groups: &[ConnectionGroup],
    ) -> Vec<String> {
        let mut path = Vec::new();
        let mut current_id = Some(group_id);
        
        while let Some(id) = current_id {
            if let Some(group) = groups.iter().find(|g| g.id == id) {
                path.insert(0, group.name.clone());
                current_id = group.parent_id;
            } else {
                break;
            }
        }
        
        path
    }
    
    /// Saves password to KeePass with proper hierarchy
    pub fn save_password(
        &mut self,
        connection: &Connection,
        password: &SecretString,
        groups: &[ConnectionGroup],
    ) -> SecretResult<()> {
        let entry_path = self.build_entry_path(connection, groups);
        
        // Ensure all parent groups exist
        self.ensure_groups_exist(&entry_path)?;
        
        // Create or update entry
        self.upsert_entry(&entry_path, connection, password)
    }
    
    /// Ensures all groups in path exist, creating them if needed
    fn ensure_groups_exist(&mut self, path: &str) -> SecretResult<()> {
        let parts: Vec<&str> = path.split('/').collect();
        let mut current_path = String::new();
        
        // Skip the last part (entry name)
        for part in &parts[..parts.len() - 1] {
            if !current_path.is_empty() {
                current_path.push('/');
            }
            current_path.push_str(part);
            
            if !self.group_exists(&current_path) {
                self.create_group(&current_path)?;
            }
        }
        
        Ok(())
    }
}
```

### 3. Cluster Dialog Auto-Refresh

The issue is that callbacks don't trigger list refresh. Fix by adding explicit refresh:

```rust
impl ClusterListDialog {
    /// Refreshes the cluster list from state
    pub fn refresh_list(&self) {
        // Clear existing rows
        while let Some(row) = self.list_box.first_child() {
            self.list_box.remove(&row);
        }
        
        // Re-populate from state
        let clusters = self.state.borrow().get_all_clusters();
        for cluster in clusters {
            self.add_cluster_row(cluster);
        }
    }
    
    /// Sets up callbacks with auto-refresh
    pub fn setup_callbacks(&self) {
        let list_ref = self.clone();
        
        // After new cluster is saved
        self.set_on_new(move || {
            // ... create cluster logic ...
            list_ref.refresh_list(); // <-- Add refresh
        });
        
        // After cluster is deleted
        self.set_on_delete(move |cluster_id| {
            // ... delete cluster logic ...
            list_ref.refresh_list(); // <-- Add refresh
        });
        
        // After cluster is edited
        self.set_on_edit(move |cluster_id| {
            // ... edit cluster logic ...
            list_ref.refresh_list(); // <-- Add refresh
        });
    }
}
```

### 4. ZeroTrust Provider Icon Detection Fix

The issue is that the sidebar doesn't use the detected provider. Fix the icon selection:

```rust
/// Enhanced provider detection with instance ID patterns
pub fn detect_provider(command: &str) -> CloudProvider {
    let cmd_lower = command.to_lowercase();
    
    // AWS SSM patterns - including instance IDs
    if cmd_lower.contains("aws ssm")
        || cmd_lower.contains("aws-ssm")
        || cmd_lower.contains("ssm start-session")
        || cmd_lower.contains("--target i-")  // EC2 instance ID pattern
        || cmd_lower.contains("--target mi-") // Managed instance ID
    {
        return CloudProvider::Aws;
    }
    
    // GCP patterns
    if cmd_lower.contains("gcloud")
        || cmd_lower.contains("iap-tunnel")
        || cmd_lower.contains("compute ssh")
    {
        return CloudProvider::Gcloud;
    }
    
    // ... other providers
    
    CloudProvider::Generic
}

/// Sidebar icon selection for ZeroTrust connections
impl ConnectionSidebar {
    fn get_icon_for_connection(&self, connection: &Connection) -> &'static str {
        match &connection.protocol_config {
            ProtocolConfig::ZeroTrust(config) => {
                // Detect provider from command
                let provider = detect_provider(&config.command);
                provider.icon_name()
            }
            // ... other protocols
        }
    }
}
```

### 5. Qt/Wayland Error Handling

Handle Qt threading issues gracefully:

```rust
/// Wrapper for FreeRDP that handles Qt/Wayland errors
pub struct SafeFreeRdpLauncher {
    /// Whether to suppress Qt warnings
    suppress_qt_warnings: bool,
}

impl SafeFreeRdpLauncher {
    /// Launches FreeRDP with Qt error suppression
    pub fn launch(&self, config: &RdpConfig) -> Result<Child, EmbeddedRdpError> {
        let mut cmd = Command::new("xfreerdp");
        
        // Set environment to suppress Qt warnings
        cmd.env("QT_LOGGING_RULES", "qt.qpa.wayland=false");
        cmd.env("QT_QPA_PLATFORM", "xcb"); // Force X11 backend for FreeRDP
        
        // Add connection arguments
        cmd.args(self.build_args(config));
        
        // Redirect stderr to suppress warnings
        cmd.stderr(Stdio::null());
        
        cmd.spawn()
            .map_err(|e| EmbeddedRdpError::FreeRdpInit(e.to_string()))
    }
}
```

## Data Models

### KeePass Entry with Hierarchy

```rust
/// Extended KDBX entry with full path support
#[derive(Debug, Clone)]
pub struct HierarchicalKdbxEntry {
    /// Entry UUID
    pub uuid: Uuid,
    /// Entry title (connection name)
    pub title: String,
    /// Full path including groups (e.g., "RustConn/Servers/Web/myserver")
    pub path: String,
    /// Username
    pub username: Option<String>,
    /// Password (encrypted)
    pub password: Option<SecretString>,
    /// URL
    pub url: Option<String>,
    /// Notes
    pub notes: Option<String>,
    /// Connection ID reference
    pub connection_id: Option<Uuid>,
}
```

### Provider Detection Cache

```rust
/// Cached provider detection for connections
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZeroTrustConfig {
    // ... existing fields ...
    
    /// Cached detected provider (for consistent icon display)
    #[serde(default)]
    pub detected_provider: Option<String>,
}
```

## Correctness Properties

*A property is a characteristic or behavior that should hold true across all valid executions of a system-essentially, a formal statement about what the system should do. Properties serve as the bridge between human-readable specifications and machine-verifiable correctness guarantees.*

### Property 1: KeePass Entry Path Matches Connection Hierarchy
*For any* connection with a group assignment, the KeePass entry path should contain all ancestor group names in order from root to leaf.
**Validates: Requirements 3.2, 3.3**

### Property 2: KeePass Entry Creation Creates All Parent Groups
*For any* KeePass entry path with multiple levels, saving the entry should result in all parent groups existing in the database.
**Validates: Requirements 3.4**

### Property 3: Cluster List Refresh After Modification
*For any* cluster add/edit/delete operation, the cluster list should immediately reflect the change without requiring dialog close/reopen.
**Validates: Requirements 4.1, 4.2, 4.3**

### Property 4: AWS SSM Command Detection
*For any* command containing "aws ssm", "aws-ssm", or EC2 instance ID patterns (i-*), the provider detection should return AWS.
**Validates: Requirements 5.1**

### Property 5: GCloud Command Detection
*For any* command containing "gcloud" or "iap-tunnel", the provider detection should return Google Cloud.
**Validates: Requirements 5.2**

### Property 6: Provider Detection Persistence
*For any* ZeroTrust connection, the detected provider should be persisted and consistently displayed across application restarts.
**Validates: Requirements 5.5**

### Property 7: FreeRDP Error Isolation
*For any* Qt/Wayland error during RDP connection, the application should continue running without crash and fall back to external mode if needed.
**Validates: Requirements 6.1, 6.2, 6.4**

### Property 8: Embedded RDP Fallback
*For any* RDP connection where embedded mode fails, the system should automatically launch external xfreerdp with user notification.
**Validates: Requirements 1.2, 6.4**

### Property 9: Protocol Icons Are Distinct
*For any* two different protocol types (SSH, RDP, VNC, SPICE), the icon names returned should be different.
**Validates: Requirements 7.1, 7.2, 7.3, 7.4**

### Property 10: VNC Viewer Detection
*For any* system with at least one VNC viewer installed, the detection function should return a valid viewer path.
**Validates: Requirements 8.1, 8.3**

### Property 11: Drop Indicator Position
*For any* drag operation over a list row, the drop indicator should be positioned either above or below the row (not as a frame around it).
**Validates: Requirements 9.1, 9.2**

### Property 12: Template Protocol Persistence
*For any* template with a non-SSH protocol, saving and loading should preserve the protocol type and all protocol-specific settings.
**Validates: Requirements 10.1, 10.2, 10.3**

### 6. Protocol-Specific Icons

Fix sidebar to use distinct icons for each protocol:

```rust
impl ConnectionSidebar {
    fn get_icon_for_protocol(&self, protocol: &ProtocolConfig) -> &'static str {
        match protocol {
            ProtocolConfig::Ssh(_) => "utilities-terminal-symbolic",
            ProtocolConfig::Rdp(_) => "computer-symbolic",  // or custom RDP icon
            ProtocolConfig::Vnc(_) => "video-display-symbolic", // distinct from RDP
            ProtocolConfig::Spice(_) => "preferences-desktop-remote-desktop-symbolic",
            ProtocolConfig::ZeroTrust(config) => {
                detect_provider(&config.command).icon_name()
            }
        }
    }
}
```

### 7. VNC Connection Fix

The VNC connection opens empty tab because the viewer process isn't launched:

```rust
impl VncSession {
    pub fn connect(&mut self, config: &VncConfig) -> Result<(), VncError> {
        // Detect available VNC viewer
        let viewer = Self::detect_vnc_viewer()?;
        
        // Build command arguments
        let args = self.build_viewer_args(config, &viewer);
        
        // Launch viewer process
        let child = Command::new(&viewer)
            .args(&args)
            .spawn()
            .map_err(|e| VncError::LaunchFailed(e.to_string()))?;
        
        self.process = Some(child);
        self.state = VncConnectionState::Connected;
        
        Ok(())
    }
    
    fn detect_vnc_viewer() -> Result<String, VncError> {
        let viewers = ["vncviewer", "tigervnc", "gvncviewer", "vinagre"];
        for viewer in viewers {
            if Command::new("which").arg(viewer).status().is_ok() {
                return Ok(viewer.to_string());
            }
        }
        Err(VncError::NoViewerInstalled)
    }
}
```

### 8. Drag-and-Drop Line Indicator

Replace frame highlight with horizontal line:

```rust
impl ConnectionSidebar {
    fn setup_drop_indicator(&self) {
        // Create thin horizontal line instead of frame
        let indicator = gtk4::Separator::new(Orientation::Horizontal);
        indicator.add_css_class("drop-indicator");
        indicator.set_visible(false);
        
        // CSS: .drop-indicator { 
        //   background: @accent_color; 
        //   min-height: 2px; 
        //   margin: 0 8px;
        // }
        
        self.drop_indicator = Some(indicator);
    }
    
    fn update_drop_position(&self, y: f64, row: &ListBoxRow) {
        let row_height = row.height() as f64;
        let row_y = row.allocation().y() as f64;
        
        // Determine if drop is above or below center
        let relative_y = y - row_y;
        let position = if relative_y < row_height / 2.0 {
            DropPosition::Before
        } else {
            DropPosition::After
        };
        
        // Position the line indicator
        self.position_indicator(row, position);
    }
}
```

### 9. Template Creation Fix

Fix template dialog to support all protocols and persist correctly:

```rust
impl TemplateDialog {
    fn setup_protocol_selector(&self) {
        let protocols = ["SSH", "RDP", "VNC", "SPICE", "ZeroTrust"];
        
        for protocol in protocols {
            self.protocol_combo.append_text(protocol);
        }
        
        // Connect to selection change to show protocol-specific fields
        self.protocol_combo.connect_changed(|combo| {
            let protocol = combo.active_text().unwrap_or_default();
            self.show_protocol_fields(&protocol);
        });
    }
    
    fn save_template(&self) -> Result<ConnectionTemplate, TemplateError> {
        let name = self.name_entry.text().to_string();
        let protocol = self.protocol_combo.active_text()
            .ok_or(TemplateError::NoProtocolSelected)?;
        
        // Build protocol config based on selection
        let protocol_config = match protocol.as_str() {
            "SSH" => self.build_ssh_config(),
            "RDP" => self.build_rdp_config(),
            "VNC" => self.build_vnc_config(),
            "SPICE" => self.build_spice_config(),
            "ZeroTrust" => self.build_zerotrust_config(),
            _ => return Err(TemplateError::InvalidProtocol),
        };
        
        let template = ConnectionTemplate::new(name, protocol_config);
        
        // Save to config manager
        self.config_manager.save_template(&template)?;
        
        Ok(template)
    }
}
```

## Error Handling

```rust
#[derive(Debug, thiserror::Error)]
pub enum VncError {
    #[error("No VNC viewer installed. Please install vncviewer, tigervnc, or gvncviewer")]
    NoViewerInstalled,
    #[error("Failed to launch VNC viewer: {0}")]
    LaunchFailed(String),
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
}

#[derive(Debug, thiserror::Error)]
pub enum TemplateError {
    #[error("No protocol selected")]
    NoProtocolSelected,
    #[error("Invalid protocol type")]
    InvalidProtocol,
    #[error("Failed to save template: {0}")]
    SaveFailed(String),
}

#[derive(Debug, thiserror::Error)]
pub enum KeePassHierarchyError {
    #[error("Failed to create group '{0}': {1}")]
    GroupCreation(String, String),
    #[error("Failed to resolve group path for connection {0}")]
    PathResolution(Uuid),
    #[error("Database not unlocked")]
    DatabaseLocked,
}

#[derive(Debug, thiserror::Error)]
pub enum EmbeddedRdpError {
    #[error("Qt/Wayland threading error: {0}")]
    QtThreadingError(String),
    #[error("FreeRDP process failed: {0}")]
    ProcessFailed(String),
    #[error("Falling back to external mode: {0}")]
    FallbackToExternal(String),
}
```

## Testing Strategy

### Dual Testing Approach

1. **Unit Tests**: Verify specific examples and edge cases
2. **Property-Based Tests**: Verify universal properties using `proptest`

### Property-Based Testing

Using `proptest` crate with minimum 100 iterations per test:

```rust
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]
    
    /// **Feature: rustconn-fixes-v2, Property 1: KeePass Entry Path Matches Connection Hierarchy**
    #[test]
    fn prop_keepass_path_matches_hierarchy(
        group_names in prop::collection::vec("[a-zA-Z0-9_-]{1,20}", 1..5),
        connection_name in "[a-zA-Z0-9_-]{1,30}",
    ) {
        let groups = create_nested_groups(&group_names);
        let connection = create_connection_in_group(&connection_name, groups.last());
        
        let path = build_entry_path(&connection, &groups);
        
        // Path should start with "RustConn"
        prop_assert!(path.starts_with("RustConn/"));
        
        // Path should contain all group names in order
        for name in &group_names {
            prop_assert!(path.contains(name));
        }
        
        // Path should end with connection name
        prop_assert!(path.ends_with(&connection_name));
    }
    
    /// **Feature: rustconn-fixes-v2, Property 4: AWS SSM Command Detection**
    #[test]
    fn prop_aws_ssm_detection(
        instance_id in "i-[a-f0-9]{17}",
        region in "(us|eu|ap)-(east|west|central)-[1-3]",
    ) {
        let commands = vec![
            format!("aws ssm start-session --target {instance_id}"),
            format!("aws ssm start-session --target {instance_id} --region {region}"),
            format!("aws-ssm-plugin {instance_id}"),
        ];
        
        for cmd in commands {
            let provider = detect_provider(&cmd);
            prop_assert_eq!(provider, CloudProvider::Aws);
        }
    }
}
```

### Test Organization

- Property tests in `rustconn-core/tests/properties/`
- Unit tests co-located with source files
- Integration tests for GTK components (manual testing)

