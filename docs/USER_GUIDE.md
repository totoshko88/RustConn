# RustConn User Guide

RustConn is a modern connection manager for Linux, designed to manage SSH, RDP, VNC, SPICE, and Zero Trust remote connections through a GTK4-based GUI with Wayland-native support. It supports custom console-based connections and integrates with secret managers (KeePassXC, libsecret). Import/export from Asbru-CM, Remmina, SSH config, Ansible inventory, and Royal TS.

## Table of Contents

1. [Getting Started](#getting-started)
2. [Connections](#connections)
3. [Groups](#groups)
4. [Documents](#documents)
5. [Sessions](#sessions)
6. [Snippets](#snippets)
7. [Templates](#templates)
8. [Import/Export](#importexport)
9. [Settings](#settings)
10. [Keyboard Shortcuts](#keyboard-shortcuts)
11. [Performance Features](#performance-features)
12. [Tracing and Debugging](#tracing-and-debugging)

---

## Getting Started

### Installation

```bash
# Install dependencies (Ubuntu/Debian)
sudo apt install libgtk-4-dev libvte-2.91-gtk4-dev libdbus-1-dev pkg-config

# Build and run
cargo run -p rustconn --release
```

### Main Interface

The main window consists of:
- **Header Bar** - Application menu, search, and actions
- **Sidebar** (left) - Connection tree with groups
- **Terminal Area** (right) - Active SSH sessions in tabs

---

## Connections

Connections are the core entities in RustConn. Each connection stores all information needed to connect to a remote host.

### Create Connection

**Purpose:** Add a new remote connection to your list.

**How to:**
1. Click **"+"** button in sidebar, or
2. Press **Ctrl+N**, or
3. Right-click in sidebar → "New Connection"

**Fields:**
- **Name** - Display name for the connection
- **Host** - Hostname or IP address
- **Port** - Connection port (default: 22 for SSH, 3389 for RDP, 5900 for VNC)
- **Protocol** - SSH, RDP, or VNC
- **Username** - Login username
- **Authentication** - Password, SSH Key, or KeePass

### Edit Connection

**Purpose:** Modify an existing connection's settings.

**How to:**
1. Double-click connection in sidebar, or
2. Select connection → Press **Enter**, or
3. Right-click → "Edit"

### Delete Connection

**Purpose:** Remove a connection from your list.

**How to:**
1. Select connection → Press **Delete**, or
2. Right-click → "Delete"

**Note:** This action cannot be undone.

### Duplicate Connection

**Purpose:** Create a copy of an existing connection with a new ID.

**How to:**
1. Right-click connection → "Duplicate"

### Connect

**Purpose:** Establish a connection to the remote host.

**How to:**
1. Double-click connection, or
2. Select connection → Press **Enter**, or
3. Right-click → "Connect"

**SSH:** Opens in embedded terminal tab
**RDP/VNC:** Opens in external window

---

## Groups

Groups help organize connections into logical categories.

### Create Group

**Purpose:** Create a folder to organize connections.

**How to:**
1. Click folder icon in sidebar, or
2. Press **Ctrl+G**, or
3. Right-click → "New Group"

### Create Subgroup

**Purpose:** Create a nested group inside another group.

**How to:**
1. Right-click on parent group → "New Subgroup"

### Rename Group

**Purpose:** Change the group's display name.

**How to:**
1. Right-click group → "Rename"

### Delete Group

**Purpose:** Remove a group and optionally its contents.

**How to:**
1. Select group → Press **Delete**, or
2. Right-click → "Delete"

**Note:** Connections inside will be moved to root level.

### Move Connection to Group

**Purpose:** Organize a connection into a group.

**How to:**
1. Drag and drop connection onto group, or
2. Right-click connection → "Move to Group" → Select group

---

## Documents

Documents are separate configuration files that can contain their own connections, groups, and variables.

### New Document

**Purpose:** Create a new empty document.

**How to:**
1. Menu → "New Document"

### Open Document

**Purpose:** Load an existing document file.

**How to:**
1. Menu → "Open Document..."
2. Select `.rustconn` file

### Save Document

**Purpose:** Save current document to disk.

**How to:**
1. Menu → "Save Document", or
2. Press **Ctrl+S**

### Close Document

**Purpose:** Close the current document.

**How to:**
1. Menu → "Close Document"

**Note:** You will be prompted to save unsaved changes.

### Export Document

**Purpose:** Export document to a portable format.

**How to:**
1. Menu → "Export Document..."
2. Choose format (JSON/YAML)
3. Select destination

### Import Document

**Purpose:** Import connections from a document file.

**How to:**
1. Menu → "Import Document..."
2. Select file to import

---

## Sessions

Sessions represent active connections to remote hosts.

### View Active Sessions

**Purpose:** See all currently active connections.

**How to:**
1. Menu → "Sessions" → "Show Sessions", or
2. Press **Ctrl+Shift+S**

### Switch Between Sessions

**Purpose:** Navigate between open terminal tabs.

**How to:**
1. Click on tab, or
2. Press **Ctrl+Tab** (next), **Ctrl+Shift+Tab** (previous), or
3. Press **Ctrl+1-9** for specific tab

### Close Session

**Purpose:** Disconnect and close a session.

**How to:**
1. Click **X** on tab, or
2. Press **Ctrl+W** (closes current tab)

### Split View

**Purpose:** View multiple terminals side by side.

**How to:**
1. Press **Ctrl+\\** for horizontal split
2. Press **Ctrl+|** for vertical split
3. Press **Ctrl+Shift+W** to close pane

---

## Snippets

Snippets are reusable command templates with variable substitution.

### Create Snippet

**Purpose:** Save a frequently used command.

**How to:**
1. Menu → "Snippets" → "New Snippet"
2. Enter name and command
3. Use `${variable}` for placeholders

### Execute Snippet

**Purpose:** Run a saved command in the current terminal.

**How to:**
1. Menu → "Snippets" → "Execute Snippet"
2. Select snippet from list
3. Fill in variable values if prompted

### Manage Snippets

**Purpose:** View, edit, or delete saved snippets.

**How to:**
1. Menu → "Snippets" → "Manage Snippets"

---

## Templates

Templates are connection presets that can be applied to new connections.

### Create Template

**Purpose:** Save connection settings as a reusable template.

**How to:**
1. Menu → "Templates" → "Manage Templates"
2. Click "New Template"
3. Configure settings

### Apply Template

**Purpose:** Use a template when creating a new connection.

**How to:**
1. Create new connection
2. Select template from dropdown
3. Fields will be pre-populated

---

## Import/Export

### Import Connections

**Purpose:** Import connections from other applications.

**How to:**
1. Click import button in sidebar, or
2. Press **Ctrl+I**, or
3. Menu → "Import..."

**Supported formats:**
- SSH Config (`~/.ssh/config`)
- Remmina profiles
- Asbru-CM configuration
- Ansible inventory (INI/YAML)
- Royal TS (.rtsz XML)
- RustConn Native (.rcn)

### Export Configuration

**Purpose:** Export all connections to a file.

**How to:**
1. Click export button in sidebar, or
2. Press **Ctrl+Shift+E**, or
3. Menu → "Export..."

**Supported formats:**
- SSH Config
- Remmina profiles
- Asbru-CM configuration
- Ansible inventory
- Royal TS (.rtsz XML)
- RustConn Native (.rcn)

---

## Settings

### Terminal Settings

- **Font Family** - Terminal font (default: Monospace)
- **Font Size** - Font size in points
- **Scrollback Lines** - Number of lines to keep in history

### Logging Settings

- **Enable Logging** - Record session output to files
- **Log Directory** - Where to store log files
- **Retention Days** - How long to keep logs

### Secret Settings

- **Preferred Backend** - KeePassXC, KDBX File, or libsecret
- **Enable Fallback** - Use libsecret if primary unavailable
- **KeePass Integration** - Configure KDBX database path

### UI Settings

- **Remember Geometry** - Save window size/position
- **Enable Tray Icon** - Show icon in system tray
- **Minimize to Tray** - Hide to tray instead of closing

---

## Keyboard Shortcuts

### Global

| Shortcut | Action |
|----------|--------|
| Ctrl+N | New Connection |
| Ctrl+G | New Group |
| Ctrl+I | Import |
| Ctrl+Shift+E | Export |
| Ctrl+, | Settings |
| Ctrl+Q | Quit |

### Navigation

| Shortcut | Action |
|----------|--------|
| Ctrl+F | Focus Search |
| Ctrl+L | Focus Sidebar |
| Ctrl+T | Focus Terminal |
| Ctrl+Tab | Next Tab |
| Ctrl+Shift+Tab | Previous Tab |
| Ctrl+1-9 | Go to Tab N |

### Terminal

| Shortcut | Action |
|----------|--------|
| Ctrl+Shift+C | Copy |
| Ctrl+Shift+V | Paste |
| Ctrl+W | Close Tab |
| Ctrl+\\ | Split Horizontal |
| Ctrl+| | Split Vertical |

### Editing

| Shortcut | Action |
|----------|--------|
| Enter | Connect/Edit |
| Delete | Delete Selected |
| F2 | Rename |

---

## Tray Icon

When enabled, RustConn shows an icon in the system tray with:

- **Show/Hide Window** - Toggle main window visibility
- **Recent Connections** - Quick access to recent connections
- **Quick Connect** - Open quick connect dialog
- **Active Sessions** - Shows count of active sessions
- **Quit** - Exit application

---

## Performance Features

RustConn includes several performance optimizations designed to handle large connection databases efficiently.

### Search Caching

**Purpose:** Speed up repeated searches by caching results.

**How it works:**
- Search results are cached with a configurable TTL (default: 30 seconds)
- Repeated searches return cached results instantly
- Cache is automatically invalidated when connections are added, modified, or deleted
- Maximum 100 cached queries to prevent unbounded memory growth

**Behavior:**
- First search: Executes full search, caches results
- Repeated search within TTL: Returns cached results immediately
- After TTL expires: Re-executes search, updates cache

### Lazy Loading

**Purpose:** Reduce startup time for large connection databases.

**How it works:**
- Only root-level groups and ungrouped connections load at startup
- Child groups and connections load when you expand a parent group
- Loaded children remain in memory for quick re-expansion
- Search always searches all connections regardless of lazy loading state

**Benefits:**
- Faster application startup
- Lower initial memory usage
- Responsive UI even with thousands of connections

### Virtual Scrolling

**Purpose:** Maintain responsive scrolling with large connection lists.

**How it works:**
- Activates automatically when connection count exceeds 100
- Only renders visible items plus a small buffer (5 items above/below)
- Selection state is preserved when items scroll in and out of view
- Targets 60fps (16ms) scroll updates

**When to expect it:**
- Sidebar with 100+ connections
- Large search result sets

### Debounced Search

**Purpose:** Prevent excessive searches during rapid typing.

**How it works:**
- 100ms delay after you stop typing before search executes
- Each keystroke resets the timer
- Visual indicator shows when search is pending

**Benefits:**
- Smoother typing experience
- Reduced CPU usage during search
- More responsive UI

### Embedded SPICE Sessions

**Purpose:** View SPICE remote sessions directly in the application window.

**Requirements:**
- Requires `spice-client` crate (version 0.2.0) - included by default.

**Behavior:**
- SPICE connections open in embedded tabs (like SSH)
- Keyboard and mouse events are forwarded to the SPICE server
- If native connection fails, falls back to external `remote-viewer`

### ZeroTrust Connections

**Purpose:** Connect through cloud provider bastion services and zero-trust access platforms.

**Supported Providers:**
- **AWS SSM** - AWS Systems Manager Session Manager
- **GCP IAP** - Google Cloud Identity-Aware Proxy
- **Azure Bastion** - Azure Bastion service
- **Azure SSH** - Azure SSH extension
- **OCI Bastion** - Oracle Cloud Infrastructure Bastion
- **Cloudflare Access** - Cloudflare Zero Trust
- **Teleport** - Gravitational Teleport
- **Tailscale SSH** - Tailscale SSH
- **HashiCorp Boundary** - HashiCorp Boundary

**How to:**
1. Create new connection → Select "ZeroTrust" protocol
2. Choose provider from dropdown
3. Configure provider-specific settings (instance ID, project, etc.)
4. Save and connect

**Note:** Each provider requires its CLI tool to be installed and configured:
- AWS: `aws` CLI with SSM plugin
- GCP: `gcloud` CLI
- Azure: `az` CLI
- Tailscale: `tailscale` CLI

---

## Tracing and Debugging

RustConn uses structured logging via the `tracing` crate for diagnostics and performance profiling.

### Enabling Tracing

Set the `RUST_LOG` environment variable before starting RustConn:

```bash
# Basic info logging
RUST_LOG=info cargo run -p rustconn

# Debug logging
RUST_LOG=debug cargo run -p rustconn

# Trace logging (very verbose)
RUST_LOG=trace cargo run -p rustconn
```

### Module-Specific Logging

Target specific modules for focused debugging:

```bash
# Search operations
RUST_LOG=rustconn_core::search=debug cargo run -p rustconn

# Connection management
RUST_LOG=rustconn_core::connection=debug cargo run -p rustconn

# Credential resolution
RUST_LOG=rustconn_core::secret=debug cargo run -p rustconn

# Import/export operations
RUST_LOG=rustconn_core::import=debug,rustconn_core::export=debug cargo run -p rustconn

# Multiple modules
RUST_LOG=rustconn_core::search=trace,rustconn_core::connection=debug cargo run -p rustconn
```

### Traced Operations

The following operations include tracing spans with timing information:

- **Connection establishment**: Protocol, host, port, duration
- **Search execution**: Query, result count, cache hit/miss
- **Import/export**: Format, item count, progress
- **Credential resolution**: Backend used, success/failure

### Performance Profiling

For performance analysis, enable info-level logging to see operation timings:

```bash
RUST_LOG=info cargo run -p rustconn 2>&1 | grep -E "(span|duration)"
```

### Log Output

Logs are written to stderr by default. Redirect to a file for analysis:

```bash
RUST_LOG=debug cargo run -p rustconn 2> rustconn.log
```

---

## Troubleshooting

### Connection Fails

1. Verify host and port are correct
2. Check network connectivity
3. Verify credentials
4. Check firewall settings

### KeePass Integration Not Working

1. Ensure KeePassXC is installed
2. Enable browser integration in KeePassXC
3. Configure KDBX path in Settings → Secrets

### Terminal Not Responding

1. Check if connection is still active
2. Try pressing Enter
3. Close and reconnect

---

## Support

- GitHub Issues: Report bugs and feature requests
- Documentation: See README.md for technical details
