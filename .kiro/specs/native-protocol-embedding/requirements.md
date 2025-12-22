# Requirements Document

## Introduction

This specification covers the implementation of native embedded RDP protocol support using IronRDP, along with improvements to credential management, connection naming, SSH key handling, external mode enhancements, and fixes for discovered issues. The goal is to provide true embedded RDP rendering within GTK4 widgets (similar to how VNC uses vnc-rs), while also improving the overall user experience for credential handling and connection management.

## Glossary

- **RustConn**: The connection manager application
- **IronRDP**: A pure Rust RDP client library for native protocol implementation
- **FreeRDP**: External RDP client used as fallback (wlfreerdp/xfreerdp)
- **Embedded Mode**: RDP session rendered directly in GTK DrawingArea widget
- **External Mode**: RDP session running in separate FreeRDP window
- **KeePass**: Password manager database (KDBX format)
- **Keyring**: System credential storage (libsecret on Linux)
- **Credential Backend**: Storage system for passwords (KeePass or Keyring)
- **VNC Client**: Reference implementation using vnc-rs for native embedding
- **DrawingArea**: GTK4 widget for custom rendering
- **Ctrl+Alt+Del**: Special key combination for Windows security screen
- **Quick Connect**: Feature to quickly connect without creating a saved connection
- **Wayland Subsurface**: Wayland protocol for embedding surfaces within other surfaces
- **SPICE**: Simple Protocol for Independent Computing Environments (virtualization display protocol)

## Requirements

### Requirement 1: Native Embedded RDP

**User Story:** As a user, I want RDP sessions to be rendered directly within the RustConn window, so that I can manage multiple RDP connections in a unified interface without separate windows.

#### Acceptance Criteria

1. WHEN a user opens an RDP connection in embedded mode THEN the RustConn System SHALL render the RDP session directly in a GTK DrawingArea widget using IronRDP
2. WHEN the user moves the mouse over the embedded RDP widget THEN the RustConn System SHALL forward mouse coordinates to the RDP server via IronRDP protocol
3. WHEN the user types on the keyboard while the embedded RDP widget has focus THEN the RustConn System SHALL forward key events to the RDP server via IronRDP protocol
4. WHEN the user clicks the Ctrl+Alt+Del button in the RDP toolbar THEN the RustConn System SHALL send the Ctrl+Alt+Del key sequence to the RDP server
5. WHEN IronRDP connection fails or is unavailable THEN the RustConn System SHALL fall back to FreeRDP external mode and notify the user
6. WHEN the user disconnects from an RDP session THEN the RustConn System SHALL properly clean up IronRDP resources and release memory
7. WHEN the embedded RDP widget is resized THEN the RustConn System SHALL request dynamic resolution change from the RDP server

### Requirement 2: Smart Credential Dialog

**User Story:** As a user, I want the application to use my saved credentials automatically, so that I don't have to enter passwords repeatedly for connections where credentials are already stored and have been used successfully.

#### Acceptance Criteria

1. WHEN a user connects to a server with credentials saved in KeePass or Keyring that were previously used successfully THEN the RustConn System SHALL use those credentials without showing the password dialog
2. WHEN a user connects to a server without saved credentials THEN the RustConn System SHALL show the password dialog
3. WHEN a saved credential fails authentication THEN the RustConn System SHALL show the password dialog with an error message and mark credentials as requiring verification
4. WHEN the password dialog is shown THEN the RustConn System SHALL pre-fill username and domain from saved connection settings
5. WHEN credentials are used successfully THEN the RustConn System SHALL mark them as verified for future automatic use

### Requirement 3: Unified Credential Storage

**User Story:** As a user, I want a simple "Save Credentials" option that automatically chooses the best storage backend, so that I don't have to understand the technical differences between KeePass and Keyring.

#### Acceptance Criteria

1. WHEN the user checks "Save Credentials" in the password dialog THEN the RustConn System SHALL store credentials to KeePass if KeePass integration is enabled, otherwise to Keyring
2. WHEN KeePass integration is enabled and credentials exist in Keyring but not in KeePass THEN the RustConn System SHALL use the Keyring credentials
3. WHEN KeePass integration is enabled and credentials exist in Keyring THEN the RustConn System SHALL display a "Save to KeePass" button in the connection settings
4. WHEN the user clicks "Save to KeePass" THEN the RustConn System SHALL copy credentials from Keyring to KeePass and remove them from Keyring
5. WHEN the system Keyring is unavailable THEN the RustConn System SHALL display a warning and disable Keyring-related options
6. WHEN checking Keyring availability THEN the RustConn System SHALL verify that libsecret service is running and accessible

### Requirement 4: Connection Naming

**User Story:** As a user, I want connections with duplicate names to be automatically distinguished, so that I can identify them easily in the connection list.

#### Acceptance Criteria

1. WHEN a user creates a connection with a name that already exists THEN the RustConn System SHALL append the protocol suffix in parentheses (e.g., "server (RDP)")
2. WHEN multiple connections have the same name and protocol THEN the RustConn System SHALL append a numeric suffix (e.g., "server (RDP) 2")
3. WHEN the user renames a connection to a unique name THEN the RustConn System SHALL remove any auto-generated suffix

### Requirement 5: SSH Key Selection

**User Story:** As a user, I want SSH connections to use only my selected key file, so that I don't get "Too many authentication failures" errors from trying multiple keys.

#### Acceptance Criteria

1. WHEN a user selects "File" authentication method with a specific key file THEN the RustConn System SHALL pass only that key using the `-i` flag to SSH
2. WHEN a user selects "File" authentication method THEN the RustConn System SHALL add `-o IdentitiesOnly=yes` to prevent SSH from trying other keys
3. WHEN a user selects "SSH Agent" authentication method THEN the RustConn System SHALL allow SSH to use all keys from the agent

### Requirement 6: External Mode Improvements

**User Story:** As a user, I want external RDP windows to have proper window decorations and remember their position, so that I can organize my workspace efficiently.

#### Acceptance Criteria

1. WHEN launching FreeRDP in external mode THEN the RustConn System SHALL include the `/decorations` flag to enable window decorations
2. WHEN the user closes an external RDP session THEN the RustConn System SHALL save the window position and size
3. WHEN the user opens the same connection again THEN the RustConn System SHALL restore the saved window position and size
4. WHEN the "Remember window position" option is disabled THEN the RustConn System SHALL use default window placement

### Requirement 7: Quick Connect Implementation

**User Story:** As a user, I want to quickly connect to RDP and VNC servers without creating a saved connection, so that I can access servers for one-time tasks efficiently.

#### Acceptance Criteria

1. WHEN a user enters a hostname in Quick Connect and selects RDP protocol THEN the RustConn System SHALL establish an RDP connection to that host
2. WHEN a user enters a hostname in Quick Connect and selects VNC protocol THEN the RustConn System SHALL establish a VNC connection to that host
3. WHEN Quick Connect requires authentication THEN the RustConn System SHALL show the password dialog
4. WHEN Quick Connect session ends THEN the RustConn System SHALL not save the connection to the connection list

### Requirement 8: Wayland Subsurface Integration

**User Story:** As a user, I want embedded protocol sessions to integrate properly with the Wayland compositor, so that I get smooth rendering and proper input handling.

#### Acceptance Criteria

1. WHEN running on Wayland THEN the RustConn System SHALL create a wl_subsurface for embedded RDP/VNC sessions
2. WHEN the parent window moves or resizes THEN the RustConn System SHALL update the subsurface position accordingly
3. WHEN the embedded session receives framebuffer updates THEN the RustConn System SHALL blit them to the Wayland surface using shared memory buffers
4. WHEN running on X11 THEN the RustConn System SHALL fall back to Cairo-based rendering

### Requirement 9: SPICE Protocol Support

**User Story:** As a user, I want to connect to SPICE servers for virtual machine access, so that I can manage VMs with full display and input support.

#### Acceptance Criteria

1. WHEN a user creates a SPICE connection THEN the RustConn System SHALL connect using the SPICE protocol
2. WHEN connected to a SPICE server THEN the RustConn System SHALL render the display in embedded mode
3. WHEN the user interacts with the SPICE session THEN the RustConn System SHALL forward keyboard and mouse input
4. WHEN SPICE native client is unavailable THEN the RustConn System SHALL fall back to external virt-viewer

### Requirement 10: IronRDP Integration Architecture

**User Story:** As a developer, I want a clean architecture for IronRDP integration, so that the codebase remains maintainable and testable.

#### Acceptance Criteria

1. WHEN implementing IronRDP client THEN the RustConn System SHALL follow the same architecture pattern as the VNC client (vnc-rs)
2. WHEN IronRDP has dependency conflicts THEN the RustConn System SHALL use a feature flag to conditionally compile IronRDP support
3. WHEN the RDP client receives framebuffer updates THEN the RustConn System SHALL convert them to the same event format as VNC for consistent GUI handling
4. WHEN serializing RDP client configuration THEN the RustConn System SHALL support round-trip serialization to JSON
