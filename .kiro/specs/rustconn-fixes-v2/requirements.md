# Requirements Document

## Introduction

This document specifies requirements for fixing critical bugs discovered during user testing of RustConn v2. The issues include: embedded RDP/VNC not working (falls back to external window), KeePass password storage not functioning with proper hierarchy, cluster dialog not refreshing after changes, and ZeroTrust provider icons not displaying for AWS SSM and gcloud connections.

## Glossary

- **RustConn**: The GTK4-based connection manager application
- **Embedded Mode**: RDP/VNC sessions rendered inside the main application window
- **External Mode**: RDP/VNC sessions running in separate windows (xfreerdp/vncviewer)
- **wlfreerdp**: Wayland-native FreeRDP client for embedded RDP
- **KeePass/KDBX**: Password database format for secure credential storage
- **ZeroTrust**: Cloud provider CLI connections (AWS SSM, gcloud IAP, Azure CLI)
- **Cluster**: A group of connections for simultaneous command execution
- **Provider Icon**: Visual indicator showing cloud provider (AWS, GCP, Azure)

## Requirements

### Requirement 1: Embedded RDP via GTK4 Integration

**User Story:** As a user, I want RDP connections to be embedded in the main window like SSH terminals, so that I have a unified workspace without external windows.

#### Acceptance Criteria

1. WHEN a user opens an RDP connection with embedded mode enabled THEN the System SHALL render the remote desktop inside a GTK4 widget within the main window
2. WHEN wlfreerdp is not available THEN the System SHALL fall back to xfreerdp in external window mode with a notification to the user
3. WHEN the embedded RDP widget receives keyboard input THEN the System SHALL forward all key events to the remote session
4. WHEN the embedded RDP widget receives mouse input THEN the System SHALL forward mouse events including clicks, movement, and scroll
5. WHEN the embedded widget is resized THEN the System SHALL notify the remote session and update the display resolution
6. WHEN the RDP session disconnects THEN the System SHALL properly cleanup resources and display a disconnected state
7. WHEN Qt/Wayland errors occur (QSocketNotifier, requestActivate) THEN the System SHALL handle them gracefully without crashing

### Requirement 2: Embedded VNC via GTK4 Integration

**User Story:** As a user, I want VNC connections to be embedded in the main window, so that I have consistent experience across all protocols.

#### Acceptance Criteria

1. WHEN a user opens a VNC connection with embedded mode enabled THEN the System SHALL render the remote desktop inside a GTK4 widget
2. WHEN native VNC embedding is not available THEN the System SHALL fall back to external vncviewer with a notification
3. WHEN the embedded VNC widget receives input THEN the System SHALL forward keyboard and mouse events to the remote session
4. WHEN the VNC session disconnects THEN the System SHALL cleanup resources and show disconnected state

### Requirement 3: KeePass Password Storage with Hierarchy

**User Story:** As a user, I want my connection passwords to be stored in KeePass with the same folder structure as my connections, so that I can easily find and manage them.

#### Acceptance Criteria

1. WHEN a user saves a connection with a password and KeePass integration is enabled THEN the System SHALL create an entry in the KeePass database
2. WHEN creating a KeePass entry THEN the System SHALL place it in a folder hierarchy matching the connection's group path (e.g., "RustConn/GroupA/SubGroup/ConnectionName")
3. WHEN the connection's group changes THEN the System SHALL move the KeePass entry to the corresponding folder
4. WHEN a KeePass folder does not exist THEN the System SHALL create it automatically
5. WHEN loading a connection THEN the System SHALL retrieve the password from the corresponding KeePass entry
6. WHEN a connection is deleted THEN the System SHALL offer to delete the corresponding KeePass entry
7. WHEN serializing credentials THEN the System SHALL store only a reference to the KeePass entry, not the password itself

### Requirement 4: Cluster Dialog Auto-Refresh

**User Story:** As a user, I want the cluster list to update immediately when I add or delete clusters, so that I see current data without reopening the dialog.

#### Acceptance Criteria

1. WHEN a user creates a new cluster THEN the System SHALL immediately add it to the cluster list in the dialog
2. WHEN a user deletes a cluster THEN the System SHALL immediately remove it from the cluster list
3. WHEN a user edits a cluster THEN the System SHALL immediately update the cluster's display in the list
4. WHEN cluster operations complete THEN the System SHALL preserve the user's scroll position in the list

### Requirement 5: ZeroTrust Provider Icon Detection

**User Story:** As a user with AWS SSM and gcloud connections, I want to see the correct provider icons in the sidebar, so that I can quickly identify connection types.

#### Acceptance Criteria

1. WHEN displaying a ZeroTrust connection with "aws ssm" or "aws-ssm" in the command THEN the System SHALL show the AWS icon
2. WHEN displaying a ZeroTrust connection with "gcloud" in the command THEN the System SHALL show the Google Cloud icon
3. WHEN displaying a ZeroTrust connection with "az " or "azure" in the command THEN the System SHALL show the Azure icon
4. WHEN the provider cannot be detected THEN the System SHALL show a generic cloud icon
5. WHEN the connection protocol is stored THEN the System SHALL persist the detected provider for consistent display

### Requirement 6: Error Handling for Wayland/Qt Integration

**User Story:** As a user on Wayland, I want the application to handle Qt/Wayland compatibility issues gracefully, so that I can use RDP without crashes or errors.

#### Acceptance Criteria

1. WHEN QSocketNotifier errors occur THEN the System SHALL log the error and continue operation without crashing
2. WHEN Wayland requestActivate warnings occur THEN the System SHALL suppress or handle them gracefully
3. WHEN FreeRDP encounters threading issues THEN the System SHALL run FreeRDP operations in a dedicated thread
4. WHEN embedded mode fails THEN the System SHALL automatically fall back to external mode with user notification

### Requirement 7: Protocol-Specific Icons in Sidebar

**User Story:** As a user, I want to see different icons for RDP and VNC connections, so that I can quickly distinguish between protocol types.

#### Acceptance Criteria

1. WHEN displaying an RDP connection THEN the System SHALL show a distinct RDP icon (monitor with remote desktop symbol)
2. WHEN displaying a VNC connection THEN the System SHALL show a distinct VNC icon (different from RDP)
3. WHEN displaying an SSH connection THEN the System SHALL show a terminal icon
4. WHEN displaying a SPICE connection THEN the System SHALL show a SPICE-specific icon

### Requirement 8: VNC Connection Functionality

**User Story:** As a user, I want VNC connections to work properly, so that I can connect to VNC servers.

#### Acceptance Criteria

1. WHEN a user opens a VNC connection THEN the System SHALL launch the VNC viewer with correct parameters
2. WHEN VNC connection is opened THEN the System SHALL NOT display an empty tab
3. WHEN VNC viewer is not installed THEN the System SHALL display an error message with installation instructions
4. WHEN VNC connection fails THEN the System SHALL display the error reason to the user

### Requirement 9: Drag-and-Drop Visual Indicator

**User Story:** As a user, I want to see a clear line indicator when dragging connections, so that I can precisely position them.

#### Acceptance Criteria

1. WHEN dragging a connection over the sidebar THEN the System SHALL display a horizontal line indicator (not a frame/box)
2. WHEN the drag position changes THEN the System SHALL move the line indicator to show exact drop position
3. WHEN dragging over a group THEN the System SHALL highlight the group with a different visual style
4. WHEN the drag ends THEN the System SHALL remove all visual indicators immediately

### Requirement 10: Template Creation and Persistence

**User Story:** As a user, I want to create templates with all protocol types and have them saved correctly, so that I can quickly create similar connections.

#### Acceptance Criteria

1. WHEN creating a new template THEN the System SHALL allow selecting any protocol type (SSH, RDP, VNC, SPICE, ZeroTrust)
2. WHEN saving a template THEN the System SHALL persist all protocol-specific settings
3. WHEN loading templates THEN the System SHALL display all saved templates with correct protocol types
4. WHEN applying a template THEN the System SHALL copy all settings to the new connection
5. WHEN a template is saved THEN the System SHALL immediately show it in the template list

