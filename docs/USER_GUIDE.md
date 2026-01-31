# RustConn User Guide

**Version 0.6.9** | GTK4/libadwaita Connection Manager for Linux

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
  - **KeePass** — Store/retrieve from KeePass database
  - **Keyring** — Store/retrieve from system keyring (libsecret)
  - **Bitwarden** — Store/retrieve from Bitwarden vault
  - **1Password** — Store/retrieve from 1Password vault
  - **Inherit** — Use credentials from parent group
  - **None** — No password (key-based auth)
- SSH key selection
- Key passphrase

**Protocol Tabs** (varies by protocol):

| Protocol | Options |
|----------|---------|
| SSH | Auth method, proxy jump (Jump Host), agent forwarding, X11 forwarding, compression, startup command |
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

- **Preferred Backend** — libsecret, KeePassXC, KDBX file, Bitwarden, 1Password
- **Enable Fallback** — Use libsecret if primary unavailable
- **KDBX Path** — KeePass database file (for KDBX backend)
- **KDBX Authentication** — Password and/or key file
- **Bitwarden Settings:**
  - Vault status and unlock button
  - Master password persistence (encrypted in settings)
  - Save to system keyring option (recommended)
  - API key authentication for automation/2FA (FIDO2, Duo)
  - Client ID and Client Secret fields
- **1Password Settings:**
  - Account status indicator
  - Sign-in button (opens terminal for interactive `op signin`)
  - Supports biometric authentication via desktop app
  - Service account support via `OP_SERVICE_ACCOUNT_TOKEN`
- **Installed Password Managers** — Auto-detected managers with versions (GNOME Secrets, KeePassXC, KeePass2, Bitwarden CLI, 1Password CLI)

**Password Source Defaults:**
When creating a new connection, the password source dropdown defaults to match your configured backend:
- KeePassXC/KDBX backend → "KeePass"
- libsecret backend → "Keyring"
- Bitwarden backend → "Bitwarden"
- 1Password backend → "1Password"

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
rustconn-cli list --format json
rustconn-cli list --group "Production" --tag "web"
rustconn-cli list --protocol ssh

# Connect
rustconn-cli connect "My Server"

# Add connection
rustconn-cli add --name "New Server" --host "192.168.1.10" --protocol ssh --user admin

# Show connection details
rustconn-cli show "My Server"

# Update connection
rustconn-cli update "My Server" --host "192.168.1.20" --port 2222

# Duplicate connection
rustconn-cli duplicate "My Server" --new-name "My Server Copy"

# Delete connection
rustconn-cli delete "My Server"

# Test connectivity
rustconn-cli test "My Server"
rustconn-cli test all --timeout 5

# Import/Export
rustconn-cli import --format ssh-config ~/.ssh/config
rustconn-cli import --format remmina ~/remmina/
rustconn-cli export --format native --output backup.rcn
rustconn-cli export --format ansible --output inventory.yml

# Snippets
rustconn-cli snippet list
rustconn-cli snippet show "Deploy Script"
rustconn-cli snippet add --name "Restart" --command "sudo systemctl restart \${service}"
rustconn-cli snippet run "Deploy" --var service=nginx --execute
rustconn-cli snippet delete "Old Snippet"

# Groups
rustconn-cli group list
rustconn-cli group show "Production"
rustconn-cli group create --name "New Group" --parent "Production"
rustconn-cli group add-connection --group "Production" --connection "Web-01"
rustconn-cli group remove-connection --group "Production" --connection "Web-01"
rustconn-cli group delete "Old Group"

# Templates
rustconn-cli template list
rustconn-cli template show "SSH Template"
rustconn-cli template create --name "New Template" --protocol ssh --port 2222
rustconn-cli template delete "Old Template"
rustconn-cli template apply "SSH Template" --name "New Connection" --host "server.example.com"

# Clusters
rustconn-cli cluster list
rustconn-cli cluster show "Web Servers"
rustconn-cli cluster create --name "DB Cluster" --broadcast
rustconn-cli cluster add-connection --cluster "DB Cluster" --connection "DB-01"
rustconn-cli cluster remove-connection --cluster "DB Cluster" --connection "DB-01"
rustconn-cli cluster delete "Old Cluster"

# Global Variables
rustconn-cli var list
rustconn-cli var show "my_var"
rustconn-cli var set my_var "my_value"
rustconn-cli var set api_key "secret123" --secret
rustconn-cli var delete "my_var"

# Secret Management (New in 0.6.7)
rustconn-cli secret status                    # Show backend status
rustconn-cli secret get "My Server"           # Get credentials
rustconn-cli secret get "My Server" --backend keepass
rustconn-cli secret set "My Server"           # Store (prompts for password)
rustconn-cli secret set "My Server" --password "pass" --backend keyring
rustconn-cli secret delete "My Server"        # Delete credentials
rustconn-cli secret verify-keepass --database ~/passwords.kdbx
rustconn-cli secret verify-keepass --database ~/passwords.kdbx --key-file ~/key.key

# Statistics
rustconn-cli stats

# Wake-on-LAN
rustconn-cli wol AA:BB:CC:DD:EE:FF
rustconn-cli wol "Server Name"
rustconn-cli wol AA:BB:CC:DD:EE:FF --broadcast 192.168.1.255 --port 9
```

### Secret Command Details

The `secret` command manages credentials stored in secret backends:

| Subcommand | Description |
|------------|-------------|
| `status` | Show available backends (Keyring, KeePass, Bitwarden) and configuration |
| `get` | Retrieve credentials for a connection |
| `set` | Store credentials (interactive password prompt if not provided) |
| `delete` | Delete credentials from backend |
| `verify-keepass` | Verify KeePass database can be unlocked |

**Backend Options:**
- `keyring` / `libsecret` — System keyring (GNOME Keyring, KDE Wallet)
- `keepass` / `kdbx` — KeePass database (requires KDBX configured in settings)
- `bitwarden` / `bw` — Bitwarden CLI

**Examples:**
```bash
# Check which backends are available
rustconn-cli secret status

# Store password in system keyring
rustconn-cli secret set "Production DB" --backend keyring

# Store password in KeePass (uses configured KDBX)
rustconn-cli secret set "Production DB" --backend keepass --user admin

# Verify KeePass database with key file
rustconn-cli secret verify-keepass -d ~/vault.kdbx -k ~/key.key
```

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

1. Install Bitwarden CLI: `npm install -g @bitwarden/cli` or download from bitwarden.com
2. Login: `bw login`
3. Unlock vault: `bw unlock`
4. Select Bitwarden backend in Settings → Secrets
5. Check vault status indicator
6. For 2FA methods not supported by CLI (FIDO2, Duo), use API key authentication:
   - Get API key from Bitwarden web vault → Settings → Security → Keys
   - Enable "Use API key authentication" in Settings → Secrets
   - Enter Client ID and Client Secret
7. For password source, select "Bitwarden" in connection dialog

### KeePass Not Working

1. Install KeePassXC
2. Enable browser integration in KeePassXC
3. Configure KDBX path in Settings → Secrets
4. Provide password/key file
5. For password source, select "KeePass" in connection dialog

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
