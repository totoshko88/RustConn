# Design Document: Split View Redesign

## Overview

This design document describes the architecture for a complete redesign of RustConn's split view functionality. The new design introduces tab-scoped split layouts where each root tab maintains its own independent panel configuration, replacing the current global split view approach.

Key architectural changes:
- **Tab-scoped layouts**: Each tab owns its split configuration instead of a global split view
- **Tree-based panel structure**: Panels are organized in a binary tree supporting recursive nesting
- **Eviction mechanism**: Dropping on occupied panels preserves displaced connections
- **Color-coded containers**: Visual identification through unique Color IDs per split container
- **Clean crate separation**: Data models in `rustconn-core`, UI in `rustconn`

## Architecture

### High-Level Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                        rustconn (GUI)                           │
├─────────────────────────────────────────────────────────────────┤
│  ┌─────────────┐  ┌──────────────┐  ┌────────────────────────┐  │
│  │ TabManager  │  │ SplitViewUI  │  │ DragDropController     │  │
│  │ (adw::Tab   │  │ (gtk::Paned  │  │ (gtk::DragSource/      │  │
│  │  View)      │  │  tree)       │  │  DropTarget)           │  │
│  └──────┬──────┘  └──────┬───────┘  └───────────┬────────────┘  │
│         │                │                      │               │
│         └────────────────┼──────────────────────┘               │
│                          │                                      │
│                    ┌─────▼─────┐                                │
│                    │ SplitView │                                │
│                    │ Adapter   │                                │
│                    └─────┬─────┘                                │
└──────────────────────────┼──────────────────────────────────────┘
                           │
┌──────────────────────────┼──────────────────────────────────────┐
│                    rustconn-core                                │
├──────────────────────────┼──────────────────────────────────────┤
│                    ┌─────▼─────┐                                │
│                    │SplitLayout│                                │
│                    │  Model    │                                │
│                    └─────┬─────┘                                │
│         ┌────────────────┼────────────────┐                     │
│   ┌─────▼─────┐    ┌─────▼─────┐    ┌─────▼─────┐              │
│   │ PanelTree │    │ ColorPool │    │DropResult │              │
│   │           │    │           │    │           │              │
│   └───────────┘    └───────────┘    └───────────┘              │
└─────────────────────────────────────────────────────────────────┘
```

### Component Interaction Flow

```
User Action          GUI Layer                    Core Layer
─────────────────────────────────────────────────────────────────
Double-click    →   TabManager.create_tab()  →   (new Root_Tab)
sidebar item        

Split Vertical  →   SplitViewUI.split()      →   SplitLayoutModel
                                                  .split_panel()

Drag tab to     →   DragDropController       →   SplitLayoutModel
empty panel         .handle_drop()               .place_in_panel()

Drag tab to     →   DragDropController       →   SplitLayoutModel
occupied panel      .handle_drop()               .evict_and_place()
                                                  → DropResult::Evicted
```

## Components and Interfaces

### Core Layer (`rustconn-core`)

#### SplitLayoutModel

The central data model managing panel tree structure for a single tab.

```rust
/// Manages the split layout for a single tab
pub struct SplitLayoutModel {
    /// Root of the panel tree (None = single panel, no splits)
    root: Option<PanelNode>,
    /// Unique color ID for this split container
    color_id: Option<ColorId>,
    /// ID of the currently focused panel
    focused_panel: Option<PanelId>,
}

impl SplitLayoutModel {
    /// Creates a new layout with a single panel
    pub fn new() -> Self;
    
    /// Splits the focused panel in the given direction
    /// Returns the ID of the new panel, or error if no focused panel
    pub fn split(&mut self, direction: SplitDirection) -> Result<PanelId, SplitError>;
    
    /// Places a session in the specified panel
    /// Returns DropResult indicating what happened
    pub fn place_in_panel(
        &mut self, 
        panel_id: PanelId, 
        session_id: SessionId
    ) -> Result<DropResult, SplitError>;
    
    /// Removes a panel and redistributes space
    /// Returns the session that was in the panel (if any)
    pub fn remove_panel(&mut self, panel_id: PanelId) -> Result<Option<SessionId>, SplitError>;
    
    /// Returns true if this is a split container (has splits)
    pub fn is_split(&self) -> bool;
    
    /// Returns all panel IDs in the tree
    pub fn panel_ids(&self) -> Vec<PanelId>;
    
    /// Returns the session in a panel (if any)
    pub fn get_panel_session(&self, panel_id: PanelId) -> Option<SessionId>;
    
    /// Sets focus to a specific panel
    pub fn set_focus(&mut self, panel_id: PanelId) -> Result<(), SplitError>;
}
```

#### PanelNode

Binary tree node representing either a split or a leaf panel.

```rust
/// A node in the panel tree
pub enum PanelNode {
    /// A leaf panel that can contain a session
    Leaf(LeafPanel),
    /// A split containing two child nodes
    Split(SplitNode),
}

/// A leaf panel in the tree
pub struct LeafPanel {
    /// Unique identifier for this panel
    pub id: PanelId,
    /// Session currently displayed (None = empty panel)
    pub session: Option<SessionId>,
}

/// A split node containing two children
pub struct SplitNode {
    /// Split direction
    pub direction: SplitDirection,
    /// First child (start/top)
    pub first: Box<PanelNode>,
    /// Second child (end/bottom)
    pub second: Box<PanelNode>,
    /// Split position (0.0 to 1.0, default 0.5)
    pub position: f64,
}

/// Split direction
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitDirection {
    Horizontal,  // Top and bottom
    Vertical,    // Left and right
}
```

#### DropResult

Result of a drop operation, indicating what action the UI should take.

```rust
/// Result of placing a session in a panel
pub enum DropResult {
    /// Session was placed in an empty panel
    Placed,
    /// Session was placed, existing session was evicted
    Evicted {
        /// The session that was displaced
        evicted_session: SessionId,
    },
}
```

#### ColorPool

Manages color ID allocation for split containers.

```rust
/// Manages color allocation for split containers
pub struct ColorPool {
    /// Available colors (indices into a palette)
    available: Vec<ColorId>,
    /// Currently allocated colors
    allocated: HashSet<ColorId>,
}

impl ColorPool {
    /// Creates a new color pool with the standard palette
    pub fn new() -> Self;
    
    /// Allocates the next available color
    pub fn allocate(&mut self) -> ColorId;
    
    /// Returns a color to the pool
    pub fn release(&mut self, color: ColorId);
}

/// A color identifier (index into palette)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColorId(pub u8);
```

#### Error Types

```rust
#[derive(Debug, thiserror::Error)]
pub enum SplitError {
    #[error("no panel is currently focused")]
    NoFocusedPanel,
    
    #[error("panel not found: {0}")]
    PanelNotFound(PanelId),
    
    #[error("cannot remove the last panel")]
    CannotRemoveLastPanel,
    
    #[error("invalid split position: {0}")]
    InvalidPosition(f64),
}
```

### GUI Layer (`rustconn`)

#### SplitViewAdapter

Bridges the core model with GTK widgets.

```rust
/// Adapts SplitLayoutModel to GTK widgets
pub struct SplitViewAdapter {
    /// The underlying data model
    model: SplitLayoutModel,
    /// Root GTK container
    root_widget: gtk4::Box,
    /// Map of panel IDs to their GTK containers
    panel_widgets: HashMap<PanelId, gtk4::Box>,
    /// Paned widgets for splits
    paned_widgets: Vec<gtk4::Paned>,
}

impl SplitViewAdapter {
    /// Creates a new adapter with empty layout
    pub fn new() -> Self;
    
    /// Returns the root widget for embedding in UI
    pub fn widget(&self) -> &gtk4::Box;
    
    /// Splits the focused panel, updating both model and UI
    pub fn split(&mut self, direction: SplitDirection) -> Result<PanelId, SplitError>;
    
    /// Handles a drop operation on a panel
    pub fn handle_drop(
        &mut self,
        panel_id: PanelId,
        source: DropSource,
    ) -> Result<DropResult, SplitError>;
    
    /// Sets up drag-and-drop for a panel widget
    fn setup_drop_target(&self, panel_id: PanelId, widget: &gtk4::Box);
    
    /// Creates the empty panel placeholder widget
    fn create_empty_placeholder(&self, panel_id: PanelId) -> adw::StatusPage;
    
    /// Rebuilds the widget tree from the model
    fn rebuild_widgets(&mut self);
}
```

#### DropSource

Represents the source of a drag operation.

```rust
/// Source of a drag-and-drop operation
pub enum DropSource {
    /// A root tab being dragged
    RootTab { session_id: SessionId },
    /// A panel from a split container
    SplitPane { 
        source_tab_id: TabId,
        panel_id: PanelId,
        session_id: SessionId,
    },
    /// A sidebar connection item
    SidebarItem { connection_id: ConnectionId },
}
```

#### TabSplitManager

Manages the relationship between tabs and their split layouts.

```rust
/// Manages split layouts for all tabs
pub struct TabSplitManager {
    /// Map of tab IDs to their split adapters
    layouts: HashMap<TabId, SplitViewAdapter>,
    /// Global color pool shared across all tabs
    color_pool: Rc<RefCell<ColorPool>>,
}

impl TabSplitManager {
    /// Creates a new manager
    pub fn new() -> Self;
    
    /// Gets or creates a split adapter for a tab
    pub fn get_or_create(&mut self, tab_id: TabId) -> &mut SplitViewAdapter;
    
    /// Removes a tab's layout (when tab is closed)
    pub fn remove(&mut self, tab_id: TabId);
    
    /// Returns the color for a tab's split container (if split)
    pub fn get_tab_color(&self, tab_id: TabId) -> Option<ColorId>;
}
```

## Data Models

### Type Definitions

```rust
/// Unique identifier for a panel within a split layout
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PanelId(pub Uuid);

/// Unique identifier for a tab
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId(pub Uuid);

/// Unique identifier for a session
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionId(pub Uuid);

/// Unique identifier for a connection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectionId(pub Uuid);
```

### State Relationships

```
TabSplitManager
    │
    ├── Tab A (TabId) ──► SplitViewAdapter
    │                         │
    │                         └── SplitLayoutModel
    │                               │
    │                               ├── color_id: ColorId(0)
    │                               └── root: PanelNode::Split
    │                                     ├── first: Leaf(session_1)
    │                                     └── second: Leaf(empty)
    │
    └── Tab B (TabId) ──► SplitViewAdapter
                              │
                              └── SplitLayoutModel
                                    │
                                    └── root: None (single panel)
```

### Panel Tree Example

```
Initial state (single panel):
┌─────────────────────┐
│     Panel A         │
│   (session_1)       │
└─────────────────────┘

After vertical split:
┌──────────┬──────────┐
│ Panel A  │ Panel B  │
│(session_1)│ (empty)  │
└──────────┴──────────┘

Tree structure:
Split(Vertical)
├── Leaf(A, session_1)
└── Leaf(B, None)

After nested horizontal split on Panel B:
┌──────────┬──────────┐
│          │ Panel B  │
│ Panel A  │ (empty)  │
│(session_1)├──────────┤
│          │ Panel C  │
│          │ (empty)  │
└──────────┴──────────┘

Tree structure:
Split(Vertical)
├── Leaf(A, session_1)
└── Split(Horizontal)
    ├── Leaf(B, None)
    └── Leaf(C, None)
```

### Color Palette

```rust
/// Standard color palette for split containers
pub const SPLIT_COLORS: &[(u8, u8, u8)] = &[
    (0x35, 0x84, 0xe4),  // Blue
    (0x2e, 0xc2, 0x7e),  // Green  
    (0xff, 0x78, 0x00),  // Orange
    (0x91, 0x41, 0xac),  // Purple
    (0x00, 0xb4, 0xd8),  // Cyan
    (0xe0, 0x1b, 0x24),  // Red
];
```



## Correctness Properties

*A property is a characteristic or behavior that should hold true across all valid executions of a system—essentially, a formal statement about what the system should do. Properties serve as the bridge between human-readable specifications and machine-verifiable correctness guarantees.*

Based on the prework analysis of acceptance criteria, the following properties have been identified for property-based testing. These properties focus on the core data model in `rustconn-core` which can be tested without GTK dependencies.

### Property 1: Layout Independence

*For any* collection of tabs managed by `TabSplitManager`, operations on one tab's `SplitLayoutModel` (split, place, remove) SHALL NOT affect any other tab's layout state.

**Validates: Requirements 1.3, 1.4, 3.1, 3.3, 3.4**

### Property 2: Split Operation Invariants

*For any* `SplitLayoutModel` with a focused panel containing a session, after calling `split(direction)`:
- The layout SHALL contain exactly one more panel than before
- The original session SHALL be in the first child panel
- The second child panel SHALL be empty (no session)
- The split direction SHALL match the requested direction

**Validates: Requirements 2.1, 2.2, 2.3, 2.4**

### Property 3: Color Allocation Uniqueness

*For any* sequence of split container creations, each `ColorId` allocated by `ColorPool` SHALL be unique until released. When all colors are allocated, the pool SHALL cycle through colors but maintain allocation tracking.

**Validates: Requirements 2.5, 6.1**

### Property 4: Recursive Nesting Integrity

*For any* `SplitLayoutModel`, after a sequence of split operations:
- The panel tree SHALL maintain valid parent-child relationships
- All panel IDs returned by `panel_ids()` SHALL be reachable from the root
- The number of leaf panels SHALL equal the number of splits plus one
- Splitting any panel SHALL increase the total panel count by exactly one

**Validates: Requirements 5.1, 5.2, 5.3**

### Property 5: Empty Panel Placement

*For any* `SplitLayoutModel` with an empty panel, calling `place_in_panel(panel_id, session_id)` SHALL:
- Return `DropResult::Placed`
- Result in `get_panel_session(panel_id)` returning `Some(session_id)`
- Not affect any other panel's session state

**Validates: Requirements 9.1, 9.3, 9.4**

### Property 6: Occupied Panel Eviction

*For any* `SplitLayoutModel` with an occupied panel containing `old_session`, calling `place_in_panel(panel_id, new_session)` SHALL:
- Return `DropResult::Evicted { evicted_session: old_session }`
- Result in `get_panel_session(panel_id)` returning `Some(new_session)`
- The evicted session ID SHALL match the original occupant

**Validates: Requirements 10.1, 10.2, 10.3, 10.4**

### Property 7: Panel Removal and Tree Collapse

*For any* `SplitLayoutModel` with more than one panel:
- Removing a panel SHALL decrease the panel count by exactly one
- Removing a panel SHALL return the session that was in it (if any)
- After removal, all remaining panels SHALL still be accessible
- Removing the last panel SHALL return an error or result in an empty layout

*For any* nested split where one child is removed:
- The remaining child SHALL be promoted to replace the split node
- The tree depth SHALL decrease by one at that location

**Validates: Requirements 4.6, 5.4, 13.1, 13.2, 13.3, 13.5**

## Error Handling

### Core Layer Errors

All errors in `rustconn-core` use `thiserror` and return `Result<T, SplitError>`:

```rust
#[derive(Debug, thiserror::Error)]
pub enum SplitError {
    #[error("no panel is currently focused")]
    NoFocusedPanel,
    
    #[error("panel not found: {0}")]
    PanelNotFound(PanelId),
    
    #[error("cannot remove the last panel")]
    CannotRemoveLastPanel,
    
    #[error("invalid split position: {0} (must be between 0.0 and 1.0)")]
    InvalidPosition(f64),
    
    #[error("session not found: {0}")]
    SessionNotFound(SessionId),
}
```

### GUI Layer Error Handling

The GUI layer (`rustconn`) handles errors by:

1. **User-facing errors**: Display via `adw::Toast` with friendly messages
2. **Technical errors**: Log via `tracing` at appropriate levels
3. **Recovery**: Attempt graceful degradation where possible

```rust
// Example error handling in GUI
match adapter.handle_drop(panel_id, source) {
    Ok(DropResult::Placed) => {
        // Update UI to show session in panel
    }
    Ok(DropResult::Evicted { evicted_session }) => {
        // Create new tab for evicted session
        tab_manager.create_tab_for_session(evicted_session);
    }
    Err(SplitError::PanelNotFound(_)) => {
        tracing::warn!("Drop target panel no longer exists");
        toast_overlay.show("Drop target is no longer available");
    }
    Err(e) => {
        tracing::error!("Drop operation failed: {}", e);
        toast_overlay.show("Failed to move connection");
    }
}
```

### Edge Cases

| Scenario | Handling |
|----------|----------|
| Drop on self | Ignore (no-op) |
| Split with no focused panel | Return `SplitError::NoFocusedPanel` |
| Remove last panel | Return `SplitError::CannotRemoveLastPanel` |
| Invalid panel ID | Return `SplitError::PanelNotFound` |
| Color pool exhausted | Cycle through colors (wrap around) |

## Testing Strategy

### Dual Testing Approach

This feature requires both unit tests and property-based tests:

- **Unit tests**: Verify specific examples, edge cases, and error conditions
- **Property tests**: Verify universal properties across randomly generated inputs

### Property-Based Testing Configuration

- **Library**: `proptest` (already used in project)
- **Location**: `rustconn-core/tests/properties/split_view.rs`
- **Registration**: Add module to `rustconn-core/tests/properties/mod.rs`
- **Iterations**: Minimum 100 iterations per property test

Each property test must be tagged with a comment referencing the design property:

```rust
// Feature: split-view-redesign, Property 2: Split Operation Invariants
// Validates: Requirements 2.1, 2.2, 2.3, 2.4
proptest! {
    #[test]
    fn split_operation_invariants(
        initial_session in any::<Option<Uuid>>(),
        direction in prop_oneof![Just(SplitDirection::Horizontal), Just(SplitDirection::Vertical)]
    ) {
        // Test implementation
    }
}
```

### Test Generators

Custom generators needed for property tests:

```rust
/// Generates a valid SplitLayoutModel with random structure
fn arb_split_layout() -> impl Strategy<Value = SplitLayoutModel>;

/// Generates a sequence of split operations
fn arb_split_sequence(max_depth: usize) -> impl Strategy<Value = Vec<SplitDirection>>;

/// Generates a valid panel ID from an existing layout
fn arb_panel_id(layout: &SplitLayoutModel) -> impl Strategy<Value = PanelId>;
```

### Unit Test Coverage

Unit tests should cover:

1. **Basic operations**: Create, split, place, remove
2. **Edge cases**: Empty layout, single panel, maximum nesting
3. **Error conditions**: Invalid panel ID, remove last panel
4. **Color pool**: Allocation, release, wrap-around

### Integration Testing

Integration tests in `rustconn` (GUI crate) should verify:

1. Widget tree matches model state after operations
2. Drag-and-drop handlers invoke correct model methods
3. Tab creation/closure properly manages layouts
4. Color indicators update correctly

### Test File Structure

```
rustconn-core/
└── tests/
    └── properties/
        ├── mod.rs              # Add: mod split_view;
        └── split_view.rs       # New: property tests

rustconn-core/
└── src/
    └── split/
        ├── mod.rs
        ├── model.rs            # SplitLayoutModel + unit tests
        ├── tree.rs             # PanelNode + unit tests
        └── color.rs            # ColorPool + unit tests
```
