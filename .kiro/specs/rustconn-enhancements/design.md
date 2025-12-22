# Design Document: RustConn Enhancements

## Overview

This design document describes the architecture and implementation approach for enhancing RustConn with automation capabilities, session management features, organization tools, and desktop integration. The design maintains RustConn's core principles: Rust-native, GTK4/Wayland-first, and security-conscious architecture.

## Architecture

The enhancements are organized into several subsystems that integrate with the existing `rustconn-core` and `rustconn` crates:

```
┌─────────────────────────────────────────────────────────────────┐
│                        rustconn (GUI)                           │
├─────────────────────────────────────────────────────────────────┤
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐             │
│  │ Tray Icon   │  │ Dashboard   │  │ Search UI   │             │
│  └─────────────┘  └─────────────┘  └─────────────┘             │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐             │
│  │ Cluster View│  │ External Win│  │ Variables UI│             │
│  └─────────────┘  └─────────────┘  └─────────────┘             │
├─────────────────────────────────────────────────────────────────┤
│                      rustconn-core                              │
├─────────────────────────────────────────────────────────────────┤
│  ┌─────────────────────────────────────────────────────────┐   │
│  │                    Automation Engine                     │   │
│  │  ┌───────────┐ ┌───────────┐ ┌───────────┐ ┌─────────┐ │   │
│  │  │ Variables │ │Key Sequence│ │  Expect   │ │  Tasks  │ │   │
│  │  └───────────┘ └───────────┘ └───────────┘ └─────────┘ │   │
│  └─────────────────────────────────────────────────────────┘   │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │                   Session Management                     │   │
│  │  ┌───────────┐ ┌───────────┐ ┌───────────┐             │   │
│  │  │  Cluster  │ │  Logging  │ │ Dashboard │             │   │
│  │  └───────────┘ └───────────┘ └───────────┘             │   │
│  └─────────────────────────────────────────────────────────┘   │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │                    Organization                          │   │
│  │  ┌───────────┐ ┌───────────┐ ┌───────────┐ ┌─────────┐ │   │
│  │  │ Documents │ │ Templates │ │Custom Props│ │ Search  │ │   │
│  │  └───────────┘ └───────────┘ └───────────┘ └─────────┘ │   │
│  └─────────────────────────────────────────────────────────┘   │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │                      Utilities                           │   │
│  │  ┌───────────┐                                          │   │
│  │  │    WOL    │                                          │   │
│  │  └───────────┘                                          │   │
│  └─────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
```

## Components and Interfaces

### 1. Variables System (`rustconn-core/src/variables/`)

```rust
/// A variable definition with optional secret flag
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Variable {
    pub name: String,
    pub value: String,
    pub is_secret: bool,
    pub description: Option<String>,
}

/// Variable scope for resolution
pub enum VariableScope {
    Global,
    Document(Uuid),
    Connection(Uuid),
}

/// Variable manager for resolution and substitution
pub struct VariableManager {
    global_vars: HashMap<String, Variable>,
    scoped_vars: HashMap<Uuid, HashMap<String, Variable>>,
}

impl VariableManager {
    /// Resolve a variable reference like ${var_name}
    pub fn resolve(&self, reference: &str, scope: VariableScope) -> Result<String, VariableError>;
    
    /// Substitute all variables in a string
    pub fn substitute(&self, input: &str, scope: VariableScope) -> Result<String, VariableError>;
    
    /// Parse and validate variable syntax
    pub fn parse_references(input: &str) -> Result<Vec<&str>, VariableError>;
    
    /// Detect circular references
    pub fn detect_cycles(&self) -> Result<(), VariableError>;
}
```

### 2. Key Sequence System (`rustconn-core/src/automation/key_sequence.rs`)

```rust
/// A key sequence element
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KeyElement {
    Text(String),
    SpecialKey(SpecialKey),
    Wait(u32), // milliseconds
    Variable(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SpecialKey {
    Enter, Tab, Escape, Backspace, Delete,
    Up, Down, Left, Right,
    F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12,
    CtrlC, CtrlD, CtrlZ,
}

/// Parsed key sequence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeySequence {
    pub elements: Vec<KeyElement>,
}

impl KeySequence {
    /// Parse a key sequence string like "user{TAB}pass{ENTER}{WAIT:1000}"
    pub fn parse(input: &str) -> Result<Self, KeySequenceError>;
    
    /// Serialize back to string format
    pub fn to_string(&self) -> String;
    
    /// Validate the sequence
    pub fn validate(&self) -> Result<(), KeySequenceError>;
}
```

### 3. Expect Automation (`rustconn-core/src/automation/expect.rs`)

```rust
/// An expect rule with pattern and response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectRule {
    pub id: Uuid,
    pub pattern: String,  // Regex pattern
    pub response: String, // Response to send (supports variables)
    pub priority: i32,
    pub timeout_ms: Option<u32>,
    pub enabled: bool,
}

/// Expect engine for pattern matching
pub struct ExpectEngine {
    rules: Vec<ExpectRule>,
    compiled_patterns: Vec<Regex>,
}

impl ExpectEngine {
    /// Check output against all rules, return matching rule
    pub fn match_output(&self, output: &str) -> Option<&ExpectRule>;
    
    /// Validate all regex patterns
    pub fn validate_patterns(&self) -> Result<(), ExpectError>;
}
```

### 4. Connection Tasks (`rustconn-core/src/automation/tasks.rs`)

```rust
/// Task execution timing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskTiming {
    PreConnect,
    PostDisconnect,
}

/// Task execution condition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskCondition {
    pub only_first_in_folder: bool,
    pub only_last_in_folder: bool,
}

/// A connection task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionTask {
    pub id: Uuid,
    pub timing: TaskTiming,
    pub command: String,
    pub condition: TaskCondition,
    pub timeout_ms: Option<u32>,
    pub abort_on_failure: bool,
}

/// Task executor
pub struct TaskExecutor {
    variable_manager: Arc<VariableManager>,
}

impl TaskExecutor {
    /// Execute a task with variable substitution
    pub async fn execute(&self, task: &ConnectionTask, scope: VariableScope) -> Result<i32, TaskError>;
}
```

### 5. Cluster Management (`rustconn-core/src/cluster/`)

```rust
/// A cluster of connections
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cluster {
    pub id: Uuid,
    pub name: String,
    pub connection_ids: Vec<Uuid>,
    pub broadcast_enabled: bool,
}

/// Active cluster session
pub struct ClusterSession {
    pub cluster_id: Uuid,
    pub sessions: HashMap<Uuid, SessionState>,
    pub broadcast_mode: bool,
}

impl ClusterSession {
    /// Send input to all sessions (broadcast mode)
    pub fn broadcast_input(&self, input: &str);
    
    /// Get status of all sessions
    pub fn get_status(&self) -> Vec<(Uuid, SessionStatus)>;
}
```

### 6. Session Logging (`rustconn-core/src/session/logger.rs`)

```rust
/// Log configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogConfig {
    pub enabled: bool,
    pub path_template: String,  // e.g., "~/logs/${connection_name}_${date}.log"
    pub timestamp_format: String,
    pub max_size_mb: u32,
    pub retention_days: u32,
}

/// Session logger
pub struct SessionLogger {
    config: LogConfig,
    file: Option<BufWriter<File>>,
    bytes_written: u64,
}

impl SessionLogger {
    /// Write output to log with timestamp
    pub fn write(&mut self, output: &[u8]) -> Result<(), LogError>;
    
    /// Rotate log if needed
    pub fn rotate_if_needed(&mut self) -> Result<(), LogError>;
    
    /// Flush and close
    pub fn close(&mut self) -> Result<(), LogError>;
}
```

### 7. Wake On LAN (`rustconn-core/src/wol/`)

```rust
/// MAC address for WOL
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MacAddress([u8; 6]);

impl MacAddress {
    /// Parse MAC address from string (supports : and - separators)
    pub fn parse(input: &str) -> Result<Self, WolError>;
    
    /// Format as string
    pub fn to_string(&self) -> String;
}

/// WOL configuration for a connection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WolConfig {
    pub mac_address: Option<MacAddress>,
    pub broadcast_address: Option<String>,
    pub port: u16,  // default 9
    pub wait_seconds: u32,
}

/// Send WOL magic packet
pub fn send_magic_packet(mac: &MacAddress, broadcast: &str, port: u16) -> Result<(), WolError>;
```

### 8. Documents (`rustconn-core/src/document/`)

```rust
/// A portable document containing connections
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub id: Uuid,
    pub name: String,
    pub connections: Vec<Connection>,
    pub groups: Vec<Group>,
    pub variables: HashMap<String, Variable>,
    pub templates: Vec<ConnectionTemplate>,
    pub created_at: DateTime<Utc>,
    pub modified_at: DateTime<Utc>,
}

/// Document manager
pub struct DocumentManager {
    documents: HashMap<Uuid, Document>,
    dirty_flags: HashMap<Uuid, bool>,
}

impl DocumentManager {
    /// Create new document
    pub fn create(&mut self, name: String) -> Uuid;
    
    /// Load document from file (with optional password)
    pub fn load(&mut self, path: &Path, password: Option<&str>) -> Result<Uuid, DocumentError>;
    
    /// Save document to file (with optional encryption)
    pub fn save(&self, id: Uuid, path: &Path, password: Option<&str>) -> Result<(), DocumentError>;
    
    /// Export document as portable file
    pub fn export(&self, id: Uuid, path: &Path) -> Result<(), DocumentError>;
}
```

### 9. Custom Properties (`rustconn-core/src/models/custom_property.rs`)

```rust
/// Custom property field type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PropertyType {
    Text,
    Url,
    Protected,  // Encrypted storage
}

/// A custom property
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomProperty {
    pub name: String,
    pub value: String,
    pub property_type: PropertyType,
}

/// Extension trait for Connection
impl Connection {
    pub fn get_custom_property(&self, name: &str) -> Option<&CustomProperty>;
    pub fn set_custom_property(&mut self, property: CustomProperty);
}
```

### 10. Search System (`rustconn-core/src/search/`)

```rust
/// Search query with operators
#[derive(Debug, Clone)]
pub struct SearchQuery {
    pub text: String,
    pub filters: Vec<SearchFilter>,
}

#[derive(Debug, Clone)]
pub enum SearchFilter {
    Protocol(ProtocolType),
    Tag(String),
    Group(Uuid),
    InCustomProperty(String),
}

/// Search result with relevance
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub connection_id: Uuid,
    pub score: f32,
    pub matched_fields: Vec<String>,
    pub highlights: Vec<(usize, usize)>,  // Start, end positions
}

/// Search engine
pub struct SearchEngine {
    index: SearchIndex,
}

impl SearchEngine {
    /// Parse search query with operators
    pub fn parse_query(input: &str) -> Result<SearchQuery, SearchError>;
    
    /// Execute search
    pub fn search(&self, query: &SearchQuery, connections: &[Connection]) -> Vec<SearchResult>;
    
    /// Fuzzy match score
    pub fn fuzzy_score(query: &str, target: &str) -> f32;
}
```

## Data Models

### Extended Connection Model

```rust
/// Enhanced connection with new fields
pub struct Connection {
    // ... existing fields ...
    
    // New fields
    pub pre_connect_task: Option<ConnectionTask>,
    pub post_disconnect_task: Option<ConnectionTask>,
    pub key_sequence: Option<KeySequence>,
    pub expect_rules: Vec<ExpectRule>,
    pub wol_config: Option<WolConfig>,
    pub local_variables: HashMap<String, Variable>,
    pub custom_properties: Vec<CustomProperty>,
    pub window_mode: WindowMode,
    pub log_config: Option<LogConfig>,
    pub template_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WindowMode {
    Embedded,
    External,
    Fullscreen,
}
```

### Session Statistics

```rust
/// Statistics for dashboard
#[derive(Debug, Clone)]
pub struct SessionStats {
    pub connection_id: Uuid,
    pub connected_at: DateTime<Utc>,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub status: SessionStatus,
}

#[derive(Debug, Clone)]
pub enum SessionStatus {
    Connecting,
    Connected,
    Disconnected,
    Error(String),
}
```

## Correctness Properties

*A property is a characteristic or behavior that should hold true across all valid executions of a system-essentially, a formal statement about what the system should do. Properties serve as the bridge between human-readable specifications and machine-verifiable correctness guarantees.*

### Property 1: Variable Substitution Consistency
*For any* string containing variable references and any variable scope, substituting variables and then checking for remaining unresolved references should yield either a fully resolved string or a list of undefined variables.
**Validates: Requirements 6.3, 6.6**

### Property 2: Variable Resolution with Override
*For any* variable name that exists in both global and local scope, resolution should return the local value when using connection scope.
**Validates: Requirements 6.2**

### Property 3: Nested Variable Resolution Depth
*For any* chain of variable references, resolution should terminate within the configured maximum depth and detect cycles.
**Validates: Requirements 6.4, 6.7**

### Property 4: Variable Serialization Round-Trip
*For any* valid variable definition, serializing to JSON and deserializing should produce an equivalent variable.
**Validates: Requirements 6.8**

### Property 5: Key Sequence Parse Round-Trip
*For any* valid key sequence string, parsing and then serializing should produce an equivalent string.
**Validates: Requirements 2.7**

### Property 6: Key Sequence Validation
*For any* key sequence string, parsing should either succeed with a valid KeySequence or fail with a descriptive error for malformed input.
**Validates: Requirements 2.6**

### Property 7: Key Sequence Variable Substitution
*For any* key sequence containing variable references, substitution should replace all valid references with their values.
**Validates: Requirements 2.4**

### Property 8: Expect Pattern Validation
*For any* expect rule pattern, validation should either confirm valid regex syntax or report the specific error.
**Validates: Requirements 4.6**

### Property 9: Expect Rule Serialization Round-Trip
*For any* valid expect rule, serializing and deserializing should produce an equivalent rule.
**Validates: Requirements 4.7**

### Property 10: Expect Pattern Matching Priority
*For any* terminal output that matches multiple expect patterns, the rule with highest priority should be selected.
**Validates: Requirements 4.3**

### Property 11: Task Variable Substitution
*For any* task command containing variable references, execution should use substituted values.
**Validates: Requirements 1.6**

### Property 12: Task Failure Handling
*For any* pre-connect task that returns non-zero exit code, the connection attempt should be aborted.
**Validates: Requirements 1.3**

### Property 13: Cluster Session Independence
*For any* cluster with multiple connections, failure of one session should not affect other sessions.
**Validates: Requirements 3.4**

### Property 14: MAC Address Parse Round-Trip
*For any* valid MAC address string, parsing and formatting should produce an equivalent string.
**Validates: Requirements 5.1**

### Property 15: WOL Magic Packet Format
*For any* MAC address, the generated magic packet should contain 6 bytes of 0xFF followed by 16 repetitions of the MAC address.
**Validates: Requirements 5.2**

### Property 16: Log Timestamp Formatting
*For any* log entry, the timestamp should be formatted according to the configured format string.
**Validates: Requirements 7.2**

### Property 17: Log Path Template Expansion
*For any* log path template with variables, expansion should produce a valid file path.
**Validates: Requirements 7.5**

### Property 18: Log Rotation Trigger
*For any* log file that exceeds the configured size limit, rotation should create a new file.
**Validates: Requirements 7.3**

### Property 19: Template Field Preservation
*For any* connection template, all field values should be preserved when serializing and deserializing.
**Validates: Requirements 8.5**

### Property 20: Template Application
*For any* template and new connection, applying the template should copy all template fields to the connection.
**Validates: Requirements 8.2**

### Property 21: Template Independence
*For any* connection created from a template, modifying the template should not affect the connection.
**Validates: Requirements 8.3**

### Property 22: Custom Property Type Preservation
*For any* custom property with a specific type, serialization and deserialization should preserve the type.
**Validates: Requirements 10.6**

### Property 23: Custom Property Search Inclusion
*For any* search query, connections with matching custom property values should be included in results.
**Validates: Requirements 10.5**

### Property 24: Document Serialization Round-Trip
*For any* valid document, serializing and deserializing should produce an equivalent document.
**Validates: Requirements 11.6**

### Property 25: Document Encryption Round-Trip
*For any* document and password, encrypting and decrypting should produce the original document.
**Validates: Requirements 11.3**

### Property 26: Document Dirty Tracking
*For any* document modification, the dirty flag should be set to true.
**Validates: Requirements 11.5**

### Property 27: Search Fuzzy Matching
*For any* search query and connection name, fuzzy matching should return a score between 0 and 1.
**Validates: Requirements 14.1**

### Property 28: Search Scope Coverage
*For any* search query, matching should check name, host, tags, group name, and custom properties.
**Validates: Requirements 14.2**

### Property 29: Search Operator Parsing
*For any* search string with operators like "protocol:ssh", parsing should extract the correct filter type and value.
**Validates: Requirements 14.4**

### Property 30: Search Result Ranking
*For any* search with multiple results, results should be ordered by descending relevance score.
**Validates: Requirements 14.3**

### Property 31: Dashboard Session Statistics
*For any* active session, the dashboard should display accurate duration and byte counts.
**Validates: Requirements 13.2**

### Property 32: Dashboard Filter Application
*For any* dashboard filter, only sessions matching the filter criteria should be displayed.
**Validates: Requirements 13.5**

### Property 33: Window Geometry Persistence
*For any* external window with "remember position" enabled, closing and reopening should restore the same geometry.
**Validates: Requirements 9.4**

## Error Handling

All new modules follow the existing error handling pattern using `thiserror`:

```rust
#[derive(Debug, thiserror::Error)]
pub enum VariableError {
    #[error("Undefined variable: {0}")]
    Undefined(String),
    #[error("Circular reference detected: {0}")]
    CircularReference(String),
    #[error("Invalid syntax: {0}")]
    InvalidSyntax(String),
    #[error("Maximum nesting depth exceeded")]
    MaxDepthExceeded,
}

#[derive(Debug, thiserror::Error)]
pub enum KeySequenceError {
    #[error("Invalid key sequence syntax: {0}")]
    InvalidSyntax(String),
    #[error("Unknown special key: {0}")]
    UnknownKey(String),
}

// Similar error types for other modules...
```

## Testing Strategy

### Dual Testing Approach

The implementation uses both unit tests and property-based tests:

1. **Unit Tests**: Verify specific examples, edge cases, and error conditions
2. **Property-Based Tests**: Verify universal properties using `proptest`

### Property-Based Testing Framework

Using `proptest` crate (already in workspace dependencies):

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn test_variable_round_trip(name in "[a-z_][a-z0-9_]*", value in ".*") {
        let var = Variable { name, value, is_secret: false, description: None };
        let json = serde_json::to_string(&var).unwrap();
        let parsed: Variable = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(var.name, parsed.name);
        prop_assert_eq!(var.value, parsed.value);
    }
}
```

### Test Organization

- Property tests in `rustconn-core/tests/property_tests/`
- Unit tests co-located with source files
- Integration tests in `rustconn-core/tests/`

### Clippy Compliance

All code must pass:
```bash
cargo clippy -p rustconn-core --all-targets -- -D warnings
cargo clippy -p rustconn --all-targets -- -D warnings
```

