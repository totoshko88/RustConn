# RustConn User Guide

**Version 0.10.9** | GTK4/libadwaita Connection Manager for Linux

RustConn is a modern connection manager designed for Linux with Wayland-first approach. It supports SSH, RDP, VNC, SPICE, MOSH, SFTP, Telnet, Serial, Kubernetes protocols and Zero Trust integrations through a native GTK4/libadwaita interface.

## Table of Contents

1. [Getting Started](#getting-started)
2. [Main Interface](#main-interface)
3. [Connections](#connections)
4. [Groups](#groups)
5. [Sessions](#sessions)
6. [Zero Trust Providers](#zero-trust-providers)
7. [Templates](#templates)
8. [Snippets](#snippets)
9. [Clusters](#clusters)
10. [Import/Export](#importexport)
11. [Tools](#tools)
    - [Global Variables](#global-variables)
    - [Password Generator](#password-generator)
    - [Connection History](#connection-history)
    - [Connection Statistics](#connection-statistics)
    - [Wake-on-LAN](#wake-on-lan)
    - [Flatpak Components](#flatpak-components)
12. [Settings](#settings)
13. [Backup & Restore](#backup--restore)
14. [Startup Action](#startup-action)
15. [Command Palette](#command-palette)
16. [Favorites](#favorites)
17. [Tab Coloring](#tab-coloring)
18. [Tab Grouping](#tab-grouping)
19. [Custom Icons](#custom-icons)
20. [Remote Monitoring](#remote-monitoring)
21. [Custom Keybindings](#custom-keybindings)
22. [Adaptive UI](#adaptive-ui)
23. [Encrypted Documents](#encrypted-documents)
24. [RDP File Association](#rdp-file-association)
25. [Keyboard Shortcuts](#keyboard-shortcuts)
26. [MOSH Protocol](#mosh-protocol)
27. [CSV Import/Export](#csv-importexport)
28. [Session Recording](#session-recording)
29. [Text Highlighting Rules](#text-highlighting-rules)
30. [Ad-hoc Broadcast](#ad-hoc-broadcast)
31. [Smart Folders](#smart-folders)
32. [Script Credentials](#script-credentials)
33. [Per-connection Terminal Theming](#per-connection-terminal-theming)
34. [CLI Usage](#cli-usage)
35. [Configuration Sync Between Machines](#configuration-sync-between-machines)
36. [Frequently Asked Questions](#frequently-asked-questions)
37. [Migration Guide](#migration-guide)
38. [Troubleshooting](#troubleshooting)
39. [Security Best Practices](#security-best-practices)

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
3. Select protocol (SSH, RDP, VNC, SPICE, MOSH, Telnet, Serial, Kubernetes)
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
- **Sidebar** — Connection tree with groups (alphabetically sorted, collapsible via F9 or on narrow windows)
- **Sidebar Toolbar** — Delete, Add Group, Group Operations, Sort, Import, Export, KeePass status
- **Session Area** — Active sessions in tabs
- **Toast Overlay** — Non-blocking notifications

### Quick Filter

Filter connections by protocol using the filter bar below search:
- Click protocol buttons (SSH, RDP, VNC, SPICE, Telnet, K8s, ZeroTrust)
- Multiple protocols can be selected (OR logic)
- Clear search field to reset filters

### Password Vault Button

Shows integration status in sidebar toolbar:
- **Highlighted** — Password manager enabled and configured
- **Dimmed** — Disabled or not configured
- Click to open appropriate password manager:
  - KeePassXC/GNOME Secrets for KeePassXC backend (in Flatpak, launches KeePassXC on the host via `flatpak-spawn`)
  - Seahorse/GNOME Settings for libsecret backend
  - Bitwarden web vault for Bitwarden backend
  - 1Password app for 1Password backend

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
- Password source selection:
  - **Prompt** — Ask for password on each connection
  - **Vault** — Store/retrieve from configured secret backend (KeePassXC, libsecret, Bitwarden, 1Password, Passbolt)
  - **Variable** — Read credentials from a named secret global variable
  - **Inherit** — Use credentials from parent group
  - **Script** — Resolve password from an external command (see [Script Credentials](#script-credentials))
  - **None** — No password (key-based auth)
- SSH key selection
- Key passphrase

**Protocol Tabs** (varies by protocol):

| Protocol | Options |
|----------|---------|
| SSH | Auth method (password, publickey, keyboard-interactive, agent, security-key/FIDO2), key source (default/file/agent), proxy jump (Jump Host), ProxyJump, IdentitiesOnly, ControlMaster, agent forwarding, Waypipe (Wayland forwarding), X11 forwarding, compression, startup command, custom SSH options, port forwarding (local/remote/dynamic) |
| RDP | Client mode (embedded/external), performance mode (quality/balanced/speed), resolution, color depth, display scale override, audio redirection, RDP gateway (host, port, username), keyboard layout, disable NLA, clipboard sharing, shared folders, custom FreeRDP arguments |
| VNC | Client mode (embedded/external), performance mode (quality/balanced/speed), encoding (Auto/Tight/ZRLE/Hextile/Raw/CopyRect), compression level, quality level, display scale override, view-only mode, scaling, clipboard sharing, custom arguments |
| SPICE | TLS encryption, CA certificate (with inline validation), skip certificate verification, USB redirection, clipboard sharing, image compression (Auto/Off/GLZ/LZ/QUIC), proxy URL, shared folders |
| MOSH | Predict mode (Adaptive/Always/Never), SSH port, UDP port range, server binary path, custom arguments |
| Telnet | Custom arguments, backspace key behavior, delete key behavior |
| Serial | Device path, baud rate, data bits, stop bits, parity, flow control, custom picocom arguments |
| Kubernetes | Kubeconfig path, context, namespace, pod, container, shell, busybox mode, busybox image, custom kubectl arguments |
| ZeroTrust | Provider-specific (AWS SSM, GCP IAP, Azure Bastion, Azure SSH, OCI Bastion, Cloudflare Access, Teleport, Tailscale SSH, HashiCorp Boundary, Hoop.dev, Generic Command), custom CLI arguments |

**Security Key / FIDO2 Authentication (SSH):**
SSH connections support hardware security keys (YubiKey, SoloKey, etc.) via the `security-key` auth method. Requirements:
- OpenSSH 8.2+ on both client and server
- `libfido2` installed on the client (`sudo apt install libfido2-1`)
- An `ed25519-sk` or `ecdsa-sk` key generated with `ssh-keygen -t ed25519-sk`
- The key file path configured in the connection's SSH key field

**Advanced Tabs:**
- **Advanced** — Window mode (Embedded/External/Fullscreen), remember window position, hide local cursor (embedded RDP/VNC/SPICE), Wake-on-LAN configuration (MAC address, broadcast, port, wait time), monitoring override (enable/disable/global, custom polling interval)
- **Automation** — Expect rules for auto-responding to terminal patterns, pattern tester with built-in templates (Sudo, SSH Host Key, Login, etc.), pre-connect task, post-disconnect task (with conditions: first/last connection only)
- **Data** — Local variables (connection-scoped, override global variables), custom properties (Text/URL/Protected metadata)
- **Logging** — Session logging (enable/disable, log path template with variables, timestamp format, max file size, retention days, granular content options: log activity, log input, log output, add timestamps)

### Automation (Expect Rules)

Expect rules automate interactive prompts during connection. Each rule matches a pattern in terminal output and sends a response.

**Configure Expect Rules:**
1. Edit connection → **Automation** tab
2. Click **Add Rule**
3. Enter pattern (text or regex) and response
4. Set priority (lower number = higher priority)
5. Use the **Test** button to verify pattern matching

**Examples:**
| Pattern | Response | Use Case |
|---------|----------|----------|
| `password:` | `${password}` | Auto-login with vault password |
| `\[sudo\] password` | `${password}` | Sudo password prompt |
| `Are you sure.*continue` | `yes` | SSH host key confirmation |
| `Select option:` | `2` | Menu navigation |

Rules execute in priority order. After matching, the response is sent followed by Enter.

### Pre/Post Connection Tasks

Run commands automatically before connecting or after disconnecting.

**Configure Tasks:**
1. Edit connection → **Tasks** tab
2. Add a **Pre-connect** task (runs before the connection is established)
3. Add a **Post-disconnect** task (runs after the session ends)
4. Set the command and optional working directory

**Examples:**
- Pre-connect: `nmcli con up VPN-Work` (connect VPN before SSH)
- Pre-connect: `ssh-add ~/.ssh/special_key` (load a specific key)
- Post-disconnect: `nmcli con down VPN-Work` (disconnect VPN after session)
- Post-disconnect: `notify-send "Session ended"` (desktop notification)

### Custom Properties

Add arbitrary key-value metadata to connections for organization and scripting.

1. Edit connection → **Advanced** tab → **Custom Properties** section
2. Click **Add Property**
3. Enter key and value (e.g., `environment` = `production`, `team` = `backend`)
4. Properties are searchable and visible in connection details

### Quick Connect (Ctrl+Shift+Q)

Temporary connection without saving:
- Supports SSH, RDP, VNC, Telnet
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
| Delete | Delete key or right-click → Delete (moves to Trash) |
| Move to Group | Drag-drop or right-click → Move to Group |

### Undo/Trash Functionality

Deleted items are moved to Trash and can be restored:
- After deleting, an "Undo" notification appears
- Click "Undo" to restore the deleted item
- Trash is persisted across sessions for recovery

### Test Connection

In connection dialog, click **Test** to verify connectivity before saving.

### Pre-connect Port Check

For RDP, VNC, and SPICE connections, RustConn performs a fast TCP port check before connecting:
- Provides faster feedback (2-3s vs 30-60s timeout) when hosts are unreachable
- Configurable globally in Settings → Connection page
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
- **Delete** — Delete key (shows choice dialog: Keep Connections, Delete All, or Cancel)

### Group Operations Mode (Bulk Actions)

The sidebar toolbar has a **list icon** button (view-list-symbolic) that activates Group Operations Mode for bulk actions on multiple connections at once.

**Activate:** Click the list icon in the sidebar toolbar (or right-click → Group Operations)

**Available actions in the toolbar:**

| Button | Action |
|--------|--------|
| New Group | Create a new group |
| Move to Group | Move all selected connections to a chosen group |
| Select All | Select all visible connections |
| Clear | Deselect all |
| Delete | Delete all selected connections (with confirmation) |

**Workflow:**

1. Click the list icon to enter Group Operations Mode
2. Checkboxes appear next to each connection in the sidebar
3. Select individual connections by clicking their checkboxes, or use **Select All**
4. Choose an action: **Move to Group** or **Delete**
5. Confirm the action in the dialog
6. Click the list icon again (or press Escape) to exit Group Operations Mode

This is useful for reorganizing large numbers of connections, cleaning up after an import, or bulk-deleting obsolete entries.

### Group Credentials

Groups can store default credentials (Username, Password, Domain) that are inherited by their children.

**Configure Group Credentials:**
1. In "New Group" or "Edit Group" dialog, fill in the **Default Credentials** section
2. Select **Password Source**:
   - **KeePass** — Store in KeePass database (hierarchical: `RustConn/Groups/{path}`)
   - **Keyring** — Store in system keyring (libsecret)
   - **Bitwarden** — Store in Bitwarden vault
3. Click the **folder icon** next to password field to load existing password from vault
4. Password source auto-selects based on your preferred backend in Settings

**Inherit Credentials:**
1. Create a connection inside the group
2. In **Authentication** tab, set **Password Source** to **Inherit from Group**
3. Connection will use group's stored credentials
4. Use **"Load from Group"** buttons to auto-fill Username and Domain from parent group

**KeePass Hierarchy:**
Group credentials are stored in KeePass with hierarchical paths:
```
RustConn/
└── Groups/
    ├── Production/           # Group password
    │   └── Web Servers/      # Nested group password
    └── Development/
        └── Local/
```

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
| RDP | Embedded IronRDP or external FreeRDP (bundled in Flatpak) |

**RDP HiDPI Support:** On HiDPI/4K displays, the embedded IronRDP client automatically sends the correct scale factor to the Windows server (e.g. 200% on a 2× display), so remote UI elements render at the correct logical size. The Scale Override setting in the connection dialog allows manual adjustment if needed.

**RDP Clipboard:** The embedded IronRDP client provides bidirectional clipboard sync via the CLIPRDR channel. Text copied on the remote desktop is automatically available locally (Ctrl+V), and local clipboard changes are announced to the server. The Copy/Paste toolbar buttons remain available as manual fallback. Clipboard sync requires the "Clipboard" option enabled in the RDP connection settings.

**RDP Quick Actions:** The embedded RDP toolbar includes a Quick Actions dropdown menu for launching common Windows administration tools on the remote desktop. Actions send scancode key sequences directly through the RDP session with a 30ms inter-key delay for reliability.

| Action | Shortcut Sent | Description |
|--------|---------------|-------------|
| Task Manager | Ctrl+Shift+Esc | Opens Windows Task Manager |
| Settings | Win+I | Opens Windows Settings |
| PowerShell | Win+R → `powershell` | Launches PowerShell via Run dialog |
| CMD | Win+R → `cmd` | Launches Command Prompt via Run dialog |
| Event Viewer | Win+R → `eventvwr` | Opens Event Viewer via Run dialog |
| Services | Win+R → `services.msc` | Opens Services console via Run dialog |

The Quick Actions menu is accessible via the dropdown button (arrow icon) on the RDP toolbar. All labels are translatable.

**Hide Local Cursor:** Embedded RDP, VNC, and SPICE viewers support hiding the local OS cursor to eliminate the "double cursor" effect (local + remote cursor visible simultaneously). Toggle "Show Local Cursor" in the connection dialog's Features section. Enabled by default for backward compatibility.
| VNC | Embedded vnc-rs or external TigerVNC |
| SPICE | Embedded spice-client or external remote-viewer |
| MOSH | MOSH via VTE terminal (external `mosh` client) |
| Telnet | Embedded VTE terminal tab (external `telnet` client) |
| Serial | Embedded VTE terminal tab (external `picocom` client) |
| Kubernetes | Embedded VTE terminal tab (external `kubectl exec`) |
| ZeroTrust | Provider CLI in terminal |
| Local Shell | Local VTE terminal tab (user's default shell) |

**Local Shell:** Open a local terminal tab without connecting to any remote host. Useful as a quick terminal emulator or for running local commands alongside remote sessions. Start via Menu → File → Local Shell, the startup action (Settings → Interface → Startup → Local Shell), or `rustconn --shell`.

### Display Mode (Window Mode)

The **Display Mode** setting in the connection dialog (Advanced tab → Window Mode) controls how RDP and VNC sessions are displayed. The setting applies per-connection.

| Mode | RDP Behavior | VNC Behavior |
|------|-------------|-------------|
| **Embedded** (default) | IronRDP widget in a notebook tab | vnc-rs widget in a notebook tab |
| **Fullscreen** | Maximizes the main window | Maximizes the main window |
| **External Window** | Launches `xfreerdp` in a separate window | Launches external VNC viewer (TigerVNC/vncviewer) in a separate window |

**Configure:**
1. Edit connection → **Advanced** tab → **Window Mode** section
2. Select **Embedded**, **External Window**, or **Fullscreen** from the dropdown
3. For External Window mode, enable **Remember Position** to save window geometry between sessions (RDP only)

**Notes:**
- Fullscreen mode maximizes the RustConn window, not the remote desktop. Use F11 to toggle true fullscreen of the entire application.
- External Window mode for VNC requires an external VNC viewer installed (TigerVNC, vncviewer, gvncviewer, or similar). If no viewer is found, a toast notification shows the install hint.
- External Window mode for RDP uses FreeRDP. In the Flatpak build, FreeRDP (SDL3 client) is bundled — no separate installation needed. On native installs, RustConn auto-detects available FreeRDP variants in priority order: `wlfreerdp3` > `wlfreerdp` > `sdl-freerdp3` > `sdl-freerdp` > `xfreerdp3` > `xfreerdp`.
- The VNC protocol tab also has its own **Client Mode** (Embedded/External) setting. When Display Mode is set to External Window, it takes precedence over the protocol-level Client Mode.

### Tab Management

- **Switch** — Click tab or Ctrl+Tab / Ctrl+Shift+Tab
- **Close** — Click X or Ctrl+Shift+W
- **Reorder** — Drag tabs

### Split View

Split view works with terminal-based sessions: SSH, Telnet, Serial, Kubernetes, Local Shell, and SFTP (mc mode).

- **Horizontal Split** — Ctrl+Shift+H
- **Vertical Split** — Ctrl+Shift+S
- **Close Pane** — Ctrl+Shift+X
- **Focus Next Pane** — Ctrl+`

### Status Indicators

Sidebar shows connection status:
- 🟢 Green dot — Connected
- 🔴 Red dot — Disconnected

### Session Restore

Enable in Settings → Interface page → Session Restore:
- Sessions saved on app close
- Restored on next startup
- Optional prompt before restore
- Configurable maximum age

### Session Reconnect

When a terminal session disconnects (SSH, Telnet, Serial, Kubernetes), a "Reconnect" banner appears at the top of the terminal tab. Click it to re-establish the connection in one click without opening the connection dialog.

- The banner appears automatically when the VTE child process exits
- Reconnect uses the same connection settings (host, credentials, protocol options)
- If the connection fails again, the banner reappears
- Close the tab normally with Ctrl+Shift+W to dismiss

### Session Logging

Three logging modes (Settings → Terminal page → Logging):
- **Activity** — Track session activity changes
- **User Input** — Capture typed commands
- **Terminal Output** — Full transcript

Optional timestamps (Settings → Terminal page → Logging):
- Enable "Timestamps" to prepend `[HH:MM:SS]` to each line in log files

Per-connection logging options (Connection dialog → Logging tab → Content Options):
- **Log Activity** — Record connection and disconnection events
- **Log Input** — Record keyboard input sent to remote
- **Log Output** — Record terminal output from remote
- **Add Timestamps** — Prepend timestamp to each log line

### Terminal Search

Open with **Ctrl+Shift+F** in any terminal session.

- **Text search** — Plain text matching (default)
- **Regex** — Toggle "Regex" checkbox for regular expression patterns; invalid patterns show an error message
- **Case sensitive** — Toggle case sensitivity
- **Highlight All** — Highlights all matches in the terminal (enabled by default)
- **Navigation** — Up/Down buttons or Enter to jump between matches; search wraps around
- Highlights are cleared automatically when closing the dialog (Close button or Escape)

Note: Terminal search is a GUI-only feature (VTE widget). Not available in CLI mode.

### Serial Console

Connect to serial devices (routers, switches, embedded boards) via `picocom`.

**Create a Serial Connection:**
1. Press **Ctrl+N** → select **Serial** protocol
2. Enter device path (e.g., `/dev/ttyUSB0`) or click **Detect Devices** to auto-scan `/dev/ttyUSB*`, `/dev/ttyACM*`, `/dev/ttyS*`
3. Configure baud rate (default: 115200), data bits, stop bits, parity, flow control
4. Click **Create**
5. Double-click to connect

**Serial Parameters:**

| Parameter | Options | Default |
|-----------|---------|---------|
| Baud Rate | 9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600 | 115200 |
| Data Bits | 5, 6, 7, 8 | 8 |
| Stop Bits | 1, 2 | 1 |
| Parity | None, Odd, Even | None |
| Flow Control | None, Hardware (RTS/CTS), Software (XON/XOFF) | None |

**Device Access (Linux):**
Serial devices require `dialout` group membership:
```bash
sudo usermod -aG dialout $USER
# Log out and back in for the change to take effect
```

**Flatpak:** Serial access works automatically (`--device=all` permission). `picocom` is bundled in the Flatpak package.

**Snap:** Connect the serial-port interface after installation:
```bash
sudo snap connect rustconn:serial-port
```
`picocom` is bundled in the Snap package.

**CLI:**
```bash
# Create a serial connection (--device is an alias for --host with serial protocol)
rustconn-cli add --name "Router" --protocol serial --device /dev/ttyUSB0 --baud-rate 9600

# Or use --host directly
rustconn-cli add --name "Switch" --protocol serial --host /dev/ttyACM0 --baud-rate 115200

# Connect (launches picocom with configured parameters)
rustconn-cli connect "Router"

# Update serial-specific settings
rustconn-cli update "Router" --device /dev/ttyACM1 --baud-rate 19200

# Dry-run to see the picocom command
rustconn-cli connect "Router" --dry-run
```

### SSH Port Forwarding

Forward TCP ports through SSH tunnels. Three modes are supported:

| Mode | SSH Flag | Description |
|------|----------|-------------|
| Local (`-L`) | `-L local_port:remote_host:remote_port` | Forward a local port to a remote destination through the tunnel |
| Remote (`-R`) | `-R remote_port:local_host:local_port` | Forward a remote port back to a local destination |
| Dynamic (`-D`) | `-D local_port` | SOCKS proxy on a local port |

**Configure Port Forwarding:**
1. Edit an SSH connection → **SSH** tab
2. Scroll to **Port Forwarding** section
3. Click **Add Rule**
4. Select direction (Local, Remote, Dynamic)
5. Enter local port, remote host, and remote port (remote host/port not needed for Dynamic)
6. Add multiple rules as needed
7. Click **Save**

**Examples:**
- Local: forward local port 8080 to remote `db-server:5432` → access the database at `localhost:8080`
- Remote: expose local port 3000 on the remote server's port 9000
- Dynamic: create a SOCKS proxy on local port 1080

**Import Support:**
Port forwarding rules are automatically imported from:
- SSH config (`LocalForward`, `RemoteForward`, `DynamicForward` directives)
- Remmina SSH profiles
- Asbru-CM configurations
- MobaXterm sessions

### SSH Session Options

The SSH tab in the connection dialog contains session-level toggles that control how the SSH connection behaves. These are in the **Session** options group.

| Option | SSH Flag | Description |
|--------|----------|-------------|
| Agent Forwarding | `-A` | Forward your local SSH agent to the remote host, allowing key-based authentication to further servers without copying keys |
| X11 Forwarding | `-X` | Forward X11 display to your local machine — run graphical X11 apps on the remote host and see them locally |
| Compression | `-C` | Compress the SSH data stream — useful on slow or high-latency connections |
| Connection Multiplexing | `ControlMaster=auto` | Reuse a single TCP connection for multiple SSH sessions to the same host. Subsequent connections open instantly without re-authenticating. RustConn adds `ControlPersist=10m` so the master connection stays alive for 10 minutes after the last session closes |
| Waypipe | `waypipe ssh ...` | Forward Wayland GUI applications (see [Waypipe](#waypipe-wayland-forwarding) below) |

**Configure:**
1. Edit an SSH connection → **SSH** tab
2. Scroll to the **Session** group
3. Toggle the desired options
4. Click **Save**

All toggles are off by default. They can be combined freely — for example, enabling both Agent Forwarding and Compression at the same time adds `-A -C` to the SSH command.

### SSH Custom Options

Pass arbitrary `-o` options to the SSH command. This is for advanced SSH configuration that doesn't have a dedicated UI toggle.

**Configure:**
1. Edit an SSH connection → **SSH** tab → **Session** group
2. In the **Custom Options** field, enter comma-separated `Key=Value` pairs

**Format:** `Key1=Value1, Key2=Value2`

You can also paste options in the `-o Key=Value` format directly from the command line — the `-o` prefix is stripped automatically.

**Examples:**

| Custom Options field | Resulting SSH flags |
|---------------------|---------------------|
| `StrictHostKeyChecking=no, ServerAliveInterval=60` | `-o StrictHostKeyChecking=no -o ServerAliveInterval=60` |
| `-o StrictHostKeyChecking=no, -o ServerAliveInterval=60` | Same result (prefix stripped) |
| `ServerAliveCountMax=3` | `-o ServerAliveCountMax=3` |
| `ProxyCommand=nc -X 5 -x proxy:1080 %h %p` | `-o ProxyCommand=nc -X 5 -x proxy:1080 %h %p` |

**Note:** For port forwarding (`-L`, `-R`, `-D`), use the dedicated **Port Forwarding** section instead of Custom Options. The subtitle in the dialog reminds you of this.

**Dangerous directives** (`ProxyCommand`, `LocalCommand`, `PermitLocalCommand`) are filtered for security — they are logged as warnings but still passed through if explicitly set.

### Startup Command

Run a command automatically after the SSH connection is established.

**Configure:**
1. Edit an SSH connection → **SSH** tab → **Session** group
2. Enter the command in the **Startup Command** field

The command is appended to the SSH invocation and executes in the remote shell immediately after login.

**Examples:**
- `htop` — open system monitor on connect
- `cd /var/log && tail -f syslog` — jump to logs
- `tmux attach || tmux new` — attach to or create a tmux session

### Waypipe (Wayland Forwarding)

Waypipe forwards Wayland GUI applications from a remote host to your local
Wayland session — the Wayland equivalent of X11 forwarding (`ssh -X`).
When enabled, RustConn wraps the SSH command as `waypipe ssh user@host`,
creating a transparent Wayland proxy between the machines.

**Requirements:**

- `waypipe` installed on **both** local and remote hosts
  (`sudo apt install waypipe` / `sudo dnf install waypipe`)
- A running **Wayland** session locally (not X11)
- The remote host does not need a running display server

**Setup:**

1. Open the connection dialog for an SSH connection
2. In the **Session** options group, enable the **Waypipe** checkbox
3. Save and connect

RustConn will execute `waypipe ssh user@host` (with automatic password injection
for vault-authenticated connections). If `waypipe` is not found on PATH, the
connection falls back to a standard SSH session with a log warning.

You can verify waypipe availability in **Settings → Clients**.

**Example — running a remote GUI application:**

After connecting with Waypipe enabled, launch any Wayland-native application
in the SSH terminal:

```bash
# Run Firefox from the remote host — the window appears on your local desktop
firefox &

# Run a file manager
nautilus &

# Run any GTK4/Qt6 Wayland app
gnome-text-editor &
```

The remote application window opens on your local Wayland desktop as if it
were a local window. Clipboard, keyboard input, and window resizing work
transparently.

**Tips:**

- The remote application must support Wayland natively. X11-only apps will
  not work through waypipe (use X11 Forwarding for those).
- For best performance over slow links, waypipe compresses the Wayland
  protocol traffic automatically. You can pass extra flags via SSH custom
  options if needed (e.g., `--compress=lz4`).
- If the remote host uses GNOME, most bundled apps (Files, Text Editor,
  Terminal, Eye of GNOME) work out of the box.
- Qt6 apps work if `QT_QPA_PLATFORM=wayland` is set on the remote host.
- To check which display protocol your local session uses:
  `echo $XDG_SESSION_TYPE` (should print `wayland`).

### Kubernetes Shell

Connect to Kubernetes pods via `kubectl exec -it`. Two modes: exec into an existing pod, or launch a temporary busybox pod.

**Create a Kubernetes Connection:**
1. Press **Ctrl+N** → select **Kubernetes** protocol
2. Configure kubeconfig path (optional, defaults to `~/.kube/config`)
3. Set context, namespace, pod name, container (optional), and shell (default: `/bin/sh`)
4. Optionally enable **Busybox mode** to launch a temporary pod instead
5. Click **Create**
6. Double-click to connect

**Kubernetes Parameters:**

| Parameter | Description | Default |
|-----------|-------------|---------|
| Kubeconfig | Path to kubeconfig file | `~/.kube/config` |
| Context | Kubernetes context | Current context |
| Namespace | Target namespace | `default` |
| Pod | Pod name to exec into | Required (exec mode) |
| Container | Container name (multi-container pods) | Optional |
| Shell | Shell to use | `/bin/sh` |
| Busybox | Launch temporary busybox pod | Off |

**Requirements:** `kubectl` must be installed and configured.

**Flatpak:** kubectl is available as a downloadable component in Flatpak Components dialog.

**CLI:**

The CLI supports creating and connecting to Kubernetes connections. Kubernetes-specific parameters (namespace, pod, container, shell, busybox mode) are configured through the GUI — the CLI `add` command creates a connection with default Kubernetes settings that you can then customize in the GUI.

```bash
# Create a Kubernetes connection (configure pod/namespace in GUI)
rustconn-cli add --name "K8s Pod" --protocol kubernetes --host ""

# Connect (launches kubectl exec with configured parameters)
rustconn-cli connect "K8s Pod"

# Dry-run to see the kubectl command
rustconn-cli connect "K8s Pod" --dry-run
```

For full control over Kubernetes parameters (namespace, pod, container, shell, busybox mode, kubeconfig path), use the GUI connection dialog.

### SFTP File Browser

Browse remote files on SSH connections via your system file manager or Midnight Commander.

SFTP is always available for SSH connections — no checkbox or flag needed. The "Open SFTP" option only appears in the sidebar context menu for SSH connections (not RDP, VNC, SPICE, or Serial).

**SSH Key Handling:**
Before opening SFTP, RustConn automatically runs `ssh-add` with your configured SSH key. This is required because neither file managers nor mc can pass identity files directly — the key must be in the SSH agent.

**Open SFTP (File Manager):**
- Right-click an SSH connection in sidebar → "Open SFTP"
- Or use the `win.open-sftp` action while a connection is selected

RustConn tries file managers in this order: `dolphin` (KDE), `nautilus` (GNOME), `xdg-open` (fallback). The `SSH_AUTH_SOCK` environment variable is injected into the spawned process so the file manager can access the SSH agent.

On KDE, if `dolphin` is not found (e.g., in Flatpak), `xdg-open` is used — which opens whichever application is registered as the `sftp://` handler. See [SFTP Troubleshooting](#sftp-troubleshooting) if the wrong application opens.

**SFTP via Midnight Commander:**

Settings → Terminal page → Behavior → enable "SFTP via mc". When enabled, "Open SFTP" opens a local shell tab with Midnight Commander connected to the remote server via `sh://user@host:port` FISH VFS panel.

Requirements for mc mode:
- Midnight Commander must be installed (`mc` in PATH). RustConn checks availability before launch.
- mc FISH VFS requires SSH key authentication — password and keyboard-interactive auth are not supported. A warning toast is shown if password auth is configured.
- In Flatpak builds, mc 4.8.32 is bundled automatically.

mc-based SFTP sessions run in a VTE terminal, so they support split view (Ctrl+Shift+H / Ctrl+Shift+S) just like SSH tabs.

**CLI:**
```bash
# Open file manager with sftp:// URI (uses xdg-open, falls back to nautilus)
rustconn-cli sftp "My Server"

# Use terminal sftp client instead
rustconn-cli sftp "My Server" --cli

# Open via Midnight Commander
rustconn-cli sftp "My Server" --mc
```

### SFTP as Connection Type

SFTP can also be created as a standalone connection type. This is useful when you primarily need file transfer access to a server (e.g., transferring files between Windows and Linux systems).

**Create an SFTP Connection:**
1. Press **Ctrl+N** → select **SFTP** protocol
2. Configure SSH settings (host, port, username, key) — SFTP reuses the SSH options tab
3. Click **Create**
4. Double-click to connect — opens file manager (or mc) directly instead of a terminal

SFTP connections use the `folder-remote-symbolic` icon in the sidebar and behave identically to the "Open SFTP" action on SSH connections, but the file manager opens automatically on Connect.

**CLI:**
```bash
rustconn-cli add --name "File Server" --host files.example.com --protocol sftp --username admin
rustconn-cli connect "File Server"
```

### SFTP Troubleshooting

**Choosing the Default SFTP Client (KDE / GNOME / other):**

RustConn opens `sftp://` URIs via `xdg-open`, which delegates to your desktop's default handler. On KDE, if FileZilla is installed, it may register itself as the default `sftp://` handler instead of Dolphin.

To set Dolphin (recommended for SSH key support):

```bash
# Option 1: edit mimeapps.list directly
# Add this line under [Default Applications] in ~/.config/mimeapps.list:
x-scheme-handler/sftp=org.kde.dolphin.desktop

# Option 2: xdg-mime (requires qt6-tools / qttools installed)
xdg-mime default org.kde.dolphin.desktop x-scheme-handler/sftp
```

If `xdg-mime` fails with "qtpaths: command not found", use Option 1 or install `qt6-qttools` (`sudo dnf install qt6-qttools` / `sudo apt install qt6-tools-dev`).

On GNOME, Nautilus handles `sftp://` by default — no changes needed.

**FileZilla Does Not Support SSH Agent:**

FileZilla uses its own SSH library and ignores the system SSH agent (`SSH_AUTH_SOCK`). Even though RustConn adds your key to the agent before opening SFTP, FileZilla will still prompt for a password.

Solutions:
- Switch the `sftp://` handler to Dolphin or Nautilus (see above) — both use OpenSSH and respect the SSH agent
- Configure the key directly in FileZilla: Site Manager → SFTP tab → Key file
- Use mc mode in RustConn (Settings → Terminal → SFTP via mc) — mc runs in the same process and inherits the agent

**Flatpak: File Manager Cannot Access SSH Key:**

In Flatpak builds, RustConn runs inside a sandbox with its own SSH agent. When `xdg-open` launches a file manager (Dolphin, Nautilus), it runs outside the sandbox and uses the host's SSH agent — which does not have the key that RustConn added.

**Flatpak: SSH Key Paths and Document Portal:**

When you select an SSH key via the file chooser in Flatpak, the system creates a temporary document portal path (e.g., `/run/user/1000/doc/XXXXXXXX/key.pem`). These paths become stale after Flatpak rebuilds or reboots. RustConn automatically copies selected keys to a stable location (`~/.var/app/io.github.totoshko88.RustConn/.ssh/`) with correct permissions (0600). At connect time, stale portal paths are resolved via fallback lookup in this directory.

Solutions for file manager SFTP (pick one):
1. **Use mc mode** (recommended) — Settings → Terminal → SFTP via mc. Midnight Commander runs inside the Flatpak sandbox and inherits RustConn's SSH agent. Works without any extra setup. This is enabled by default in Flatpak builds.
2. **Add the key on the host** — run `ssh-add ~/.ssh/your_key` in a regular terminal before opening SFTP. The file manager will then find the key in the host agent.
3. **Store keys in `~/.ssh/`** — keys in `~/.ssh/` are accessible to both the Flatpak sandbox and the host.

This limitation does not affect native packages (deb, rpm, Snap) where RustConn and the file manager share the same SSH agent.

---

## Zero Trust Providers

RustConn supports connecting through identity-aware proxy services (Zero Trust). Instead of direct SSH/RDP to a host, the connection is tunneled through a provider's CLI tool that handles authentication and authorization.

### Setup

1. Create or edit a connection (Ctrl+N or Ctrl+E)
2. Go to the **Zero Trust** tab
3. Select your provider from the dropdown
4. Fill in the provider-specific fields
5. Optionally add custom CLI arguments in the **Advanced** section

The Zero Trust tab is available for SSH connections. RustConn constructs the appropriate CLI command and runs it in a VTE terminal.

When selecting a provider, RustConn checks if the required CLI tool is available on PATH. If not found, a warning is displayed with instructions to install the tool or use Flatpak Components.

### Providers

#### AWS Session Manager

Connects via `aws ssm start-session`. Requires the AWS CLI and Session Manager plugin.

| Field | Description | Example |
|-------|-------------|---------|
| Instance ID | EC2 instance ID | `i-0abc123def456` |
| AWS Profile | Named profile from `~/.aws/credentials` | `default`, `production` |
| Region | AWS region | `us-east-1` |

**Prerequisites:** `aws` CLI, `session-manager-plugin`, configured AWS credentials.

#### GCP IAP Tunnel

Connects via `gcloud compute ssh --tunnel-through-iap`. Requires the Google Cloud SDK.

| Field | Description | Example |
|-------|-------------|---------|
| Instance Name | Compute Engine VM name | `web-server-01` |
| Zone | GCP zone | `us-central1-a` |
| Project | GCP project ID | `my-project-123` |

**Prerequisites:** `gcloud` CLI, authenticated (`gcloud auth login`), IAP-enabled firewall rule.

**Flatpak:** The Flatpak sandbox mounts `~/.config/gcloud/` as read-only to share your host credentials. RustConn automatically redirects gcloud's writable config to `~/.var/app/io.github.totoshko88.RustConn/config/gcloud/` via the `CLOUDSDK_CONFIG` environment variable. On first use, credential files are bootstrapped from the host mount.

If you installed gcloud via Flatpak Components and haven't authenticated on the host, run inside a RustConn Local Shell:
```bash
gcloud auth login
gcloud config set project YOUR_PROJECT_ID
```

If gcloud was already configured on the host before installing the Flatpak, credentials are copied automatically and no extra steps are needed.

#### Azure Bastion

Connects via `az network bastion ssh`. Requires the Azure CLI with bastion extension.

| Field | Description | Example |
|-------|-------------|---------|
| Target Resource ID | Full ARM resource ID of the target VM | `/subscriptions/.../vm-name` |
| Resource Group | Resource group containing the Bastion | `my-rg` |
| Bastion Name | Name of the Bastion host | `my-bastion` |

**Prerequisites:** `az` CLI, `az extension add --name bastion`, authenticated (`az login`).

**Flatpak:** The Flatpak sandbox mounts `~/.azure/` as read-only to share your host credentials. RustConn automatically redirects Azure CLI's writable config to `~/.var/app/io.github.totoshko88.RustConn/config/azure/` via the `AZURE_CONFIG_DIR` environment variable. On first use, credential files (`azureProfile.json`, `msal_token_cache.json`, etc.) are bootstrapped from the host mount.

#### Azure SSH (AAD)

Connects via `az ssh vm` using Azure Active Directory authentication. No SSH keys needed.

| Field | Description | Example |
|-------|-------------|---------|
| VM Name | Azure VM name | `my-vm` |
| Resource Group | Resource group containing the VM | `my-rg` |

**Prerequisites:** `az` CLI, `az extension add --name ssh`, AAD-enabled VM, authenticated.

**Flatpak:** Same as Azure Bastion — `AZURE_CONFIG_DIR` is redirected automatically.

#### OCI Bastion

Connects via Oracle Cloud Infrastructure Bastion service.

| Field | Description | Example |
|-------|-------------|---------|
| Bastion OCID | OCID of the Bastion resource | `ocid1.bastion.oc1...` |
| Target OCID | OCID of the target compute instance | `ocid1.instance.oc1...` |
| Target IP | Private IP of the target | `10.0.1.5` |
| SSH Public Key | Path to SSH public key for managed SSH session | `~/.ssh/id_rsa.pub` |
| Session TTL | Session duration in seconds (default: 1800) | `3600` |

**Prerequisites:** `oci` CLI, configured OCI credentials (`~/.oci/config`).

**Flatpak:** The host `~/.oci/` directory is not mounted. RustConn redirects the OCI config file to `~/.var/app/io.github.totoshko88.RustConn/config/oci/config` via the `OCI_CLI_CONFIG_FILE` environment variable. You need to configure OCI CLI from a RustConn Local Shell after installing it via Flatpak Components.

#### Cloudflare Access

Connects through Cloudflare Zero Trust tunnel.

| Field | Description | Example |
|-------|-------------|---------|
| Hostname | Cloudflare Access hostname | `ssh.example.com` |

**Prerequisites:** `cloudflared` installed, Cloudflare Access application configured for the hostname.

**Flatpak:** Cloudflare Access SSH uses browser-based authentication with short-lived tokens — no persistent config directory is needed for the SSH proxy use case. Install `cloudflared` via Flatpak Components.

#### Teleport

Connects via Gravitational Teleport.

| Field | Description | Example |
|-------|-------------|---------|
| Node Name | Teleport node name | `web-01` |
| Cluster | Teleport cluster name (optional) | `production` |

**Prerequisites:** `tsh` CLI, authenticated (`tsh login`).

**Flatpak:** The host `~/.tsh/` directory is not mounted. RustConn redirects Teleport's config directory to `~/.var/app/io.github.totoshko88.RustConn/config/tsh/` via the `TELEPORT_HOME` environment variable. Run `tsh login` from a RustConn Local Shell after installing Teleport via Flatpak Components.

#### Tailscale SSH

Connects via Tailscale's built-in SSH.

| Field | Description | Example |
|-------|-------------|---------|
| Tailscale Host | Machine name or Tailscale IP | `my-server` or `100.64.0.1` |

**Prerequisites:** `tailscale` installed and connected (`tailscale up`), SSH enabled on the target node.

#### HashiCorp Boundary

Connects via HashiCorp Boundary proxy.

| Field | Description | Example |
|-------|-------------|---------|
| Target ID | Boundary target identifier | `ttcp_1234567890` |
| Controller Address | Boundary controller URL | `https://boundary.example.com` |

**Prerequisites:** `boundary` CLI, authenticated (`boundary authenticate`).

**Flatpak:** Boundary uses system keyring via D-Bus for credential storage, which works natively in the Flatpak sandbox. Install Boundary via Flatpak Components.

#### Hoop.dev

Connects via Hoop.dev zero-trust access gateway using `hoop connect`. Hoop.dev is an access gateway for databases and servers that provides secure, auditable access with SSO authentication and data masking capabilities.

| Field | Description | Example |
|-------|-------------|---------|
| Connection Name | Hoop.dev connection identifier (required) | `my-database` |
| Gateway URL | API gateway URL (optional, for self-hosted) | `https://app.hoop.dev` |
| gRPC URL | gRPC server URL (optional, for self-hosted) | `grpcs://app.hoop.dev:8443` |

**Generated command:** `hoop connect <connection-name> [--api-url <url>] [--grpc-url <url>]`

**Prerequisites:**

1. Install the `hoop` CLI:
   ```bash
   curl -s -L https://releases.hoop.dev/release/install-cli.sh | sh
   ```
   Or via Homebrew: `brew tap hoophq/brew https://github.com/hoophq/brew.git && brew install hoop`

2. Configure the gateway (once per machine):
   - Managed instance: `hoop login` (gateway URL defaults to `https://use.hoop.dev`)
   - Self-hosted: `hoop config create --api-url https://your-gateway.tld` then `hoop login`

3. Authenticate: `hoop login` opens your browser for SSO. The access token is stored in `$HOME/.hoop/config.toml`.

**Environment variables (alternative to config file):**

| Variable | Description |
|----------|-------------|
| `HOOP_APIURL` | Gateway API URL (e.g., `https://use.hoop.dev`) |
| `HOOP_GRPCURL` | gRPC URL (e.g., `grpcs://use.hoop.dev:8443`) |
| `HOOP_TOKEN` | Access token or API key |
| `HOOP_TLSCA` | TLS CA certificate path (for self-signed certs) |

**CLI usage:**

```bash
# GUI: create a ZeroTrust connection with provider "Hoop.dev"
# CLI:
rustconn-cli add --name "Production DB" --host localhost --protocol zt \
  --provider hoop_dev --hoop-connection-name my-database

# With self-hosted gateway:
rustconn-cli add --name "Staging DB" --host localhost --protocol zt \
  --provider hoop_dev --hoop-connection-name staging-db \
  --hoop-gateway-url https://hoop.internal.company.com \
  --hoop-grpc-url grpcs://hoop.internal.company.com:8443
```

**Flatpak:** The host `~/.hoop/` directory is mounted as read-only to share authentication tokens and configuration. Install `hoop` via Flatpak Components if not available on the host.

#### Generic Command

For providers not listed above. Enter a custom command template that RustConn will execute.

| Field | Description | Example |
|-------|-------------|---------|
| Command Template | Full command to execute | `my-proxy connect ${host}` |

The command template supports `${host}`, `${user}`, and `${port}` placeholder substitution. These are replaced with the connection's host, username, and port values at runtime.

### Custom Arguments

All providers support an **Additional CLI arguments** field in the Advanced section. These arguments are appended to the generated command. Use this for provider-specific flags not covered by the UI fields.

---

## Templates

Templates are connection presets that store protocol settings, authentication defaults, tags, custom properties, and automation tasks. When you create a connection from a template, all configured fields are copied into the new connection.

### Manage Templates

Menu → Tools → **Manage Templates** (or `rustconn-cli template list`)

The dialog lists all templates grouped by protocol. Each row shows the template name, protocol, and default host/port.

### Create Template

**From scratch:**

1. Open Manage Templates
2. Click **Create Template**
3. Enter a name and optional description
4. Select protocol (SSH, RDP, VNC, SPICE)
5. Configure default settings: host, port, username, domain, tags
6. Optionally set protocol-specific options (e.g., SSH key path, RDP resolution)
7. Optionally add pre/post connection tasks and WoL configuration
8. Save

**From an existing connection:**

1. Right-click a connection in the sidebar → **Create Template from Connection**
2. Enter a template name
3. All settings from the connection are copied into the template
4. Edit any fields you want to change as defaults
5. Save

### Edit Template

1. Open Manage Templates
2. Select a template from the list
3. Click **Edit** (or double-click)
4. Modify any fields
5. Save — existing connections created from this template are not affected

### Delete Template

1. Open Manage Templates → select template → **Delete**
2. Or: `rustconn-cli template delete "Template Name"`

Deleting a template does not affect connections previously created from it.

### Use Template

**From Quick Connect (Ctrl+Shift+Q):**
- Select a template from the dropdown at the top of the Quick Connect dialog
- Template fields pre-fill the form; override host and other fields as needed

**From Manage Templates:**
- Select a template → click **Create Connection**
- Enter a connection name and host → the rest is pre-filled from the template

**From CLI:**
```bash
rustconn-cli template apply "SSH Template" --name "New Server" --host "10.0.0.5"
```

### Template Fields

Templates support all connection fields:

| Field | Description |
|-------|-------------|
| Protocol | SSH, RDP, VNC, or SPICE |
| Host / Port | Default remote endpoint (can be left empty) |
| Username / Domain | Default authentication identity |
| Password Source | None, KeePass, Keyring, Bitwarden, etc. |
| Tags | Default tags for organization |
| Protocol Config | All protocol-specific options (SSH key, RDP resolution, etc.) |
| Custom Properties | Arbitrary key-value metadata |
| Pre/Post Tasks | Automation tasks to run before/after connection |
| WoL Config | Wake-on-LAN settings |

---

## Snippets

Reusable command templates with variable substitution. Snippets let you define frequently used commands once and execute them in any active terminal session with one action.

### Syntax

Snippets use `${variable}` placeholders that are resolved at execution time. Variables can have default values and descriptions.

```bash
# Simple variable
ssh ${user}@${host} -p ${port}

# Service management
sudo systemctl restart ${service}

# Log tailing with filter
journalctl -u ${service} -f --since "${since}"

# Database backup
pg_dump -h ${host} -U ${user} -d ${database} > /tmp/${database}_backup.sql
```

### Variable Features

Each variable in a snippet can have:

| Property | Description |
|----------|-------------|
| Name | Used in `${name}` placeholders |
| Description | Shown as hint in the execution dialog |
| Default Value | Pre-filled when executing; user can override |

### Manage Snippets

Menu → Tools → **Manage Snippets** (or `rustconn-cli snippet list`)

The dialog shows all snippets with name, command preview, category, and tags. You can:

- **Create** — Click **+** to add a new snippet with name, command, description, category, and tags
- **Edit** — Select a snippet and click **Edit** (or double-click)
- **Delete** — Select a snippet and click **Delete**
- **Search** — Filter snippets by name or command text

### Execute Snippet

**From GUI:**

1. Connect to a terminal session (SSH, Telnet, Serial, Kubernetes, or local shell)
2. Menu → Tools → **Execute Snippet** (or use Command Palette → Snippets)
3. Select a snippet from the list
4. Fill in variable values (defaults are pre-filled)
5. Click **Execute** — the resolved command is sent to the active terminal

**From CLI:**

```bash
# List available snippets
rustconn-cli snippet list

# Show snippet details
rustconn-cli snippet show "Deploy Script"

# Execute with variable substitution
rustconn-cli snippet run "Restart Service" --var service=nginx --execute

# Add a new snippet
rustconn-cli snippet add --name "Restart" --command "sudo systemctl restart \${service}"

# Delete a snippet
rustconn-cli snippet delete "Old Snippet"
```

### Organization

Snippets support categories and tags for filtering:

- **Category** — Group related snippets (e.g., "Deployment", "Monitoring", "Database")
- **Tags** — Additional labels for cross-cutting concerns (e.g., "production", "sudo")

---

## Clusters

Clusters group multiple connections for simultaneous management. The primary use case is broadcast mode: type a command once and it is sent to all connected cluster members at the same time.

### Create Cluster

**From GUI:**

1. Menu → Tools → **Manage Clusters**
2. Click **Create**
3. Enter a cluster name
4. Add connections by selecting from the list
5. Optionally enable **Broadcast by default**
6. Save

**From CLI:**

```bash
rustconn-cli cluster create --name "Web Servers" --broadcast
```

### Add / Remove Members

**From Manage Clusters dialog:**

1. Select a cluster → click **Edit**
2. Use **Add Connection** to pick connections from the sidebar list
3. Use the **Remove** button next to a member to remove it
4. Save

**From CLI:**

```bash
rustconn-cli cluster add-connection --cluster "Web Servers" --connection "Web-01"
rustconn-cli cluster add-connection --cluster "Web Servers" --connection "Web-02"
rustconn-cli cluster remove-connection --cluster "Web Servers" --connection "Web-01"
```

### Connect Cluster

1. Open Manage Clusters → select a cluster → **Connect All**
2. RustConn opens a terminal tab for each member connection
3. Each member shows its connection status (Pending → Connecting → Connected)
4. If a member fails, the error is shown per-member; other members continue

### Broadcast Mode

When broadcast is enabled, every keystroke you type in the focused terminal is sent to all connected cluster members simultaneously.

- **Enable:** Toggle the broadcast switch in the cluster toolbar
- **Disable:** Toggle it off to return to single-session input
- Only connected members receive broadcast input; disconnected or errored members are skipped

Use cases:
- Rolling out configuration changes across multiple servers
- Running the same diagnostic command on all nodes
- Updating packages on a fleet of machines

### Disconnect Cluster

- **Disconnect All** — Closes all member sessions at once
- Individual members can be disconnected independently without affecting the cluster

### Delete Cluster

1. Manage Clusters → select → **Delete**
2. Or: `rustconn-cli cluster delete "Web Servers"`

Deleting a cluster does not delete the underlying connections.

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
- Remote Desktop Manager (JSON)
- RDP files (.rdp — Microsoft Remote Desktop)
- Virt-Viewer (.vv files — SPICE/VNC from libvirt, Proxmox VE)
- Libvirt / GNOME Boxes (domain XML — VNC, SPICE, RDP from QEMU/KVM VMs)
- RustConn Native (.rcn)

Double-click source to start import immediately.

**Merge Strategies:**
When importing connections that already exist, choose a merge strategy:
- **Skip Existing** — Keep current connections, skip duplicates
- **Overwrite** — Replace existing connections with imported data
- **Rename** — Import as new connections with a suffix

**Import Preview:**
For large imports (10+ connections), a preview is shown before applying. You can review which connections will be created, updated, or skipped, and change the action for individual entries.

**Import Source Details:**

| Source | Auto-scan | File picker | Protocols | Notes |
|--------|:---------:|:-----------:|-----------|-------|
| SSH Config | `~/.ssh/config` | Any file | SSH | Host blocks → connections |
| Remmina | `~/.local/share/remmina/` | — | SSH, RDP, VNC, SFTP | One `.remmina` per connection (see Flatpak note below) |
| Asbru-CM | `~/.config/pac/` | YAML file | SSH, VNC, RDP | Variables converted to `${VAR}` |
| Ansible | `/etc/ansible/hosts` | INI/YAML file | SSH | Groups preserved |
| Royal TS | — | `.rtsz` file | All | Folder hierarchy → groups |
| MobaXterm | — | `.mxtsessions` | SSH, RDP, VNC, Telnet, Serial | INI-based sessions |
| Remote Desktop Manager | — | JSON file | SSH, RDP, VNC | Devolutions JSON export |
| RDP File | — | `.rdp` file | RDP | Microsoft Remote Desktop format; address, credentials, gateway, resolution, audio, clipboard |
| Virt-Viewer | — | `.vv` file | SPICE, VNC | From libvirt, Proxmox VE, oVirt |
| Libvirt / GNOME Boxes | `/etc/libvirt/qemu/`, `~/.config/libvirt/qemu/` | XML file | VNC, SPICE, RDP | Domain XML `<graphics>` elements |
| Libvirt Daemon (virsh) | `qemu:///session` | — | VNC, SPICE, RDP | Queries running libvirtd via `virsh` |
| RustConn Native | — | `.rcn` file | All | Full-fidelity backup |

**Remmina import in Flatpak:**

In Flatpak, the sandbox redirects `~/.local/share/` to `~/.var/app/io.github.totoshko88.RustConn/data/`. RustConn checks both the sandbox path and the host path `~/.local/share/remmina/`, but the host path requires filesystem access. Grant it with:

```bash
flatpak override --user --filesystem=~/.local/share/remmina:ro io.github.totoshko88.RustConn
```

Alternatively, copy your Remmina profiles into the sandbox directory:

```bash
mkdir -p ~/.var/app/io.github.totoshko88.RustConn/data/remmina/
cp ~/.local/share/remmina/*.remmina ~/.var/app/io.github.totoshko88.RustConn/data/remmina/
```

**Libvirt / GNOME Boxes import:**

Two import modes are available:

- **Auto-scan** ("Libvirt / GNOME Boxes") — scans standard libvirt directories for domain XML files. Covers both system-level QEMU/KVM VMs (`/etc/libvirt/qemu/`, may require root read access) and user-session VMs (`~/.config/libvirt/qemu/`). GNOME Boxes stores its VMs in the same user-session directory, so they are imported automatically.
- **Single file** ("Libvirt XML File") — import from a specific `.xml` file. Useful for `virsh dumpxml <domain>` output or XML files copied from another host.

Each `<graphics>` element in the domain XML becomes a separate connection. If a VM has both VNC and SPICE consoles, two connections are created (e.g. "myvm (VNC)", "myvm (SPICE)").

Imported fields: VM name, UUID (stored as tag), description, graphics type, listen address, port, TLS port (SPICE), password. VMs with `autoport="yes"` and no resolved port use the protocol default (5900 for VNC/SPICE, 3389 for RDP) — edit the port after starting the VM. Headless VMs (no `<graphics>` element) are skipped with a note.

**Libvirt Daemon import (virsh):**

A third mode queries a running libvirtd instance directly via `virsh`. This resolves autoport assignments from running VMs and discovers transient domains that have no on-disk XML.

Select "Libvirt Daemon (virsh)" in the import dialog. By default it connects to `qemu:///session` (user-level VMs, no root required). The importer runs `virsh list --all --name` to enumerate domains, then `virsh dumpxml <name>` for each one, feeding the XML into the same parser used by the static import.

Supported URIs (pass via CLI or future UI enhancement):
- `qemu:///session` — user-level VMs (default)
- `qemu:///system` — system-level VMs (requires `libvirt` group membership or root)
- `qemu+ssh://host/system` — remote libvirtd over SSH

Requirements: `virsh` must be installed (`libvirt-client` package on most distros). If `virsh` is not found, the source appears greyed out in the import dialog.

**Libvirt Daemon import in Flatpak:**

Inside the Flatpak sandbox, `virsh` is not bundled and the libvirt socket is not accessible by default. To use the daemon import from Flatpak:

1. Install `virsh` on the host system:

   ```bash
   # Debian/Ubuntu
   sudo apt install libvirt-clients

   # Fedora
   sudo dnf install libvirt-client

   # Arch Linux
   sudo pacman -S libvirt

   # openSUSE
   sudo zypper install libvirt-client
   ```

2. Grant the Flatpak access to the host `virsh` binary and the libvirt socket:

   ```bash
   flatpak override --user \
     --filesystem=/run/libvirt:ro \
     --filesystem=/usr/bin/virsh:ro \
     --filesystem=/usr/lib:ro \
     io.github.totoshko88.RustConn
   ```

   For user-session VMs, also grant access to the user socket:

   ```bash
   flatpak override --user \
     --filesystem=xdg-run/libvirt:ro \
     io.github.totoshko88.RustConn
   ```

3. Verify that `virsh` works inside the sandbox:

   ```bash
   flatpak run --command=virsh io.github.totoshko88.RustConn -c qemu:///session list --all
   ```

If the above is too complex, use the static import modes instead: either auto-scan the XML directories (grant `--filesystem=~/.config/libvirt:ro`) or import individual XML files via the file picker.

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

**Format Limitations:**

| Format | Protocols | Passwords | Groups | Notes |
|--------|-----------|-----------|--------|-------|
| SSH Config | SSH only | Key paths only | No | Standard `~/.ssh/config` format |
| Remmina | SSH, RDP, VNC, SFTP | Encrypted | No | One `.remmina` file per connection |
| Asbru-CM | SSH, VNC, RDP | Encrypted | Yes | YAML-based, supports variables |
| Ansible | SSH only | No | Yes (groups) | INI or YAML inventory format |
| Royal TS | All | Encrypted | Yes | XML `.rtsz` archive |
| MobaXterm | SSH, RDP, VNC, Telnet | Encrypted | Yes | INI-based `.mxtsessions` |
| RustConn Native | All | Encrypted | Yes | Full-fidelity backup format |

**Batch Workflow:**
1. Open Export dialog (Ctrl+Shift+E)
2. Select format
3. Choose output path
4. Toggle "Include passwords" if needed
5. Click Export — progress bar shows status
6. Result summary shows exported/skipped counts
7. "Open Location" button opens the output directory

---

## Tools

### Global Variables

Global variables allow you to use placeholders in connection fields that are resolved at connection time.

**Syntax:** `${VARIABLE_NAME}`

**Supported Fields:**
- Host
- Username
- Domain (RDP)

**Define Variables:**
1. Menu → Tools → **Variables...**
2. Click **Add Variable**
3. Enter name and value
4. Optionally mark as **Secret** (value hidden, stored in vault)
5. Click **Save**

**Secret Variables:**
- Toggle visibility with the eye icon (Show/Hide)
- Load secret value from vault with the vault icon
- Secret variable values are auto-saved to the configured vault backend on dialog save
- Secret values are cleared from the settings file (stored only in vault)

**Use in Connections:**
1. Create or edit a connection
2. In Host, Username, or Domain field, enter `${VARIABLE_NAME}`
3. When connecting, the variable is replaced with its value

**Example:**
```
Variable: PROD_USER = admin
Variable: PROD_DOMAIN = corp.example.com

Connection Username: ${PROD_USER}
Connection Domain: ${PROD_DOMAIN}

At connection time:
  Username → admin
  Domain → corp.example.com
```

**Asbru-CM Import:**
When importing from Asbru-CM, the `<GV:VAR_NAME>` syntax is automatically converted to `${VAR_NAME}`. However, you must manually define the variable values in Tools → Variables.

**Tips:**
- Variable names are case-sensitive
- Undefined variables remain as literal text (e.g., `${UNDEFINED}` stays unchanged)
- Use variables for shared credentials across multiple connections
- Combine with Group Credentials for hierarchical credential management

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

- Search and filter past connections by name, host, protocol, or username
- Connect directly from history
- Delete individual entries with the ✕ button on each row
- Clear all history (with confirmation dialog)

### Connection Statistics

Menu → Tools → **Connection Statistics**

Tracks usage patterns across all connections:

- **Total connections** — number of connection attempts
- **Success rate** — percentage of successful connections vs failures
- **Connection duration** — average and total time spent connected
- **Most used connections** — ranked by frequency
- **Protocol breakdown** — usage distribution across SSH, RDP, VNC, etc.
- **Last connected** — timestamp of most recent session per connection

Use the **Reset** button to clear all statistics. Statistics are stored locally and not included in exports.

### Wake-on-LAN

Wake sleeping machines before connecting by sending WoL magic packets.

**Configure WoL for a connection:**
1. Edit connection → **WOL** tab
2. Enter MAC address (e.g., `AA:BB:CC:DD:EE:FF`)
3. Optionally set broadcast address and port
4. Save

**Send WoL from sidebar:**
- Right-click connection → **Wake On LAN**
- Toast notification confirms success or failure

**Auto-WoL on connect:**
- If a connection has WoL configured, a magic packet is sent automatically when you connect
- The connection proceeds immediately (fire-and-forget, does not wait for the machine to boot)
- Use the `wait_seconds` setting in WOL tab to add a delay if needed

**Standalone WoL dialog:**
- Menu → Tools → **Wake On LAN...**
- Pick a connection with WoL configured from the dropdown, or enter MAC address manually
- Set broadcast address and port
- Click **Send** to send the magic packet

**CLI:**
```bash
rustconn-cli wol AA:BB:CC:DD:EE:FF
rustconn-cli wol "Server Name"
rustconn-cli wol AA:BB:CC:DD:EE:FF --broadcast 192.168.1.255 --port 9
```

All GUI sends use 3 retries at 500 ms intervals for reliability.

### Flatpak Components

**Available only in Flatpak environment**

Menu → **Flatpak Components...**

Download and install additional CLI tools directly within the Flatpak sandbox:

**Zero Trust CLIs:**
- AWS CLI, AWS SSM Plugin
- Google Cloud CLI
- Azure CLI
- OCI CLI
- Teleport, Tailscale
- Cloudflare Tunnel
- HashiCorp Boundary
- Hoop.dev

**Password Manager CLIs:**
- Bitwarden CLI
- 1Password CLI

**Protocol Clients (optional):**
- TigerVNC Viewer

**Features:**
- One-click Install/Remove/Update
- Progress indicators with cancel support
- SHA256 checksum verification
- Automatic PATH configuration for Local Shell
- Python-based CLIs installed via pip
- .deb packages extracted automatically

**Installation Location:** `~/.var/app/io.github.totoshko88.RustConn/cli/`

**Note:** Installed CLIs are automatically detected in Settings → Connection page → Clients.

---

## Settings

Access via **Ctrl+,** or Menu → **Settings**

The settings dialog uses `adw::PreferencesDialog` with built-in search. Settings are organized into 4 pages:

| Page | Icon | Contents |
|------|------|----------|
| Terminal | `utilities-terminal-symbolic` | Terminal + Logging |
| Interface | `applications-graphics-symbolic` | Appearance, Window, Startup, System Tray, Session Restore + Keybindings |
| Secrets | `channel-secure-symbolic` | Secret backends + SSH Agent |
| Connection | `network-server-symbolic` | Clients + Monitoring |

### Terminal page

**Terminal group:**
- **Font** — Family and size
- **Scrollback** — History buffer lines
- **Color Theme** — Dark, Light, Solarized, Monokai, Dracula
- **Cursor** — Shape (Block/IBeam/Underline) and blink mode
- **Behavior** — Scroll on output/keystroke, hyperlinks, mouse autohide, bell, SFTP via mc

**Logging group:**
- **Enable Logging** — Global toggle
- **Log Directory** — Path for session log files
- **Retention Days** — Auto-cleanup period
- **Logging Modes** — Activity, user input, terminal output
- **Timestamps** — Prepend `[HH:MM:SS]` to each line in session log files

### Interface page

**Appearance group:**
- **Theme** — System, Light, Dark (libadwaita `StyleManager`)
- **Language** — UI language selector (restart required)
- **Color tabs by protocol** — Colored circle indicator on tabs (SSH=green, RDP=blue, VNC=purple, SPICE=orange, Serial=yellow, K8s=cyan)

**Window group:**
- **Remember size** — Restore window geometry on startup

**Startup group:**
- **On startup** — Do nothing, Local Shell, or connect to a specific saved connection

**System Tray group:**
- **Show icon** — Display icon in system tray
- **Minimize to tray** — Hide window instead of closing (requires tray icon enabled)

**Session Restore group:**
- **Enabled** — Reconnect to previous sessions on startup
- **Ask first** — Prompt before restoring sessions
- **Max age** — Hours before sessions expire (1–168)

**Keybindings group:**
- Customizable keyboard shortcuts for 30+ actions across 6 categories
- Record button to capture key combinations
- Per-shortcut Reset and Reset All to Defaults

### Secrets page

**Secret backend group:**
- **Preferred Backend** — libsecret, KeePassXC, KDBX file, Bitwarden, 1Password, Passbolt, Pass (passwordstore.org)
- **Enable Fallback** — Use libsecret if primary unavailable
- **Credential Encryption** — Backend master passwords encrypted with AES-256-GCM + Argon2id (machine-specific key); legacy XOR migrated transparently
- **Bitwarden Settings:**
  - Vault status and unlock button
  - Master password persistence (encrypted in settings)
  - Save to system keyring option (recommended, requires `libsecret-tools`)
  - Auto-unlock from keyring on startup when vault is locked
  - API key authentication for automation/2FA (FIDO2, Duo)
  - Client ID and Client Secret fields
- **1Password Settings:**
  - Account status indicator
  - Sign-in button (opens terminal for interactive `op signin`)
  - Supports biometric authentication via desktop app
  - Service account token entry (`OP_SERVICE_ACCOUNT_TOKEN`)
  - Save token to system keyring (auto-loads on startup)
  - Save token encrypted in settings (machine-specific)
- **Passbolt Settings:**
  - CLI detection and version display
  - Server URL entry (auto-fills from `go-passbolt-cli` config)
  - "Open Vault" button to open Passbolt web vault in browser
  - GPG passphrase entry for decrypting credentials
  - Save passphrase to system keyring (auto-loads on startup)
  - Save passphrase encrypted in settings (machine-specific)
  - Server configuration status check (configured/not configured/auth failed)
  - Requires `passbolt configure` CLI setup before use
- **Pass (passwordstore.org) Settings:**
  - CLI detection and version display (`pass` binary)
  - Custom `PASSWORD_STORE_DIR` path (defaults to `~/.password-store`)
  - Credentials stored as `RustConn/<connection-name>` entries
  - GPG-encrypted files — requires `gpg` and `pass` on PATH
  - "Open Store" button to browse password store directory
- **KeePassXC KDBX Settings:**
  - Database path and key file selection
  - Password and/or key file authentication
  - Save password to system keyring (auto-loads on startup)
  - Save password encrypted in settings (machine-specific)
- **System Keyring Requirements:**
  - Requires `libsecret-tools` package (`secret-tool` binary)
  - Works with GNOME Keyring, KDE Wallet, and other Secret Service providers
  - "Save password" and "Save to system keyring" are mutually exclusive per backend
  - If `secret-tool` is not installed, toggling keyring option shows a warning
- **Installed Password Managers** — Auto-detected managers with versions (GNOME Secrets, KeePassXC, KeePass2, Bitwarden CLI, 1Password CLI, Passbolt CLI, Pass)

**Password Source Defaults:**
When creating a new connection, the password source dropdown shows:
- **Prompt** — Ask for password on each connection
- **Vault** — Store/retrieve from configured secret backend
- **Variable** — Read from a named secret global variable
- **Inherit** — Use credentials from parent group
- **None** — No password (key-based auth)

**SSH Agent group:**
- **Status** — Agent running/stopped indicator with socket path
- **Loaded Keys** — Currently loaded SSH keys with remove option
- **Available Keys** — Keys in `~/.ssh/` with add option

### Connection page

**Clients group:**

Auto-detected CLI tools with versions:

Protocol Clients: SSH, RDP (FreeRDP), VNC (TigerVNC), SPICE (remote-viewer), Telnet, Serial (picocom), Kubernetes (kubectl)

Zero Trust: AWS, GCP, Azure, OCI, Cloudflare, Teleport, Tailscale, Boundary, Hoop.dev

Searches PATH and user directories (`~/bin/`, `~/.local/bin/`, `~/.cargo/bin/`).

**Monitoring group:**
- **Enable monitoring** — Global toggle for remote host metrics collection
- **Polling interval** — Seconds between metric updates (1–60, default: 3)
- **Visible Metrics** — Toggle individual metrics: CPU, Memory, Disk, Network, Load Average, System Info

---

## Backup & Restore

Back up your entire RustConn configuration (connections, groups, snippets, clusters, templates, history, keybindings, variables, and settings) as a single ZIP archive.

### Create a Backup

1. Open **Settings** (Ctrl+,) → **Interface** page
2. Scroll to **Backup & Restore** section
3. Click **Backup**
4. Choose a save location
5. RustConn creates a ZIP containing all configuration files

### Restore from Backup

1. Open **Settings** → **Interface** page → **Backup & Restore**
2. Click **Restore**
3. Select a previously created backup ZIP
4. Confirm the restore (overwrites current configuration)
5. Restart RustConn for all changes to take effect

### What's Included

| Included | Not Included |
|----------|-------------|
| Connections and groups | Passwords (stored in secret backend) |
| Templates and snippets | Encrypted documents |
| Clusters | SSH keys |
| Global variables (names only; secret values are in vault) | Session logs |
| Keybindings | Flatpak-installed CLI tools |
| Application settings | |
| Connection history and statistics | |

> **Important:** The `.machine-key` file (`~/.local/share/rustconn/.machine-key`) is **not** included in backups. This key is used to encrypt credentials stored locally (AES-256-GCM). If you restore a backup on a different machine, locally encrypted credentials cannot be decrypted without the original `.machine-key`. To migrate encrypted credentials: copy `.machine-key` from the old machine to the same path on the new machine **before** restoring the backup, or re-enter passwords after restore.

After restoring on a new machine, re-enter passwords or configure the same secret backend. See also the [Migration Guide](#migration-guide) for full machine-to-machine migration.

---

## Startup Action

Configure which session opens automatically when RustConn starts. Useful for users who always work with the same connection or want RustConn as their default terminal.

### Settings (GUI)

1. Open **Settings** (Ctrl+,)
2. Go to **Interface** page
3. Find the **Startup** group
4. Select an action from the **On startup** dropdown:
   - **Do nothing** — default behavior, no session opens
   - **Local Shell** — open a local terminal tab
   - **\<Connection Name\> (Protocol)** — connect to a specific saved connection

The setting is persisted and applied on every launch.

### CLI Override

CLI flags override the persisted setting for a single launch:

```bash
# Open a local shell
rustconn --shell

# Connect by name (case-insensitive)
rustconn --connect "Production Server"

# Connect by UUID
rustconn --connect 550e8400-e29b-41d4-a716-446655440000
```

### Use RustConn as Default Terminal

Create a custom `.desktop` file that launches RustConn with a local shell:

```ini
[Desktop Entry]
Name=RustConn Shell
Exec=rustconn --shell
Icon=io.github.totoshko88.RustConn
Type=Application
Categories=System;TerminalEmulator;
```

Save as `~/.local/share/applications/rustconn-shell.desktop`, then set it as the default terminal in your desktop environment settings.

### Notes

- CLI flags (`--shell`, `--connect`) take priority over the persisted setting
- If `--connect` specifies a name that doesn't match any saved connection, a toast notification is shown
- The startup action runs after the main window is presented, so the UI is fully loaded before the session opens

---

## Command Palette

Open with **Ctrl+P** (connections) or **Ctrl+Shift+P** (commands).

A VS Code-style quick launcher with fuzzy search. Type to filter, then select with arrow keys and Enter.

### Modes

| Prefix | Mode | Description |
|--------|------|-------------|
| *(none)* | Connections | Fuzzy search saved connections; Enter to connect |
| `>` | Commands | Application commands (New Connection, Import, Settings, etc.) |
| `@` | Tags | Filter connections by tag |
| `#` | Groups | Filter connections by group |

### Usage

1. Press **Ctrl+P** to open
2. Start typing to filter connections
3. Type `>` to switch to command mode
4. Press **Enter** to execute, **Escape** to dismiss

The palette shows up to 20 results with match highlighting. Results are ranked by fuzzy match score.

---

## Favorites

Pin frequently used connections to a dedicated "Favorites" section at the top of the sidebar.

### Pin a Connection

- Right-click a connection → **Pin to Favorites**
- The connection appears in the ★ Favorites group at the top of the sidebar

### Unpin a Connection

- Right-click a pinned connection → **Unpin from Favorites**
- The connection returns to its original group

Favorites persist across sessions. Pinned connections remain in their original group as well — the Favorites section shows a reference, not a move.

---

## Tab Coloring

Optional colored circle indicators on terminal tabs to visually distinguish protocols at a glance.

| Protocol | Color |
|----------|-------|
| SSH | 🟢 Green |
| RDP | 🔵 Blue |
| VNC | 🟣 Purple |
| SPICE | 🟠 Orange |
| Serial | 🟡 Yellow |
| Kubernetes | 🔵 Cyan |

### Enable/Disable

Settings → Interface page → Appearance → **Color tabs by protocol**

---

## Tab Grouping

Organize open tabs into named groups with color-coded indicators.

### Assign a Tab to a Group

1. Right-click a tab in the tab bar
2. Select **Assign to Group**
3. Choose an existing group or type a new name (e.g. "Production", "Staging")

### Remove from Group

- Right-click a grouped tab → **Remove from Group**

Groups are visual only — they add a colored label to the tab title. Each group gets a unique color from a rotating palette. Groups are session-scoped and not persisted.

---

## Custom Icons

Set custom emoji or GTK icon names on connections and groups to visually distinguish them in the sidebar.

### Supported Icon Types

| Type | Example | How It Renders |
|------|---------|----------------|
| Emoji / Unicode | `🇺🇦`, `🏢`, `🔒`, `🐳` | Displayed as text next to the name |
| GTK icon name | `starred-symbolic`, `network-server-symbolic` | Rendered as a symbolic icon |

### Set a Custom Icon

1. Edit a connection or group
2. Enter an emoji or GTK icon name in the **Icon** field
3. Save

Leave the field empty to use the default icon (folder for groups, protocol-based for connections).

### Tips

- Emoji icons work with 1–2 character Unicode sequences (flags, objects, symbols)
- GTK icon names must match installed icon theme entries (e.g. `computer-symbolic`, `folder-remote-symbolic`)
- Icons appear in the sidebar tree, making it easy to spot important connections at a glance

---

## Remote Monitoring

MobaXterm-style monitoring bar below SSH terminals showing real-time system metrics from remote Linux hosts. Completely agentless — no software needs to be installed on the remote host. RustConn collects data by parsing `/proc/*` and `df` output over a separate SSH connection. For Telnet and Kubernetes sessions, monitoring is available if the host is also reachable via SSH.

### How It Works

Monitoring runs in a **separate SSH process** alongside your interactive terminal session. This means:

- Your terminal is never interrupted by monitoring commands
- Monitoring uses the same credentials (password, key, jump host) as the main connection
- Data is collected at a configurable polling interval (default: every 3 seconds)
- A one-time system info command runs on the first tick to gather static host details

The monitoring collector sends two shell commands to the remote host:

1. **Metrics command** (every tick) — reads `/proc/stat`, `/proc/meminfo`, `/proc/loadavg`, `/proc/net/dev`, and `df -Pk /` in a single invocation
2. **System info command** (once) — reads `uname -r`, `/etc/os-release`, `/proc/uptime`, `/proc/cpuinfo`, `/proc/meminfo`, `hostname -f`, and `hostname -I`

All output is delimited by marker lines (`---RUSTCONN_*---`) so the parser can reliably extract each section even if individual commands produce unexpected output.

### Monitoring Bar

When enabled, a compact 28px bar appears below the terminal:

```
[CPU: ████░░ 45%] [RAM: ██░░ 62%] [Disk: ██░░ 78%] [1.23 0.98 0.76] [↓ 1.2 MB/s ↑ 0.3 MB/s] [Ubuntu 24.04 (6.8.0) · x86_64 · 15.6 GiB · 8C/16T · 10.0.1.5]
```

Each section can be individually toggled on or off in Settings.

### Displayed Metrics

| Metric | Source | Details |
|--------|--------|---------|
| CPU usage | `/proc/stat` | Percentage with level bar; delta-based calculation between two snapshots |
| Memory usage | `/proc/meminfo` | Percentage with level bar; swap usage shown in tooltip when swap is present |
| Disk usage | `df -Pk` | Root filesystem percentage with level bar; all mount points shown in tooltip |
| Load average | `/proc/loadavg` | 1, 5, 15 minute values; running/total process count in tooltip |
| Network throughput | `/proc/net/dev` | Download/upload rates (auto-scaled: B/s, KB/s, MB/s, GB/s); sums all non-loopback interfaces |
| System info | One-time collection | See below |

### System Info Section

The rightmost section of the monitoring bar shows static host information collected once when monitoring starts:

**Bar label:** `Ubuntu 24.04 (6.8.0) · x86_64 · 15.6 GiB · 8C/16T · 10.0.1.5`

Components (separated by `·`):
- Distribution name and kernel version
- CPU architecture (x86_64, aarch64, etc.)
- Total physical RAM
- CPU cores/threads (e.g. `8C/16T` for 8 cores with hyperthreading, or `4C` when cores = threads)
- Primary private IP address (first address from `hostname -I`)

**Tooltip** (hover over the system info section):
- **Uptime** — live counter that updates on every polling tick (e.g. `3d 5h 12m`), not just when system info was first collected
- **Hostname** — FQDN if available (from `hostname -f`), otherwise short hostname
- **IPv4 addresses** — all IPv4 addresses grouped together
- **IPv6 addresses** — all IPv6 addresses grouped together

This is useful when you connect to a host by its public IP or DNS name but need to know its private network addresses for internal routing, firewall rules, or documentation.

### Disk Tooltip (Multiple Mount Points)

The disk level bar in the monitoring bar always shows the root filesystem (`/`). When the remote host has multiple mounted filesystems, hovering over the disk section shows a tooltip with all of them:

```
/: 48.8 GiB/97.7 GiB (50%)
/home: 9.8 GiB/195.3 GiB (5%)
/var: 39.1 GiB/48.8 GiB (80%)
```

Each line shows: mount point, used/total size, and usage percentage.

The following filesystem types are automatically filtered out:
- `tmpfs`, `devtmpfs` (RAM-based virtual filesystems)
- `squashfs`, `overlay` (read-only image mounts)
- Snap loop mounts (`/snap/*`, `/var/snap/*`)

If only the root filesystem is present, no tooltip is shown on the disk section.

### Stopped Indication

If the monitoring collector encounters 3 consecutive errors (SSH timeout, parse failure, or unsupported OS), it stops automatically. When this happens:

- A **⚠ warning icon** appears in the monitoring bar
- The entire bar **dims to 50% opacity** to indicate stale data
- The system info tooltip shows **"⚠ Monitoring stopped — metrics may be stale"**
- The last received metrics remain visible (not cleared)

This prevents log spam from broken connections while still showing the last known state.

### Enable Monitoring

1. Open **Settings** (Ctrl+,) → **Connection** page → **Monitoring** group
2. Toggle **Enable monitoring**
3. Configure polling interval (1–60 seconds, default: 3)
4. Select which metrics to display (CPU, Memory, Disk, Network, Load, System Info)

All settings take effect immediately for new sessions. Existing sessions keep their current configuration until reconnected.

### Per-Connection Override

Each connection can override the global monitoring setting:
1. Edit connection → **Advanced** tab
2. Set monitoring to **Enabled**, **Disabled**, or **Use global setting**
3. Optionally override the polling interval

### Authentication

Monitoring uses the same authentication method as the main connection:

| Auth Method | How Monitoring Connects |
|-------------|------------------------|
| SSH key | `ssh -i <key> -o BatchMode=yes` |
| Password | `SSH_ASKPASS` mechanism (automatic password injection via temp script) |
| Jump host | `-J user@bastion:port` flag added to SSH command |
| Flatpak | Uses writable `known_hosts` path inside the sandbox |

Host key verification uses `StrictHostKeyChecking=accept-new` — new host keys are accepted on first connection, but changed keys are rejected (protection against MITM attacks).

### Requirements

- Remote host must be **Linux** (reads `/proc/*` and standard GNU coreutils)
- **No agent installation** needed on the remote host
- Works with **SSH**, **Telnet**, and **Kubernetes** connections
- SSH connection timeout: 5 seconds; command timeout: 10 seconds

### Troubleshooting

| Problem | Cause | Solution |
|---------|-------|----------|
| Bar doesn't appear | Monitoring disabled globally or for this connection | Check Settings → Monitoring and connection Advanced tab |
| Bar appears but shows "—" | First tick hasn't completed yet | Wait for the polling interval (default 3s) |
| Bar dims with ⚠ icon | 3 consecutive collection errors | Check SSH connectivity; reconnect the session |
| No private IP shown | `hostname -I` not available on remote host | Install `inetutils` or `hostname` package on the remote host |
| High CPU on remote host | Polling interval too low | Increase interval to 5–10 seconds in Settings |

---

## Custom Keybindings

Customize all keyboard shortcuts via Settings → Interface page → Keybindings.

### Customize a Shortcut

1. Open **Settings** (Ctrl+,) → **Keybindings** tab
2. Find the action you want to change
3. Click **Record** next to it
4. Press the desired key combination
5. The new shortcut is saved immediately

### Reset a Shortcut

- Click the ↩ (undo) button next to any shortcut to reset it to default
- Click **Reset All to Defaults** at the bottom to reset everything

### Available Actions

Over 30 customizable actions across 6 categories: Application, Connections, Navigation, Terminal, Split View, and View. See the [Keyboard Shortcuts](#keyboard-shortcuts) section for the full default list.

---

## Adaptive UI

RustConn adapts to different window sizes using `adw::Breakpoint` and responsive dialog sizing.

**Main window breakpoints:**
- Below 600sp: split view buttons hidden from header bar (still accessible via keyboard shortcuts or menu)
- Below 400sp: sidebar collapses to overlay mode (toggle with F9 or swipe gesture)

**Dialogs:** All dialogs have minimum size constraints and scroll their content. They can be resized down to ~350px width without clipping.

---

## Encrypted Documents

Store sensitive notes, certificates, and credentials in AES-256-GCM encrypted documents within RustConn.

### Create a Document

1. Menu → File → **New Document** (or use the sidebar Documents section)
2. Enter document name
3. Set a protection password (optional — unprotected documents are still encrypted at rest with the app master key)
4. Write content in the editor
5. Save with **Ctrl+S**

### Open a Document

1. Click a document in the sidebar Documents section
2. If password-protected, enter the password when prompted
3. The document opens in an editor tab

### Document Protection

- **Set Protection** — Right-click document → Set Protection; enter a password
- **Remove Protection** — Right-click document → Remove Protection; confirm with current password
- Protected documents require the password each time they are opened
- Unprotected documents are encrypted with the application master key (transparent to the user)

### Document Operations

| Action | Method |
|--------|--------|
| Create | Menu → File → New Document |
| Open | Click in sidebar |
| Save | Ctrl+S or close prompt |
| Close | Close tab (prompts to save if modified) |
| Delete | Right-click → Delete |
| Set/Remove Protection | Right-click → Set/Remove Protection |

### Tips

- Documents are stored encrypted in the RustConn configuration directory
- Use documents to store SSH keys, API tokens, connection notes, or runbooks
- The dirty indicator (●) in the sidebar shows unsaved changes
- Documents persist across sessions

### Use Cases

- **Runbooks** — Step-by-step procedures for incident response or maintenance tasks
- **API Tokens** — Store tokens for services accessed via SSH tunnels
- **SSH Key Passphrases** — Keep passphrases for keys not stored in the SSH agent
- **Network Diagrams** — Text-based network topology notes (ASCII art, Mermaid syntax)
- **Compliance Notes** — Audit trail documentation for regulated environments

### Backup Considerations

- Documents are stored as encrypted files in `~/.config/rustconn/documents/`
- The Settings Backup/Restore feature (Settings → Interface) includes documents
- RustConn Native export (.rcn) does NOT include documents — back them up separately
- If you lose the master key or document password, the content cannot be recovered

---

## RDP File Association

RustConn registers as a handler for `.rdp` files. Double-clicking an `.rdp` file in your file manager opens RustConn and connects automatically.

### How It Works

1. Double-click an `.rdp` file (or run `rustconn file.rdp` from the terminal)
2. RustConn parses the file and creates a temporary connection with the extracted settings
3. The connection starts immediately — no dialog is shown

### Supported .rdp Fields

| .rdp Field | RustConn Mapping |
|------------|-----------------|
| `full address:s:host:port` | Host and port |
| `username:s:` | Username |
| `domain:s:` | Domain |
| `gatewayhostname:s:` | RDP Gateway host |
| `gatewayusername:s:` | RDP Gateway username |
| `gatewayaccesstoken:s:` | RDP Gateway port (parsed from token) |
| `desktopwidth:i:` / `desktopheight:i:` | Resolution |
| `session bpp:i:` | Color depth |
| `audiomode:i:` | Audio redirection (0=local, 1=remote, 2=off) |
| `redirectclipboard:i:` | Clipboard sharing |

Fields not present in the `.rdp` file use RustConn defaults. Passwords are never stored in `.rdp` files — RustConn prompts for credentials if needed.

### Desktop Integration

The `.desktop` file registers the `application/x-rdp` MIME type. On most desktops, `.rdp` files will automatically associate with RustConn after installation. If not, set the association manually:

```bash
xdg-mime default io.github.totoshko88.RustConn.desktop application/x-rdp
```

### CLI Usage

```bash
# Open an .rdp file directly
rustconn ~/Downloads/server.rdp

# Works with absolute and relative paths
rustconn /tmp/production-vm.rdp
```

---

## Keyboard Shortcuts

Press **Ctrl+?** or **F1** for searchable shortcuts dialog.

Note: Sidebar-scoped shortcuts (F2, Delete, Ctrl+E, Ctrl+D, Ctrl+C, Ctrl+V, Ctrl+M) only work when the sidebar has focus. They do not intercept input in VTE terminals or embedded viewers.

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
| Ctrl+M | Move to Group |

### Terminal

| Shortcut | Action |
|----------|--------|
| Ctrl+Shift+C | Copy |
| Ctrl+Shift+V | Paste |
| Ctrl+Shift+F | Terminal Search |
| Ctrl+Shift+W | Close Tab |
| Ctrl+Tab | Next Tab |
| Ctrl+Shift+Tab | Previous Tab |

### Terminal Keybinding Modes

RustConn uses the VTE terminal emulator, which passes all keystrokes directly to the running shell. To enable vim or emacs-style keybindings, configure your shell:

| Shell | Vim Mode | Emacs Mode (default) |
|-------|----------|---------------------|
| Bash | `set -o vi` in `~/.bashrc` | `set -o emacs` in `~/.bashrc` |
| Zsh | `bindkey -v` in `~/.zshrc` | `bindkey -e` in `~/.zshrc` |
| Fish | `fish_vi_key_bindings` | `fish_default_key_bindings` |

These settings apply to all terminal sessions (SSH, Telnet, Serial, Kubernetes, local shell). RustConn does not intercept or remap shell keybindings.

### Split View

| Shortcut | Action |
|----------|--------|
| Ctrl+Shift+H | Split Horizontal |
| Ctrl+Shift+S | Split Vertical |
| Ctrl+Shift+X | Close Pane |
| Ctrl+` | Focus Next Pane |

### Application

| Shortcut | Action |
|----------|--------|
| Ctrl+F | Search |
| Ctrl+P | Command Palette (Connections) |
| Ctrl+Shift+P | Command Palette (Commands) |
| Ctrl+I | Import |
| Ctrl+Shift+E | Export |
| Ctrl+, | Settings |
| F11 | Toggle Fullscreen |
| F9 | Toggle Sidebar |
| Ctrl+? / F1 | Keyboard Shortcuts |
| Ctrl+Q | Quit |

---

## MOSH Protocol

MOSH (Mobile Shell) provides a roaming, always-on terminal session that survives network changes, high latency, and intermittent connectivity. Unlike SSH, MOSH uses UDP for the session transport after an initial SSH handshake.

**Create a MOSH Connection:**
1. Press **Ctrl+N** → select **MOSH** protocol
2. Enter host and username
3. Configure MOSH-specific options in the **MOSH** tab:

| Parameter | Description | Default |
|-----------|-------------|---------|
| SSH Port | Port for the initial SSH handshake | 22 |
| Port Range | UDP port range for MOSH session (e.g., `60000:60010`) | System default |
| Predict Mode | Local echo prediction: Adaptive, Always, Never | Adaptive |
| Server Binary | Path to `mosh-server` on the remote host (optional) | Auto-detect |
| Custom Arguments | Additional arguments passed to `mosh` | — |

**Requirements:**
- `mosh` installed on the local machine (`sudo apt install mosh` / `sudo dnf install mosh`)
- `mosh-server` installed on the remote host
- UDP ports open between client and server (default: 60000–61000)

**CLI:**
```bash
# Create a MOSH connection
rustconn-cli add --name "Server" --protocol mosh --host example.com --username user

# With options
rustconn-cli add --name "Server" --protocol mosh --host example.com \
  --mosh-predict always --mosh-port-range "60000:60010"

# Connect
rustconn-cli connect "Server"

# Dry-run to see the mosh command
rustconn-cli connect "Server" --dry-run
```

**Predict Modes:**
- **Adaptive** (default) — enables local echo prediction when latency is detected
- **Always** — always show predicted text (useful on very slow links)
- **Never** — disable prediction entirely

---

## CSV Import/Export

Import connections from CSV files or export your connections to CSV format. Follows RFC 4180 for proper handling of quoted fields, embedded delimiters, and newlines.

### CSV Import

1. **Menu → Import** or **Ctrl+I** → select **CSV** format
2. Choose the CSV file
3. RustConn auto-detects column mapping from headers (`name`, `host`, `port`, `protocol`, `username`, `group`, `tags`, `description`)
4. Review the column mapping preview and adjust if needed
5. Select delimiter (comma, semicolon, or tab)
6. Click **Import**

**Auto Column Mapping:**
If the first row contains recognized header names, columns are mapped automatically. Unknown columns are ignored with a warning.

**Tags:** Semicolon-separated in the `tags` column: `web;production;eu`

**Groups:** Slash-separated path in the `group` column: `Production/Web Servers`

### CSV Export

1. **Menu → Export** → select **CSV** format
2. Choose delimiter and fields to include
3. Click **Export**

**CLI:**
```bash
# Import from CSV
rustconn-cli import --format csv --file connections.csv
rustconn-cli import --format csv --file data.csv --delimiter ";"

# Export to CSV
rustconn-cli export --format csv --file backup.csv
rustconn-cli export --format csv --file backup.csv --delimiter ";" --fields "name,host,port,protocol"
```

---

## Session Recording

Record terminal sessions in scriptreplay-compatible format for later playback. Recordings capture terminal output with timing information and automatically sanitize sensitive data (passwords, API keys, tokens).

### Enable Recording

**Per-connection:**
1. Edit connection → **Advanced** tab
2. Enable **Session Recording**
3. Save

When recording is active, the tab title shows a **●REC** indicator.

### Recording Files

Recordings are saved to `$XDG_DATA_HOME/rustconn/recordings/` (typically `~/.local/share/rustconn/recordings/`) with two files per session:

| File | Contents |
|------|----------|
| `{name}_{timestamp}.data` | Raw terminal output bytes |
| `{name}_{timestamp}.timing` | Timing data (delay + byte count per chunk) |

### Playback

Use the standard `scriptreplay` command:

```bash
scriptreplay --timing=session.timing session.data
```

### Sanitization

Recordings automatically redact:
- Password prompts and responses
- API keys and tokens
- AWS credentials
- Private key content

---

## Text Highlighting Rules

Define regex-based patterns to highlight matching text in terminal output with custom colors. Rules can be global (apply to all connections) or per-connection.

### Built-in Defaults

| Rule | Pattern | Colors |
|------|---------|--------|
| ERROR | `ERROR` | Red foreground |
| WARNING | `WARNING` | Yellow foreground |
| CRITICAL/FATAL | `CRITICAL\|FATAL` | Red background |

### Configure Global Rules

1. **Settings → Terminal** → **Highlighting Rules** section
2. Click **Add Rule**
3. Enter rule name, regex pattern, and choose foreground/background colors
4. Toggle **Enabled** to activate/deactivate individual rules

### Configure Per-connection Rules

1. Edit connection → **Advanced** tab → **Highlighting Rules** section
2. Add rules that apply only to this connection
3. Per-connection rules take priority over global rules

### Rule Properties

| Property | Description |
|----------|-------------|
| Name | Display name for the rule |
| Pattern | Regular expression (Rust regex syntax) |
| Foreground Color | Text color in `#RRGGBB` format (optional) |
| Background Color | Background color in `#RRGGBB` format (optional) |
| Enabled | Toggle rule on/off |

Invalid regex patterns are rejected with an error message during validation.

---

## Ad-hoc Broadcast

Send keystrokes to multiple terminal sessions simultaneously. Useful for running the same command on several servers at once without setting up a cluster.

### Usage

1. Click the **Broadcast** toggle button in the toolbar (or use the keyboard shortcut)
2. Checkboxes appear on each terminal tab
3. Select the terminals you want to broadcast to
4. Type in any selected terminal — keystrokes are sent to all selected terminals
5. Click the Broadcast button again to deactivate

### Differences from Cluster Broadcast

| Feature | Ad-hoc Broadcast | Cluster Broadcast |
|---------|-----------------|-------------------|
| Setup | No setup — select terminals on the fly | Requires pre-defined cluster |
| Scope | Any open terminal tabs | Connections in a cluster |
| Persistence | Session-only | Saved in configuration |

Both broadcast modes can be used independently and do not conflict with each other.

---

## Smart Folders

Smart Folders are dynamic, filter-based views that automatically group connections matching specific criteria. Unlike regular groups, Smart Folders don't move connections — they show a live, read-only list of matching connections.

### Create a Smart Folder

1. Right-click in the **Smart Folders** sidebar section → **New Smart Folder**
2. Enter a name
3. Configure filter criteria (all filters use AND logic):

| Filter | Description | Example |
|--------|-------------|---------|
| Protocol | Match connections of a specific protocol | SSH |
| Tags | Connection must have ALL listed tags | `production`, `web` |
| Host Pattern | Glob pattern matching against host | `*.prod.example.com` |
| Parent Group | Connections in a specific group | Production |

4. Click **Create**

### Behavior

- Smart Folders appear in a dedicated sidebar section with a 🔍 icon
- Connections in Smart Folders are read-only (no drag-drop)
- Double-click a connection to connect (same as regular connections)
- Right-click a Smart Folder → **Edit** or **Delete**
- Empty filter criteria → empty result (not "match all")

### CLI

```bash
# List all smart folders
rustconn-cli smart-folders list

# Show connections matching a smart folder
rustconn-cli smart-folders show "Prod SSH"

# Create a smart folder
rustconn-cli smart-folders create --name "Prod SSH" --protocol ssh --host-pattern "*.prod.*"

# Delete a smart folder
rustconn-cli smart-folders delete "Prod SSH"
```

---

## Script Credentials

Resolve passwords dynamically by running an external script or command. The script's stdout is used as the password. This is useful for integrating with custom secret management tools, HashiCorp Vault, or any command-line credential source.

### Configure

1. Edit connection → **Authentication** tab
2. Set **Password Source** to **Script**
3. Enter the command in the script field (e.g., `vault kv get -field=password secret/myserver`)
4. Click **Test** to verify the script returns a password
5. Save

### Behavior

- The command is parsed via `shell-words` (supports quoting and escaping)
- Executed without a shell (direct process spawn) for security
- 30-second timeout — if the script doesn't complete, the connection fails with an error
- stdout is trimmed and stored as `SecretString` (zeroed on drop)
- Non-zero exit code → error with stderr message

### Examples

```bash
# HashiCorp Vault
vault kv get -field=password secret/servers/web-01

# AWS Secrets Manager
aws secretsmanager get-secret-value --secret-id myserver --query SecretString --output text

# Custom script
/usr/local/bin/get-password.sh web-01

# Pass (passwordstore.org)
pass show servers/web-01
```

### CLI

```bash
rustconn-cli add --name "Server" --protocol ssh --host example.com \
  --password-source script --password-script "vault kv get -field=password secret/myserver"
```

---

## Per-connection Terminal Theming

Override terminal colors (background, foreground, cursor) on a per-connection basis. Useful for visually distinguishing production vs. development environments.

### Configure

1. Edit connection → **Advanced** tab → **Terminal Theme** section
2. Click the color buttons to set:
   - **Background** color
   - **Foreground** (text) color
   - **Cursor** color
3. Colors are in `#RRGGBB` or `#RRGGBBAA` format
4. Click **Reset** to clear overrides and use the global theme
5. Save

### Behavior

- Color overrides are applied when the VTE terminal is created for the session
- Uses VTE `set_color_background()`, `set_color_foreground()`, `set_color_cursor()` APIs
- If no override is set, the global terminal theme is used
- Overrides are stored in the connection configuration and exported/imported with the connection

### Tips

- Use a red-tinted background for production servers
- Use a green-tinted background for development/staging
- Combine with tab coloring for maximum visual distinction

---

## CLI Usage

The `rustconn-cli` binary provides full connection management from the terminal. It shares the same configuration files as the GUI (`~/.config/rustconn/`), so changes made in either tool are immediately visible to the other.

### GUI Startup Flags

The GUI binary (`rustconn`) accepts startup flags:

```bash
rustconn --shell                        # Open local shell on startup
rustconn --connect "My Server"          # Connect by name (case-insensitive)
rustconn --connect 550e8400-...         # Connect by UUID
rustconn --version                      # Print version
rustconn --help                         # Print usage
```

These flags override the startup action configured in Settings.

### Global Options

Every `rustconn-cli` command accepts these flags:

| Flag | Short | Description |
|------|-------|-------------|
| `--config <PATH>` | `-c` | Path to configuration directory (overrides default and `RUSTCONN_CONFIG_DIR`) |
| `--verbose` | `-v` | Increase log verbosity (`-v` info, `-vv` debug, `-vvv` trace) |
| `--quiet` | `-q` | Suppress all output except errors |
| `--no-color` | — | Disable colored output (also respects `NO_COLOR` env var) |

### Environment Variables

| Variable | Description |
|----------|-------------|
| `RUSTCONN_CONFIG_DIR` | Override the default configuration directory (`~/.config/rustconn/`) |
| `NO_COLOR` | Disable colored output when set to any value (see [no-color.org](https://no-color.org)) |
| `RUST_LOG` | Override log level filter (e.g. `RUST_LOG=debug rustconn-cli list`) |

### Connection Lookup

Most commands accept a connection by name or UUID. The lookup order is:

1. Exact name match
2. UUID match
3. Case-insensitive name match
4. Prefix match (e.g. `"Prod"` matches `"Production DB"`)
5. If no match is found, fuzzy substring suggestions are shown

```
$ rustconn-cli show "prodction"
Error: Connection not found: 'prodction'. Did you mean: Production DB, Production Web?
```

### Output Formats

Commands that list data (`list`, `snippet list`, `group list`, etc.) support three output formats:

| Format | Flag | Description |
|--------|------|-------------|
| `table` | `--format table` | Human-readable table (default in terminal) |
| `json` | `--format json` | Machine-readable JSON (default when piped) |
| `csv` | `--format csv` | Comma-separated values |

When stdout is not a terminal (piped or redirected), the format automatically switches from `table` to `json` for scripting convenience. Long table output is paged through `less` when available.

### Exit Codes

| Code | Meaning |
|------|---------|
| `0` | Success |
| `1` | General error (configuration, validation, I/O, export, import) |
| `2` | Connection failure (test failed, connection not found, connection error) |

### Connection Management

#### list — List connections

```bash
rustconn-cli list                                    # All connections (table)
rustconn-cli list --format json                      # JSON output
rustconn-cli list --format csv                       # CSV output
rustconn-cli list --protocol ssh                     # Filter by protocol
rustconn-cli list --group "Production"               # Filter by group name
rustconn-cli list --tag "web"                        # Filter by tag
rustconn-cli list --group "Production" --tag "web"   # Combine filters
```

#### show — Show connection details

Displays all fields for a connection including protocol-specific configuration (SSH auth method, key path, RDP resolution, serial device settings, monitoring config).

```bash
rustconn-cli show "My Server"
rustconn-cli show 550e8400-e29b-41d4-a716-446655440000   # By UUID
```

#### add — Add a new connection

```bash
# SSH (default protocol)
rustconn-cli add --name "Web Server" --host 192.168.1.10 --user admin
rustconn-cli add --name "FIDO2 Server" --host 10.0.0.5 --key ~/.ssh/id_ed25519_sk --auth-method security-key

# RDP
rustconn-cli add --name "Windows DC" --host 10.0.0.1 --protocol rdp --user administrator

# VNC
rustconn-cli add --name "VNC Host" --host 10.0.0.2 --protocol vnc --port 5901

# Serial
rustconn-cli add --name "Router Console" --protocol serial --host /dev/ttyUSB0 --baud-rate 9600
rustconn-cli add --name "Switch" --protocol serial --device /dev/ttyACM0 --baud-rate 115200

# Telnet
rustconn-cli add --name "Legacy Switch" --host 192.168.1.1 --protocol telnet --port 23

# With custom icon
rustconn-cli add --name "DB Primary" --host db.example.com --user postgres --icon "🗄️"
```

Options:

| Flag | Short | Description |
|------|-------|-------------|
| `--name` | `-n` | Connection name (required) |
| `--host` | `-H` | Hostname, IP, or device path (required) |
| `--port` | `-p` | Port number (defaults: SSH=22, RDP=3389, VNC=5900, Telnet=23) |
| `--protocol` | `-P` | Protocol type: `ssh`, `rdp`, `vnc`, `spice`, `telnet`, `serial`, `sftp`, `kubernetes` (default: `ssh`) |
| `--user` | `-u` | Username |
| `--key` | `-k` | Path to SSH private key file |
| `--auth-method` | — | SSH auth: `password`, `publickey`, `keyboard-interactive`, `agent`, `security-key` |
| `--device` | — | Serial device path (alias for `--host` with serial protocol) |
| `--baud-rate` | — | Serial baud rate: 9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600 |
| `--icon` | — | Custom icon (emoji or GTK icon name, e.g. `"🏢"`, `"starred-symbolic"`) |

#### update — Update an existing connection

```bash
rustconn-cli update "My Server" --host 192.168.1.20 --port 2222
rustconn-cli update "My Server" --new-name "Renamed Server"
rustconn-cli update "My Server" --auth-method security-key --key ~/.ssh/id_ed25519_sk
rustconn-cli update "My Server" --icon "⭐"
```

All flags from `add` are available (except `--protocol`), plus `--new-name` to rename. Only specified fields are changed; unspecified fields remain unchanged.

#### duplicate — Duplicate a connection

```bash
rustconn-cli duplicate "My Server"                          # Creates "My Server (copy)"
rustconn-cli duplicate "My Server" --new-name "Staging"     # Custom name
```

The duplicate gets a new UUID, fresh timestamps, and no `last_connected` value.

#### delete — Delete a connection

```bash
rustconn-cli delete "My Server"              # Prompts for confirmation
rustconn-cli delete "My Server" --force      # Skip confirmation
```

In non-interactive mode (piped stdin), confirmation is automatically assumed.

#### connect — Initiate a connection

```bash
rustconn-cli connect "My Server"
rustconn-cli connect "My Server" --dry-run   # Show command without executing
```

The `--dry-run` flag prints the exact command that would be executed (e.g. `ssh -p 22 admin@192.168.1.10`), useful for debugging or scripting.

For SFTP connections, `connect` prints a hint to use `rustconn-cli sftp` instead.

#### test — Test connectivity

```bash
rustconn-cli test "My Server"                # Test single connection
rustconn-cli test all                        # Test all connections
rustconn-cli test all --timeout 5            # Custom timeout (seconds, default: 10)
```

Output shows colored pass/fail indicators with latency measurements. When testing all connections, a summary with pass rate is printed at the end. Exit code is `2` if any test fails.

### SFTP

Open an SFTP session for an SSH connection. Three modes are available:

```bash
rustconn-cli sftp "My Server"                # Open in file manager (Dolphin/Nautilus/xdg-open)
rustconn-cli sftp "My Server" --cli          # Interactive sftp CLI session
rustconn-cli sftp "My Server" --mc           # Open in Midnight Commander
```

The command automatically manages SSH agent keys before connecting. Only SSH connections are supported; other protocols return an error.

### Import & Export

#### export

```bash
rustconn-cli export --format native --output backup.rcn
rustconn-cli export --format ansible --output inventory.yml
rustconn-cli export --format ssh-config --output config
rustconn-cli export --format remmina --output ~/remmina-export/
rustconn-cli export --format royal-ts --output connections.rtsz
rustconn-cli export --format moba-xterm --output sessions.mxtsessions
rustconn-cli export --format asbru --output asbru.yml
```

| Format | Description |
|--------|-------------|
| `native` | RustConn native format (`.rcn`) — preserves all fields |
| `ansible` | Ansible inventory (YAML) |
| `ssh-config` | OpenSSH config format |
| `remmina` | Remmina `.remmina` files |
| `asbru` | Asbru-CM YAML |
| `royal-ts` | Royal TS JSON (`.rtsz`) |
| `moba-xterm` | MobaXterm sessions (`.mxtsessions`) |

#### import

```bash
rustconn-cli import --format ssh-config ~/.ssh/config
rustconn-cli import --format remmina ~/remmina/
rustconn-cli import --format native backup.rcn
rustconn-cli import --format ansible inventory.yml
rustconn-cli import --format royal-ts connections.rtsz
rustconn-cli import --format moba-xterm sessions.mxtsessions
rustconn-cli import --format asbru asbru.yml
```

Import formats match the export formats listed above. Passwords are never included in import/export files — re-enter them after importing.

### Sync

Synchronize connections from an external inventory source (JSON or YAML). Useful for keeping RustConn in sync with a CMDB, NetBox, or Ansible inventory.

```bash
rustconn-cli sync inventory.json --source netbox
rustconn-cli sync inventory.yml --source ansible --remove-stale
rustconn-cli sync inventory.json --source netbox --dry-run
```

| Flag | Description |
|------|-------------|
| `--source` | Source identifier for tagging synced connections (required) |
| `--remove-stale` | Remove connections from this source that are no longer in the inventory |
| `--dry-run` | Show what would change without saving |

The sync report shows added, updated, removed, and skipped counts.

### Groups

```bash
rustconn-cli group list                                          # List all groups
rustconn-cli group list --format json                            # JSON output
rustconn-cli group show "Production"                             # Show group details and connections
rustconn-cli group create --name "Staging"                       # Create a top-level group
rustconn-cli group create --name "EU" --parent "Production"      # Create a child group
rustconn-cli group create --name "DB" --description "Databases" --icon "🗄️"
rustconn-cli group add-connection -g "Production" -c "Web-01"    # Add connection to group
rustconn-cli group remove-connection -g "Production" -c "Web-01" # Remove from group
rustconn-cli group delete "Old Group"                            # Delete a group
```

Groups support hierarchical nesting via `--parent`. The `show` subcommand lists all connections belonging to the group.

### Templates

Templates define reusable connection presets. Create a template once, then apply it to create connections with pre-filled fields.

```bash
rustconn-cli template list                                       # List all templates
rustconn-cli template list --protocol ssh                        # Filter by protocol
rustconn-cli template show "SSH Bastion"                         # Show template details
rustconn-cli template create --name "SSH Bastion" --protocol ssh --port 2222 --user ops
rustconn-cli template create --name "RDP Standard" --protocol rdp --description "Standard RDP"
rustconn-cli template delete "Old Template"

# Create a connection from a template
rustconn-cli template apply "SSH Bastion" --name "Prod Bastion" --host bastion.example.com
rustconn-cli template apply "SSH Bastion" --host 10.0.0.1 --user admin   # Override fields
```

When applying a template, `--host` and `--name` fill in the connection-specific values. `--port` and `--user` can override the template defaults.

### Clusters

Clusters group connections for broadcast command execution (send the same input to all sessions simultaneously).

```bash
rustconn-cli cluster list
rustconn-cli cluster show "Web Servers"                          # Show cluster and its connections
rustconn-cli cluster create --name "DB Cluster" --broadcast      # Create with broadcast enabled
rustconn-cli cluster create --name "Mixed" --connections "DB-01,DB-02,Web-01"  # Pre-populate
rustconn-cli cluster add-connection -C "DB Cluster" -c "DB-01"
rustconn-cli cluster remove-connection -C "DB Cluster" -c "DB-01"
rustconn-cli cluster delete "Old Cluster"
```

### Snippets

Snippets are reusable command templates with variable substitution. Variables use `${var}` syntax.

```bash
rustconn-cli snippet list
rustconn-cli snippet list --category deploy                      # Filter by category
rustconn-cli snippet list --tag production                       # Filter by tag
rustconn-cli snippet show "Deploy Script"                        # Show details and variables

# Create a snippet
rustconn-cli snippet add --name "Restart" --command "sudo systemctl restart \${service}"
rustconn-cli snippet add --name "Deploy" --command "cd /app && git pull && \${restart_cmd}" \
    --category deploy --tags "production,ci" --description "Standard deploy"

# Run a snippet (preview mode — prints the expanded command)
rustconn-cli snippet run "Deploy" --var service=nginx --var restart_cmd="systemctl restart nginx"

# Run a snippet (execute mode — actually runs the command)
rustconn-cli snippet run "Deploy" --var service=nginx --execute

rustconn-cli snippet delete "Old Snippet"
```

The `run` subcommand without `--execute` only prints the expanded command (safe preview). With `--execute`, it runs the command via `sh -c` and warns about shell metacharacters in variable values.

### Variables

Global variables can be referenced in snippets and connection templates. Secret variables are masked in output.

```bash
rustconn-cli var list
rustconn-cli var list --format json
rustconn-cli var show "my_var"
rustconn-cli var set my_var "my_value"
rustconn-cli var set api_key "secret123" --secret                # Masked in output
rustconn-cli var set deploy_env "staging" --description "Current deploy target"
rustconn-cli var delete "my_var"
```

### Secrets

Manage credentials stored in secret backends. Supports system keyring (libsecret), KeePass (KDBX), Bitwarden, 1Password, Passbolt, and Pass.

```bash
rustconn-cli secret status                                       # Show backend availability
rustconn-cli secret get "My Server"                              # Get credentials (default backend)
rustconn-cli secret get "My Server" --backend keepass            # Specific backend
rustconn-cli secret set "My Server"                              # Store (prompts for password)
rustconn-cli secret set "My Server" --user admin --backend keyring
rustconn-cli secret delete "My Server"
rustconn-cli secret delete "My Server" --backend bitwarden
rustconn-cli secret verify-keepass --database ~/vault.kdbx
rustconn-cli secret verify-keepass -d ~/vault.kdbx -k ~/key.key  # With key file
```

| Subcommand | Description |
|------------|-------------|
| `status` | Show available backends and their configuration status |
| `get` | Retrieve credentials for a connection |
| `set` | Store credentials (interactive password prompt if `--password` is omitted) |
| `delete` | Delete credentials from a backend |
| `verify-keepass` | Verify that a KeePass database can be unlocked |

Backend aliases:

| Backend | Aliases |
|---------|---------|
| System keyring (libsecret) | `keyring`, `libsecret` |
| KeePass KDBX | `keepass`, `kdbx` |
| Bitwarden | `bitwarden`, `bw` |
| 1Password | `1password`, `op` |
| Passbolt | `passbolt` |
| Pass (passwordstore.org) | `pass` |

> **Security note:** Prefer the interactive password prompt (omit `--password`) over passing passwords on the command line, which may be visible in process listings.

### Wake-on-LAN

Send magic packets to wake sleeping machines. Accepts a MAC address directly or a connection name (if the connection has WoL configured).

```bash
rustconn-cli wol AA:BB:CC:DD:EE:FF
rustconn-cli wol "Server Name"                                   # Uses MAC from connection config
rustconn-cli wol AA:BB:CC:DD:EE:FF --broadcast 192.168.1.255 --port 9
```

Three packets are sent with retry. Default broadcast address is `255.255.255.255`, default port is `9`.

### Statistics

```bash
rustconn-cli stats
```

Shows a summary: total connections by protocol, groups, templates, clusters, snippets, variables, recently used connections (last 7 days), and ever-connected count.

### Shell Completions & Man Page

```bash
# Generate shell completions
rustconn-cli completions bash > ~/.local/share/bash-completion/completions/rustconn-cli
rustconn-cli completions zsh > ~/.local/share/zsh/site-functions/_rustconn-cli
rustconn-cli completions fish > ~/.config/fish/completions/rustconn-cli.fish

# Generate man page
rustconn-cli man-page > ~/.local/share/man/man1/rustconn-cli.1
```

Supported shells: `bash`, `zsh`, `fish`, `elvish`, `powershell`.

### Scripting Examples

```bash
# Backup all connections to JSON
rustconn-cli list --format json > connections-backup.json

# Test all connections and fail CI if any are unreachable
rustconn-cli test all --timeout 5 || echo "Some connections failed"

# List SSH connections as CSV for processing
rustconn-cli list --format csv --protocol ssh | tail -n +2 | while IFS=, read -r name host port _; do
    echo "Checking $name at $host:$port"
done

# Dry-run a connection to see the command
rustconn-cli connect "Production DB" --dry-run

# Sync from inventory with dry-run first
rustconn-cli sync inventory.yml --source ansible --dry-run
rustconn-cli sync inventory.yml --source ansible --remove-stale
```

---

## Frequently Asked Questions

### Where are my passwords stored?

RustConn never stores passwords in plain text. Depending on your configured secret backend:

- **libsecret** (default): Stored in your desktop keyring (GNOME Keyring, KDE Wallet)
- **KeePassXC**: Stored in your KeePassXC database via browser integration protocol
- **KDBX file**: Stored in a local KeePass-format database encrypted with your master password
- **Bitwarden / 1Password / Passbolt**: Stored in the respective cloud vault; RustConn retrieves them on demand
- **Pass**: Stored in GPG-encrypted files under `~/.password-store/`

Connection files themselves (in `~/.config/rustconn/connections/`) contain only metadata and credential references, never actual passwords.

### How do I migrate RustConn to another machine?

The simplest approach is [Backup & Restore](#backup--restore):

1. On the old machine: **Settings → Interface → Backup & Restore → Backup**
2. Copy the resulting ZIP file to the new machine
3. On the new machine: **Settings → Interface → Backup & Restore → Restore**
4. Restart RustConn

This exports connections, groups, snippets, clusters, templates, history, and settings. Passwords are not included in the backup since they live in your secret backend. You will need to re-enter credentials or configure the same secret backend on the new machine.

Alternatively, copy `~/.config/rustconn/` manually.

### Can I use RustConn without a secret backend?

Yes. If no external backend is configured, RustConn uses libsecret (your desktop keyring) by default. If libsecret is unavailable (e.g., headless or minimal desktop), you can use a local KDBX file as a fully offline backend.

### How do I share connections with my team?

1. Select the connections or group you want to share
2. **File > Export** and choose a format (Native `.rcn` preserves all fields; SSH Config or CSV for interoperability)
3. Send the exported file to your colleagues
4. They import it via **File > Import**

Passwords are never included in exports. Each team member configures their own credentials.

### Why does RustConn ask for my keyring password on startup?

Your desktop keyring (GNOME Keyring, KDE Wallet) may be locked. RustConn requests access to retrieve stored credentials. To avoid this prompt, configure your keyring to unlock automatically on login, or switch to a different secret backend (e.g., KeePassXC, KDBX file).

### How do I connect to a host behind a jump server?

In the SSH connection dialog, go to the **Advanced** tab and set the **Proxy Jump** field to your bastion host (e.g., `user@bastion.example.com`). RustConn passes this as `-J` to the SSH command. You can chain multiple jump hosts separated by commas.

### Can I run RustConn in Flatpak?

Yes. RustConn is available as a Flatpak. Some external CLI tools (xfreerdp, vncviewer, picocom) are not bundled but can be downloaded via **Tools > Flatpak Components**. Serial device access requires additional Flatpak permissions. See [Flatpak Components](#flatpak-components) for details.

### How do I reset RustConn to default settings?

Remove or rename the configuration directory:

```bash
mv ~/.config/rustconn ~/.config/rustconn.backup
```

On next launch, RustConn creates fresh defaults. Your backup remains available if you need to restore specific files.

---

## Migration Guide

This guide covers end-to-end migration from other connection managers to RustConn.

### From Remmina

Remmina stores connections as individual `.remmina` files in `~/.local/share/remmina/`.

1. **File > Import > Remmina**
2. Select the Remmina data directory or individual `.remmina` files
3. Review the import preview: protocol, host, port, and username are mapped automatically
4. Choose a merge strategy if you have existing connections (Skip, Overwrite, or Rename)
5. Click **Import**

Mapped fields: protocol (SSH, RDP, VNC, SFTP), host, port, username, SSH key path, color depth, resolution. Fields not mapped: Remmina-specific plugins, custom scripts.

After import, re-enter passwords (Remmina encrypts them with its own key) and verify SSH key paths.

### From MobaXterm

MobaXterm stores sessions in its configuration file (`MobaXterm.ini`) or the Windows registry.

1. Export sessions from MobaXterm: **Settings > Configuration > Export**
2. Copy the `.mxtsessions` file to your Linux machine
3. **File > Import > MobaXterm**
4. Select the exported file
5. Review and import

Mapped fields: protocol (SSH, RDP, VNC, Telnet, Serial), host, port, username, SSH key. Fields not mapped: MobaXterm macros, X11 forwarding settings, tunnels (configure manually in RustConn).

### From Royal TS

Royal TS uses `.rtsz` (JSON-based) export files.

1. In Royal TS: **File > Export > Royal TS Document (.rtsz)**
2. Copy the file to your Linux machine
3. **File > Import > Royal TS**
4. Select the `.rtsz` file
5. Review the import preview: folder structure is preserved as RustConn groups

Mapped fields: protocol, host, port, username, description, folder hierarchy. Fields not mapped: Royal TS credentials (re-enter in RustConn), custom plugins, tasks.

### From SSH Config

If you already have an `~/.ssh/config` file with your hosts defined:

1. **File > Import > SSH Config**
2. Select your SSH config file (default: `~/.ssh/config`)
3. Each `Host` block becomes an SSH connection in RustConn

Mapped fields: HostName, Port, User, IdentityFile, ProxyJump. Fields not mapped: Match blocks, LocalForward, RemoteForward (configure manually).

### From Ansible Inventory

1. **File > Import > Ansible**
2. Select your inventory file (INI or YAML format)
3. Host groups become RustConn groups; hosts become SSH connections

Mapped fields: ansible_host, ansible_port, ansible_user, ansible_ssh_private_key_file. Group variables are applied to all hosts in the group.

### From Libvirt / GNOME Boxes

Libvirt stores VM definitions as XML files in `/etc/libvirt/qemu/` (system) and `~/.config/libvirt/qemu/` (user session). GNOME Boxes uses the same user-session directory.

**Auto-scan (recommended):**

1. **File > Import > Libvirt / GNOME Boxes**
2. RustConn scans both directories automatically
3. Review the import preview: each VM with a graphical console becomes a VNC, SPICE, or RDP connection
4. Click **Import**

**Single file (for remote hosts or `virsh dumpxml`):**

1. On the remote host, run: `virsh dumpxml <domain-name> > myvm.xml`
2. Copy the XML file to your machine
3. **File > Import > Libvirt XML File**
4. Select the XML file
5. Review and import

Mapped fields: VM name, UUID (tag), description, graphics type (VNC/SPICE/RDP), listen address, port, TLS port, password. Fields not mapped: disk images, network interfaces, CPU/memory (these are hypervisor settings, not connection parameters).

After import: if VMs use `autoport`, the actual port is assigned at VM startup — edit the connection port after starting the VM. Passwords from XML are stored in your configured secret backend.

### Post-Migration Checklist

After importing from any source:

- [ ] Re-enter passwords (no import format includes plaintext credentials)
- [ ] Verify SSH key paths (may differ between Windows and Linux)
- [ ] Test a connection from each protocol type
- [ ] Organize imported connections into groups if the source format did not preserve hierarchy
- [ ] Set up your preferred secret backend if you have not already
- [ ] Delete the import source file if it contains sensitive data

---

## Configuration Sync Between Machines

RustConn stores all configuration in `~/.config/rustconn/`:

```
~/.config/rustconn/
├── config.toml           # Application settings
├── connections.toml      # Connections (hosts, ports, usernames)
├── groups.toml           # Group hierarchy and credentials
├── snippets.toml         # Command snippets
├── clusters.toml         # Broadcast clusters
├── templates.toml        # Connection templates
├── smart_folders.toml    # Smart Folders
├── history.toml          # Connection history (local)
└── trash.toml            # Trash (local)
```

### Git (Recommended)

```bash
cd ~/.config/rustconn
git init
echo "history.toml" >> .gitignore
echo "trash.toml" >> .gitignore
git add -A && git commit -m "Initial config"
git remote add origin <your-repo-url>
git push -u origin main
```

On the second machine:
```bash
git clone <your-repo-url> ~/.config/rustconn
```

Advantages: version history, merge conflict visibility, offline support.

### Syncthing / rsync

```bash
# rsync (one-time or cron):
rsync -avz ~/.config/rustconn/ user@remote:~/.config/rustconn/

# Syncthing: add ~/.config/rustconn as a shared folder
```

### CLI Export/Import

```bash
# Export on machine A:
rustconn-cli export --format json --output connections.json

# Import on machine B:
rustconn-cli import --format json connections.json
```

### Built-in Backup/Restore

Settings → Backup → Export creates a ZIP archive of all configuration files. Import the archive on another machine via Settings → Backup → Import.

**Tips:**
- `history.toml` and `trash.toml` are machine-local — exclude them from sync
- Passwords stored in KeePass/libsecret/Bitwarden are not in the config files — sync your vault separately
- After syncing, restart RustConn to pick up changes

---

## Troubleshooting

### Connection Issues

1. Verify host/port: `ping hostname`
2. Check credentials
3. SSH key permissions: `chmod 600 ~/.ssh/id_rsa`
4. Firewall settings

### 1Password Not Working

1. Install 1Password CLI: download from 1password.com/downloads/command-line
2. Sign in: `op signin` (requires 1Password desktop app for biometric auth)
3. Or use service account: set `OP_SERVICE_ACCOUNT_TOKEN` environment variable
4. Select 1Password backend in Settings → Secrets
5. Check account status indicator
6. For password source, select "1Password" in connection dialog

### Bitwarden Not Working

See [BITWARDEN_SETUP.md](BITWARDEN_SETUP.md) for a detailed step-by-step guide.

**Quick checklist:**

1. Install Bitwarden CLI:
   - **Flatpak:** Menu → Flatpak Components → Install Bitwarden CLI (host-installed `bw` is NOT accessible inside the sandbox)
   - **Native:** `npm install -g @bitwarden/cli` or download from bitwarden.com
2. For self-hosted servers: `bw config server https://your-server` **before** logging in
3. Login: `bw login`
4. Unlock vault: `bw unlock`
5. Select Bitwarden backend in Settings → Secrets
6. Check vault status indicator
7. For 2FA methods not supported by CLI (FIDO2, Duo), use API key authentication:
   - Get API key from Bitwarden web vault → Settings → Security → Keys
   - Enable "Use API key authentication" in Settings → Secrets
   - Enter Client ID and Client Secret
8. Enable "Save to system keyring" for automatic vault unlock on startup
9. For password source, select "Vault" in connection dialog

**Common error — "Failed to run bw: No such file or directory":**
This means `bw` is not found in PATH. Flatpak users must install `bw` via Flatpak Components — the host system `bw` binary is not visible inside the sandbox.

### System Keyring Not Working

1. Install `libsecret-tools`: `sudo apt install libsecret-tools` (Debian/Ubuntu) or `sudo dnf install libsecret` (Fedora)
2. Verify: `secret-tool --version`
3. Ensure a Secret Service provider is running (GNOME Keyring, KDE Wallet)
4. If "Install libsecret-tools for keyring" warning appears, install the package above
5. "Save password" and "Save to system keyring" are mutually exclusive — only one can be active
6. **Flatpak users:** `secret-tool` is bundled in the Flatpak package — no separate installation needed. Ensure your desktop has a Secret Service provider (GNOME Keyring or KDE Wallet)

### Passbolt Not Working

1. Install Passbolt CLI (`go-passbolt-cli`): download from github.com/passbolt/go-passbolt-cli
2. Configure: `passbolt configure --serverAddress https://your-server.com --userPrivateKeyFile key.asc --userPassword`
3. Verify: `passbolt list resource`
4. Select Passbolt backend in Settings → Secrets
5. For password source, select "Vault" in connection dialog

### KeePass Not Working

1. Install KeePassXC
2. Enable browser integration in KeePassXC
3. Configure KDBX path in Settings → Secrets
4. Provide password/key file
5. For password source, select "KeePass" in connection dialog
6. **Flatpak users:** KeePassXC installed on the host system is detected and used automatically via `flatpak-spawn --host`. All KDBX operations (read, write, delete, verify, group management) work transparently inside the sandbox. No additional configuration needed.

### Pass (passwordstore.org) Not Working

1. Install `pass`: `sudo apt install pass` (Debian/Ubuntu) or `sudo dnf install pass` (Fedora)
2. Initialize store: `pass init <gpg-id>`
3. Verify: `pass ls`
4. Select Pass backend in Settings → Secrets
5. Optionally set custom `PASSWORD_STORE_DIR` if not using `~/.password-store`
6. For password source, select "Vault" in connection dialog

### Embedded RDP/VNC Issues

1. Check IronRDP/vnc-rs features enabled
2. For external: verify FreeRDP/TigerVNC installed
3. **Flatpak:** FreeRDP (SDL3 client) is bundled in the Flatpak build — external RDP works out of the box without installing anything. TigerVNC can be installed via Flatpak Components.
4. Wayland vs X11 compatibility
5. HiDPI/4K: IronRDP sends scale factor automatically; use Scale Override in connection dialog if remote UI is too small or too large
6. FreeRDP passwords are passed via stdin (`/from-stdin`), not command-line arguments
7. Clipboard not syncing: ensure "Clipboard" is enabled in RDP connection settings; text is synced automatically via CLIPRDR channel, Copy/Paste buttons are manual fallback
8. RDP Gateway: IronRDP does not support RD Gateway; connections with a gateway configured automatically fall back to external FreeRDP with a toast notification

### Session Restore Issues

1. Enable in Settings → Interface page → Session Restore
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

### Serial Device Access

1. Add your user to the `dialout` group: `sudo usermod -aG dialout $USER`
2. Log out and back in for group changes to take effect
3. Verify device permissions: `ls -la /dev/ttyUSB0`
4. **Flatpak users:** Serial devices require the `--device=all` permission. If using the Flathub build, file a request if serial access is blocked
5. Ensure `picocom` is installed: `sudo apt install picocom` (Debian/Ubuntu) or `sudo dnf install picocom` (Fedora)

### Kubernetes Connection Issues

1. Verify `kubectl` is installed and in PATH
2. Check cluster access: `kubectl cluster-info`
3. Verify pod exists: `kubectl get pods -n <namespace>`
4. Check container name if pod has multiple containers
5. For busybox mode: ensure the target container has `/bin/sh` available
6. **Flatpak users:** `kubectl` must be installed via Flatpak Components — the host binary is not accessible inside the sandbox

### Flatpak Permissions

If features are not working in the Flatpak build:

1. **File access:** Flatpak has limited filesystem access. Use `flatpak override --user --filesystem=home io.github.totoshko88.RustConn` for broader access
2. **SSH agent:** The Flatpak build forwards `SSH_AUTH_SOCK` from the host via `--socket=ssh-auth`. However, this hard-overwrites `SSH_AUTH_SOCK` inside the sandbox, which means:
   - Custom socket paths set in Settings → SSH Agent are limited to paths accessible inside the sandbox (`~/.var/app/...`, `$XDG_RUNTIME_DIR/...`)
   - 1Password SSH agent socket (`~/.1password/agent.sock`) is **not** mounted by default — add it via `flatpak override --user --filesystem=home/.1password:ro io.github.totoshko88.RustConn`
   - A sandbox-internal `ssh-agent` is not accessible to host processes (e.g., file managers launched via `xdg-open`)
   - KeePassXC agent socket (`$XDG_RUNTIME_DIR/ssh-agent/`) and Bitwarden agent socket (`~/.var/app/com.bitwarden.desktop/data/`) are pre-configured in the Flatpak manifest
3. **Serial devices:** Requires `--device=all` permission
4. **CLI tools:** Host-installed binaries (bw, kubectl, pass, op) are NOT visible inside the sandbox. Use Menu → Flatpak Components to install them
5. **Secret Service:** GNOME Keyring / KDE Wallet access works via D-Bus portal
6. **KeePassXC:** Host-installed `keepassxc-cli` is detected and used automatically via `flatpak-spawn --host` — no manual configuration needed
7. **Zero Trust / Kubernetes:** Cloud CLIs (`aws`, `gcloud`, `az`, `kubectl`) on the host are detected and executed via `flatpak-spawn --host`. Config directories (`~/.aws`, `~/.config/gcloud`, `~/.azure`, `~/.kube`) are mounted into the sandbox so credentials are shared. CLI config paths are redirected to writable sandbox directories via `CLOUDSDK_CONFIG`, `AZURE_CONFIG_DIR`, `TELEPORT_HOME`, `OCI_CLI_CONFIG_FILE`. Boundary uses the system keyring via D-Bus
8. **FreeRDP:** Bundled in the Flatpak build (SDL3 client). External RDP works out of the box on Wayland without `DISPLAY`

### Monitoring Issues

1. Verify SSH connection works normally before enabling monitoring
2. Check that the remote host has `uptime`, `free`, `df`, and `cat /proc/loadavg` available
3. Monitoring uses a separate SSH session — ensure `MaxSessions` in `sshd_config` allows multiple sessions
4. If metrics show "N/A", the remote command may have timed out — increase the polling interval in Settings → Connection → Monitoring

---

## Security Best Practices

### Choosing a Secret Backend

RustConn supports 7 secret backends. Choose based on your environment:

| Backend | Best For | Security Level |
|---------|----------|---------------|
| System Keyring (libsecret) | Desktop Linux with GNOME Keyring or KDE Wallet | High — OS-managed, session-locked |
| KeePassXC | Users who already use KeePassXC | High — AES-256 encrypted database |
| Bitwarden | Teams using Bitwarden | High — cloud-synced, E2E encrypted |
| 1Password | Teams using 1Password | High — cloud-synced, E2E encrypted |
| Passbolt | Self-hosted team password management | High — GPG-based |
| Pass (passwordstore.org) | CLI-oriented users, git-synced passwords | High — GPG-encrypted files |
| KDBX File | Offline/air-gapped environments | High — AES-256, local file only |

Configure your preferred backend in Settings → Secrets. RustConn falls back to the system keyring if the preferred backend is unavailable.

### Master Password

RustConn can encrypt its configuration with a master password:

- Set via Settings → Secrets → **Master Password**
- Protects the local connection database (`connections.json`)
- Uses Argon2 key derivation + AES-256-GCM encryption
- You will be prompted for the master password on startup

If you forget the master password, the encrypted configuration cannot be recovered. Keep a backup of your unencrypted export (`.rcn` file) in a secure location.

### Credential Hygiene

- Use **SSH keys** instead of passwords whenever possible (Ed25519 or ECDSA recommended)
- Use **FIDO2/Security Keys** for the strongest SSH authentication (requires OpenSSH 8.2+)
- Set **Password Source** to a vault backend (KeePass, Bitwarden, etc.) rather than storing passwords in the RustConn config
- Use **Group Credentials** to avoid duplicating the same password across multiple connections
- Enable **Inherit from Group** on child connections to centralize credential management
- Rotate credentials regularly; RustConn resolves passwords from the vault at connection time, so updating the vault entry is sufficient

### Configuration Backup

RustConn stores its data in `~/.config/rustconn/`:

| File | Contents |
|------|----------|
| `connections.json` | All connections and groups |
| `settings.json` | Application settings |
| `templates.json` | Connection templates |
| `snippets.json` | Command snippets |
| `clusters.json` | Cluster definitions |
| `keybindings.json` | Custom keyboard shortcuts |
| `variables.json` | Global variables |

**Backup options:**

1. **Native export** — `rustconn-cli export --format native --output backup.rcn` (includes connections, groups, credentials references)
2. **Copy config directory** — `cp -r ~/.config/rustconn/ ~/backup/rustconn/`
3. **Flatpak path** — `~/.var/app/io.github.totoshko88.RustConn/config/rustconn/`

Passwords stored in external vaults (KeePass, Bitwarden, etc.) are not included in the config backup — back up those separately.

### Network Security

- RustConn performs a **pre-connect port check** before establishing connections (disable per-connection with "Skip Port Check")
- SSH connections verify host keys via the system `known_hosts` file
- Use **SSH Proxy Jump** for connections behind bastion hosts instead of exposing internal hosts
- Use **Zero Trust providers** (AWS SSM, Teleport, Boundary, etc.) to eliminate direct SSH exposure
- Enable **session logging** for audit trails (Settings → Connection → Session Logging)

---

## Support

- **GitHub:** https://github.com/totoshko88/RustConn
- **Issues:** https://github.com/totoshko88/RustConn/issues
- **Releases:** https://github.com/totoshko88/RustConn/releases

**Made with ❤️ in Ukraine 🇺🇦**
