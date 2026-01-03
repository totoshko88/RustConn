# RustConn User Guide

RustConn is a modern connection manager for Linux, designed to manage SSH, RDP, VNC, SPICE, and Zero Trust remote connections through a GTK4/libadwaita GUI with Wayland-native support.

## Table of Contents

1. [Getting Started](#getting-started)
2. [Main Interface](#main-interface)
3. [Connections](#connections)
4. [Groups](#groups)
5. [Sessions](#sessions)
6. [Templates](#templates)
7. [Snippets](#snippets)
8. [Clusters](#clusters)
9. [Import/Export](#importexport)
10. [Tools](#tools)
11. [Settings](#settings)
12. [Keyboard Shortcuts](#keyboard-shortcuts)
13. [CLI Usage](#cli-usage)
14. [Troubleshooting](#troubleshooting)

---

## Getting Started

### Installation

**Flatpak (recommended):**
```bash
flatpak install io.github.totoshko88.RustConn
```

**Debian/Ubuntu:**
```bash
sudo dpkg -i rustconn_0.5.5_amd64.deb
sudo apt-get install -f
```

**AppImage:**
```bash
chmod +x RustConn-0.5.5-x86_64.AppImage
./RustConn-0.5.5-x86_64.AppImage
```

**From source:**
```bash
sudo apt install libgtk-4-dev libvte-2.91-gtk4-dev libadwaita-1-dev libdbus-1-dev pkg-config
cargo build --release -p rustconn
```

---

## Main Interface

The main window consists of:

- **Header Bar** — Application menu, search field, and action buttons
- **Sidebar** (left) — Connection tree with groups, sorted alphabetically
- **Session Area** (right) — Active sessions in tabs (SSH terminals, embedded RDP/VNC)
- **Toast Overlay** — Non-blocking notifications for operations

### Color Scheme

Switch between System, Light, and Dark themes in Settings → Appearance.

---

## Connections

### Create Connection

1. Click **"+"** button in header bar, or press **Ctrl+N**
2. Fill in connection details across tabs:
   - **Basic** — Name, host, port, protocol, group
   - **Authentication** — Username, password, SSH key
   - **Protocol-specific** — SSH options, RDP settings, VNC config
   - **Automation** — Expect scripts, startup commands
   - **Tags** — Organize with custom tags
3. Click **Create**

### Edit Connection

- Double-click connection, or
- Select → Press **Enter**, or
- Right-click → **Edit**

### Rename Connection

- Press **F2**, or
- Right-click → **Rename**

### View Connection Details

- Right-click → **View Details** — Opens Info tab with all connection properties

### Connect

- Double-click connection, or
- Right-click → **Connect**

**Session types by protocol:**
- **SSH** — Embedded VTE terminal tab
- **RDP** — Embedded IronRDP widget or external FreeRDP
- **VNC** — Embedded vnc-rs widget or external TigerVNC
- **SPICE** — Embedded spice-client or external remote-viewer
- **ZeroTrust** — Provider-specific (AWS SSM, GCP IAP, Azure Bastion, etc.)

### Quick Connect

Press **Ctrl+K** for temporary connection without saving:
- Supports SSH, RDP, VNC protocols
- Can use templates to pre-fill settings
- Connection is not saved to database

### Duplicate / Copy / Paste

- **Duplicate** — Right-click → Duplicate (creates copy with new ID)
- **Copy** — Ctrl+C (copies connection to clipboard)
- **Paste** — Ctrl+V (pastes copied connection)

### Delete Connection

- Select → Press **Delete**, or
- Right-click → **Delete**

---

## Groups

Groups organize connections into folders with hierarchical structure.

### Create Group

- Click folder icon in header bar, or
- Press **Ctrl+G**, or
- Right-click in sidebar → **New Group**

### Create Subgroup

Right-click on parent group → **New Subgroup**

### Rename Group

- Press **F2**, or
- Right-click → **Rename**

### Move to Group

- Drag and drop connection onto group, or
- Right-click → **Move to Group** → Select destination

### Sorting

Connections and groups are sorted alphabetically by default. Drag-and-drop to reorder manually.

---

## Sessions

### Active Sessions

Sessions appear as tabs in the session area. Each tab shows:
- Connection name
- Protocol icon
- Connection status indicator (green = connected, red = disconnected)
- Close button

### Session Management

- **Switch tabs** — Click tab or Ctrl+Tab / Ctrl+Shift+Tab
- **Close tab** — Click X or Ctrl+W
- **Split view** — Ctrl+\\ (horizontal) or Ctrl+| (vertical)

### Session Restore

Enable in Settings → Session to restore sessions on startup:
- Sessions are saved when app closes
- Option to prompt before restoring
- Configurable maximum session age

### Session Logging

Enable in Settings → Logging:
- **Activity logging** — Track session activity changes
- **User input logging** — Capture commands typed
- **Terminal output logging** — Record full transcript

### Connection History

View past connections in Tools → Connection History:
- Search and filter history
- Connect directly from history
- View statistics (success rate, duration)

---

## Templates

Templates are connection presets for quick creation.

### Manage Templates

Menu → Tools → **Manage Templates**

### Create Template

1. Open Manage Templates
2. Click **Create Template**
3. Configure protocol-specific settings
4. Save template

### Use Template

- In Quick Connect dialog, select template from dropdown
- Template fills protocol, host, port, username automatically

### Create Connection from Template

In Manage Templates, select template → Click **Create** to create a new connection based on it.

---

## Snippets

Snippets are reusable command templates with variable substitution.

### Create Snippet

Menu → Tools → **Manage Snippets** → Create

Variables use `${variable}` syntax:
```bash
ssh ${user}@${host} -p ${port}
```

### Execute Snippet

1. Select active terminal session
2. Menu → Tools → **Execute Snippet**
3. Select snippet, fill variables
4. Command is sent to terminal

---

## Clusters

Clusters allow executing commands on multiple connections simultaneously.

### Create Cluster

Menu → Tools → **Manage Clusters** → Create

Select connections to include in the cluster.

### Execute Cluster Command

1. Select cluster
2. Enter command
3. Command executes on all cluster members

---

## Import/Export

### Import Connections

Press **Ctrl+I** or Menu → **Import**

**Supported formats:**
- SSH Config (`~/.ssh/config`)
- Remmina profiles
- Asbru-CM configuration
- Ansible inventory (INI/YAML)
- Royal TS (.rtsz XML)
- RustConn Native (.rcn)

**Tip:** Double-click import source to start import immediately.

### Export Connections

Press **Ctrl+Shift+E** or Menu → **Export**

**Supported formats:**
- SSH Config
- Remmina profiles
- Asbru-CM configuration
- Ansible inventory
- Royal TS (.rtsz XML)
- RustConn Native (.rcn)

---

## Tools

### Password Generator

Menu → Tools → **Password Generator**

Features:
- Configurable length (4-128 characters)
- Character sets: lowercase, uppercase, digits, special, extended
- Exclude ambiguous characters (0, O, l, 1, I)
- Real-time strength indicator with entropy calculation
- Crack time estimation
- Copy to clipboard

**Security Tips** (shown in dialog):
- Use 16+ characters for critical accounts
- Never reuse passwords across services
- Store in password manager, not plain text
- Enable 2FA when available
- Change passwords after breach reports

### Wake-on-LAN

Send magic packets to wake remote machines:
- Configure MAC address in connection settings
- Right-click connection → **Wake-on-LAN**

### Connection Statistics

Menu → Tools → **Connection Statistics**

View success rates, connection durations, and usage patterns.

---

## Settings

Access via **Ctrl+,** or Menu → **Settings**

### Appearance

- **Color Scheme** — System, Light, or Dark theme
- **Remember Window Geometry** — Save size/position

### Terminal

- **Font Family** — Terminal font (default: Monospace)
- **Font Size** — Size in points
- **Scrollback Lines** — History buffer size

### Session

- **Enable Session Restore** — Restore sessions on startup
- **Prompt Before Restore** — Ask before restoring
- **Maximum Session Age** — Hours to keep saved sessions

### Logging

- **Enable Logging** — Record session output
- **Log Directory** — Storage location
- **Retention Days** — Auto-cleanup period
- **Logging Modes** — Activity, user input, terminal output

### Secrets

- **Preferred Backend** — libsecret, KeePassXC, or KDBX file
- **Enable Fallback** — Use libsecret if primary unavailable
- **KDBX Path** — Path to KeePass database file
- **KDBX Password/Key File** — Database credentials

### Tray Icon

- **Enable Tray Icon** — Show in system tray
- **Minimize to Tray** — Hide instead of close
- **Start Minimized** — Launch to tray

---

## Keyboard Shortcuts

Press **Ctrl+?** or **F1** to open searchable shortcuts dialog.

### Connections

| Shortcut | Action |
|----------|--------|
| Ctrl+N | New Connection |
| Ctrl+G | New Group |
| Ctrl+K | Quick Connect |
| Enter | Connect / Edit |
| F2 | Rename |
| Delete | Delete |
| Ctrl+C | Copy Connection |
| Ctrl+V | Paste Connection |
| Ctrl+D | Duplicate |

### Terminal

| Shortcut | Action |
|----------|--------|
| Ctrl+Shift+C | Copy |
| Ctrl+Shift+V | Paste |
| Ctrl+W | Close Tab |
| Ctrl+\\ | Split Horizontal |
| Ctrl+\| | Split Vertical |

### Navigation

| Shortcut | Action |
|----------|--------|
| Ctrl+F | Focus Search |
| Ctrl+Tab | Next Tab |
| Ctrl+Shift+Tab | Previous Tab |
| Ctrl+1-9 | Go to Tab N |

### Application

| Shortcut | Action |
|----------|--------|
| Ctrl+I | Import |
| Ctrl+Shift+E | Export |
| Ctrl+, | Settings |
| Ctrl+? / F1 | Keyboard Shortcuts |
| Ctrl+Q | Quit |

---

## CLI Usage

RustConn includes a CLI tool for headless operations.

### Basic Commands

```bash
# List connections
rustconn-cli list
rustconn-cli list --group "Production" --tag "web"

# Connect
rustconn-cli connect "My Server"

# Import/Export
rustconn-cli import ssh-config ~/.ssh/config
rustconn-cli export native backup.rcn
```

### Snippet Commands

```bash
rustconn-cli snippet list
rustconn-cli snippet show "Deploy Script"
rustconn-cli snippet run "Deploy Script" --execute
```

### Group Commands

```bash
rustconn-cli group list
rustconn-cli group create "New Group"
rustconn-cli group add-connection "Group Name" "Connection Name"
```

### Wake-on-LAN

```bash
rustconn-cli wol AA:BB:CC:DD:EE:FF
rustconn-cli wol "My Server"  # Uses connection's MAC
```

---

## Troubleshooting

### Connection Fails

1. Verify host and port are correct
2. Check network connectivity: `ping hostname`
3. Verify credentials
4. Check firewall settings
5. For SSH: verify key permissions (`chmod 600 ~/.ssh/id_rsa`)

### KeePass Integration Not Working

1. Ensure KeePassXC is installed
2. Enable browser integration in KeePassXC settings
3. Configure KDBX path in Settings → Secrets
4. Provide database password or key file

### Embedded RDP/VNC Not Working

1. Check if IronRDP/vnc-rs features are enabled
2. For external mode, verify FreeRDP/TigerVNC is installed
3. Check display server (Wayland vs X11) compatibility

### Session Restore Not Working

1. Verify enabled in Settings → Session
2. Check maximum session age setting
3. Ensure app was closed normally (not killed)

### Tray Icon Not Appearing

1. Verify system tray support (requires `ksni` feature)
2. Check desktop environment compatibility
3. Some DEs require extensions for tray support

### Debug Logging

Enable detailed logging for troubleshooting:

```bash
RUST_LOG=debug rustconn 2> rustconn.log
```

Module-specific logging:
```bash
RUST_LOG=rustconn_core::connection=debug rustconn
RUST_LOG=rustconn_core::secret=debug rustconn
```

---

## Support

- **GitHub Issues:** https://github.com/totoshko88/RustConn/issues
- **Documentation:** See README.md and CHANGELOG.md
