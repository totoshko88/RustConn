# Requirements Document

## Introduction

This document specifies the requirements for a complete redesign of the split view functionality in RustConn. The new design introduces tab-scoped split layouts where each root tab maintains its own independent panel configuration, replacing the current global split view approach. Key features include color-coded split containers for visual identification, empty panel placeholders with drag-drop targets, an eviction mechanism for occupied panels, and support for recursive nesting of splits.

## Glossary

- **Root_Tab**: A top-level tab in the main tab bar that represents either a single connection or a Split_Container
- **Split_Container**: A tab that has been transformed via split commands to contain multiple Panel instances with their own layout
- **Panel**: A rectangular area within a Split_Container that can display a connection session or remain empty
- **Empty_Panel**: A Panel with no active connection, displaying a "Drag Tab Here" placeholder
- **Occupied_Panel**: A Panel currently displaying an active connection session
- **Color_ID**: A unique color identifier assigned to each Split_Container for visual distinction
- **Eviction**: The process of moving an existing connection from an Occupied_Panel to a new Root_Tab when replaced by another connection
- **Drop_Target**: A UI element that accepts drag-and-drop operations
- **Sidebar_Item**: An entry in the connection sidebar representing a server/connection

## Requirements

### Requirement 1: Connection Instantiation

**User Story:** As a user, I want new connections to always open in their own independent tab, so that I can manage connections without affecting existing split layouts.

#### Acceptance Criteria

1. WHEN a user initiates a new connection via the main menu, THE System SHALL create a new Root_Tab for that connection
2. WHEN a user double-clicks a Sidebar_Item, THE System SHALL create a new Root_Tab for that connection
3. WHEN a new Root_Tab is created, THE System SHALL initialize it as independent and not part of any Split_Container
4. THE System SHALL ensure each newly created tab operates independently from existing Split_Containers

### Requirement 2: Split View Initialization

**User Story:** As a user, I want to split my current tab into multiple panels, so that I can view multiple connections simultaneously within the same tab.

#### Acceptance Criteria

1. WHEN a user invokes "Split Vertical" on a Root_Tab, THE System SHALL transform that tab into a Split_Container with two vertical panels
2. WHEN a user invokes "Split Horizontal" on a Root_Tab, THE System SHALL transform that tab into a Split_Container with two horizontal panels
3. WHEN a split is performed, THE System SHALL place the original connection in the first panel
4. WHEN a split is performed, THE System SHALL create an Empty_Panel in the second position
5. THE System SHALL assign a unique Color_ID to each Split_Container upon creation
6. THE System SHALL display the Color_ID indicator in both the tab header and panel borders

### Requirement 3: Tab-Scoped Split Layouts

**User Story:** As a user, I want each tab to maintain its own split layout independently, so that I can organize different workspaces without interference.

#### Acceptance Criteria

1. THE System SHALL maintain separate panel configurations for each Root_Tab
2. WHEN switching between tabs, THE System SHALL preserve and restore each tab's unique split layout
3. THE System SHALL support N independent tabs, each with its own panel configuration
4. WHEN a Split_Container tab is closed, THE System SHALL clean up only that tab's panel configuration

### Requirement 4: Empty Panel State

**User Story:** As a user, I want empty panels to clearly indicate they are ready to receive connections, so that I can easily understand where to drop new content.

#### Acceptance Criteria

1. WHEN an Empty_Panel is created, THE System SHALL display a "Drag Tab Here" placeholder text
2. THE Empty_Panel SHALL NOT accept keyboard focus
3. THE Empty_Panel SHALL only respond to drop operations
4. THE Empty_Panel SHALL provide visual feedback when a draggable item hovers over it
5. THE Empty_Panel SHALL display a close button (X icon) in the top-right corner
6. WHEN the close button on an Empty_Panel is clicked, THE System SHALL remove that panel from the Split_Container

### Requirement 5: Recursive Nesting

**User Story:** As a user, I want to split panels within an existing split layout, so that I can create complex multi-panel arrangements.

#### Acceptance Criteria

1. WHEN a user invokes split on a Panel within a Split_Container, THE System SHALL create a nested split within that panel
2. THE System SHALL support arbitrary depth of nested splits within a single tab
3. THE System SHALL maintain proper parent-child relationships for nested Panel structures
4. WHEN a nested Panel is closed, THE System SHALL properly collapse the split hierarchy

### Requirement 6: Visual Identification

**User Story:** As a user, I want split containers to be visually distinct, so that I can easily identify which panels belong to which tab.

#### Acceptance Criteria

1. WHEN a Split_Container is created, THE System SHALL assign it a unique Color_ID from a predefined palette
2. THE System SHALL display the Color_ID as an indicator in the tab header
3. THE System SHALL paint panel borders within the Split_Container using the assigned Color_ID
4. THE System SHALL ensure Color_IDs are visually distinct and accessible in both light and dark themes

### Requirement 7: Drag-and-Drop Sources

**User Story:** As a user, I want to drag connections from multiple sources, so that I can flexibly arrange my workspace.

#### Acceptance Criteria

1. THE System SHALL support dragging Root_Tabs as drag sources
2. THE System SHALL support dragging Panels from any Split_Container as drag sources
3. THE System SHALL support dragging Sidebar_Items as drag sources
4. WHEN a drag operation begins, THE System SHALL provide visual feedback indicating the item being dragged

### Requirement 8: Drop Target Visualization

**User Story:** As a user, I want clear visual feedback when dragging over drop targets, so that I know where my connection will be placed.

#### Acceptance Criteria

1. WHEN a draggable item enters a valid Drop_Target, THE System SHALL highlight the target zone with a focus border
2. WHEN a draggable item leaves a Drop_Target, THE System SHALL remove the highlight
3. THE System SHALL clearly distinguish between Empty_Panel and Occupied_Panel drop targets

### Requirement 9: Drop on Empty Panel

**User Story:** As a user, I want to drop connections onto empty panels, so that I can populate my split layout.

#### Acceptance Criteria

1. WHEN a Root_Tab is dropped on an Empty_Panel, THE System SHALL move the connection to that panel
2. WHEN a Root_Tab is dropped on an Empty_Panel, THE System SHALL remove the source tab from the tab bar
3. WHEN a Panel from another Split_Container is dropped on an Empty_Panel, THE System SHALL move the connection to the target panel
4. WHEN a Sidebar_Item is dropped on an Empty_Panel, THE System SHALL create a new connection in that panel

### Requirement 10: Drop on Occupied Panel (Eviction)

**User Story:** As a user, I want to replace connections in occupied panels while preserving the displaced connection, so that I don't lose active sessions.

#### Acceptance Criteria

1. WHEN a Root_Tab is dropped on an Occupied_Panel, THE System SHALL move the new connection into the panel
2. WHEN a Root_Tab is dropped on an Occupied_Panel, THE System SHALL evict the existing connection to a new Root_Tab
3. WHEN a Panel is dropped on an Occupied_Panel, THE System SHALL swap the connections and evict the displaced one
4. WHEN a Sidebar_Item is dropped on an Occupied_Panel, THE System SHALL create a new connection in the panel and evict the existing one
5. THE System SHALL ensure evicted connections remain fully functional in their new Root_Tab

### Requirement 11: Architecture Separation

**User Story:** As a developer, I want clear separation between UI and business logic, so that the codebase remains maintainable and testable.

#### Acceptance Criteria

1. THE System SHALL implement all split layout data models in the rustconn-core crate
2. THE System SHALL implement all GTK4/libadwaita UI code in the rustconn crate
3. THE rustconn-core crate SHALL NOT import gtk4, vte4, or adw dependencies
4. THE System SHALL use trait abstractions to decouple UI from business logic

### Requirement 12: GNOME HIG Compliance

**User Story:** As a user, I want the split view to follow GNOME design guidelines, so that it feels native and consistent with other applications.

#### Acceptance Criteria

1. THE System SHALL use libadwaita widgets where equivalent gtk4 widgets exist
2. THE System SHALL follow GNOME HIG spacing guidelines (12px margins, 6px between related elements)
3. THE System SHALL ensure all interactive elements are keyboard accessible
4. THE System SHALL support both light and dark themes consistently

### Requirement 13: Panel Lifecycle and Cleanup

**User Story:** As a user, I want panels to automatically close when their connection ends, and the main tab to close when empty, to keep my workspace clean.

#### Acceptance Criteria

1. WHEN a connection in an Occupied_Panel is closed (via context menu or server disconnect), THE System SHALL remove that Panel from the Split_Container
2. WHEN a Panel is removed, THE System SHALL automatically redistribute the available space to adjacent Panels
3. WHEN the last remaining Panel in a Split_Container is closed, THE System SHALL close the parent Root_Tab
4. THE System SHALL provide a context menu on Occupied_Panels with options: "Close Connection" and "Move to New Tab"
5. WHEN "Move to New Tab" is selected, THE System SHALL move the connection to a new Root_Tab and remove the original Panel
