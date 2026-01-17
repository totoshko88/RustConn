# RustConn User Guide

**Version 0.6.4** | GTK4/libadwaita Connection Manager for Linux

RustConn is a modern connection manager designed for Linux with Wayland-first approach. It supports SSH, RDP, VNC, SPICE protocols and Zero Trust integrations through a native GTK4/libadwaita interface.

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

### Quick Start

1. Install RustConn (see [INSTALL.md](INSTALL.md))
2. Launch from application menu or run `rustconn`
3. Create your first connection with **Ctrl+N**
4. Double-click to connect

### First Connection

1. Press **Ctrl+N** or click **+** in header bar
2. Enter connection name and host
3. Select protocol (SSH, RDP, VNC, SPICE)
4. Configure authentication (password or SSH key)
5. Click **Create**
6. Double-click the connection to connect

---

## Main Interface

### Layout

```
┌─────────────────────────────────────────────────────────────┐
│  Header Bar: Menu | Search | + | Quick Connect | Split      │
├──────────────────┬──────────────────────────────────────────┤
│                  │                                          │
│    Sidebar       │         Session Area                     │
│                  │                                          │
│  ▼ Production    │  ┌─────┬─────┬─────┐                    │
│    ├─ Web-01     │  │ Tab1│ Tab2│ Tab3│                    │
│    ├─ Web-02     │  └─────┴─────┴─────┘                    │
│    └─ DB-01      │                                          │
│  ▼ Development   │    Terminal / Embedded RDP / VNC         │
│    └─ Dev-VM     │                                          │
│                  │                                          │
├──────────────────┤                                          │
│ Toolbar: 🗑️ 📁 ⚙️ │                                          │
└──────────────────┴──────────────────────────────────────────┘
```

### Components

- **Header Bar** — Application menu, search, action buttons
- **Sidebar** — Connection tree with groups (alphabetically sorted)
- **Sidebar Toolbar** — Delete, Add Group, Group Operations, Sort, Import, Export, KeePass status
- **Session Area** — Active sessions in tabs
- **Toast Overlay** — Non-blocking notifications

### Quick Filter

Filter connections by protocol using the filter bar below search:
- Click protocol buttons (SSH, RDP, VNC, SPICE, ZeroTrust)
- Multiple protocols can be selected (OR logic)
- Clear search field to reset filters

### Password Vault Button

Shows integration status in sidebar toolbar:
- **Highlighted** — Password manager enabled and configured
- **Dimmed** — Disabled or not configured
- Click to open appropriate password manager:
  - KeePassXC/GNOME Secrets for KeePassXC backend
  - Seahorse/GNOME Settings for libsecret backend
  - Bitwarden web vault for Bitwarden backend

---

## Connections

### Create Connection (Ctrl+N)

**Basic Tab:**
- Name, Host, Port
- Protocol selection
- Parent group
- Tags

**Authentication Tab:**
- Username
- Password (stored securely via libsecret/KeePass)
- SSH key selection
- Key passphrase

**Protocol Tabs** (varies by protocol):

| Protocol | Options |
|----------|---------|
| SSH | Auth method, proxy jump (Jump Host), agent forwarding, startup command |
| RDP | Resolution, color depth, audio, gateway, shared folders |
| VNC | Encoding, compression, quality, view-only, scaling |
| SPICE | TLS, USB redirection, clipboard, image compression |
| ZeroTrust | Provider-specific (AWS SSM, GCP IAP, Azure, etc.) |

**Advanced Tabs:**
- **Display** — Window mode settings
- **Logging** — Session logging configuration
- **WOL** — Wake-on-LAN MAC address
- **Variables** — Local variables for automation
- **Automation** — Expect rules for auto-login
- **Tasks** — Pre/post connection commands
- **Custom Properties** — Metadata fields

### Quick Connect (Ctrl+K)

Temporary connection without saving:
- Supports SSH, RDP, VNC
- Optional template selection for pre-filling
- Password field for RDP/VNC

### Connection Actions

| Action | Method |
|--------|--------|
| Connect | Double-click, Enter, or right-click → Connect |
| Edit | Ctrl+E or right-click → Edit |
| Rename | F2 or right-click → Rename |
| View Details | Right-click → View Details (opens Info tab) |
| Duplicate | Ctrl+D or right-click → Duplicate |
| Copy/Paste | Ctrl+C / Ctrl+V |
| Delete | Delete key or right-click → Delete |
| Move to Group | Drag-drop or right-click → Move to Group |

### Test Connection

In connection dialog, click **Test** to verify connectivity before saving.

### Pre-connect Port Check

For RDP, VNC, and SPICE connections, RustConn performs a fast TCP port check before connecting:
- Provides faster feedback (2-3s vs 30-60s timeout) when hosts are unreachable
- Configurable globally in Settings → Connection
- Per-connection "Skip port check" option for special cases (firewalls, port knocking, VPN)

---

## Groups

### Create Group

- **Ctrl+Shift+N** or click folder icon
- Right-click in sidebar → **New Group**
- Right-click on group → **New Subgroup**

### Group Operations

- **Rename** — F2 or right-click → Rename
- **Move** — Drag-drop or right-click → Move to Group
- **Delete** — Delete key (moves children to root)

### Group Credentials (New in 0.6.1)
Groups can now store default credentials (Username, Password, Domain) that are inherited by their children.

1. **Configure**:
   - In "New Group" or "Edit Group" dialog, fill in the **Default Credentials** section.
   - Passwords are stored securely in the system keyring.

2. **Inherit**:
   - Create a connection inside the group.
   - In **Authentication** tab, set **Password Source** to **Inherit from Group**.

### Sorting

- Alphabetical by default (case-insensitive, by full path)
- Drag-drop for manual reordering
- Click Sort button in toolbar to reset

---

## Sessions

### Session Types

| Protocol | Session Type |
|----------|--------------|
| SSH | Embedded VTE terminal tab |
| RDP | Embedded IronRDP or external FreeRDP |
| VNC | Embedded vnc-rs or external TigerVNC |
| SPICE | Embedded spice-client or external remote-viewer |
| ZeroTrust | Provider CLI in terminal |

### Tab Management

- **Switch** — Click tab or Ctrl+Tab / Ctrl+Shift+Tab
- **Close** — Click X or Ctrl+W
- **Reorder** — Drag tabs

### Split View

- **Horizontal Split** — Ctrl+Shift+H
- **Vertical Split** — Ctrl+Shift+S
- **Close Pane** — Ctrl+Shift+W
- **Focus Next Pane** — Ctrl+`

### Status Indicators

Sidebar shows connection status:
- 🟢 Green dot — Connected
- 🔴 Red dot — Disconnected

### Session Restore

Enable in Settings → Session:
- Sessions saved on app close
- Restored on next startup
- Optional prompt before restore
- Configurable maximum age

### Session Logging

Three logging modes (Settings → Logging):
- **Activity** — Track session activity changes
- **User Input** — Capture typed commands
- **Terminal Output** — Full transcript

---

## Templates

Templates are connection presets for quick creation.

### Manage Templates

Menu → Tools → **Manage Templates**

### Create Template

1. Open Manage Templates
2. Click **Create Template**
3. Configure protocol and settings
4. Save

### Use Template

- **Quick Connect** — Select template from dropdown
- **Manage Templates** — Select → **Create** to make connection

Double-click template to create connection from it.

---

## Snippets

Reusable command templates with variable substitution.

### Syntax

```bash
ssh ${user}@${host} -p ${port}
sudo systemctl restart ${service}
```

### Manage Snippets

Menu → Tools → **Manage Snippets**

### Execute Snippet

1. Select active terminal
2. Menu → Tools → **Execute Snippet**
3. Select snippet, fill variables
4. Command sent to terminal

---

## Clusters

Execute commands on multiple connections simultaneously.

### Create Cluster

Menu → Tools → **Manage Clusters** → Create

### Broadcast Mode

Enable broadcast switch to send input to all cluster members.

---

## Import/Export

### Import (Ctrl+I)

**Supported formats:**
- SSH Config (`~/.ssh/config`)
- Remmina profiles
- Asbru-CM configuration
- Ansible inventory (INI/YAML)
- Royal TS (.rtsz XML)
- MobaXterm sessions (.mxtsessions)
- RustConn Native (.rcn)

Double-click source to start import immediately.

### Export (Ctrl+Shift+E)

**Supported formats:**
- SSH Config
- Remmina profiles
- Asbru-CM configuration
- Ansible inventory
- Royal TS (.rtsz XML)
- MobaXterm sessions (.mxtsessions)
- RustConn Native (.rcn)

Options:
- Include passwords (where supported)
- Export selected only

---

## Tools

### Password Generator

Menu → Tools → **Password Generator**

Features:
- Length: 4-128 characters
- Character sets: lowercase, uppercase, digits, special, extended
- Exclude ambiguous (0, O, l, 1, I)
- Strength indicator with entropy
- Crack time estimation
- Copy to clipboard

### Connection History

Menu → Tools → **Connection History**

- Search and filter past connections
- Connect directly from history
- Reset history

### Connection Statistics

Menu → Tools → **Connection Statistics**

- Success rate visualization
- Connection duration tracking
- Reset statistics

### Wake-on-LAN

Right-click connection → **Wake-on-LAN**

Requires MAC address configured in connection WOL tab.

---

## Settings

Access via **Ctrl+,** or Menu → **Settings**

### Appearance

- **Theme** — System, Light, Dark (libadwaita StyleManager)
- **Remember Window Geometry**

### Terminal

- **Font** — Family and size
- **Scrollback** — History buffer lines
- **Color Theme** — Dark, Light, Solarized, Monokai, Dracula
- **Cursor** — Shape (Block/IBeam/Underline) and blink mode
- **Behavior** — Scroll on output/keystroke, hyperlinks, mouse autohide, bell

### Session

- **Enable Session Restore**
- **Prompt Before Restore**
- **Maximum Session Age** (hours)

### Logging

- **Enable Logging**
- **Log Directory**
- **Retention Days**
- **Logging Modes** — Activity, user input, terminal output

### Secrets

- **Preferred Backend** — libsecret, KeePassXC, KDBX file, Bitwarden
- **Enable Fallback** — Use libsecret if primary unavailable
- **KDBX Path** — KeePass database file (for KDBX backend)
- **KDBX Authentication** — Password and/or key file
- **Bitwarden Settings** — Vault status, unlock, master password persistence
- **Installed Password Managers** — Auto-detected managers with versions (GNOME Secrets, KeePassXC, KeePass2, Bitwarden CLI, 1Password CLI)

### SSH Agent

- **Loaded Keys** — Currently loaded SSH keys
- **Available Keys** — Keys in ~/.ssh/
- **Add/Remove Keys** — Manage agent keys

### Clients

Auto-detected CLI tools with versions:

**Protocol Clients:** SSH, RDP (FreeRDP), VNC (TigerVNC), SPICE (remote-viewer)

**Zero Trust:** AWS, GCP, Azure, OCI, Cloudflare, Teleport, Tailscale, Boundary

Searches PATH and user directories (`~/bin/`, `~/.local/bin/`, `~/.cargo/bin/`).

### Tray Icon

- **Enable Tray Icon**
- **Minimize to Tray**
- **Start Minimized**

### Connection

- **Pre-connect Port Check** — Enable/disable TCP port check before RDP/VNC/SPICE
- **Port Check Timeout** — Timeout in seconds (default: 3)

---

## Keyboard Shortcuts

Press **Ctrl+?** or **F1** for searchable shortcuts dialog.

### Connections

| Shortcut | Action |
|----------|--------|
| Ctrl+N | New Connection |
| Ctrl+Shift+N | New Group |
| Ctrl+Shift+Q | Quick Connect |
| Ctrl+E | Edit Connection |
| F2 | Rename |
| Delete | Delete |
| Ctrl+D | Duplicate |
| Ctrl+C / Ctrl+V | Copy / Paste |

### Terminal

| Shortcut | Action |
|----------|--------|
| Ctrl+Shift+C | Copy |
| Ctrl+Shift+V | Paste |
| Ctrl+Shift+F | Terminal Search |
| Ctrl+W | Close Tab |
| Ctrl+Tab | Next Tab |
| Ctrl+Shift+Tab | Previous Tab |

### Split View

| Shortcut | Action |
|----------|--------|
| Ctrl+Shift+H | Split Horizontal |
| Ctrl+Shift+S | Split Vertical |
| Ctrl+Shift+W | Close Pane |
| Ctrl+` | Focus Next Pane |

### Application

| Shortcut | Action |
|----------|--------|
| Ctrl+F | Search |
| Ctrl+I | Import |
| Ctrl+Shift+E | Export |
| Ctrl+, | Settings |
| F11 | Toggle Fullscreen |
| Ctrl+? / F1 | Keyboard Shortcuts |
| Ctrl+Q | Quit |

---

## CLI Usage

### Commands

```bash
# List connections
rustconn-cli list
rustconn-cli list --group "Production" --tag "web"

# Connect
rustconn-cli connect "My Server"

# Duplicate connection
rustconn-cli duplicate "My Server" --name "My Server Copy"

# Import/Export
rustconn-cli import ssh-config ~/.ssh/config
rustconn-cli export native backup.rcn

# Snippets
rustconn-cli snippet list
rustconn-cli snippet run "Deploy" --execute

# Groups
rustconn-cli group list
rustconn-cli group create "New Group"
rustconn-cli group add-connection "Group Name" "Connection Name"
rustconn-cli group remove-connection "Group Name" "Connection Name"

# Templates
rustconn-cli template list
rustconn-cli template show "SSH Template"
rustconn-cli template create --name "New Template" --protocol ssh
rustconn-cli template delete "Old Template"
rustconn-cli template apply "SSH Template" --name "New Connection" --host "server.example.com"

# Clusters
rustconn-cli cluster list
rustconn-cli cluster show "Web Servers"
rustconn-cli cluster create --name "DB Cluster"
rustconn-cli cluster add-connection "DB Cluster" "DB-01"
rustconn-cli cluster remove-connection "DB Cluster" "DB-01"
rustconn-cli cluster delete "Old Cluster"

# Global Variables
rustconn-cli var list
rustconn-cli var show "my_var"
rustconn-cli var set "my_var" "my_value"
rustconn-cli var delete "my_var"

# Statistics
rustconn-cli stats

# Wake-on-LAN
rustconn-cli wol AA:BB:CC:DD:EE:FF
rustconn-cli wol "Server Name"
```

---

## Troubleshooting

### Connection Issues

1. Verify host/port: `ping hostname`
2. Check credentials
3. SSH key permissions: `chmod 600 ~/.ssh/id_rsa`
4. Firewall settings

### Bitwarden Not Working

1. Install Bitwarden CLI: `npm install -g @bitwarden/cli` or download from bitwarden.com
2. Login: `bw login`
3. Unlock vault: `bw unlock`
4. Select Bitwarden backend in Settings → Secrets
5. Check vault status indicator

### KeePass Not Working

1. Install KeePassXC
2. Enable browser integration in KeePassXC
3. Configure KDBX path in Settings → Secrets
4. Provide password/key file

### Embedded RDP/VNC Issues

1. Check IronRDP/vnc-rs features enabled
2. For external: verify FreeRDP/TigerVNC installed
3. Wayland vs X11 compatibility

### Session Restore Issues

1. Enable in Settings → Session
2. Check maximum age setting
3. Ensure normal app close (not killed)

### Tray Icon Missing

1. Requires `tray-icon` feature
2. Check DE tray support
3. Some DEs need extensions

### Debug Logging

```bash
RUST_LOG=debug rustconn 2> rustconn.log

# Module-specific
RUST_LOG=rustconn_core::connection=debug rustconn
RUST_LOG=rustconn_core::secret=debug rustconn
```

---

## Support

- **GitHub:** https://github.com/totoshko88/RustConn
- **Issues:** https://github.com/totoshko88/RustConn/issues
- **Releases:** https://github.com/totoshko88/RustConn/releases

**Made with ❤️ in Ukraine 🇺🇦**
