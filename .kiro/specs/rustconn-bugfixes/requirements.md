# Requirements Document

## Introduction

This document specifies requirements for bug fixes, UI improvements, and new features for RustConn connection manager. The changes address critical bugs reported by users, improve drag-and-drop UX, add native export/import format, enhance CLI connection feedback, and propose GTK4 upgrade with performance optimizations.

## Glossary

- **RustConn**: The GTK4-based connection manager application
- **Tray Icon**: System notification area icon for quick access
- **Drag-and-Drop**: UI interaction for reorganizing connections by dragging
- **ZeroTrust CLI**: Cloud provider CLI tools (AWS CLI, gcloud, Azure CLI) used for connections
- **SSH Agent**: System service managing SSH keys for authentication
- **Cluster**: A group of connections managed together
- **Template**: Predefined connection settings for quick creation
- **Session Logging**: Recording terminal output to files

## Requirements

### Requirement 1: Tray Icon Display

**User Story:** As a user, I want to see the RustConn logo in the system tray, so that I can easily identify the application.

#### Acceptance Criteria

1. WHEN the application starts with tray feature enabled THEN the System SHALL display the org.rustconn.RustConn.svg icon in the system tray
2. WHEN the tray icon is displayed THEN the System SHALL use the scalable SVG icon from rustconn/assets/icons/hicolor/scalable/apps/
3. IF the SVG icon fails to load THEN the System SHALL fall back to the 32x32 PNG icon

### Requirement 2: Drag-and-Drop Visual Feedback

**User Story:** As a user, I want to see a clear visual indicator when dragging connections, so that I can precisely position them in the tree.

#### Acceptance Criteria

1. WHEN a user drags a connection over the sidebar THEN the System SHALL display a horizontal line indicator between items showing the drop position
2. WHEN the drag position changes THEN the System SHALL update the line indicator position in real-time
3. WHEN a user drags over a group folder THEN the System SHALL highlight the folder to indicate drop-into-group action
4. WHEN the drag operation ends THEN the System SHALL remove all visual indicators immediately

### Requirement 3: GTK PopoverMenu Warning Fix

**User Story:** As a developer, I want to eliminate GTK warnings about broken widget state, so that the application runs cleanly without console errors.

#### Acceptance Criteria

1. WHEN context menus are displayed and dismissed THEN the System SHALL properly manage GtkPopoverMenu widget lifecycle
2. WHEN the application runs THEN the System SHALL produce zero GTK warnings related to "Broken accounting of active state"
3. WHEN popover menus are closed THEN the System SHALL ensure proper cleanup of widget references

### Requirement 4: ZeroTrust CLI Provider Icons

**User Story:** As a user with cloud CLI connections, I want to see provider logos for my connections, so that I can quickly identify AWS, GCloud, or Azure connections.

#### Acceptance Criteria

1. WHEN ZeroTrust CLI providers are detected THEN the System SHALL cache their logo icons locally
2. WHEN displaying a ZeroTrust connection THEN the System SHALL show the corresponding provider icon (AWS, GCloud, Azure)
3. WHEN a provider icon is not available THEN the System SHALL display a generic cloud icon
4. WHEN icons are cached THEN the System SHALL store them in the user's cache directory (~/.cache/rustconn/icons/)

### Requirement 5: CLI Connection Output Feedback

**User Story:** As a user, I want to see clear feedback when connecting via CLI, so that I understand what is happening and can copy commands if needed.

#### Acceptance Criteria

1. WHEN a CLI or ZeroTrust connection starts THEN the System SHALL display an English message with emoji indicating connection type and target host
2. WHEN a command is executed THEN the System SHALL echo the exact command being run to the terminal
3. WHEN displaying connection info THEN the System SHALL use format: "🔗 Connecting via [protocol] to [host]..."
4. WHEN displaying command THEN the System SHALL use format: "⚡ Executing: [command]"

### Requirement 6: SSH IdentitiesOnly Option

**User Story:** As a user, I want to specify that only my selected SSH key should be used, so that I avoid "Too many authentication failures" errors.

#### Acceptance Criteria

1. WHEN a user configures an SSH connection THEN the System SHALL provide a checkbox for "Use only specified key (IdentitiesOnly)"
2. WHEN IdentitiesOnly is enabled THEN the System SHALL add "-o IdentitiesOnly=yes" to the SSH command
3. WHEN a specific key is selected with IdentitiesOnly THEN the System SHALL prevent SSH from trying other keys from the agent
4. WHEN serializing SSH config THEN the System SHALL preserve the IdentitiesOnly setting

### Requirement 7: Connection Tree State Preservation

**User Story:** As a user, I want the connection tree to maintain its state when I edit connections, so that I don't lose my place in the hierarchy.

#### Acceptance Criteria

1. WHEN a user edits a connection THEN the System SHALL preserve the expanded/collapsed state of all groups
2. WHEN a user edits a connection THEN the System SHALL preserve the scroll position in the sidebar
3. WHEN a connection is saved THEN the System SHALL restore selection to the edited connection
4. WHEN the tree is refreshed THEN the System SHALL NOT reset expansion state unless explicitly requested by user

### Requirement 8: SSH Agent Key Selection Persistence

**User Story:** As a user, I want my SSH key selection to be saved, so that I don't have to reselect it every time I edit a connection.

#### Acceptance Criteria

1. WHEN a user selects an SSH key via ssh-agent THEN the System SHALL persist the key fingerprint in the connection configuration
2. WHEN loading a connection with a saved key THEN the System SHALL restore the key selection in the dialog
3. WHEN the selected key is no longer available in ssh-agent THEN the System SHALL display a warning and allow reselection
4. WHEN serializing connection THEN the System SHALL preserve the ssh_agent_key_fingerprint field

### Requirement 9: Session Logging Fix

**User Story:** As a user, I want session logging to actually write log files, so that I can review session history.

#### Acceptance Criteria

1. WHEN logging is enabled in Settings THEN the System SHALL create log files in the configured directory
2. WHEN a session produces output THEN the System SHALL write the output to the log file with timestamps
3. WHEN the default log directory does not exist THEN the System SHALL create it automatically
4. WHEN logging fails THEN the System SHALL display an error notification to the user

### Requirement 10: Cluster Save Fix

**User Story:** As a user, I want to save new clusters, so that I can manage groups of connections together.

#### Acceptance Criteria

1. WHEN a user clicks "New Cluster" and fills the form THEN the System SHALL save the cluster to configuration
2. WHEN a cluster is saved THEN the System SHALL persist it across application restarts
3. WHEN the cluster list is refreshed THEN the System SHALL display all saved clusters
4. WHEN saving fails THEN the System SHALL display an error message with details

### Requirement 11: Template Creation Fix

**User Story:** As a user, I want to create new templates, so that I can quickly create similar connections.

#### Acceptance Criteria

1. WHEN a user clicks "New Template" button THEN the System SHALL open the template creation dialog
2. WHEN the template dialog opens THEN the System SHALL allow entering all template fields
3. WHEN a template is saved THEN the System SHALL persist it to configuration
4. WHEN the button is clicked THEN the System SHALL respond immediately without requiring additional actions

### Requirement 12: Menu Copy/Paste Fix

**User Story:** As a user, I want Copy/Paste to work for duplicating connections, so that I can quickly create similar connections.

#### Acceptance Criteria

1. WHEN Copy is selected with a connection highlighted THEN the System SHALL store the connection data in internal clipboard
2. WHEN Paste is selected after Copy THEN the System SHALL create a duplicate connection with "(Copy)" suffix
3. WHEN Paste is selected THEN the System SHALL add the duplicated connection to the same group as the original
4. WHEN no connection is selected THEN the System SHALL disable Copy menu item
5. WHEN no connection is in clipboard THEN the System SHALL disable Paste menu item
6. WHEN Copy/Paste completes THEN the System SHALL refresh the sidebar to show the new connection

### Requirement 13: Native RustConn Export/Import Format

**User Story:** As a user, I want to export and import connections in RustConn's native format, so that I can backup and share my complete configuration.

#### Acceptance Criteria

1. WHEN a user exports connections THEN the System SHALL offer "RustConn Native (.rcn)" as an export format option
2. WHEN exporting to native format THEN the System SHALL include all connection fields, groups, templates, clusters, and variables
3. WHEN importing from native format THEN the System SHALL restore all data including groups hierarchy and metadata
4. WHEN the native format is used THEN the System SHALL use JSON with schema version for forward compatibility
5. WHEN importing THEN the System SHALL validate schema version and handle migrations if needed
6. WHEN serializing to native format THEN the System SHALL preserve all data for round-trip consistency

### Requirement 14: GTK4 Upgrade to 4.20

**User Story:** As a developer, I want to upgrade to GTK 4.20, so that I can use the latest features and improvements.

#### Acceptance Criteria

1. WHEN building the application THEN the System SHALL compile against GTK 4.20 or later
2. WHEN using GTK features THEN the System SHALL use modern APIs and deprecate old patterns
3. WHEN the upgrade is complete THEN the System SHALL pass all existing tests
4. WHEN new GTK features are available THEN the System SHALL document potential UI improvements

### Requirement 15: Performance Optimization

**User Story:** As a user, I want the application to be responsive and efficient, so that I can work without delays.

#### Acceptance Criteria

1. WHEN the application starts THEN the System SHALL complete initialization within 2 seconds on standard hardware
2. WHEN loading large connection lists (100+ items) THEN the System SHALL render the sidebar within 500ms
3. WHEN searching connections THEN the System SHALL display results within 100ms of typing
4. WHEN memory usage is measured THEN the System SHALL use less than 100MB for typical usage (50 connections)

### Requirement 16: Embedded RDP/VNC via Wayland Subsurface

**User Story:** As a user, I want RDP and VNC connections to be embedded in the main window like SSH, so that I have a consistent experience across all protocols.

#### Acceptance Criteria

1. WHEN a user opens an RDP connection in embedded mode THEN the System SHALL render the remote desktop inside a GTK widget using wlfreerdp
2. WHEN a user opens a VNC connection in embedded mode THEN the System SHALL render the remote desktop inside a GTK widget
3. WHEN rendering RDP/VNC THEN the System SHALL use Wayland wl_subsurface for native compositor integration
4. WHEN FreeRDP renders a frame THEN the System SHALL capture the pixel buffer via EndPaint callback and blit to wl_buffer
5. WHEN the RDP/VNC session is active THEN the System SHALL forward keyboard and mouse input to the remote session
6. WHEN the remote session disconnects THEN the System SHALL properly cleanup the subsurface and buffers
7. WHEN the embedded widget is resized THEN the System SHALL notify the remote session of resolution change
8. IF wlfreerdp is not available THEN the System SHALL fall back to external window mode with xfreerdp

