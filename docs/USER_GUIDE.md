# RustConn User Guide

**Version 0.16.7** | GTK4/libadwaita Connection Manager for Linux

RustConn is a modern connection manager designed for Linux with Wayland-first approach. It supports SSH, RDP, VNC, SPICE, MOSH, SFTP, Telnet, Serial, Kubernetes, Web protocols and Zero Trust integrations through a native GTK4/libadwaita interface.

## Table of Contents

1. [Getting Started](#getting-started)
2. [Main Interface](#main-interface)
3. [Connections](#connections)
4. [Protocols](#protocols)
   - [SSH](#ssh)
   - [RDP](#rdp)
   - [VNC](#vnc)
   - [SPICE](#spice)
   - [MOSH](#mosh-protocol)
   - [Telnet](#telnet)
   - [Serial Console](#serial-console)
   - [Kubernetes](#kubernetes-shell)
   - [SFTP](#sftp-file-browser)
   - [Zero Trust Providers](#zero-trust-providers)
   - [Web Bookmarks](#web-bookmarks)
5. [Sessions & Terminal](#sessions--terminal)
   - [Session Types & Display Modes](#session-types)
   - [Tab Management](#tab-management)
   - [Split View](#split-view)
   - [Terminal Search](#terminal-search)
   - [Session Restore & Reconnect](#session-restore)
   - [Session Logging](#session-logging)
   - [Session Recording](#session-recording)
   - [Terminal Activity Monitor](#terminal-activity-monitor)
   - [Text Highlighting Rules](#text-highlighting-rules)
   - [Per-connection Terminal Theming](#per-connection-terminal-theming)
6. [Organization](#organization)
   - [Groups](#groups)
   - [Group Automation](#group-automation-expect-rules--post-login-scripts)
   - [Favorites](#favorites)
   - [Smart Folders](#smart-folders)
   - [Dynamic Folders](#dynamic-folders)
   - [Custom Icons](#custom-icons)
   - [Tab Coloring](#tab-coloring)
   - [Tab Grouping](#tab-grouping)
7. [Productivity Tools](#productivity-tools)
   - [Templates](#templates)
   - [Snippets](#snippets)
   - [Clusters & Broadcast](#clusters)
   - [Ad-hoc Broadcast](#ad-hoc-broadcast)
   - [Command Palette](#command-palette)
   - [Global Variables](#global-variables)
   - [Password Generator](#password-generator)
   - [Wake-on-LAN](#wake-on-lan)
   - [Connection History & Statistics](#connection-history)
   - [Encrypted Documents](#encrypted-documents)
   - [Remote Monitoring](#remote-monitoring)
   - [SSH Tunnel Manager](#ssh-tunnel-manager)
8. [Settings](#settings)
   - [Custom Keybindings](#custom-keybindings)
   - [Adaptive UI](#adaptive-ui)
   - [Startup Action](#startup-action)
   - [Backup & Restore](#backup--restore)
9. [Import, Export & Migration](#import-export--migration)
   - [Import](#import-ctrli)
   - [Export](#export-ctrlshifte)
   - [CSV Import/Export](#csv-importexport)
   - [RDP File Association](#rdp-file-association)
   - [Migration Guide](#migration-guide)
   - [Configuration Sync Between Machines](#configuration-sync-between-machines)
10. [Cloud Sync](#cloud-sync)
    - [Group Sync](#group-sync)
    - [Simple Sync](#simple-sync)
    - [SSH Key Inheritance](#ssh-key-inheritance)
    - [Credential Resolution](#credential-resolution)
11. [Security](#security)
    - [Secret Backends](#choosing-a-secret-backend)
    - [Credential Hygiene](#credential-hygiene)
    - [Network Security](#network-security)
12. [Troubleshooting & FAQ](#troubleshooting--faq)
13. [Keyboard Shortcuts](#keyboard-shortcuts)
14. [CLI Reference](CLI_REFERENCE.md)

---

## Getting Started

### Quick Start

1. Install RustConn (see [INSTALL.md](INSTALL.md))
2. Launch from application menu or run `rustconn`
3. Create your first connection with **Ctrl+N**
4. Double-click to connect

### First Connection

1. Press **Ctrl+N** or click **+** in header bar — the **Connection Wizard** opens
2. Select a protocol (SSH, RDP, VNC, SPICE, MOSH, SFTP, Telnet, Serial, Kubernetes, Zero Trust, Web)
3. Enter host and connection details
4. Configure authentication (password, SSH key, or SSH Agent) and terminal theme
5. Click **Save & Connect**
6. For advanced options, click **Advanced…** at any step to open the full connection editor

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

### Connection Wizard (Ctrl+N)

The Connection Wizard provides a streamlined 3-step flow for creating connections:

**Step 1 — Protocol Selection:**
Protocols are displayed in a grouped FlowBox layout that wraps adaptively on narrow windows:
- **Secure Shell:** SSH, MOSH, SFTP
- **Remote Desktop:** RDP, VNC, SPICE
- **Terminal:** Telnet, Serial, Custom Command
- **Other:** Kubernetes, Zero Trust, Web

**Step 2 — Connection Details:**
Adaptive form based on the selected protocol:
- Host, Port, Username (for network protocols)
- Jump Host dropdown for SSH tunneling (SSH, MOSH, SFTP, RDP, VNC, SPICE)
- Device and Baud Rate (Serial)
- Context, Namespace, Pod, Container (Kubernetes)
- Provider-specific fields (Zero Trust)
- URL (Web)

**Step 3 — Authentication & Finish:**
- Authentication method: Password, Key File, or SSH Agent (SSH family)
- Password only (RDP, VNC, SPICE)
- Terminal theme selector for VTE-based protocols
- Custom icon (emoji or icon name)
- **Save** or **Save & Connect** buttons

Click **Advanced…** at any step to open the full connection editor with all entered data pre-filled.

### Full Connection Editor (Ctrl+Shift+N)

The full editor provides access to all connection options:**Basic Tab:**
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

**Security Key / FIDO2 Authentication (SSH):**
SSH connections support hardware security keys (YubiKey, SoloKey, etc.) via the `security-key` auth method. Requirements:
- OpenSSH 8.2+ on both client and server
- `libfido2` installed on the client (`sudo apt install libfido2-1`)
- An `ed25519-sk` or `ecdsa-sk` key generated with `ssh-keygen -t ed25519-sk`
- The key file path configured in the connection's SSH key field

**Advanced Tabs:**
- **Advanced** — Window mode (Embedded/External/Fullscreen), remember window position, hide local cursor (embedded RDP/VNC/SPICE), Wake-on-LAN configuration (MAC address, broadcast, port, wait time), monitoring override (enable/disable per connection, overrides global setting)
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

> **Tip:** You can also configure Expect Rules at the group level (Edit Group → **Automation** tab). Connections with empty automation config automatically inherit rules from their parent group chain. See [Group Automation](#group-automation-expect-rules--post-login-scripts) for details.

**Examples:**
| Pattern | Response | Use Case |
|---------|----------|----------|
| `password:` | `${password}` | Auto-login with vault password |
| `\[sudo\] password` | `${password}` | Sudo password prompt |
| `Are you sure.*continue` | `yes` | SSH host key confirmation |
| `Select option:` | `2` | Menu navigation |
| `Enter token:` | `${MFA_TOKEN}` | Use a global variable for MFA |

Rules execute in priority order. After matching, the response is sent followed by Enter.

**Variable Substitution in Responses:**

Expect rule responses support `${VARIABLE_NAME}` placeholders that are resolved at connection time using Global Variables. This lets you use dynamic values (passwords, tokens, environment-specific strings) without hardcoding them into rules.

- `${password}` — resolves to the connection's password from the configured secret backend
- `${MY_VARIABLE}` — resolves to the value of the global variable `MY_VARIABLE` (defined in Menu → Tools → Variables)
- Undefined variables remain as literal text (e.g., `${UNKNOWN}` is sent as-is)
- If substitution fails, the raw response text is used as a fallback (with a warning in the log)

**Example — multi-step login with variables:**

| Pattern | Response | Description |
|---------|----------|-------------|
| `Username:` | `${PROD_USER}` | Global variable with username |
| `Password:` | `${password}` | Password from vault |
| `Verification code:` | `${OTP_CODE}` | Global variable (set before connecting) |
| `Select environment` | `production` | Static text, no variable needed |

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

### Script Credentials

Resolve passwords dynamically by running an external script or command. The script's stdout is used as the password. This is useful for integrating with custom secret management tools, HashiCorp Vault, or any command-line credential source.

**Configure:**
1. Edit connection → **Authentication** tab
2. Set **Password Source** to **Script**
3. Enter the command in the script field (e.g., `vault kv get -field=password secret/myserver`)
4. Click **Test** to verify the script returns a password
5. Save

**Behavior:**
- The command is parsed via `shell-words` (supports quoting and escaping)
- Executed without a shell (direct process spawn) for security
- 30-second timeout — if the script doesn't complete, the connection fails with an error
- stdout is trimmed and stored as `SecretString` (zeroed on drop)
- Non-zero exit code → error with stderr message

**Examples:**
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

### Quick Connect (Ctrl+Shift+Q)

Temporary connection without saving:
- Supports SSH, RDP, VNC, Telnet
- Optional template selection for pre-filling
- Password field for RDP/VNC
- **Runtime history** — last 15 Quick Connect sessions are remembered during the app lifetime (not persisted to disk); shown as a "Recent" section with type-ahead filtering by host/username; clicking an entry fills protocol, host, port, and username fields instantly

### Connection Actions

| Action | Method |
|--------|--------|
| Connect | Double-click, Enter, or right-click → Connect |
| Edit | Ctrl+E or right-click → Edit |
| Rename | F2 or right-click → Rename |
| Duplicate | Ctrl+D or right-click → Duplicate |
| Copy/Paste | Ctrl+C / Ctrl+V (sidebar focus) or Menu → Copy/Paste Connection |
| Delete | Delete key or right-click → Delete (moves to Trash) |
| Move to Group | Drag-drop or right-click → Move to Group |
| Run Snippet | Right-click → Run Snippet... (sends snippet to active session) |
| Start/Stop Recording | Right-click connected session → Start/Stop Recording |

**Opening the context menu:** Besides right-click, the context menu for the selected row can be opened with the **Menu** key or **Shift+F10** (standard GNOME keyboard access, anchored to the selected row), or via a **touch long-press** on touchscreens. The menu supports Up/Down (with wrap-around) plus Home/End navigation and is announced to screen readers.

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

### Copy Username / Copy Password

Right-click a connection in the sidebar → **Copy Username** or **Copy Password**.

- **Copy Username** copies the username from cached credentials (resolved during a previous connection) or falls back to the username stored on the connection model
- **Copy Password** copies the password from cached credentials; you must connect at least once so credentials are resolved and cached
- Password is auto-cleared from clipboard after 30 seconds (only if the clipboard still contains the copied password)
- Toast notifications confirm the action or explain why it failed

### Check if Online

Right-click a connection → **Check if Online** to probe whether the host is reachable.

- Starts an async TCP port probe (polls every 5s for up to 2 minutes)
- If the host comes online within the timeout, RustConn auto-connects
- Toast notifications show progress and result

### Connect All in Folder

Right-click a group in the sidebar → **Connect All** to open all connections in that group (including nested subgroups) simultaneously.

### Auto-reconnect on Session Failure

When an SSH session disconnects unexpectedly (server reboot, network failure), RustConn automatically starts polling the host (every 5s for up to 5 minutes) and reconnects when the server comes back online. The reconnect banner is still shown for manual reconnect if auto-reconnect times out.

---

## Protocols

Protocol-specific options are configured in the connection dialog's protocol tab. This section covers each protocol's unique features and settings.

**Protocol Options Summary:**

| Protocol | Options |
|----------|---------|
| SSH | Auth method (password, publickey, keyboard-interactive, agent, security-key/FIDO2), key source (default/file/agent), proxy jump (Jump Host), ProxyJump, IdentitiesOnly, ControlMaster, agent forwarding, Waypipe (Wayland forwarding), X11 forwarding, compression, startup command, verbose mode, custom SSH options, port forwarding (local/remote/dynamic) |
| RDP | Client mode (embedded/external), performance mode (quality/balanced/speed), resolution, color depth, display scale override, audio redirection, RDP gateway (host, port, username), keyboard layout, disable NLA, clipboard sharing, shared folders, mouse jiggler (prevent idle disconnect, configurable interval 10–600s), autotype (send text as keystrokes, configurable inter-character and initial delay), custom FreeRDP arguments |
| VNC | Client mode (embedded/external), performance mode (quality/balanced/speed), encoding (Auto/Tight/ZRLE/Hextile/Raw/CopyRect), compression level, quality level, display scale override, view-only mode, scaling, clipboard sharing, custom arguments |
| SPICE | TLS encryption, CA certificate (with inline validation), skip certificate verification, USB redirection, clipboard sharing, image compression (Auto/Off/GLZ/LZ/QUIC), proxy URL, shared folders |
| MOSH | Predict mode (Adaptive/Always/Never), SSH port, UDP port range, server binary path, custom arguments |
| Telnet | Custom arguments, backspace key behavior, delete key behavior |
| Serial | Device path, baud rate, data bits, stop bits, parity, flow control, custom picocom arguments |
| Kubernetes | Kubeconfig path, context, namespace, pod, container, shell, busybox mode, busybox image, custom kubectl arguments |
| ZeroTrust | Provider-specific (AWS SSM, GCP IAP, Azure Bastion, Azure SSH, OCI Bastion, Cloudflare Access, Teleport, Tailscale SSH, HashiCorp Boundary, Hoop.dev, Generic Command), custom CLI arguments |
| Web | URL (opens in system browser), credentials for copy-to-clipboard |

### SSH

#### Port Forwarding

Forward TCP ports through SSH tunnels. Three modes are supported:

| Mode | SSH Flag | Description |
|------|----------|-------------|
| Local (`-L`) | `-L local_port:remote_host:remote_port` | Forward a local port to a remote destination through the tunnel |
| Remote (`-R`) | `-R remote_port:local_host:local_port` | Forward a remote port back to a local destination |
| Dynamic (`-D`) | `-D local_port` | SOCKS proxy on a local port |

**Configure Port Forwarding:**
1. Edit an SSH connection → **Protocol** tab
2. Scroll to **Port Forwarding** section
3. Select direction (Local, Remote, Dynamic)
4. Enter local port, remote host, and remote port (remote host/port hidden for Dynamic)
5. Click **Add Forward**
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

#### Session Options

The SSH tab in the connection dialog contains session-level toggles that control how the SSH connection behaves. These are in the **Session** options group.

| Option | SSH Flag | Description |
|--------|----------|-------------|
| Agent Forwarding | `-A` | Forward your local SSH agent to the remote host, allowing key-based authentication to further servers without copying keys |
| X11 Forwarding | `-X` | Forward X11 display to your local machine — run graphical X11 apps on the remote host and see them locally |
| Compression | `-C` | Compress the SSH data stream — useful on slow or high-latency connections |
| Connection Multiplexing | `ControlMaster=auto` | Reuse a single TCP connection for multiple SSH sessions to the same host. Subsequent connections open instantly without re-authenticating. RustConn adds `ControlPersist=10m` so the master connection stays alive for 10 minutes after the last session closes. When RustConn exits, all ControlMaster sockets are automatically closed to prevent stale sockets from lingering in the filesystem |
| Waypipe | `waypipe ssh ...` | Forward Wayland GUI applications (see [Waypipe](#waypipe-wayland-forwarding) below) |
| Verbose | `-v` | Show detailed SSH debug output in the terminal for diagnosing connection issues (auth failures, key negotiation, resets) |

**Configure:**
1. Edit an SSH connection → **Protocol** tab
2. Scroll to the **Session** group
3. Toggle the desired options
4. Click **Save**

All toggles are off by default. They can be combined freely — for example, enabling both Agent Forwarding and Compression at the same time adds `-A -C` to the SSH command.

#### Custom Options

Pass arbitrary `-o` options to the SSH command. This is for advanced SSH configuration that doesn't have a dedicated UI toggle.

**Configure:**
1. Edit an SSH connection → **Protocol** tab → **Session** group
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

#### Startup Command

Run a command automatically after the SSH connection is established.

**Configure:**
1. Edit an SSH connection → **SSH** tab → **Session** group
2. Enter the command in the **Startup Command** field

The command is appended to the SSH invocation and executes in the remote shell immediately after login.

**Examples:**
- `htop` — open system monitor on connect
- `cd /var/log && tail -f syslog` — jump to logs
- `tmux attach || tmux new` — attach to or create a tmux session

#### Verbose Mode (Connection Debugging)

Enable SSH debug output to diagnose connection issues such as resets by remote devices, authentication failures, or key negotiation problems.

**Configure:**
1. Edit an SSH connection → **Protocol** tab → **Session** group
2. Enable the **Verbose** checkbox
3. Save and connect

When enabled, RustConn adds the `-v` flag to the SSH command. Detailed debug output appears directly in the terminal, showing each handshake phase, key exchange, authentication method tried, and any errors.

**When to use:**
- Connection is reset by the remote device without explanation
- Authentication fails but the reason is unclear
- Need to verify which SSH key or auth method is being used
- Diagnosing proxy jump or port forwarding issues

**Tip:** Disable verbose mode after debugging — the output is noisy and clutters normal terminal sessions.

#### Waypipe (Wayland Forwarding)

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

### RDP

#### Mouse Jiggler

Prevents idle disconnect by sending periodic mouse movements to the remote RDP session.

- Configure in Connection Dialog → RDP → Features: enable **Mouse Jiggler** and set the interval (10–600 seconds, default 60)
- Auto-starts when the RDP session connects, auto-stops on disconnect
- Works with both IronRDP embedded and FreeRDP external modes

#### File Transfer

RustConn provides two methods for transferring files to and from RDP sessions:

**Shared Folders (Drive Redirection):**

Map local directories into the remote session. Files appear as network drives (`\\tsclient\<share_name>`) inside the remote desktop.

1. Open Connection Dialog → RDP → Shared Folders
2. Add a local directory and give it a share name
3. Connect — the folder is accessible in Windows Explorer under "This PC → Network Locations"

Works with both IronRDP embedded and FreeRDP external modes.

**Clipboard File Transfer (IronRDP embedded mode only):**

When the remote Windows user copies files to the clipboard (Ctrl+C in Explorer), RustConn detects the file list and shows a **"Save N Files"** button in the RDP toolbar.

1. On the remote desktop, select files and press Ctrl+C
2. The "Save N Files" button appears in the RustConn toolbar
3. Click it and choose a local folder — files are downloaded from the remote clipboard

This uses the RDP clipboard channel (`CF_HDROP` / `FILEDESCRIPTORW` format) and works without shared folders. Progress is tracked per-file. Only available in embedded mode (IronRDP), not with FreeRDP external.

#### HiDPI Support

On HiDPI/4K displays, the embedded IronRDP client automatically sends the correct scale factor to the Windows server (e.g. 200% on a 2× display), so remote UI elements render at the correct logical size. The Scale Override setting in the connection dialog allows manual adjustment if needed.

#### Dynamic Resolution on Resize

When you resize the RustConn window, the embedded RDP session automatically adjusts its resolution to match the new window size. This works in two ways depending on server capabilities:

1. **Display Control Channel (MS-RDPEDISP)** — modern Windows 10/11 desktops and properly configured Windows Server with Remote Desktop Session Host support seamless in-place resolution changes without disconnecting. The session continues uninterrupted.

2. **Automatic reconnect fallback** — if the server does not support the Display Control Channel (common on Windows Server 2008/2012/2016 without the RDSH role, or older RDP configurations), RustConn automatically performs a brief reconnect with the new resolution. This avoids distorted scaling where the remote desktop is stretched or squished to fit the new window size.

**"Reconnect on Resize" option** (Connection Dialog → Protocol → Features):

| Setting | Behavior |
|---------|----------|
| Unchecked (default) | Tries dynamic resize first; if server doesn't support it, falls back to reconnect automatically |
| Checked | Always reconnects immediately on resize without attempting dynamic resize; useful when you know the server doesn't support Display Control and want to skip the 500ms detection delay |

**Tip:** For Windows Server connections where you notice a brief reconnect on every resize, enable "Reconnect on Resize" to make the transition slightly faster by skipping the Display Control probe.

#### Clipboard

The embedded IronRDP client provides bidirectional clipboard sync via the CLIPRDR channel. Text copied on the remote desktop is automatically available locally (Ctrl+V), and local clipboard changes are announced to the server. The Copy/Paste toolbar buttons remain available as manual fallback. Clipboard sync requires the "Clipboard" option enabled in the RDP connection settings.

#### Autotype (Type as Keystrokes)

When server-side paste is blocked (GPO, Citrix policy, UAC dialogs, password fields that reject Ctrl+V), the Autotype feature sends text character-by-character as individual keystrokes using the RDP Unicode Keyboard Event PDU. This bypasses all clipboard restrictions and is keyboard-layout independent.

**Two input modes:**

- **Type Clipboard** — reads the local clipboard and types its contents into the remote session
- **Type Text…** — opens a dialog where you enter text (with optional password mode that hides input) and sends it as keystrokes; the text never touches the system clipboard, making it ideal for passwords

**Per-connection timing settings** (Connection Dialog → RDP → Features):

| Setting | Range | Default | Purpose |
|---------|-------|---------|---------|
| Autotype Delay | 5–200 ms | 20 ms | Pause between each character. Increase for Citrix gateways or slow connections that drop characters |
| Autotype Initial Delay | 0–5000 ms | 0 ms | Pause before typing starts. Gives time to focus the target input field |

**Technical details:**
- Uses `TS_UNICODE_KEYBOARD_EVENT` PDU — layout-independent (DE/US/FR mismatches don't matter)
- Iterates by Unicode grapheme clusters (composed characters like é, ñ are sent as single units)
- Only available in embedded IronRDP mode (external FreeRDP runs in a separate process)

#### Quick Actions

The embedded RDP toolbar includes a Quick Actions dropdown menu for launching common Windows administration tools on the remote desktop. Actions send scancode key sequences directly through the RDP session with a 30ms inter-key delay for reliability.

The menu is split into two sections separated by a divider:

**Quick Shortcuts:**

| Action | Shortcut Sent | Description |
|--------|---------------|-------------|
| Settings | Win+I | Opens Windows Settings |
| Task Manager | Ctrl+Shift+Esc | Opens Windows Task Manager |

**Admin Consoles (alphabetical):**

| Action | Shortcut Sent | Description |
|--------|---------------|-------------|
| Computer Management | Win+R → `compmgmt.msc` | Opens Computer Management (disks, services, users, event log) |
| Device Manager | Win+R → `devmgmt.msc` | Opens Device Manager |
| Disk Management | Win+R → `diskmgmt.msc` | Opens Disk Management console |
| Event Viewer | Win+R → `eventvwr.msc` | Opens Event Viewer |
| Registry Editor | Win+R → `regedit` | Opens Registry Editor |
| Resource Monitor | Win+R → `resmon` | Opens Resource Monitor (CPU, memory, disk, network) |
| Server Manager | Win+R → `servermanager` | Opens Windows Server Manager |
| Services | Win+R → `services.msc` | Opens Services console |

The Quick Actions menu is accessible via the dropdown button (arrow icon) on the RDP toolbar. All labels are translatable.

#### RDP Scripts

The Scripts dropdown (terminal icon) in the RDP toolbar provides two sections:

**Shell Launchers:**

Open a shell on the remote Windows machine via Win+R. The user sees when the shell is ready (prompt appears) before running scripts.

| Launcher | Action |
|----------|--------|
| PowerShell | Win+R → `powershell` → Enter |
| PowerShell (Admin) | Win+R → elevated PowerShell via UAC |
| CMD | Win+R → `cmd` → Enter |
| CMD (Admin) | Win+R → elevated CMD via UAC |

**Scripts (User Snippets):**

Snippets with target "Windows" or "Any" (configured in the Snippet dialog → Target field) appear in the Scripts section. When clicked, the snippet command is sent via autotype (Unicode keyboard events) into the already-open shell, followed by Enter.

**How It Works:**
1. Click a Shell Launcher to open a shell on the remote machine
2. Wait for the shell prompt to appear (user controls timing)
3. Click a script from the Scripts section — it types the command and presses Enter

This approach eliminates timing issues: no clipboard delays, no shell startup guessing.

#### Snippet Target Platform

Snippets can be marked with a target execution platform:

| Target | Where visible | Use case |
|--------|---------------|----------|
| Terminal (SSH/Local) | Terminal context menu | Linux/Unix commands |
| Windows (RDP) | RDP Scripts dropdown | PowerShell/CMD commands |
| Any | Both contexts | Universal commands (e.g., `ping`) |

Configure the target in the Snippet dialog → **Target** field.

#### RemoteApp (RAIL)

Launch individual remote applications instead of a full desktop session. The remote application window appears on your local desktop as if it were a native window — no full desktop is rendered.

**Configure RemoteApp:**
1. Open Connection Dialog → RDP → **RemoteApp** section
2. Enter the **Program** path — either an alias (`||notepad`) or a full path (`C:\Program Files\app.exe`)
3. Optionally set **Arguments** (command-line arguments passed to the application)
4. Optionally set **Display Name** (shown in taskbar and window title)

**How It Works:**
- RemoteApp uses the RAIL (Remote Applications Integrated Locally) protocol extension
- RustConn automatically uses FreeRDP for RemoteApp sessions — IronRDP does not support RAIL
- FreeRDP must be installed on the system (bundled in Flatpak builds)
- The Arguments and Display Name fields appear only after entering a Program path

**Program Path Format:**

| Format | Example | Description |
|--------|---------|-------------|
| Alias | `\|\|notepad` | Published RemoteApp alias (configured on the RD server) |
| Full path | `C:\Program Files\app.exe` | Direct path to the executable on the remote server |

**Import from .rdp files:**

RemoteApp settings are automatically imported from `.rdp` files containing `remoteapplicationprogram`, `remoteapplicationcmdline`, and `remoteapplicationname` fields.

**Limitations:**
- Requires FreeRDP (not available with IronRDP embedded mode)
- The RDP server must have RemoteApp programs published or allow arbitrary program execution
- Not all RDP servers support RAIL (Windows Server with Remote Desktop Session Host role required)

#### Hide Local Cursor

Embedded RDP, VNC, and SPICE viewers support hiding the local OS cursor to eliminate the "double cursor" effect (local + remote cursor visible simultaneously). Toggle "Show Local Cursor" in the connection dialog's Features section. Enabled by default for backward compatibility.

### VNC

VNC connections support embedded (vnc-rs) or external (TigerVNC) client modes. Configure encoding (Auto/Tight/ZRLE/Hextile/Raw/CopyRect), compression level, quality level, display scale override, view-only mode, scaling, and clipboard sharing in the VNC protocol tab.

### SPICE

SPICE connections support TLS encryption, CA certificate validation, USB redirection, clipboard sharing, image compression (Auto/Off/GLZ/LZ/QUIC), proxy URL, and shared folders. Available as embedded (spice-client) or external (remote-viewer).

### MOSH Protocol

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

**Predict Modes:**
- **Adaptive** (default) — enables local echo prediction when latency is detected
- **Always** — always show predicted text (useful on very slow links)
- **Never** — disable prediction entirely

### Telnet

Telnet connections run in an embedded VTE terminal tab using the external `telnet` client. Configure custom arguments, backspace key behavior, and delete key behavior in the Telnet protocol tab.

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

#### SFTP as Connection Type

SFTP can also be created as a standalone connection type. This is useful when you primarily need file transfer access to a server (e.g., transferring files between Windows and Linux systems).

**Create an SFTP Connection:**
1. Press **Ctrl+N** → select **SFTP** protocol
2. Configure SSH settings (host, port, username, key) — SFTP reuses the SSH options tab
3. Click **Create**
4. Double-click to connect — opens file manager (or mc) directly instead of a terminal

SFTP connections use the `folder-remote-symbolic` icon in the sidebar and behave identically to the "Open SFTP" action on SSH connections, but the file manager opens automatically on Connect.

#### SFTP Troubleshooting

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

### Zero Trust Providers

RustConn supports connecting through identity-aware proxy services (Zero Trust). For detailed provider setup and configuration, see the dedicated [Zero Trust Providers](ZERO_TRUST.md) guide.

Supported providers: AWS Session Manager, GCP IAP Tunnel, Azure Bastion, Azure SSH (AAD), OCI Bastion, Cloudflare Access, Teleport, Tailscale SSH, HashiCorp Boundary, Hoop.dev, Generic Command.

### Web Bookmarks

Web bookmark connections store website URLs and open them in the system's default browser. No embedded browser is used — RustConn delegates to the OS via GTK4 `UriLauncher` (portal-aware, works in Flatpak).

**Use cases:**
- Quick access to web-based admin panels (AWS Console, Grafana, Proxmox, etc.)
- Storing credentials for web services alongside SSH/RDP connections
- Organizing all infrastructure access points in one place

**Creating a Web bookmark:**
1. Click "New Connection" → select **Web** protocol
2. Enter the full URL in the **URL** field (must start with `http://` or `https://`)
3. Optionally set username/password — these are stored in the configured secret backend for copy-to-clipboard via the sidebar context menu
4. Click Save

**Connecting:**
- Double-click the connection in the sidebar → the URL opens in your default browser
- The sidebar status briefly shows "connecting" (yellow) then clears — Web bookmarks have no persistent session

**Context menu actions:**
- **Copy Username** / **Copy Password** — copies stored credentials to clipboard (auto-clears after 30 seconds)

**CLI:**
```bash
rustconn-cli add --name "AWS Console" --protocol web --host "https://console.aws.amazon.com"
```

**Limitations:**
- No embedded browser — always opens in the system default
- No auto-fill — credentials are for copy-to-clipboard only
- Port field is hidden (not applicable for URLs)

---

## Sessions & Terminal

### Session Types

| Protocol | Session Type |
|----------|--------------|
| SSH | Embedded VTE terminal tab |
| RDP | Embedded IronRDP or external FreeRDP (bundled in Flatpak) |
| VNC | Embedded vnc-rs or external TigerVNC |
| SPICE | Embedded spice-client or external remote-viewer |
| MOSH | MOSH via VTE terminal (external `mosh` client) |
| Telnet | Embedded VTE terminal tab (external `telnet` client) |
| Serial | Embedded VTE terminal tab (external `picocom` client) |
| Kubernetes | Embedded VTE terminal tab (external `kubectl exec`) |
| ZeroTrust | Provider CLI in terminal |
| Local Shell | Local VTE terminal tab (user's default shell) |

**Local Shell:** Open a local terminal tab without connecting to any remote host. Useful as a quick terminal emulator or for running local commands alongside remote sessions. Start via Menu → File → Local Shell, the startup action (Settings → Interface → Startup → Local Shell), or `rustconn --shell`.

**Custom Shell Command:** Configure a custom command for Local Shell in Settings → Terminal → Local Shell → Command. When set, Local Shell runs this command instead of the default login shell. Examples:
- `fish` — use Fish shell instead of bash
- `bash --norc` — bash without loading .bashrc
- `neofetch && bash` — show system info on startup, then drop into bash
- `tmux new-session` — start a tmux session
- `/usr/bin/zsh -l` — explicit path to zsh as login shell

Leave the field empty to use the system default shell (`$SHELL`).

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
- **Close** — Click X or Ctrl+W / Ctrl+Shift+W
- **Reorder** — Drag tabs
- **Tab Overview** — Click the grid icon (▦) at the right end of the tab bar, or press **Ctrl+Shift+O**, to open a full-screen grid view of all open tabs. Useful when you have many tabs open and need to visually locate a session. Click any thumbnail to switch to it.
- **Tab Switcher** — Press **Ctrl+%** (or open Command Palette with **Ctrl+P** and type `%`) to fuzzy-search across all open tabs by name. Results show protocol type and tab group. Select and press Enter to switch instantly.
- **Pin Tab** — Right-click a tab → **Pin Tab**. Pinned tabs stay at the left edge of the tab bar and are never scrolled out of view. Useful for long-running sessions you need constant access to. Right-click again → **Unpin Tab** to restore normal behavior.

### Split View

Split view works with terminal-based sessions: SSH, Telnet, Serial, Kubernetes, Local Shell, and SFTP (mc mode).

- **Horizontal Split** — Ctrl+Shift+H splits the current tab horizontally (side by side)
- **Vertical Split** — Ctrl+Shift+S splits the current tab vertically (top and bottom)
- **Close Pane** — Ctrl+Shift+X closes the focused pane; if only one pane remains, the split is dissolved and the session returns to normal tab mode
- **Focus Next Pane** — Ctrl+` cycles focus between panes
- **Select Tab** — click the "Select Tab..." button in an empty pane to pick which session to display; sessions already in other split views show a colored indicator
- **Move between splits** — a session can be moved from one split to another via "Select Tab"; the original split keeps a placeholder in the vacated panel, and the session's own tab shows a "Displayed in Split View" page with a "Go to Split View" button
- **Tab Overview** — split-view tabs render correctly in Tab Overview (Ctrl+Shift+O) with live thumbnails showing the split layout

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
- Close the tab normally with Ctrl+W to dismiss

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

### Session Recording

Record terminal sessions in scriptreplay-compatible format for later playback. Recordings capture terminal output with timing information and automatically sanitize sensitive data (passwords, API keys, tokens).

**Enable Recording (per-connection):**
1. Edit connection → **Advanced** tab
2. Enable **Session Recording**
3. Save

When recording is active, the tab title shows a **●REC** indicator. A red `media-record-symbolic` dot also appears next to the connection in the sidebar (with tooltip and screen-reader label) while any of its sessions is being recorded, so an active recording is visible at a glance.

**Recording Files:**
Recordings are saved to `$XDG_DATA_HOME/rustconn/recordings/` (typically `~/.local/share/rustconn/recordings/`) with two files per session:

| File | Contents |
|------|----------|
| `{name}_{timestamp}.data` | Raw terminal output bytes |
| `{name}_{timestamp}.timing` | Timing data (delay + byte count per chunk) |

**Playback:**
```bash
scriptreplay --timing=session.timing session.data
```

**Sanitization:** Recordings automatically redact password prompts and responses, API keys and tokens, AWS credentials, and private key content.

### Terminal Activity Monitor

Per-session activity and silence detection for terminal tabs, inspired by KDE Konsole. Each SSH terminal session can independently track output events and notify you when activity resumes after a quiet period or when a terminal goes silent.

**Monitoring Modes:**

| Mode | Behavior | Default Timeout |
|------|----------|-----------------|
| **Off** | No monitoring (default) | — |
| **Activity** | Notify when new output appears after a configurable quiet period | 10 seconds |
| **Silence** | Notify when no output occurs for a configurable duration | 30 seconds |

**Activity mode** is useful when you've started a long-running command in a background tab and want to know when it produces output again.

**Silence mode** is useful when you're watching a stream of output (logs, compilation) and want to know when it stops — indicating the process has finished or stalled.

**Notification Channels:**
1. **Tab indicator icon** — an icon appears on the tab (ℹ for activity, ⚠ for silence)
2. **In-app toast** — a toast message like "Activity detected: Web-01" or "Silence detected: Build-Server"
3. **Desktop notification** — a system notification when the RustConn window is not focused

The tab indicator and notification are cleared automatically when you switch to that tab.

**Configure Global Defaults:**
1. Open **Settings** (Ctrl+,) → **Monitoring** tab
2. Set **Default Mode** (Off / Activity / Silence)
3. Set **Default Quiet Period** (1–300 seconds, default: 10)
4. Set **Default Silence Timeout** (1–600 seconds, default: 30)

**Per-Connection Override:**
Edit connection → **Advanced** tab → **Activity Monitor** section.

**Quick Mode Toggle:** Right-click any terminal tab → **Monitor: Off/Activity/Silence** to cycle through modes.

### Text Highlighting Rules

Define regex-based patterns to highlight matching text in terminal output with custom colors. Rules can be global (apply to all connections) or per-connection.

**Built-in Defaults:**

| Rule | Pattern | Colors |
|------|---------|--------|
| ERROR | `ERROR` | Red foreground |
| WARNING | `WARNING` | Yellow foreground |
| CRITICAL/FATAL | `CRITICAL\|FATAL` | Red background |

**Configure Global Rules:**
1. **Settings → Terminal** → **Highlighting Rules** section
2. Click **Add Rule**
3. Enter rule name, regex pattern, and choose foreground/background colors
4. Toggle **Enabled** to activate/deactivate individual rules

**Configure Per-connection Rules:**
1. Edit connection → **Advanced** tab → **Highlighting Rules** section
2. Add rules that apply only to this connection
3. Per-connection rules take priority over global rules

**Rule Properties:**

| Property | Description |
|----------|-------------|
| Name | Display name for the rule |
| Pattern | Regular expression (Rust regex syntax) |
| Foreground Color | Text color in `#RRGGBB` format (optional) |
| Background Color | Background color in `#RRGGBB` format (optional) |
| Enabled | Toggle rule on/off |

Invalid regex patterns are rejected with an error message during validation.

**Note:** Lines containing only whitespace are not processed by the highlight overlay. This prevents stale highlights from appearing after the `clear` command erases the terminal screen. Highlight rules that intentionally match whitespace-only patterns will not render.

### Per-connection Terminal Theming

Override terminal colors (background, foreground, cursor) on a per-connection basis. Useful for visually distinguishing production vs. development environments.

**Configure:**
1. Edit connection → **Advanced** tab → **Terminal Theme** section
2. Click the color buttons to set Background, Foreground (text), and Cursor colors
3. Colors are in `#RRGGBB` or `#RRGGBBAA` format
4. Click **Reset** to clear overrides and use the global theme
5. Save

**Tips:**
- Use a red-tinted background for production servers
- Use a green-tinted background for development/staging
- Combine with tab coloring for maximum visual distinction

---

## Organization

### Groups

#### Create Group

- **Ctrl+Shift+G** or click folder icon
- Right-click in sidebar → **New Group**
- Right-click on group → **New Subgroup**

#### Group Operations

- **Rename** — F2 or right-click → Rename
- **Move** — Drag-drop or right-click → Move to Group
- **Delete** — Delete key (shows choice dialog: Keep Connections, Delete All, or Cancel)

#### Group Operations Mode (Bulk Actions)

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

#### Group Credentials

Groups can store default credentials (Username, Password, Domain) that are inherited by their children.

**Configure Group Credentials:**
1. Right-click a group → **Edit Group** → **Identity** tab
2. Expand the **Default Credentials** section (toggle the switch to enable)
2. Select **Password Source**:
   - **Prompt** — Ask for password on each connection
   - **Vault** — Store in the configured secret backend (KeePass, Keyring, Bitwarden, 1Password, Passbolt); click the **folder icon** to load an existing password from the vault
   - **Variable** — Use a named secret global variable (dropdown shows only variables marked as secret in Tools → Variables)
   - **Inherit** — Inherit from parent group
   - **None** — No password
3. Password source determines which UI is shown: Vault shows a password field with load button, Variable shows a dropdown of secret variables

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

#### Group Automation (Expect Rules & Post-login Scripts)

Groups can define Expect Rules and Post-login Scripts that are automatically inherited by all connections in the group (and subgroups). This lets you configure automation once for hundreds of connections.

**Configure Group Automation (GUI):**
1. Right-click a group → **Edit Group** → **Automation** tab
2. Toggle the **Automation** switch to enable
3. **Expect Rules** — add rules that auto-respond to terminal patterns:
   - Click **Add Rule** to create a blank rule
   - Or click **From Template** to pick a preset — SSH-specific templates are marked with "(SSH)":
     - Sudo Password (SSH) — auto-respond to `[sudo] password for ...:`
     - SSH Host Key Confirmation (SSH) — auto-accept host key fingerprint
     - Login Prompt — auto-fill username and password at login prompts
     - Press Enter to Continue — auto-dismiss "Press Enter" prompts
     - MOTD Pager — auto-dismiss `--More--` pager prompts
   - Each rule has: Pattern (regex), Response, Priority, Timeout, Enabled/One-shot toggles
   - Each rule has ↑ ↓ 🗑️ buttons at the top-right to reorder or delete individual rules
   - Use the **➕** button next to Response to insert variable placeholders (`${password}`, `${username}`, `${host}`, `${port}`, `\n`) without typing them manually
   - Invalid regex patterns are highlighted in red with an error message
   - Click **Clear All** to remove all rules at once
4. **Pattern Tester** (collapsible) — type text to test against your rules in real time; shows which rule matches and what response would be sent
5. **Post-login Scripts** — add individual commands to run after login:
   - Click **Add Script** to add a new command entry
   - Each script has its own row with a delete button
   - Example commands: `cd /app`, `source .env`, `export TERM=xterm-256color`
6. Click **Save**

> **Note:** Disabling the Automation switch shows a confirmation dialog — all rules and scripts will be cleared.

> **Tip:** Responses support `${password}`, `${username}`, `${host}`, `${port}`, and `${VARIABLE_NAME}` placeholders that are resolved at connection time. `${password}` is automatically filled from the connection's configured secret backend (Vault, Variable, etc.). Use the ➕ button next to the Response field to insert these without typing.

**Configure Group Automation (CLI):**

```bash
# Add a sudo password expect rule
rustconn-cli group edit "Production" \
  --add-expect-rule '{"pattern":"\\[sudo\\] password for \\w+:","response":"${password}\\n","priority":10,"timeout_ms":30000,"one_shot":true}'

# Add multiple rules at once
rustconn-cli group edit "Production" \
  --add-expect-rule '{"pattern":"password:\\s*$","response":"${password}\\n"}' \
  --add-expect-rule '{"pattern":"yes/no","response":"yes\\n"}'

# Clear all rules and start fresh
rustconn-cli group edit "Production" --clear-expect-rules \
  --add-expect-rule '{"pattern":"login:","response":"${username}\\n"}'

# Add post-login scripts
rustconn-cli group edit "Production" \
  --add-post-login-script "cd /app" \
  --add-post-login-script "source .env"

# Clear and replace post-login scripts
rustconn-cli group edit "Production" --clear-post-login-scripts \
  --add-post-login-script "export TERM=xterm-256color"

# View group automation config
rustconn-cli group show "Production"
```

**Inheritance Rules:**
- Connections with **empty** automation config automatically inherit from their parent group
- If a connection has its own Expect Rules, group rules are **not** merged — the connection's rules take precedence
- Expect rules and post-login scripts are inherited independently — rules may come from one group and scripts from another
- Inheritance walks up the group hierarchy: child group → parent group → grandparent group
- To override group automation for a specific connection, add at least one rule in the connection's Automation tab

**Example — Sudo password for all servers in a group:**
1. Edit the "Production Servers" group
2. Enable Automation → click **From Template** → select **Sudo Password (SSH)**
3. The template adds a rule: pattern `\[sudo\] password for \w+:` → response `${password}\n`
4. Save the group
5. All SSH connections in "Production Servers" now auto-respond to sudo prompts using their vault password

#### Sorting

- Alphabetical by default (case-insensitive, by full path)
- Drag-drop for manual reordering
- Click Sort button in toolbar to reset

### Favorites

Pin frequently used connections to a dedicated "Favorites" section at the top of the sidebar.

**Pin a Connection:**
- Right-click a connection → **Pin / Unpin**
- The connection appears in the ★ Favorites group at the top of the sidebar

**Unpin a Connection:**
- Right-click a pinned connection → **Pin / Unpin**
- The connection returns to its original group

Favorites persist across sessions. Pinned connections remain in their original group as well — the Favorites section shows a reference, not a move.

### Smart Folders

Smart Folders are dynamic, filter-based views that automatically group connections matching specific criteria. Unlike regular groups, Smart Folders don't move connections — they show a live, read-only list of matching connections.

**Create a Smart Folder:**

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

**Custom Icons:**

Smart Folders support custom emoji icons displayed in the sidebar instead of the default 📁. Set the icon in the "Icon" field when creating or editing a Smart Folder, or via CLI:

```bash
rustconn-cli smart-folder create --name "Production" --icon "🏢" --protocol ssh
rustconn-cli smart-folder edit "Production" --icon "🚀"   # Change icon
rustconn-cli smart-folder edit "Production" --icon "none"  # Reset to default 📁
```

**Behavior:**
- Smart Folders appear in a dedicated sidebar section with a 🔍 icon
- Connections in Smart Folders are read-only (no drag-drop)
- Double-click a connection to connect (same as regular connections)
- Right-click a Smart Folder → **Edit** or **Delete**
- Empty filter criteria → empty result (not "match all")

### Dynamic Folders

Dynamic Folders generate connections automatically by executing a user-defined script. Unlike Smart Folders (which filter existing connections), Dynamic Folders create new connections from external data sources — cloud APIs, infrastructure tools, or custom scripts.

**Use Cases:**
- Import EC2 instances from AWS: `aws ec2 describe-instances --query ...`
- List Kubernetes nodes: `kubectl get nodes -o json | jq ...`
- Query Ansible inventory: `ansible-inventory --list | jq ...`
- Scan Proxmox VMs: custom API script
- Read from a CMDB or asset database

**Configure a Dynamic Folder:**
1. Create or edit a group
2. In the group settings, expand **Dynamic Folder** section
3. Enter the script command (executed via `sh -c`)
4. Optionally set:
   - **Working Directory** — where the script runs
   - **Timeout** — maximum execution time (default: 30 seconds)
   - **Refresh Interval** — auto-refresh period (leave empty for manual only)
5. Save

**Script Output Format:**

The script must output a JSON array to stdout:

```json
[
  {
    "name": "web-01",
    "host": "10.0.1.10",
    "port": 22,
    "protocol": "ssh",
    "username": "admin",
    "group": "web-servers",
    "tags": ["production", "nginx"],
    "description": "Primary web server"
  },
  {
    "name": "db-master",
    "host": "10.0.2.5",
    "protocol": "ssh"
  }
]
```

**Required fields:** `name`, `host`

**Optional fields:**

| Field | Default | Description |
|-------|---------|-------------|
| `port` | Protocol default (22, 3389, 5900...) | Connection port |
| `protocol` | `ssh` | One of: `ssh`, `rdp`, `vnc`, `spice`, `telnet`, `mosh` |
| `username` | None | Login username |
| `group` | None | Sub-group path within the dynamic folder |
| `tags` | `[]` | Tags for filtering |
| `description` | None | Connection description |

**Behavior:**
- Generated connections are **read-only** — they cannot be edited or moved manually
- Connections have stable IDs across refreshes (based on name + host + protocol hash)
- Invalid entries (empty name or host) are skipped with a warning
- The script's stderr is logged for debugging
- Non-zero exit code → error shown in sidebar tooltip

**Refresh:**
- **Manual:** Right-click the dynamic group → **Refresh Dynamic Folder**
- **Automatic:** If refresh interval is configured, the folder refreshes periodically

**CLI Commands:**

```bash
# List all groups with dynamic folders
rustconn-cli dynamic-folder list

# Show configuration and generated connections
rustconn-cli dynamic-folder show "AWS Servers"

# Create/update dynamic folder on a group
rustconn-cli dynamic-folder set "AWS Servers" --script 'aws ec2 describe-instances ...' --timeout 60

# Refresh (execute script and update connections)
rustconn-cli dynamic-folder refresh "AWS Servers"

# Remove dynamic folder and generated connections
rustconn-cli dynamic-folder remove "AWS Servers"

# JSON output for scripting
rustconn-cli dynamic-folder list --format json
```

**Example Scripts:**

```bash
# AWS EC2 instances (requires aws-cli)
aws ec2 describe-instances --query 'Reservations[].Instances[].{name:Tags[?Key==`Name`].Value|[0],host:PrivateIpAddress}' --output json

# Kubernetes nodes
kubectl get nodes -o json | jq '[.items[] | {name: .metadata.name, host: (.status.addresses[] | select(.type=="InternalIP") | .address), protocol: "ssh"}]'

# Simple static list from a file
cat ~/infrastructure/servers.json

# Proxmox VMs via API
curl -s -k "https://proxmox:8006/api2/json/nodes/pve/qemu" -H "Authorization: PVEAPIToken=..." | jq '[.data[] | {name: .name, host: .name, protocol: "spice"}]'
```

### Custom Icons

Set custom emoji or GTK icon names on connections and groups to visually distinguish them in the sidebar.

**Supported Icon Types:**

| Type | Example | How It Renders |
|------|---------|----------------|
| Emoji / Unicode | `🇺🇦`, `🏢`, `🔒`, `🐳` | Displayed as text next to the name |
| GTK icon name | `starred-symbolic`, `network-server-symbolic` | Rendered as a symbolic icon |

**Set a Custom Icon:**
1. Edit a connection or group
2. Enter an emoji or GTK icon name in the **Icon** field
3. Save

Leave the field empty to use the default icon (folder for groups, protocol-based for connections).

### Tab Coloring

Optional colored circle indicators on terminal tabs to visually distinguish protocols at a glance.

| Protocol | Color |
|----------|-------|
| SSH | 🟢 Green |
| RDP | 🔵 Blue |
| VNC | 🟣 Purple |
| SPICE | 🟠 Orange |
| Serial | 🟡 Yellow |
| Kubernetes | 🔵 Cyan |

**Enable/Disable:** Settings → Interface page → Appearance → **Color tabs by protocol**

### Tab Grouping

Organize open tabs into named groups with a visible `[GroupName]` prefix in the tab title.

**Assign a Tab to a Group:**
1. Right-click a tab in the tab bar
2. Select **Set Group...**
3. Pick an existing group from the pill buttons, or type a new group name
4. Click **Apply**

The tab title changes to `[GroupName] ConnectionName` and the tooltip shows the group name.

**Remove from Group:** Right-click a grouped tab → **Remove from Group**

**Close All in Group:** Right-click a grouped tab → **Close All in Group** (with confirmation dialog)

**Monitor Mode Toggle:** Right-click any tab → **Monitor: Off/Activity/Silence** to cycle monitoring mode.

Groups are visual only — they are session-scoped and not persisted across restarts.

---

## Productivity Tools

### Templates

Templates are connection presets that store protocol settings, authentication defaults, tags, custom properties, and automation tasks. When you create a connection from a template, all configured fields are copied into the new connection — including the template's icon.

**Manage Templates:** Menu → Tools → **Manage Templates** (or `rustconn-cli template list`)

**Create Template:**
- **From scratch:** Open Manage Templates → Click **Create Template** → configure name, icon (emoji or GTK icon name), protocol, default settings
- **From existing connection:** Right-click a connection → **Create Template from Connection**

**Use Template:**
- **From Connection Wizard (Ctrl+N):** Choose "Custom Command" → template grid shows your templates and predefined ones; click to fill command and name
- **From Quick Connect (Ctrl+Shift+Q):** Select a template from the dropdown — fields pre-fill the form
- **From Manage Templates:** Select a template → click **Create Connection**
- **From CLI:** `rustconn-cli template apply "SSH Template" --name "New Server" --host "10.0.0.5"`

**Template Fields:** Protocol, Host/Port, Username/Domain, Password Source, Tags, Icon, Protocol Config, Custom Properties, Pre/Post Tasks, WoL Config.

**Predefined Templates:** RustConn ships with 20 built-in templates for common CLI tools that don't have dedicated protocol support:

| Category | Templates |
|----------|-----------|
| Remote Desktop | 🖥️ RustDesk, 🔴 AnyDesk, 🌐 Remmina |
| Containers | 🐳 Docker, 🦭 Podman, 📦 LXC/LXD, 🧊 Incus, 🗃️ Distrobox |
| Virtualization | 🖧 Virsh Console, 🟠 Proxmox VM, 🟡 Proxmox CT |
| Hardware | 🔌 IPMI SOL, 🔧 Picocom, 🐟 Redfish BMC |
| Cloud Access | 🛡️ WireGuard+SSH, 🚀 Teleport App, 🎛️ Cockpit |
| Automation | ⚙️ Ansible, ⏰ WoL+SSH, ❄️ Nix Remote Build |

Click "More…" in the wizard grid to browse all predefined templates grouped by category. Your own ZeroTrust templates (created via Manage Templates) appear first in the grid automatically.

### Snippets

Reusable command templates with variable substitution. Snippets let you define frequently used commands once and execute them in any active terminal session with one action.

**Syntax:** Snippets use `${variable}` placeholders that are resolved at execution time.

```bash
# Service management
sudo systemctl restart ${service}

# Database backup
pg_dump -h ${host} -U ${user} -d ${database} > /tmp/${database}_backup.sql
```

**Variable Features:** Each variable can have a Name, Description (shown as hint), and Default Value (pre-filled when executing).

**Manage Snippets:** Menu → Tools → **Manage Snippets** (or `rustconn-cli snippet list`)

**Execute Snippet:**
1. Connect to a terminal session (SSH, Telnet, Serial, Kubernetes, or local shell)
2. Menu → Tools → **Execute Snippet** (or use Command Palette → Snippets)
3. Select a snippet, fill in variable values, click **Execute**

**Global Variables Auto-Resolution:**

Snippet variables are automatically resolved from Global Variables (Menu → Tools → Variables) before execution. This means you can define common values once and reuse them across all snippets without manual input.

**Resolution order:**
1. **Global Variables** — if a `${VAR}` matches a defined global variable (including vault-backed secrets), its value is used automatically
2. **Snippet defaults** — if no global variable matches, the snippet's own default value is used
3. **Manual input** — if neither is available, the variable input dialog appears

If all variables are resolved automatically, the snippet executes immediately without showing any dialog.

**Example — zero-prompt snippet execution:**
```bash
# Snippet: sudo systemctl restart ${SERVICE_NAME}
# Global Variable: SERVICE_NAME = nginx
# Result: executes immediately → "sudo systemctl restart nginx\n"
```

**Example — partial resolution:**
```bash
# Snippet: pg_dump -h ${DB_HOST} -U ${DB_USER} -d ${database}
# Global Variables: DB_HOST = db.prod.internal, DB_USER = admin
# Result: dialog appears with DB_HOST and DB_USER pre-filled, only "database" needs input
```

**Organization:** Snippets support categories and tags for filtering.

### Clusters

Clusters group multiple connections for simultaneous management. The primary use case is broadcast mode: type a command once and it is sent to all connected cluster members at the same time.

**Create Cluster:**
1. Menu → Tools → **Manage Clusters**
2. Click **Create** → enter name → add connections → optionally enable **Broadcast by default**
3. Save

**Connect Cluster:** Open Manage Clusters → select a cluster → **Connect All**. RustConn opens a terminal tab for each member connection.

**Broadcast Mode:** When enabled, every keystroke you type in the focused terminal is sent to all connected cluster members simultaneously. Toggle the broadcast switch in the cluster toolbar.

**Use cases:**
- Rolling out configuration changes across multiple servers
- Running the same diagnostic command on all nodes
- Updating packages on a fleet of machines

### Ad-hoc Broadcast

Send keystrokes to multiple terminal sessions simultaneously without setting up a cluster.

**Usage:**
1. Click the **Broadcast** toggle button in the toolbar
2. Checkboxes appear on each terminal tab
3. Select the terminals you want to broadcast to
4. Type in any selected terminal — keystrokes are sent to all selected terminals
5. Click the Broadcast button again to deactivate

| Feature | Ad-hoc Broadcast | Cluster Broadcast |
|---------|-----------------|-------------------|
| Setup | No setup — select terminals on the fly | Requires pre-defined cluster |
| Scope | Any open terminal tabs | Connections in a cluster |
| Persistence | Session-only | Saved in configuration |

### Command Palette

Open with **Ctrl+P** (connections) or **Ctrl+Shift+P** (commands).

A VS Code-style quick launcher with fuzzy search. Type to filter, then select with arrow keys and Enter.

| Prefix | Mode | Description |
|--------|------|-------------|
| *(none)* | Connections | Fuzzy search saved connections; Enter to connect |
| `>` | Commands | Application commands (New Connection, Import, Settings, etc.) |
| `@` | Tags | Filter connections by tag |
| `#` | Groups | Filter connections by group |
| `%` | Open Tabs | Fuzzy search open tabs by name; Enter to switch |

The palette shows up to 20 results with match highlighting. Results are ranked by fuzzy match score. In `%` mode, results include protocol type and tab group name for quick identification.

### Global Variables

Global variables allow you to use placeholders in connection fields that are resolved at connection time.

**Syntax:** `${VARIABLE_NAME}`

**Supported Fields:** Host, Username, Domain (RDP)

**Define Variables:**
1. Menu → Tools → **Variables...**
2. Click **+** (Add Variable) in the header bar → a new expanded row appears with focus on the name field
3. Enter name and value
4. Optionally mark as **Secret** (value hidden, stored in vault)
5. Add a description for documentation purposes
6. Click **Save**

**Collapsible Rows:**

When you have many variables, each one is displayed as a collapsed row showing only its name. Click the expander arrow to reveal the full editing form. When you add a new variable, all existing rows collapse automatically so you can focus on the new entry.

**Duplicate Name Protection:**

Variable names must be unique (case-insensitive). If you try to save with duplicate names, the dialog highlights the conflicting entries in red and expands them — saving is blocked until you rename or delete the duplicates.

**Secret Variables:** Toggle visibility with the eye icon. Secret values are auto-saved to the configured vault backend on dialog save and cleared from the settings file.

**KeePass Custom Entry Path:**

When using KeePass/KeePassXC as the secret backend, secret variables can reference an existing entry in your KeePass database instead of the default `RustConn/rustconn/var/{name}` path:

1. Mark the variable as **Secret**
2. A **KeePass entry** field appears (only when KeePass backend is active)
3. Enter the full path to an existing entry, e.g., `Internet/MyRouter` or `Network/Switches/RADIUS`
4. The password is read from that entry's Password attribute at connection time

This avoids duplicating secrets — you can reuse entries already in your KeePass database. When a custom path is set, RustConn reads from the entry but never creates or overwrites it.

If the field is left empty, the default path `RustConn/rustconn/var/{name}` is used (created automatically on save).

**Vault Entry Name (Bitwarden, 1Password, Passbolt, Pass):**

When using Bitwarden, 1Password, Passbolt, or Pass as the secret backend, secret variables can reference an existing vault entry by its exact name:

1. Mark the variable as **Secret**
2. A **Vault entry** field appears (only when a non-KeePass backend is active)
3. Enter the exact name of an existing vault entry, e.g., `AD Credentials` or `Production DB`
4. Leave the password field empty — it will be fetched from the vault at connection time

**How it works:**
- At connection time, RustConn searches your vault for an entry matching the exact name (case-sensitive) and reads the password from it.
- Nothing is written back to the vault — the entry is treated as read-only.
- No credentials are stored locally on disk; only the reference (entry name) is persisted in settings.
- You do not need to enter the password in the variable dialog — the password field can be left blank.

This is the non-KeePass equivalent of "KeePass entry" — it allows reusing credentials already stored in your vault without duplicating them under the `rustconn/var/` namespace.

If the **Vault entry** field is left empty, RustConn uses the default key `rustconn/var/{name}` (Bitwarden item named `RustConn: rustconn/var/{name}`). In that case, you must enter the password value, which will be saved to the vault once on creation.

**Example:**
```
Variable: PROD_USER = admin
Variable: PROD_DOMAIN = corp.example.com
Variable: RADIUS (secret, KeePass entry: Network/RADIUS_Secret)

Connection Username: ${PROD_USER}  →  admin
Connection Domain: ${PROD_DOMAIN}  →  corp.example.com
Connection Password Source: Variable → RADIUS  →  reads from KeePass entry "Network/RADIUS_Secret"
```

**Tips:**
- Variable names are case-sensitive
- Undefined variables remain as literal text
- Combine with Group Credentials for hierarchical credential management

**Using Variables as Password Source (shared credentials):**

To reuse the same credentials across multiple connections (e.g., one Active Directory account for many RDP sessions):

1. Create a secret variable in **Tools → Variables** (e.g., `AD_PASSWORD`, mark as Secret)
2. In the **Vault entry** field, type the exact name of your existing Bitwarden/1Password/Passbolt/Pass entry (e.g., `AD Credentials`)
3. Leave the password field empty — RustConn will fetch it from the vault at connect time
4. In each connection dialog → **Password** dropdown → select **Variable**
5. Choose your secret variable from the dropdown that appears

All connections that reference the same variable share the credential — change it once in your vault, all connections pick up the updated value. Nothing is duplicated or written back to the vault.

The "+" button next to the dropdown opens the Variables manager directly if you have not created any secret variables yet.

> **Note:** If you do not have an existing vault entry (and want RustConn to manage the secret for you), leave the "Vault entry" field blank and enter the password directly. It will be stored under `rustconn/var/{name}` in your vault.

### Password Generator

Menu → Tools → **Password Generator**

Features: Length (4-128 characters), character sets (lowercase, uppercase, digits, special, extended), exclude ambiguous (0, O, l, 1, I), strength indicator with entropy, crack time estimation, copy to clipboard.

### Wake-on-LAN

Wake sleeping machines before connecting by sending WoL magic packets.

**Configure WoL for a connection:**
1. Edit connection → **WOL** tab
2. Enter MAC address (e.g., `AA:BB:CC:DD:EE:FF`)
3. Optionally set broadcast address and port
4. Save

**Send WoL from sidebar:** Right-click connection → **Wake On LAN**. After sending, RustConn polls the host (every 5s for up to 5 minutes) and auto-connects when online.

**Auto-WoL on connect:** If a connection has WoL configured, a magic packet is sent automatically when you connect (fire-and-forget).

**Standalone WoL dialog:** Menu → Tools → **Wake On LAN...**

All GUI sends use 3 retries at 500 ms intervals for reliability.

### Connection History

Menu → Tools → **Connection History**

- Search and filter past connections by name, host, protocol, or username
- Connect directly from history
- Delete individual entries or clear all history

### Connection Statistics

Menu → Tools → **Connection Statistics**

Tracks: total connections, success rate, connection duration (average/total), most used connections, protocol breakdown, last connected timestamps. Use **Reset** to clear all statistics.

### Encrypted Documents

Store sensitive notes, certificates, and credentials in AES-256-GCM encrypted documents within RustConn.

**Create:** Menu → File → **New Document** → enter name → optionally set protection password → write content → save with Ctrl+S.

**Protection:** Right-click document → Set/Remove Protection. Protected documents require the password each time they are opened. Unprotected documents are encrypted with the application master key.

**Use Cases:** Runbooks, API tokens, SSH key passphrases, network diagrams, compliance notes.

**Backup:** Documents are stored in `~/.config/rustconn/documents/`. They are **not** included in Settings Backup/Restore or in RustConn Native export (.rcn) — back up the `documents/` directory manually if needed.

### Remote Monitoring

MobaXterm-style monitoring bar below SSH terminals showing real-time system metrics from remote Linux hosts. Completely agentless — no software needs to be installed on the remote host. RustConn collects data by parsing `/proc/*` and `df` output over a separate SSH connection. For Telnet and Kubernetes sessions, monitoring is available if the host is also reachable via SSH.

**Monitoring Bar:**
```
[CPU: ████░░ 45%] [RAM: ██░░ 62%] [Disk: ██░░ 78%] [1.23 0.98 0.76] [↓ 1.2 MB/s ↑ 0.3 MB/s] [Ubuntu 24.04 (6.8.0) · x86_64 · 15.6 GiB · 8C/16T · 10.0.1.5]
```

**Displayed Metrics:**

| Metric | Source | Details |
|--------|--------|---------|
| CPU usage | `/proc/stat` | Percentage with level bar; delta-based calculation |
| Memory usage | `/proc/meminfo` | Percentage with level bar; swap in tooltip |
| Disk usage | `df -Pk` | Root filesystem; all mount points in tooltip |
| Load average | `/proc/loadavg` | 1, 5, 15 minute values |
| Network throughput | `/proc/net/dev` | Download/upload rates (auto-scaled) |
| System info | One-time collection | Distro, kernel, arch, RAM, CPU cores, IP |

**Enable Monitoring:**
1. Open **Settings** (Ctrl+,) → **Monitoring** page → **General** group
2. Toggle **Enable monitoring**
3. Configure polling interval (1–60 seconds, default: 3)
4. Select which metrics to display in the **Visible Metrics** group

**Per-Connection Override:** Edit connection → **Advanced** tab → **Remote Monitoring** section → toggle **Enable Monitoring** ON or OFF. This overrides the global setting for this specific connection — if global monitoring is disabled but the toggle is ON, monitoring will still run for this connection (and vice versa).

**Requirements:** Remote host must be Linux. No agent installation needed. Works with SSH, Telnet, and Kubernetes connections.

### Flatpak Components

**Available only in Flatpak environment**

Menu → **Flatpak Components...**

Download and install additional CLI tools directly within the Flatpak sandbox:

**Zero Trust CLIs:** AWS CLI, AWS SSM Plugin, Google Cloud CLI, Azure CLI, OCI CLI, Teleport, Tailscale, Cloudflare Tunnel, HashiCorp Boundary, Hoop.dev

**Password Manager CLIs:** Bitwarden CLI, 1Password CLI

**Protocol Clients:** TigerVNC Viewer

**Features:** One-click Install/Remove/Update, progress indicators with cancel support, SHA256 checksum verification, automatic PATH configuration.

**Installation Location:** `~/.var/app/io.github.totoshko88.RustConn/cli/`

### SSH Tunnel Manager

Standalone window for managing SSH port-forwarding tunnels that run independently of terminal sessions. Unlike per-connection port forwarding (which requires an active SSH terminal tab), standalone tunnels run in the background as headless `ssh -N` processes.

**Open:** Menu → **SSH Tunnels** or **Ctrl+T**

**Create a Tunnel:**
1. Open SSH Tunnel Manager (Ctrl+T)
2. Click **Add Tunnel** (+ button or the button on the empty state page)
3. Enter a name (e.g., "MySQL prod", "SOCKS proxy")
4. Select an existing SSH connection — the tunnel inherits host, port, username, SSH key, jump host, and credentials from this connection
5. Add one or more port forwarding rules:

| Direction | SSH Flag | Example | Description |
|-----------|----------|---------|-------------|
| Local (`-L`) | `-L 3306:db.internal:3306` | Forward local port 3306 to `db.internal:3306` through the tunnel |
| Remote (`-R`) | `-R 9000:localhost:3000` | Expose local port 3000 on the remote server's port 9000 |
| Dynamic (`-D`) | `-D 1080` | SOCKS proxy on local port 1080 |

6. Optionally enable **Auto-start** (tunnel starts when RustConn launches) and **Auto-reconnect** (tunnel restarts if the SSH process exits unexpectedly)
7. Click **Save**

**Manage Tunnels:**

The tunnel manager window shows two groups:
- **Active** — currently running tunnels with a stop button
- **Stopped** — idle tunnels with a start button

Each tunnel row displays the connection name, forwarding summary (e.g., "L 3306→db:3306, D 1080"), and status. Click the toggle to start or stop a tunnel. Use the edit (pencil) and delete (trash) buttons to modify or remove tunnels.

**Tunnel Options:**

| Option | Description | Default |
|--------|-------------|---------|
| Auto-start | Start this tunnel automatically when RustConn launches | Off |
| Auto-reconnect | Restart the tunnel if the SSH process exits unexpectedly | Off |
| Enabled | Disabled tunnels are skipped by auto-start | On |

**Use Cases:**
- Access a database behind a firewall: `L 3306:db.internal:3306` → connect your DB client to `localhost:3306`
- SOCKS proxy for browsing through a remote network: `D 1080` → configure browser to use `localhost:1080`
- Expose a local dev server to a remote machine: `R 8080:localhost:3000`
- Persistent tunnels that survive terminal tab closes — the tunnel keeps running until you stop it or quit RustConn

#### Visual Tunnel Builder (Wizard)

The Visual Tunnel Builder is a 3-step wizard dialog that guides you through creating or editing SSH tunnels. It replaces the previous flat dialog with a structured workflow and a visual path diagram showing the tunnel chain.

**Open:** From the SSH Tunnel Manager window (Ctrl+T), click **Add Tunnel** to create a new tunnel, or click the **Edit** (pencil) button on an existing tunnel.

**Step 1 — Connection & Name:**
- Enter a tunnel name (1–128 characters)
- Select an SSH connection from the dropdown (filter by typing in the search field)
- If the selected connection has a jump host configured, the bastion is shown automatically on the path diagram
- Optionally override the jump host by selecting a different SSH connection as the bastion
- If no SSH connections exist, a prompt offers to create one
- The visual path diagram updates in real time: **localhost → bastion → target**

**Step 2 — Port Forwards & Options:**
- Add port forwarding rules using the **Add Forward** button (up to 20 rules per tunnel):

| Direction | Fields | Example |
|-----------|--------|---------|
| Local (`-L`) | Local port, remote host, remote port | `L 3306 → db.internal:3306` |
| Remote (`-R`) | Local port, remote host, remote port | `R 9000 → localhost:3000` |
| Dynamic (`-D`) | Local port only | `D 1080 (SOCKS)` |

- Each rule is shown as a collapsible row with a dynamic summary title
- Port validation: 1–65535 required; ports below 1024 show a privilege warning
- Remote host is required for Local and Remote directions
- Toggle **Auto-start** (tunnel starts with RustConn) and **Auto-reconnect** (restart on unexpected exit)
- The path diagram reflects the current configuration

**Step 3 — Review & Confirm:**
- Full visual path diagram with status indicators (in edit mode: Running, Starting, Failed, Stopped)
- Summary of all configured parameters
- Monospace SSH command preview showing the exact `ssh` command that will be executed (e.g., `ssh -N -L 3306:db:3306 -J bastion user@target -p 22`)
- **Copy** button copies the SSH command to clipboard (toast confirms "Copied")
- If no port forwarding rules are configured, an info message is displayed
- Click **Create** (new tunnel) or **Save** (editing) to finish

**Status Indicators (Edit Mode):**

When editing an existing tunnel, the path diagram shows the current tunnel status:

| Status | Visual |
|--------|--------|
| Running | Green nodes with animated connection line |
| Starting | Yellow/warning nodes with pulsing animation |
| Failed | Red/error nodes with error tooltip |
| Stopped | Dimmed/inactive nodes |

**Difference from Per-connection Port Forwarding:**

| Feature | Per-connection (SSH tab) | Standalone Tunnel Manager |
|---------|--------------------------|---------------------------|
| Lifecycle | Tied to terminal session | Independent background process |
| Terminal | Opens a terminal tab | No terminal (headless `ssh -N`) |
| Management | Configured per-connection | Centralized in Tunnel Manager |
| Auto-start | No | Yes |
| Auto-reconnect | No | Yes |

---

## Settings

Access via **Ctrl+,** or Menu → **Settings**

The settings dialog uses `adw::PreferencesDialog` with built-in search. Settings are organized into 6 pages:

| Page | Icon | Contents |
|------|------|----------|
| Terminal | `utilities-terminal-symbolic` | Terminal + Logging |
| Interface | `applications-graphics-symbolic` | Appearance, Window, Startup, System Tray, Session Restore, Keybindings + Backup & Restore |
| Secrets | `channel-secure-symbolic` | Secret backends + SSH Agent |
| Connection | `network-server-symbolic` | Clients |
| Monitoring | `power-profile-performance-symbolic` | Remote host metrics (General + Visible Metrics) + Terminal Activity Monitor defaults |
| Cloud Sync | `emblem-synchronizing-symbolic` | Sync directory, synced groups, simple sync |

### Terminal page

**Terminal group:** Font (family and size), Scrollback (history buffer lines), Color Theme (Dark, Light, Solarized, Monokai, Dracula, plus user-created custom themes), Cursor (shape and blink mode), Behavior (scroll on output/keystroke, hyperlinks, mouse autohide, bell, SFTP via mc, copy on select, close tab on clean exit).

**Close tab on clean exit:** When enabled, tabs are automatically closed when the remote session exits cleanly (exit code 0, e.g. user typed `exit` or `logout`) instead of showing the reconnect overlay. Disabled by default.

**Local Shell group:** Command — custom command to run in Local Shell tabs instead of the default login shell (e.g. `fish`, `bash --norc`, `neofetch && bash`). Leave empty for system default.

**Custom Themes:** Click the **+** button next to the theme dropdown to create a new custom theme. The theme editor lets you set background, foreground, cursor, and all 16 ANSI palette colors. Custom themes are saved to `~/.config/rustconn/custom_themes.json` and appear alongside built-in themes. Edit or delete custom themes with the pencil and trash buttons.

**Logging group:** Enable Logging (global toggle), Log Directory, Retention Days, Logging Modes (activity, user input, terminal output), Timestamps.

### Interface page

**Appearance group:** Theme (System, Light, Dark), Language (UI language selector, restart required), Color tabs by protocol, Sidebar width (260–500 pixels, default 320).

**Window group:** Remember size (restore window geometry on startup).

**Startup group:** On startup — Do nothing, Local Shell, or connect to a specific saved connection.

**System Tray group:** Show icon, Minimize to tray (hide window instead of closing).

**Session Restore group:** Enabled, Ask first, Max age (1–168 hours).

**Keybindings group:** Customizable keyboard shortcuts for 30+ actions across 6 categories. Record button to capture key combinations. Per-shortcut Reset and Reset All to Defaults.

### Secrets page

**Secret backend group:**
- **Preferred Backend** — libsecret, KeePassXC, KDBX file, Bitwarden, 1Password, Passbolt, Pass (passwordstore.org)
- **Enable Fallback** — Use libsecret if primary unavailable
- **Credential Encryption** — Backend master passwords encrypted with AES-256-GCM + Argon2id (machine-specific key)
- **Bitwarden Settings:** Vault status, unlock button, master password persistence, save to system keyring, auto-unlock, API key authentication for 2FA
- **1Password Settings:** Account status, sign-in button, biometric auth support, service account token
- **Passbolt Settings:** CLI detection, server URL, GPG passphrase, server configuration status
- **Pass Settings:** CLI detection, custom `PASSWORD_STORE_DIR`, GPG-encrypted files
- **KeePassXC KDBX Settings:** Database path, key file, password/key file authentication
- **System Keyring Requirements:** Requires `libsecret-tools` (`secret-tool` binary)
- **Installed Password Managers** — Auto-detected managers with versions

**SSH Agent group:** Status (running/stopped with socket path), Loaded Keys (with remove option), Available Keys (keys in `~/.ssh/` with add option).

### Connection page

**Clients group:** Auto-detected CLI tools with versions — Protocol Clients (SSH, RDP, VNC, SPICE, Telnet, Serial, Kubernetes) and Zero Trust (AWS, GCP, Azure, OCI, Cloudflare, Teleport, Tailscale, Boundary, Hoop.dev). Searches PATH and user directories.

**Monitoring group:** Enable monitoring (global toggle), Polling interval (1–60 seconds, default: 3), Visible Metrics (CPU, Memory, Disk, Network, Load Average, System Info).

### Custom Keybindings

Customize all keyboard shortcuts via Settings → Interface page → Keybindings.

1. Open **Settings** (Ctrl+,) → **Keybindings** tab
2. Find the action you want to change
3. Click **Record** next to it
4. Press the desired key combination
5. The new shortcut is saved immediately

Click the ↩ button next to any shortcut to reset it to default, or **Reset All to Defaults** at the bottom.

### Keyboard Passthrough Mode

When working in remote sessions with TUI applications (nvim, tmux, htop, mc), RustConn's keyboard shortcuts can conflict with the remote application's bindings. Keyboard passthrough mode disables all application shortcuts so every key combination reaches the remote session.

**Toggle passthrough:**
- Press **Ctrl+Shift+Backspace** (works in both normal and passthrough mode)
- Or use the menu: ☰ → Keyboard Passthrough
- Or use the command palette: Ctrl+P → "Toggle Keyboard Passthrough"

**When passthrough is active:**
- All application shortcuts are disabled (Ctrl+N, Ctrl+F, Ctrl+P, etc. go to the terminal)
- The F10 primary-menu key is also suspended, so F10 reaches the remote session (e.g. Midnight Commander)
- Only three shortcuts remain active: the passthrough toggle itself (Ctrl+Shift+Backspace), Quit (Ctrl+Q), and Fullscreen (F11)
- A toast notification confirms the mode change
- The menu item shows a checkmark when active

**Customization:** The list of shortcuts that remain active in passthrough mode can be configured in `config.toml` under `[keybindings] passthrough_exceptions`.

### Adaptive UI

RustConn adapts to different window sizes using `adw::Breakpoint` and responsive dialog sizing.

**Main window breakpoints:**
- Below 600sp: split view buttons hidden from header bar (still accessible via keyboard shortcuts or menu)
- Below 400sp: sidebar collapses to overlay mode (toggle with F9 or swipe gesture)

**Dialogs:** All dialogs have minimum size constraints and scroll their content. They can be resized down to ~350px width without clipping.

### Startup Action

Configure which session opens automatically when RustConn starts.

**Settings (GUI):**
1. Open **Settings** (Ctrl+,) → **Interface** page → **Startup** group
2. Select: **Do nothing**, **Local Shell**, or **\<Connection Name\> (Protocol)**

**CLI Override:** `rustconn --shell` or `rustconn --connect "Production Server"` (overrides persisted setting for a single launch).

**Use RustConn as Default Terminal:** Create a `.desktop` file with `Exec=rustconn --shell` and set it as the default terminal in your desktop environment settings.

### Backup & Restore

Back up your entire RustConn configuration as a single ZIP archive.

**Create a Backup:** Settings → Interface → Backup & Restore → **Backup** → choose save location.

**Restore from Backup:** Settings → Interface → Backup & Restore → **Restore** → select ZIP → confirm → restart RustConn.

> **Note:** After restoring, any changes made in the Settings dialog before closing it will be discarded. Close the dialog and restart RustConn to apply the restored configuration.

**What's Included:**

| Included | Not Included |
|----------|-------------|
| Connections and groups | Passwords (stored in secret backend) |
| Templates and snippets | Encrypted documents |
| Clusters | SSH keys |
| Global variables (names only; secret values are in vault) | Session logs |
| Keybindings | Flatpak-installed CLI tools |
| Application settings | |
| Connection history and statistics | |

> **Important:** The `.machine-key` file (`~/.local/share/rustconn/.machine-key`) is **not** included in backups. This key is used to encrypt credentials stored locally (AES-256-GCM). To migrate encrypted credentials to a different machine, copy `.machine-key` from the old machine **before** restoring the backup, or re-enter passwords after restore.

---

## Import, Export & Migration

### Import (Ctrl+I)

**Supported formats:**
- SSH Config (`~/.ssh/config`)
- Remmina profiles
- Asbru-CM configuration
- Ansible inventory (INI/YAML)
- Royal TS (.rtsz XML)
- MobaXterm sessions (.mxtsessions)
- SecureCRT sessions (.ini directory)
- Remote Desktop Manager (JSON)
- RDP files (.rdp — Microsoft Remote Desktop)
- Virt-Viewer (.vv files — SPICE/VNC from libvirt, Proxmox VE)
- Libvirt / GNOME Boxes (domain XML — VNC, SPICE, RDP from QEMU/KVM VMs)
- RustConn Native (.rcn)

Double-click source to start import immediately.

**Merge Strategies:**
- **Skip Existing** — Keep current connections, skip duplicates
- **Overwrite** — Replace existing connections with imported data
- **Rename** — Import as new connections with a suffix

**Duplicate Handling:** If imported connections have names that already exist, RustConn shows a dialog with the duplicate count and three choices — **Cancel**, **Skip Duplicates**, or **Import All** — instead of silently creating renamed copies.

**Import Preview:** For large imports (10+ connections), a preview is shown before applying.

**Import Source Details:**

| Source | Auto-scan | File picker | Protocols | Notes |
|--------|:---------:|:-----------:|-----------|-------|
| SSH Config | `~/.ssh/config` | Any file | SSH | Host blocks → connections |
| Remmina | `~/.local/share/remmina/` | — | SSH, RDP, VNC, SFTP | One `.remmina` per connection |
| Asbru-CM | `~/.config/pac/` | YAML file | SSH, VNC, RDP | Variables converted to `${VAR}` |
| Ansible | `/etc/ansible/hosts` | INI/YAML file | SSH | Groups preserved |
| Royal TS | — | `.rtsz` file | All | Folder hierarchy → groups |
| MobaXterm | — | `.mxtsessions` | SSH, RDP, VNC, Telnet, Serial | INI-based sessions |
| SecureCRT | `~/.vandyke/Config/Sessions/` | Directory or `.ini` | SSH, Telnet, RDP, VNC | Folder hierarchy → groups |
| Remote Desktop Manager | — | JSON file | SSH, RDP, VNC | Devolutions JSON export |
| RDP File | — | `.rdp` file | RDP | Microsoft Remote Desktop format |
| Virt-Viewer | — | `.vv` file | SPICE, VNC | From libvirt, Proxmox VE, oVirt |
| Libvirt / GNOME Boxes | `/etc/libvirt/qemu/`, `~/.config/libvirt/qemu/` | XML file | VNC, SPICE, RDP | Domain XML `<graphics>` elements |
| Libvirt Daemon (virsh) | `qemu:///session` | — | VNC, SPICE, RDP | Queries running libvirtd via `virsh` |
| RustConn Native | — | `.rcn` file | All | Full-fidelity backup |

### Export (Ctrl+Shift+E)

**Supported formats:** SSH Config, Remmina profiles, Asbru-CM, Ansible inventory, Royal TS (.rtsz), MobaXterm (.mxtsessions), SecureCRT (.ini), RustConn Native (.rcn).

Options: Include passwords (where supported), Export selected only.

**Format Limitations:**

| Format | Protocols | Passwords | Groups | Notes |
|--------|-----------|-----------|--------|-------|
| SSH Config | SSH only | Key paths only | No | Standard `~/.ssh/config` format |
| Remmina | SSH, RDP, VNC, SFTP | Encrypted | No | One `.remmina` file per connection |
| Asbru-CM | SSH, VNC, RDP | Encrypted | Yes | YAML-based, supports variables |
| Ansible | SSH only | No | Yes (groups) | INI or YAML inventory format |
| Royal TS | All | Encrypted | Yes | XML `.rtsz` archive |
| MobaXterm | SSH, RDP, VNC, Telnet | Encrypted | Yes | INI-based `.mxtsessions` |
| SecureCRT | SSH, Telnet, RDP, VNC | No | Yes | Directory of `.ini` files |
| RustConn Native | All | Encrypted | Yes | Full-fidelity backup format |

### CSV Import/Export

Import connections from CSV files or export to CSV format. Follows RFC 4180.

**CSV Import:**
1. Menu → Import or Ctrl+I → select CSV format
2. Choose the CSV file
3. RustConn auto-detects column mapping from headers (`name`, `host`, `port`, `protocol`, `username`, `group`, `tags`, `description`)
4. Review mapping, select delimiter (comma, semicolon, tab)
5. Click Import

**Tags:** Semicolon-separated in the `tags` column: `web;production;eu`

**Groups:** Slash-separated path in the `group` column: `Production/Web Servers`

### RDP File Association

RustConn registers as a handler for `.rdp` files. Double-clicking an `.rdp` file opens RustConn and connects automatically.

**How It Works:**
1. Double-click an `.rdp` file (or run `rustconn file.rdp`)
2. RustConn parses the file and creates a temporary connection
3. The connection starts immediately

**Supported .rdp Fields:** `full address`, `username`, `domain`, `gatewayhostname`, `gatewayusername`, `desktopwidth`/`desktopheight`, `session bpp`, `audiomode`, `redirectclipboard`, `remoteapplicationprogram`, `remoteapplicationcmdline`, `remoteapplicationname`.

**Desktop Integration:**
```bash
xdg-mime default io.github.totoshko88.RustConn.desktop application/x-rdp
```

### Virt-Viewer (.vv) File Association

RustConn registers as a handler for `.vv` files (SPICE/VNC connections from libvirt, Proxmox VE, oVirt). Double-clicking a `.vv` file opens RustConn and connects automatically.

**How It Works:**
1. Double-click a `.vv` file (or run `rustconn file.vv`)
2. RustConn parses the file and creates a connection with all settings (host, port, TLS, proxy, password)
3. The connection starts immediately

**Proxmox VE SPICE Tickets:**

Proxmox VE generates short-lived `.vv` files ("SPICE tickets") valid for 30–40 seconds. RustConn handles these correctly:
- Inline PEM CA certificates are automatically saved to `~/.local/share/rustconn/certs/` and configured in connection settings
- The proxy URL (`pvespiceproxy`) is preserved in the SPICE configuration
- The connection starts immediately after import, meeting the ticket TTL requirement

**Supported Fields:** `type` (spice/vnc), `host`, `port`, `tls-port`, `password`, `title`, `proxy`, `ca` (file path or inline PEM), `host-subject`, `delete-this-file`.

**Desktop Integration:**
```bash
xdg-mime default io.github.totoshko88.RustConn.desktop application/x-virt-viewer
```

### Migration Guide

#### From Remmina

1. **File > Import > Remmina** → select data directory
2. Review import preview → choose merge strategy → Import
3. Re-enter passwords and verify SSH key paths after import

> **Flatpak ↔ Flatpak:** If both RustConn and Remmina are installed as Flatpaks, the Remmina import button may show "Not Found" because Remmina stores its `.remmina` files inside its own sandbox (`~/.var/app/org.remmina.Remmina/data/remmina/`), which RustConn cannot access by default. See [Flatpak Sandbox Overrides → Remmina Import](#flatpak-sandbox-overrides) below for the fix.

#### From MobaXterm

1. Export sessions from MobaXterm → copy `.mxtsessions` file to Linux
2. **File > Import > MobaXterm** → select file → Import

#### From SecureCRT

1. Locate SecureCRT sessions directory (`~/.vandyke/Config/Sessions/` on Linux, or `%APPDATA%\VanDyke\Config\Sessions\` on Windows — copy to Linux)
2. **File > Import > SecureCRT** → select the `Sessions` directory → Import
3. Folder hierarchy is preserved as connection groups; SSH keys, usernames, ports, X11/agent forwarding settings are imported

#### From Royal TS

1. In Royal TS: **File > Export > Royal TS Document (.rtsz)**
2. **File > Import > Royal TS** → select file → Import (folder structure preserved as groups)

#### From SSH Config

1. **File > Import > SSH Config** → select `~/.ssh/config`
2. Each `Host` block becomes an SSH connection

#### From Ansible Inventory

1. **File > Import > Ansible** → select inventory file
2. Host groups become RustConn groups; hosts become SSH connections

#### From Libvirt / GNOME Boxes

1. **File > Import > Libvirt / GNOME Boxes** (auto-scan) or select individual XML files
2. Each `<graphics>` element becomes a VNC, SPICE, or RDP connection

#### Post-Migration Checklist

- [ ] Re-enter passwords (no import format includes plaintext credentials)
- [ ] Verify SSH key paths (may differ between Windows and Linux)
- [ ] Test a connection from each protocol type
- [ ] Organize imported connections into groups
- [ ] Set up your preferred secret backend
- [ ] Delete the import source file if it contains sensitive data

### Configuration Sync Between Machines

RustConn stores all configuration in `~/.config/rustconn/`:

```
~/.config/rustconn/
├── config.toml           # Application settings
├── connections.toml      # Connections (hosts, ports, usernames)
├── groups.toml           # Group hierarchy and credentials
├── snippets.toml         # Command snippets
├── clusters.toml         # Broadcast clusters
├── templates.toml        # Connection templates
├── history.toml          # Connection history (local)
└── trash.toml            # Trash (local)
```

> **Note:** Smart Folders, global variables, keybindings, and all other application settings are stored inside `config.toml` — there is no separate `smart_folders.toml` file.

**Git (Recommended):**
```bash
cd ~/.config/rustconn
git init
echo "history.toml" >> .gitignore
echo "trash.toml" >> .gitignore
git add -A && git commit -m "Initial config"
git remote add origin <your-repo-url>
git push -u origin main
```

**Syncthing / rsync:**
```bash
rsync -avz ~/.config/rustconn/ user@remote:~/.config/rustconn/
```

**Tips:**
- `history.toml` and `trash.toml` are machine-local — exclude them from sync
- Passwords stored in KeePass/libsecret/Bitwarden are not in the config files — sync your vault separately
- After syncing, restart RustConn to pick up changes

---

## Cloud Sync

Synchronize connection configurations between devices and team members through any shared cloud directory (Google Drive, Syncthing, Nextcloud, Dropbox, USB drive — anything that syncs files).

### Group Sync

Group Sync is designed for teams. Each root group exports to a dedicated `.rcn` file using a Master/Import access model.

- **Master** — full control, exports changes to the sync file
- **Import** — read-only, imports changes from the sync file

**Enable Group Sync:**
1. Go to Settings → Cloud Sync → set a Sync Directory
2. Right-click a root group → Edit Group → **Cloud Sync** tab → set sync mode to "Master"
3. The group is exported to `<sync-dir>/<group-slug>.rcn`

**Import a shared group:**
1. Go to Settings → Cloud Sync → "Available in Cloud" section
2. Click "Import" next to the `.rcn` file
3. The group appears in the sidebar with a sync indicator (⟳)

Import groups are read-only for synced fields (name, host, port, protocol). Local-only fields (SSH key path, sort order, pinned status) remain editable. Changes from the Master are auto-imported when the file watcher detects updates (3s debounce).

Credentials are never synced — only variable names are included. Each team member configures their own secret backend values locally.

### Simple Sync

Simple Sync is for personal multi-device use. A single `full-sync.rcn` file contains all connections, groups, templates, snippets, and clusters with UUID-based bidirectional merge.

**Enable:** Settings → Cloud Sync → toggle "Sync everything between your devices"

Deletions are tracked via tombstones (auto-cleaned after 30 days). The `device_id` prevents circular self-sync.

### SSH Key Inheritance

Groups can define SSH settings (auth method, key path, proxy jump, agent socket) that child connections inherit. This avoids duplicating key paths across dozens of connections and keeps `ssh_key_path` local-only per device.

### Flatpak: Granting Filesystem Access for Cloud Sync

Flatpak sandboxes restrict filesystem access by default. Cloud Sync requires read/write access to your sync directory (e.g. Google Drive, Syncthing, Nextcloud folder).

> **Automatic detection:** When selecting a sync directory in Flatpak, RustConn detects XDG Document Portal paths (temporary FUSE mounts that don't support inotify) and shows a warning dialog with the exact `flatpak override` command needed. You can copy the command directly from the dialog.

**Grant access to a specific directory:**

```bash
flatpak override --user --filesystem=/path/to/your/sync/folder io.github.totoshko88.RustConn
```

**Common examples:**

```bash
# Google Drive (via GNOME Online Accounts)
flatpak override --user --filesystem=xdg-run/gvfs io.github.totoshko88.RustConn

# Syncthing default folder
flatpak override --user --filesystem=~/Sync io.github.totoshko88.RustConn

# Nextcloud
flatpak override --user --filesystem=~/Nextcloud io.github.totoshko88.RustConn

# Dropbox
flatpak override --user --filesystem=~/Dropbox io.github.totoshko88.RustConn

# Custom path
flatpak override --user --filesystem=/mnt/shared/rustconn-sync io.github.totoshko88.RustConn
```

**Verify access:**

```bash
flatpak info --show-permissions io.github.totoshko88.RustConn | grep filesystem
```

**Revoke access:**

```bash
flatpak override --user --nofilesystem=/path/to/folder io.github.totoshko88.RustConn
```

After granting access, restart RustConn and set the sync directory in Settings → Cloud Sync.

> **Note:** You can also use [Flatseal](https://flathub.org/apps/com.github.tchx84.Flatseal) for a graphical interface to manage Flatpak permissions.

**Configure:**
1. Edit a group → SSH Settings section
2. Set SSH Key Path, Auth Method, Proxy Jump, or Agent Socket
3. Child connections with Key Source = "Inherit" use the group's values

The inheritance chain walks from the connection's immediate group up to the root, returning the first value found.

### Credential Resolution

When connecting to a synced connection that references an unconfigured variable or secret backend, RustConn shows an interactive dialog instead of silently failing:

- **Variable Not Configured** — an `AdwAlertDialog` prompts you to enter the variable value and select a storage backend (LibSecret, KeePassXC, Bitwarden, 1Password). Click "Save & Connect" to store the value and proceed, or "Cancel" to abort.
- **Secret Backend Not Configured** — shown when the connection's password source references a vault that isn't set up on this device. Choose "Enter Password Manually" to proceed with a one-time password prompt, or "Open Settings" to configure the backend first.
- **Vault Entry Missing** — if the vault is configured but the specific credential entry doesn't exist, a warning toast is shown ("Vault entry not found for '…'") and the connection proceeds without stored credentials; the protocol handler prompts for a password (RDP/VNC password dialog, SSH terminal prompt).

**Sidebar sync indicators** show the current sync state for each synced group:
- ⟳ (`emblem-synchronizing-symbolic`) — synced successfully, tooltip shows "Master — synced to cloud" or "Import — synced from cloud"
- ⚠ (`dialog-warning-symbolic`) — last sync operation failed, tooltip shows the specific error (e.g. "Sync error: Parse error: invalid JSON")

**Sync failure banner:** When a sync operation fails (manual *Sync Now* or background auto-export), a persistent `adw::Banner` appears below the header bar and stays until you dismiss it or the next successful sync clears it. Success messages still use transient toasts.

### Auto-Login with Stored Passwords

RustConn can automatically fill SSH password prompts using credentials stored in your vault. For this to work:

1. **Set Password Source to "Vault"** — in the connection dialog (Basic tab → Authentication → Password Source), select "Vault". Other sources (Prompt, None) will not trigger auto-login.

2. **Store the password in your vault** — use the "Load from vault" button (📂) to verify the password is retrievable. The lookup key format depends on your backend:
   - **KeePass/KDBX**: `RustConn/GroupName/ConnectionName (protocol)` — hierarchical path matching your group structure
   - **Keyring (libsecret)**: `ConnectionName (protocol)` — e.g. "MyServer (ssh)"
   - **Bitwarden/1Password/Passbolt/Pass**: `rustconn/ConnectionName`

3. **Test before connecting** — click the ✓ button next to the password field to run a credential resolution test. It shows the exact lookup key used and whether the vault returned a password.

4. **How it works at connect time**:
   - RustConn resolves credentials from the vault asynchronously
   - The resolved password is cached in memory for the session
   - When the SSH terminal shows a `password:` prompt, the password is automatically sent
   - The prompt is detected in 15+ languages (English, German, French, Spanish, Ukrainian, Chinese, Japanese, Korean, etc.)
   - Passphrase prompts ("Enter passphrase for key…") are excluded to avoid sending the wrong secret

#### Using Variables for Auto-Login

Instead of storing passwords directly per-connection, you can use **Global Variables** (Menu → Tools → Variables) as a shared credential source:

1. **Set Password Source to "Variable"** — select "Variable" and choose the variable name from the dropdown (e.g. `RADIUS`, `DEPLOY_KEY`).

2. **Store the variable value** — go to Menu → Tools → Variables and create a secret variable with the password value. The value is stored in your configured vault backend.

3. **First-time connection on a new device** — if the variable has not been configured on this device yet, RustConn shows a **"Variable Not Configured"** dialog:
   - Enter the password value
   - Select a storage backend (LibSecret, KeePassXC, Bitwarden, 1Password)
   - Click "Save & Connect" to store the value and proceed immediately

   This dialog only appears once per device. After saving, subsequent connections use the stored value automatically.

4. **Sharing across connections** — multiple connections can reference the same variable. Update the variable value once and all connections pick up the new password at next connect.

#### Password Source Summary

| Source | Behavior | Auto-Login |
|--------|----------|------------|
| **Vault** | Resolves password from configured secret backend using connection name as lookup key | ✅ Yes |
| **Variable** | Resolves password from a named Global Variable stored in vault | ✅ Yes |
| **Script** | Executes an external command and uses stdout as password | ✅ Yes |
| **Inherit** | Walks up the group hierarchy to find the first parent with Vault credentials | ✅ Yes |
| **Prompt** | Always asks for password at connect time | ❌ No |
| **None** | No password — relies on SSH keys or other auth methods | ❌ No |

**Common issues:**

| Symptom | Cause | Fix |
|---------|-------|-----|
| Password prompt appears despite vault configured | Password Source set to "Prompt" or "None" | Change to "Vault" or "Variable" in connection dialog |
| "Vault entry not found" toast | Entry name in vault doesn't match lookup key | Use ✓ test button to see expected key, rename vault entry |
| "Variable Not Configured" dialog appears | Variable value not stored on this device | Enter the value and click "Save & Connect" |
| Password sent but rejected | Wrong password stored in vault | Update the vault entry with correct password |
| Non-English prompt not detected | Unsupported language | Open an issue — we support 15+ languages |
| KeePass lookup fails | Database locked or wrong path | Check Settings → Secrets → KDBX path and password |

---

## Security

### Choosing a Secret Backend

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

**Fallback Behavior:**

When the preferred backend (e.g., KeePassXC) cannot be reached — database password not configured, database file locked, or CLI tool not installed — RustConn automatically falls back to the system keyring (libsecret) for both reading and writing secrets. This requires the "Enable fallback" option in Settings → Secrets (enabled by default).

- **Reading:** If KeePass returns no result, RustConn checks libsecret before showing the "Variable Not Configured" dialog
- **Writing:** If KeePass save fails, the secret is stored in libsecret instead
- **Variable Not Configured dialog:** When a connection requires a variable that has no value on this device, a dialog appears letting you enter the value and choose which backend to store it in — this choice is respected regardless of the global preferred backend setting

> **Tip:** If you see the "Variable Not Configured" dialog repeatedly, check that your KeePass database password is configured in Settings → Secrets. Without it, RustConn cannot read or write entries in the database.

### Credential Hygiene

- Use **SSH keys** instead of passwords whenever possible (Ed25519 or ECDSA recommended)
- Use **FIDO2/Security Keys** for the strongest SSH authentication (requires OpenSSH 8.2+)
- Set **Password Source** to a vault backend rather than storing passwords in the RustConn config
- Use **Group Credentials** to avoid duplicating the same password across multiple connections
- Enable **Inherit from Group** on child connections to centralize credential management
- Rotate credentials regularly; RustConn resolves passwords from the vault at connection time

### Network Security

- RustConn performs a **pre-connect port check** before establishing connections
- SSH connections verify host keys via the system `known_hosts` file
- Use **SSH Proxy Jump** for connections behind bastion hosts
- Use **Zero Trust providers** to eliminate direct SSH exposure
- Enable **session logging** for audit trails

---

## Troubleshooting & FAQ

### Frequently Asked Questions

**Where are my passwords stored?**
Depending on your configured secret backend: libsecret (desktop keyring), KeePassXC (database), KDBX file (local encrypted file), Bitwarden/1Password/Passbolt (cloud vault), or Pass (GPG-encrypted files). Connection files themselves never contain actual passwords.

**How do I migrate RustConn to another machine?**
Use [Backup & Restore](#backup--restore): Backup on old machine → copy ZIP → Restore on new machine → restart. Re-enter passwords or configure the same secret backend.

**Can I use RustConn without a secret backend?**
Yes. libsecret (desktop keyring) is used by default. If unavailable, use a local KDBX file as a fully offline backend.

**How do I share connections with my team?**
Export (File > Export) in Native `.rcn`, SSH Config, or CSV format → send to colleagues → they import via File > Import. Passwords are never included.

**Why does RustConn ask for my keyring password on startup?**
Your desktop keyring may be locked. Configure it to unlock automatically on login, or switch to a different secret backend.

**How do I connect to a host behind a jump server?**
Set the **Proxy Jump** field in the SSH connection dialog's Advanced tab (e.g., `user@bastion.example.com`). Chain multiple jump hosts with commas.

**How do I reset RustConn to default settings?**
```bash
mv ~/.config/rustconn ~/.config/rustconn.backup
```

### Connection Issues

1. Verify host/port: `ping hostname`
2. Check credentials
3. SSH key permissions: `chmod 600 ~/.ssh/id_rsa`
4. Firewall settings

### Libvirt VM Hostname Resolution (NSS Module)

If connecting to libvirt VMs by hostname fails, install the libvirt NSS module:
```bash
# Fedora
sudo dnf install libvirt-nss
# Debian/Ubuntu
sudo apt install libnss-libvirt
```
Add `libvirt libvirt_guest` to the `hosts` line in `/etc/nsswitch.conf`.

**Flatpak users:** Use the VM's IP address instead of hostname, or configure a local DNS entry.

### 1Password Not Working

1. Install 1Password CLI from 1password.com/downloads/command-line
2. Sign in: `op signin`
3. Or use service account: set `OP_SERVICE_ACCOUNT_TOKEN`
4. Select 1Password backend in Settings → Secrets

### Bitwarden Not Working

See [BITWARDEN_SETUP.md](BITWARDEN_SETUP.md) for a detailed guide.

Quick checklist:
1. Install Bitwarden CLI (Flatpak: via Flatpak Components; Native: `npm install -g @bitwarden/cli`)
2. For self-hosted: `bw config server https://your-server` before logging in
3. Login: `bw login` → Unlock: `bw unlock`
4. Select Bitwarden backend in Settings → Secrets
5. For 2FA (FIDO2, Duo): use API key authentication
6. Enable "Save to system keyring" for auto-unlock

### System Keyring Not Working

1. Install `libsecret-tools`: `sudo apt install libsecret-tools` or `sudo dnf install libsecret`
2. Verify: `secret-tool --version`
3. Ensure a Secret Service provider is running (GNOME Keyring, KDE Wallet)
4. Flatpak: `secret-tool` is bundled — ensure desktop has a Secret Service provider

### Passbolt Not Working

1. Install `go-passbolt-cli` from github.com/passbolt/go-passbolt-cli
2. Configure: `passbolt configure --serverAddress https://your-server.com --userPrivateKeyFile key.asc --userPassword`
3. Verify: `passbolt list resource`

### KeePass Not Working

1. Install KeePassXC → enable browser integration
2. Configure KDBX path in Settings → Secrets
3. Flatpak: KeePassXC on host is detected automatically via `flatpak-spawn --host`

### Pass (passwordstore.org) Not Working

1. Install `pass`: `sudo apt install pass` or `sudo dnf install pass`
2. Initialize store: `pass init <gpg-id>`
3. Select Pass backend in Settings → Secrets

### Embedded RDP/VNC Issues

1. Check IronRDP/vnc-rs features enabled
2. For external: verify FreeRDP/TigerVNC installed
3. Flatpak: FreeRDP (SDL3) is bundled; TigerVNC via Flatpak Components
4. HiDPI: use Scale Override in connection dialog
5. Clipboard not syncing: ensure "Clipboard" is enabled in RDP settings
6. RDP Gateway: IronRDP doesn't support RD Gateway; falls back to external FreeRDP

### Session Restore Issues

1. Enable in Settings → Interface → Session Restore
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

1. Add user to `dialout` group: `sudo usermod -aG dialout $USER`
2. Log out and back in
3. Flatpak: `--device=all` permission (automatic)
4. Snap: `sudo snap connect rustconn:serial-port`

### Kubernetes Connection Issues

1. Verify `kubectl` is installed and in PATH
2. Check cluster access: `kubectl cluster-info`
3. Verify pod exists: `kubectl get pods -n <namespace>`
4. Flatpak: install `kubectl` via Flatpak Components

### Terminal Clear Not Working (Ctrl+L / `clear` command)

VTE handles screen clearing by scrolling content into scrollback rather than erasing. For Flatpak builds missing `clear`:
```bash
printf '\033[H\033[2J\033[3J'
# Or add alias to ~/.bashrc:
alias clear='printf "\033[H\033[2J\033[3J"'
```

### Flatpak Permissions

1. **File access:** `flatpak override --user --filesystem=home io.github.totoshko88.RustConn`
2. **SSH agent:** Forwarded via `--socket=ssh-auth`; alternative agent sockets need manual override
3. **Serial devices:** `--device=all` permission
4. **CLI tools:** Host binaries not visible — use Flatpak Components
5. **Secret Service:** Works via D-Bus portal
6. **KeePassXC:** Detected via `flatpak-spawn --host`
7. **Zero Trust / Kubernetes:** Cloud CLIs detected via `flatpak-spawn --host`; config dirs mounted
8. **FreeRDP:** Bundled (SDL3 client)

### Monitoring Issues

1. Verify SSH connection works normally
2. Check remote host has `uptime`, `free`, `df`, `cat /proc/loadavg`
3. Ensure `MaxSessions` in `sshd_config` allows multiple sessions
4. Increase polling interval if metrics show "N/A"

### Flatpak Sandbox Overrides

The Flatpak build ships with minimal sandbox permissions. Some features require manual overrides:

**SSH Agent Sockets:**
```bash
# KeePassXC
flatpak override --user --filesystem=xdg-run/ssh-agent:ro io.github.totoshko88.RustConn
# Bitwarden
flatpak override --user --filesystem=home/.var/app/com.bitwarden.desktop/data:ro io.github.totoshko88.RustConn
# GPG agent
flatpak override --user --filesystem=xdg-run/gnupg:ro io.github.totoshko88.RustConn
# 1Password
flatpak override --user --filesystem=home/.1password:ro io.github.totoshko88.RustConn
```

**Hoop.dev:**
```bash
flatpak override --user --filesystem=home/.hoop:ro io.github.totoshko88.RustConn
```

**Remmina Import (Flatpak ↔ Flatpak):**

When Remmina is also installed as a Flatpak, its connection files live inside its own sandbox at `~/.var/app/org.remmina.Remmina/data/remmina/` instead of the standard `~/.local/share/remmina/`. RustConn cannot see this directory by default, so the import button shows "Not Found."

*Option A — Grant read access (recommended):*
```bash
flatpak override --user --filesystem=home/.var/app/org.remmina.Remmina/data/remmina:ro io.github.totoshko88.RustConn
```

*Option B — Flatseal (GUI):*
1. Open [Flatseal](https://flathub.org/apps/com.github.tchx84.Flatseal) → select **RustConn**
2. Scroll to **Filesystem → Other files** → add `~/.var/app/org.remmina.Remmina/data/remmina:ro`

*Option C — Symlink (no permission changes):*
```bash
ln -s ~/.var/app/org.remmina.Remmina/data/remmina ~/.local/share/remmina
```

Restart RustConn after any of the above. The Remmina import should now detect connection files.

**RDP Shared Folders:**
```bash
flatpak override --user --filesystem=home io.github.totoshko88.RustConn
```

**View/Reset Overrides:**
```bash
flatpak override --user --show io.github.totoshko88.RustConn
flatpak override --user --reset io.github.totoshko88.RustConn
```

---

## Keyboard Shortcuts

Press **Ctrl+?** or **F1** for searchable shortcuts dialog.

Note: Sidebar-scoped shortcuts (F2, Delete, Ctrl+E, Ctrl+D, Ctrl+C, Ctrl+V, Ctrl+M) only work when the sidebar has focus.

### Connections

| Shortcut | Action |
|----------|--------|
| Ctrl+N | New Connection (Wizard) |
| Ctrl+Shift+N | New Connection (Advanced) |
| Ctrl+Shift+G | New Group |
| Ctrl+Shift+Q | Quick Connect |
| Ctrl+I | Import |
| Ctrl+Shift+E | Export |
| Ctrl+E | Edit Connection (sidebar) |
| F2 | Rename |
| Delete | Delete |
| Ctrl+D | Duplicate |
| Ctrl+C / Ctrl+V | Copy / Paste |
| Ctrl+M | Move to Group |
| Enter | Connect to selected |
| Menu / Shift+F10 | Open context menu for selected row |

### Terminal

| Shortcut | Action |
|----------|--------|
| Ctrl+Shift+C | Copy |
| Ctrl+Shift+V | Paste |
| Ctrl+Shift+F | Terminal Search |
| Ctrl+W / Ctrl+Shift+W | Close Tab |
| Ctrl+Tab / Ctrl+PageDown | Next Tab |
| Ctrl+Shift+Tab / Ctrl+PageUp | Previous Tab |
| Ctrl+Shift+T | Local Shell |
| Ctrl+Shift+O | Tab Overview |
| Ctrl+% | Switch to Open Tab |
| Ctrl+Scroll | Zoom in/out (font size) |
| Ctrl+Plus / Ctrl+Minus | Zoom in/out (font size) |
| Ctrl+0 | Reset zoom |

### Terminal Keybinding Modes

RustConn uses VTE, which passes all keystrokes to the shell. Configure vim/emacs mode in your shell:

| Shell | Vim Mode | Emacs Mode (default) |
|-------|----------|---------------------|
| Bash | `set -o vi` in `~/.bashrc` | `set -o emacs` in `~/.bashrc` |
| Zsh | `bindkey -v` in `~/.zshrc` | `bindkey -e` in `~/.zshrc` |
| Fish | `fish_vi_key_bindings` | `fish_default_key_bindings` |

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
| Ctrl+1 / Alt+1 | Focus Sidebar |
| Ctrl+2 / Alt+2 | Focus Terminal |
| Ctrl+, | Settings |
| F11 | Toggle Fullscreen |
| F9 | Toggle Sidebar |
| Ctrl+H | Connection History |
| Ctrl+Shift+I | Statistics |
| Ctrl+G | Password Generator |
| Ctrl+Shift+L | Wake On LAN |
| Ctrl+T | SSH Tunnel Manager |
| F10 | Open Menu (suspended in passthrough mode) |
| Ctrl+? / F1 | Keyboard Shortcuts |
| Ctrl+Shift+Backspace | Toggle Keyboard Passthrough |
| Ctrl+Shift+B | Toggle Split Broadcast |
| Ctrl+Q | Quit |

> **Note:** Quitting with **Ctrl+Q** (or closing the window) while session tabs are open shows a "Close RustConn?" confirmation dialog with the number of open tabs, instead of silently disconnecting everything. This is skipped when minimize-to-tray is enabled (the app keeps running in the tray).

---

## Support

- **GitHub:** https://github.com/totoshko88/RustConn
- **Issues:** https://github.com/totoshko88/RustConn/issues
- **Releases:** https://github.com/totoshko88/RustConn/releases

**Made with ❤️ in Ukraine 🇺🇦**
