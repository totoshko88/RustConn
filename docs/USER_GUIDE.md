# RustConn User Guide

RustConn is a modern connection manager for Linux, designed to manage SSH, RDP, VNC, and SPICE remote connections through a GTK4-based GUI with Wayland-native support.

## Table of Contents

1. [Getting Started](#getting-started)
2. [Supported Protocols](#supported-protocols)
3. [Connections](#connections)
4. [Groups](#groups)
5. [Documents](#documents)
6. [Sessions](#sessions)
7. [Snippets](#snippets)
8. [Templates](#templates)
9. [Import/Export](#importexport)
10. [Settings](#settings)
11. [Keyboard Shortcuts](#keyboard-shortcuts)
12. [Performance Features](#performance-features)
13. [Troubleshooting](#troubleshooting)

---

## Getting Started

### Installation

```bash
# Install dependencies (Ubuntu/Debian)
sudo apt install libgtk-4-dev libvte-2.91-gtk4-dev libdbus-1-dev pkg-config

# Install protocol clients
sudo apt install openssh-client freerdp2-x11 tigervnc-viewer virt-viewer

# Build and run
cargo run -p rustconn --release
```

### Main Interface

The main window consists of:
- **Header Bar** - Application menu, search, and actions
- **Sidebar** (left) - Connection tree with groups
- **Session Area** (right) - Active sessions in tabs (SSH terminals, embedded RDP/VNC/SPICE)

---

## Supported Protocols

RustConn supports four remote connection protocols:

### SSH (Secure Shell)

| Feature | Details |
|---------|---------|
| Default Port | 22 |
| Session Type | Embedded VTE terminal |
| Authentication | Password, SSH key, SSH agent |
| Features | PTY support, port forwarding, X11 forwarding |

**How it works:** SSH connections open in embedded terminal tabs using VTE4 for full terminal emulation.

### RDP (Remote Desktop Protocol)

| Feature | Details |
|---------|---------|
| Default Port | 3389 |
| Session Type | External window (FreeRDP) or Embedded (IronRDP) |
| Authentication | Password, NLA, TLS |
| Features | Clipboard sharing, drive redirection, audio |

**External client:** Requires `xfreerdp` or `xfreerdp3` (FreeRDP package)

**Embedded mode:** Build with `--features rdp-embedded` for native IronRDP support

### VNC (Virtual Network Computing)

| Feature | Details |
|---------|---------|
| Default Port | 5900 |
| Session Type | External window (TigerVNC) or Embedded (vnc-rs) |
| Authentication | VNC password, None |
| Features | Cross-platform, multiple security types |

**External client:** Requires `vncviewer` (TigerVNC or TightVNC)

**Embedded mode:** Build with `--features vnc-embedded` for native VNC support

### SPICE (Simple Protocol for Independent Computing Environments)

| Feature | Details |
|---------|---------|
| Default Port | 5900 |
| Session Type | External window (remote-viewer) or Embedded |
| Authentication | Password, TLS |
| Features | Optimized for VMs, USB redirection, audio |

**External client:** Requires `remote-viewer` (virt-viewer package)

**Embedded mode:** Build with `--features spice-embedded` for native SPICE support

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
- **Protocol** - SSH, RDP, VNC, or SPICE
- **Host** - Hostname or IP address
- **Port** - Connection port (auto-filled based on protocol)
- **Username** - Login username
- **Authentication** - Password, SSH Key, or KeePass
- **Window Mode** - Embedded, External Window, or Fullscreen

### Protocol-Specific Settings

#### SSH Options
- SSH key file path
- Key passphrase
- Port forwarding rules
- X11 forwarding
- Agent forwarding

#### RDP Options
- Domain
- Resolution (width × height)
- Color depth
- Clipboard sharing
- Drive redirection
- Gateway settings

#### VNC Options
- Color depth
- Compression level
- Quality level
- View only mode

#### SPICE Options
- TLS port
- Certificate verification
- USB redirection
- Shared folders

### Edit Connection

**How to:**
1. Double-click connection in sidebar, or
2. Select connection → Press **Enter**, or
3. Right-click → "Edit"

### Delete Connection

**How to:**
1. Select connection → Press **Delete**, or
2. Right-click → "Delete"

### Connect

**How to:**
1. Double-click connection, or
2. Select connection → Press **Enter**, or
3. Right-click → "Connect"

**Behavior by protocol:**
- **SSH:** Opens in embedded terminal tab
- **RDP/VNC/SPICE (External):** Opens in separate window
- **RDP/VNC/SPICE (Embedded):** Opens in embedded tab (if feature enabled)

---

## Groups

Groups help organize connections into logical categories.

### Create Group

**How to:**
1. Click folder icon in sidebar, or
2. Press **Ctrl+G**, or
3. Right-click → "New Group"

### Create Subgroup

**How to:**
1. Right-click on parent group → "New Subgroup"

### Move Connection to Group

**How to:**
1. Drag and drop connection onto group, or
2. Right-click connection → "Move to Group" → Select group

---

## Documents

Documents are separate configuration files that can contain their own connections, groups, and variables.

### Document Operations

- **New Document:** Menu → "New Document"
- **Open Document:** Menu → "Open Document..." (`.rustconn` files)
- **Save Document:** Menu → "Save Document" or **Ctrl+S**
- **Close Document:** Menu → "Close Document"

---

## Sessions

Sessions represent active connections to remote hosts.

### View Active Sessions

**How to:**
1. Menu → "Sessions" → "Show Sessions", or
2. Press **Ctrl+Shift+S**

### Switch Between Sessions

**How to:**
1. Click on tab, or
2. Press **Ctrl+Tab** (next), **Ctrl+Shift+Tab** (previous), or
3. Press **Ctrl+1-9** for specific tab

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

**How to:**
1. Menu → "Snippets" → "New Snippet"
2. Enter name and command
3. Use `${variable}` for placeholders

**Example:**
```bash
ssh -L ${local_port}:localhost:${remote_port} ${host}
```

### Execute Snippet

**How to:**
1. Menu → "Snippets" → "Execute Snippet"
2. Select snippet from list
3. Fill in variable values if prompted

---

## Templates

Templates are connection presets that can be applied to new connections.

### Create Template

**How to:**
1. Menu → "Templates" → "Manage Templates"
2. Click "New Template"
3. Configure default settings for each protocol

### Apply Template

**How to:**
1. Create new connection
2. Select template from dropdown
3. Fields will be pre-populated

---

## Import/Export

### Supported Formats

| Format | Import | Export | Description |
|--------|--------|--------|-------------|
| SSH Config | ✅ | ✅ | `~/.ssh/config` format |
| Remmina | ✅ | ✅ | Remmina `.remmina` profiles |
| Asbru-CM | ✅ | ✅ | Asbru Connection Manager |
| Ansible | ✅ | ✅ | Ansible inventory (INI/YAML) |
| Native | ✅ | ✅ | RustConn `.rustconn` format |

### Import Connections

**How to:**
1. Click import button in sidebar, or
2. Press **Ctrl+I**, or
3. Menu → "Import..."

### Export Connections

**How to:**
1. Click export button in sidebar, or
2. Press **Ctrl+Shift+E**, or
3. Menu → "Export..."

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
| Ctrl+\| | Split Vertical |

---

## Performance Features

### Search Caching

- Search results cached with configurable TTL (default: 30 seconds)
- Cache invalidated when connections change
- Maximum 100 cached queries

### Lazy Loading

- Only root-level groups load at startup
- Child groups load when expanded
- Search always searches all connections

### Virtual Scrolling

- Activates with 100+ connections
- Only renders visible items
- Targets 60fps scroll updates

---

## Troubleshooting

### Connection Fails

1. Verify host and port are correct
2. Check network connectivity
3. Verify credentials
4. Check firewall settings

### Protocol Client Not Found

Install the required client:
```bash
# RDP
sudo apt install freerdp2-x11

# VNC
sudo apt install tigervnc-viewer

# SPICE
sudo apt install virt-viewer
```

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
