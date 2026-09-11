# RustConn User Guide

**Version 0.21.10** | GTK4/libadwaita Connection Manager for Linux

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
   - [Detached Session Windows](#detached-session-windows)
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
   - [Clusters](#clusters)
   - [Workspace Profiles](#workspace-profiles)
   - [Port Knocking](#port-knocking)
   - [Broadcast Input](#broadcast-input)
   - [Command Palette](#command-palette)
   - [Global Variables](#global-variables)
   - [Password Generator](#password-generator)
   - [Wake-on-LAN](#wake-on-lan)
   - [Connection History & Statistics](#connection-history)
   - [Remote Monitoring](#remote-monitoring)
   - [Flatpak Components](#flatpak-components)
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
- **Network Mode** — inherit a jump host from the group or the global settings, or go *Direct* (see [Network Mode and Where a Jump Host Comes From](#network-mode-and-where-a-jump-host-comes-from))
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

**SSH Agent Authentication:**
Set the auth method (or the key source) to **SSH Agent** and pick one of the keys the agent currently holds from the dropdown. That choice is honoured at connect time: RustConn restricts the connection to the selected key with `-i <public key> -o IdentitiesOnly=yes`, so the other keys in the agent are not offered. The identity passed to `ssh` is the key's *public* half, written to `$XDG_RUNTIME_DIR/rustconn/agent-keys/`; the private key never leaves the agent and you are asked to confirm at most once. This matters on an agent holding many keys, where a server with a low `MaxAuthTries` can refuse the connection before the right key is reached.

If the agent is not running when the session starts, or no longer holds the selected key, the connection still goes ahead with every agent key offered, and the reason is written to the log.

**Security Key / FIDO2 Authentication (SSH):**
SSH connections support hardware security keys (YubiKey, SoloKey, etc.) via the `security-key` auth method. Requirements:
- OpenSSH 8.2+ on both client and server
- `libfido2` installed on the client (`sudo apt install libfido2-1`)
- An `ed25519-sk` or `ecdsa-sk` key generated with `ssh-keygen -t ed25519-sk`
- The key file path configured in the connection's SSH key field

**PKCS#11 / Smart-Card Authentication (SSH):**
For hardware tokens that expose keys through a PKCS#11 library (YubiKey PIV, OpenSC smart cards, etc.), set the **PKCS#11 Provider** field in the connection editor (**SSH options → Session** group) to the path of the provider library, for example `/usr/lib64/libykcs11.so.2`. RustConn maps this to `ssh -o PKCS11Provider=<path>`, so the token's keys are offered automatically without loading them into the SSH agent first.

- Works alongside any auth method — the provider simply offers the token's keys.
- The PIN/touch prompt appears directly in the session terminal.
- `IdentitiesOnly` is **not** forced, so the token keys are always offered. Leave the field empty (or set `none`) to disable it, including an inherited provider.
- The directive is imported automatically from `~/.ssh/config` (`PKCS11Provider …`).

> **Through a jump host:** OpenSSH does **not** pass `-o PKCS11Provider` to `ProxyJump` child connections. To authenticate the bastion itself with the token, enable the **PKCS#11 Provider** field on the *jump-host connection* — RustConn injects it into the first hop's `ProxyCommand` for terminal SSH and for RDP/VNC/SPICE tunnels. With a jump host the token may prompt once per hop, because each hop is a separate SSH process.

#### How an SSH password reaches the server

**Changed in 0.21.2.** A stored SSH password is now handed to OpenSSH itself rather than typed into the terminal. Nothing about configuring a connection changes — this is here because it changes what you *see* and what happens when a stored password is wrong.

RustConn writes the password to a file only your account can read, in your session's runtime directory, and tells `ssh` where to find it. OpenSSH asks for it at the moment it needs it, so a slow connection can no longer miss its own prompt — that was the defect behind a jump-host session sitting at `password:` doing nothing. The helper answers only the account-password question OpenSSH asks; a key passphrase, a host-key confirmation, a one-time code and a "your password has expired" prompt are all left to you, so a stored password cannot end up answering the wrong question. The file is removed as soon as it is read, and again when the session ends.

What you will notice:

- **No password appears to be typed.** The prompt simply does not show up. That is expected.
- **A wrong stored password fails the connection instead of asking again.** RustConn allows one attempt, which is what stops a stale saved password from walking an account into a lockout. Fix the password in the connection, or set the Password Source to **Prompt**, and reconnect.
- **A host you are connecting to for the first time is accepted automatically** unless the connection sets its own `StrictHostKeyChecking`. A host key that has *changed* is still refused — that check is untouched.

Some connections still have their password typed into the terminal, exactly as before 0.21.2, and for those the **Password Prompt** field under [Automatic Login](#automatic-login-telnet--serial) still applies:

| Connection | Why |
|------------|-----|
| Auth method **Security Key** or **Keyboard Interactive** | The touch or the challenge is the point of the method; RustConn must not answer ahead of it |
| A **PKCS#11 Provider** is set | Same reason — the token's PIN prompt comes first |
| A custom SSH option redirects authentication or routing (`IdentityFile`, `PreferredAuthentications`, `ProxyJump`, `ProxyCommand`, `BatchMode` and similar) | You have taken authentication over by hand, and RustConn will not override what you wrote |
| A **ProxyJump** or **ProxyCommand** set in your own `~/.ssh/config` | The bastion it starts would be asked for a password in the same form as the target, and RustConn will not risk sending one host's password to another |

The last row is detected by asking `ssh -G` what your configuration resolves to, which costs a few milliseconds and touches the network not at all. A bastion configured **in RustConn** is handled and does not fall back — each hop gets its own password out of band.

### Network Mode and Where a Jump Host Comes From

A connection can reach its target through a bastion without naming one itself. Three tiers are consulted, nearest first, and the nearest one that has a value wins — separately for each of the two ways a bastion can be named, *Jump Host* and *ProxyJump*:

| Tier | Where it is set | Applies to |
|------|-----------------|------------|
| Connection | Edit connection → protocol page → **Connection** → *Jump Host* / *ProxyJump* (SSH, SFTP and MOSH share the SSH page; RDP, VNC and SPICE each have a *Jump Host* of their own) | this connection only |
| Group | Edit group → **SSH Settings** → *Jump Host* / *SSH Proxy Jump* | every connection in the group, and in its subgroups |
| Global | Settings → **Connection** → *Network* → *Global Jump Host* / *Global ProxyJump* | every connection that inherits, including ungrouped ones |

**Network Mode** decides whether the inherited tiers are consulted at all. It is on the connection editor's **Basic** page, beside Host and Port — not on a protocol page, because the choice belongs to the connection and applies whatever its protocol:

- **Inherit from group or global** (default) — walk the group chain, then the global settings.
- **Direct** — connect straight to the host and ignore any inherited bastion. A jump host set on the connection *itself* still applies; *Direct* refuses inherited ones, not explicit ones.

The *ProxyJump* field's subtitle names the bastion a connection has inherited — a group's or the global one, whether it was typed as `user@host` or picked from the dropdown — so a connection routed through one is not silently doing so. It follows *Network Mode* as you change it, and says so when *Direct* is ignoring an inherited bastion.

Three notes on scope:

- The **global** tier exists because a group cannot stand in for "everything". There is no single implicit root group — a top-level group is just a group with no parent, and there can be many — and an ungrouped connection has no chain to walk, so nothing set on a group can reach it.
- *Jump Host* and *ProxyJump* are two ways of naming a bastion, not two candidates for one slot. A *Jump Host* is a saved connection, so it also carries its port, its identity file and its own bastion chain — none of which fit in the text field. Each of the two resolves its own tier chain independently, so setting both is legitimate: they become two hops of one chain, the *Jump Host* contacted first and the *ProxyJump* value reached through it.
- An empty *ProxyJump* means "no bastion" at every tier, and falls through to the next one. This matters if you edit `config.toml` or `connections.toml` by hand: a stored empty string is not a value, it is nothing.

> **Tip:** The *Jump Host* dropdown (and the Web "Tunnel Through SSH" picker) is
> type-to-search. With a long connection list, open it and start typing part of a
> name or host to filter down to the one you want instead of scrolling.

For RDP, VNC and SPICE a bastion means an SSH tunnel to it, set up automatically before the viewer starts. Those protocols have no SOCKS option of their own, and they inherit — and refuse, via *Direct* — exactly like SSH does.

**Advanced Tabs:**
- **Advanced** — Window mode (Embedded/External/Fullscreen), remember window position, hide local cursor (embedded RDP/VNC/SPICE), Wake-on-LAN configuration (MAC address, broadcast, port, wait time), monitoring override (enable/disable per connection, overrides global setting)
- **Automation** — Automatic login (expected text of the device's username/password prompts), expect rules for auto-responding to terminal patterns, pattern tester with built-in templates (Sudo, SSH Host Key, Login, etc.), pre-connect task, post-disconnect task (with conditions: first/last connection only)
- **Data** — Local variables (connection-scoped, override global variables), custom properties (Text/URL/Protected metadata)
- **Logging** — Session logging (enable/disable, log path template with variables, timestamp format, max file size, retention days, granular content options: log activity, log input, log output, add timestamps)

### Automatic Login (Telnet & Serial)

Telnet and a serial console have no authentication protocol: the device prints a prompt and expects the account name and the password to be typed into the terminal. RustConn does that for you — the account name comes from the connection's **Username**, the password from its **Password Source**. Nothing is typed if neither is set.

Prompt wording differs wildly between vendors, so the expected text is configurable per connection or per group (Edit connection → **Automation** tab → **Automatic Login**):

| Field | Meaning |
|-------|---------|
| **Username Prompt** | Expected text before the account name is typed. Empty = built-in detection. |
| **Password Prompt** | Expected text before the password is typed. Empty = built-in detection. |

Both fields are matched as a **case-insensitive substring** of the line under the cursor — paste the literal wording the device prints, not a regex (regex belongs in Expect Rules).

The built-in detection already covers the common forms, so most devices need no configuration at all:

| Device | Prompt | Detected by default |
|--------|--------|---------------------|
| Huawei OLT MA5800 | `>>User name:` | yes |
| Huawei S6700 | `Username:` | yes |
| Datacom switches | `login:` | yes |
| Cisco / PuTTY-style | `login as:` | yes |
| Any of the above | `Password:` (and its translations) | yes |

Set the fields only when a device words its prompt differently, for example `Enter operator ID >`.

**Behaviour and limits:**

- Each step fires **once**. If the device rejects the credentials and prompts again, RustConn stops and hands the session over to you — automatic retries are how an account gets locked out.
- Watching stops after the terminal has been **idle** for 10 seconds — the clock restarts on every piece of output, so a device that spends a long time on its banner keeps the watcher alive instead of outliving it (changed in 0.21.2; the window used to run from the moment the session started, which meant a slow banner could outlast it). A hard ceiling of 2 minutes applies regardless, so a device printing a periodic heartbeat cannot keep the watcher running forever. The 10 seconds can be changed per connection (Edit connection → **Automation** → **Automatic Login** → *Login Timeout*) or per group (Edit group → **Automation** → *Login Timeout*); `0` there means "use the inherited value or the built-in default". The same value is stored as `login_timeout_secs` in the automation section of `connections.toml`.
- `Last login:` and `Last failed login:` in an MOTD are explicitly not treated as username prompts.
- The **Password Prompt** field also applies to SSH, but only to connections where RustConn types the password into the terminal. Since 0.21.2 most SSH connections instead hand the password to OpenSSH itself, which asks for it directly and never shows a prompt to match — see [How an SSH password reaches the server](#how-an-ssh-password-reaches-the-server). The field still applies to the SSH connections that fall back to the terminal watcher, listed there. The **Username Prompt** field is unused for SSH either way — the account name travels in the command line.
- Telnet transmits everything, including the password, **in clear text**. Automatic login does not change that; use SSH where you can.

> **Tip:** Set both fields on a *group* (Edit Group → **Automation** tab) to cover every device in a vendor folder at once. See [Group Automation](#group-automation-expect-rules--post-login-scripts).

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
| `Are you sure.*continue` | `yes` | SSH host key confirmation (first connection) |
| `Select option:` | `2` | Menu navigation |
| `Enter token:` | `${MFA_TOKEN}` | Use a global variable for MFA |

Rules execute in priority order. After matching, the response is sent followed by Enter.

**Built-in templates:** the **Test** dialog offers ready-made rule sets you can drop
in — Sudo Password, SSH Host Key Confirmation, Login Prompt, Press Enter to
Continue, and a pager (`--More--`) dismisser. The SSH Host Key Confirmation
template answers both prompt forms OpenSSH uses on a *first* connection: the
single-line `Are you sure you want to continue connecting (yes/no/[fingerprint])?`
and the separate follow-up line newer versions print,
`Please type 'yes', 'no' or the fingerprint:`. The Press Enter to Continue template
also matches the bare `any key to continue` banner that pagers and appliance
prompts emit without a leading "Press".

> **Note on changed host keys:** none of the built-in rules answer the
> `REMOTE HOST IDENTIFICATION HAS CHANGED` warning. A changed key is exactly what a
> man-in-the-middle attack looks like, so RustConn never auto-accepts it — resolve a
> legitimate key rotation by hand (`ssh-keygen -R <host>`) so the decision stays
> explicit.

**Variable Substitution in Responses:**

Expect rule responses support `${VARIABLE_NAME}` placeholders that are resolved at connection time. This lets you use dynamic values (passwords, tokens, environment-specific strings) without hardcoding them into rules.

Four placeholders are built in and come from the connection you are opening:

- `${password}` — the connection's password, read from the configured **Password Source** at connection time. Nothing is written into the rule, so the response stays safe to export or sync.
- `${username}` — the connection's Username, when it is set
- `${host}` — the connection's Host
- `${port}` — the connection's Port

Everything else resolves by name against your global variables (Menu → Tools → Variables), so `${MY_VARIABLE}` picks up the value of `MY_VARIABLE`. For the connection being opened a built-in wins over a global of the same name.

A set of **dynamic built-ins** is also available without defining anything, useful for tokens, banners or per-connection log lines:

| Variable | Expands to |
|----------|-----------|
| `${DATE_Y}` | Local year (`2026`) |
| `${DATE_M}` `${DATE_D}` | Local month / day, zero-padded |
| `${TIME_H}` `${TIME_M}` `${TIME_S}` | Local hour (24h) / minute / second, zero-padded |
| `${TIMESTAMP}` | Seconds since the Unix epoch |
| `${ENV_NAME}` | Value of the `NAME` environment variable (empty if unset) |

The environment form uses an underscore — `${ENV_HOME}`, not `${ENV:HOME}` — because a `${...}` reference cannot contain a colon. A variable *you* define with one of these names shadows the built-in, so defining `TIMESTAMP` yourself keeps your value. These resolve wherever `${...}` substitution runs (Expect responses, custom commands, and variable values), not in the [session-log Log Path](#session-logging) template, which has its own `${date}` / `${time}` set.

**Ask at connect time (`@ask:`).** Give a variable a value that starts with `@ask:` and RustConn asks you for it in a dialog every time a connection that references it opens, then substitutes what you type. This lets one connection serve several targets without duplicating it — the classic case is a host that changes:

| Variable value | What happens at connect |
|----------------|-------------------------|
| `@ask:Enter the ticket number` | A text field asks for the value |
| `@ask?:One-time code` | A hidden field (input not shown), for a token or PIN |
| `@ask:Select host\|prod.example\|staging.example` | A dropdown to pick one of the listed options |

Set a connection's Host to `${target}` and define a local variable `target` = `@ask:Select host|prod.example.com|staging.example.com`; on connect you choose the host from a dropdown. The prompt is collected from every field substituted at launch — Host, Username, an SSH startup command, a Custom Command, and enabled Expect responses — and all a connection's questions appear in one dialog. Answers apply to that connect only (the stored variable keeps its `@ask:` directive, so the next connect asks again) and are reused if the session reconnects. A hidden (`@ask?:`) answer is never written to disk. Cancelling the dialog cancels the connection.

Behaviour worth knowing:

- Undefined variables remain as literal text (e.g. `${UNKNOWN}` is sent as-is), and a warning naming them goes to the log. A `${password}` that stays literal means the connection has no Password Source configured.
- Unlike a Custom Command, the response is typed straight into the session rather than passed to a shell, so a password containing `$`, `!`, `;` or any other shell metacharacter is sent unchanged.
- A value containing a line break is rejected and the rule is skipped, because the newline would submit the answer before the rest of it was typed.
- Escape sequences in the rule (`\n`, `\t`, `\\`) are expanded before substitution, so a backslash inside a resolved password is left alone.
- A rule with **One-shot** enabled is discarded once it fires. A prompt that repeats — which is what a rejected password looks like — is handed back to you rather than answered again, since automatic retries are how an account gets locked out.

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
- **Recent history** — the last 15 Quick Connect sessions are remembered and persist across restarts (stored in settings; host, port, protocol and username only — never a password); shown as a "Recent" section with type-ahead filtering by host/username; clicking an entry fills protocol, host, port, and username fields instantly

### Connection Actions

| Action | Method |
|--------|--------|
| Connect | Double-click, Enter, or right-click → Connect (a double-click focuses an already-running session; Shift/Ctrl+double-click, or Settings → Interface → Connections → "Open a new session on every double-click", starts another one) |
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

### After the Computer Wakes from Sleep

Suspending the machine leaves every open connection half-open: the link is dead but neither side has said so. RustConn handles that on two levels.

**Kernel-level detection.** Embedded RDP and VNC sessions enable TCP keepalive on their socket — the first probe after 15 s of silence, then up to three probes 5 s apart — so a connection that did not survive the sleep is reported within about 30 s instead of hanging indefinitely. A 30 s `TCP_USER_TIMEOUT` covers the other half: without it, input you send to a dead session would be retransmitted by the kernel for 13 to 30 minutes before failing.

**Immediate feedback.** Waiting even 30 s in front of a picture that may be minutes old is misleading, so RustConn notices the resume itself. Every embedded RDP session that still reports itself as connected is **dimmed** and shown its reconnect banner, with a note that the connection may have been lost while the computer slept. The dimming is the honest signal: RustConn does not yet know whether the session is alive.

Nothing is disconnected on suspicion. A session that outlived a short sleep un-dims by itself as soon as its next frame arrives. Five seconds after the resume, sessions that have meanwhile been confirmed dead are reconnected — but only those, and only where auto-reconnect is enabled for the connection.

> **Note:** Reconnecting an RDP session starts a new logon; it does not resume the same Windows session, because the embedded client cannot present the server's auto-reconnect cookie. Whether your open windows come back is up to the server's own session policy.

### Network Change Monitor

RustConn monitors network interface changes via `gio::NetworkMonitor` and reacts immediately when a network switch occurs (e.g. WiFi → Ethernet, VPN reconnect, dock/undock):

1. **Stale socket cleanup** — all SSH `ControlMaster` sockets are closed (`ssh -O exit`) so new connections do not attempt to multiplex through dead master connections
2. **Toast notification** — "Network changed — cleaning stale connections" informs the user
3. **Auto-reconnect** — disconnected sessions with auto-reconnect enabled are reconnected without waiting for the backoff timer (3 s delay after socket cleanup to let background `ssh -O exit` complete)
4. **Embedded RDP/VNC** — embedded (non-VTE) sessions in disconnected/error state are reconnected via their native `reconnect()` method, not only through the VTE reconnect overlay

**Smart throttling:**

| Condition | Behavior |
|-----------|----------|
| Connectivity below `Full` (captive portal, limited) | Reconnect skipped; toast: "Network limited — full connectivity not yet available" |
| >3 network-change events within 60 seconds (VPN flapping) | Quiet mode: socket cleanup only, no toast spam or wasted reconnection attempts |

**SSH keepalive defaults (0.18.8+):**

All SSH sessions now apply `ServerAliveInterval=15` + `ServerAliveCountMax=3` by default (unless the user configures a custom value via Custom Options). Dead connections are detected within ~45 seconds instead of relying on TCP timeout (15+ minutes). This makes network-change detection faster: the SSH client notices the dead link promptly and exits, triggering the reconnect flow.

---

## Protocols

Protocol-specific options are configured in the connection dialog's protocol tab. This section covers each protocol's unique features and settings.

**Protocol Options Summary:**

| Protocol | Options |
|----------|---------|
| SSH | Auth method (password, publickey, keyboard-interactive, agent, security-key/FIDO2), key source (default/file/agent), PKCS#11 provider (hardware token/smart card), proxy jump (Jump Host), ProxyJump, IdentitiesOnly, ControlMaster, agent forwarding, Waypipe (Wayland forwarding), X11 forwarding, compression, startup command, verbose mode, backspace/delete key behavior, custom SSH options, port forwarding (local/remote/dynamic) |
| RDP | Client mode (embedded/external), external window sizing (fit to screen/fullscreen/custom resolution/all monitors), performance mode (quality/balanced/speed), resolution, color depth, display scale override, audio redirection, RDP gateway (host, port, username), keyboard layout, disable NLA, clipboard sharing, shared folders, mouse jiggler (prevent idle disconnect, configurable interval 10–600s), autotype (send text as keystrokes, configurable inter-character and initial delay), custom FreeRDP arguments |
| VNC | Client mode (embedded/external), performance mode (quality/balanced/speed), encoding (Auto/Tight/ZRLE/Hextile/Raw/CopyRect), compression level, quality level, display scale override, view-only mode, scaling, clipboard sharing, custom arguments |
| SPICE | TLS encryption, CA certificate (with inline validation), skip certificate verification, USB redirection, clipboard sharing, image compression (Auto/Off/GLZ/LZ/QUIC), proxy URL, shared folders |
| MOSH | Predict mode (Adaptive/Always/Never), SSH port, UDP port range, server binary path, backspace/delete key behavior, custom arguments |
| Telnet | Custom arguments, backspace key behavior, delete key behavior |
| Serial | Device path, baud rate, data bits, stop bits, parity, flow control, custom picocom arguments |
| Kubernetes | Kubeconfig path, context, namespace, pod, container, shell, busybox mode, busybox image, custom kubectl arguments |
| ZeroTrust | Provider-specific (AWS SSM, GCP IAP, Azure Bastion, Azure SSH, OCI Bastion, Cloudflare Access, Teleport, Tailscale SSH, HashiCorp Boundary, Hoop.dev, Generic Command), custom CLI arguments |
| Web | URL, browser mode (Embedded/System/Custom), credential autofill, JavaScript toggle, accept invalid TLS certs, zoom level, tunnel through an SSH connection (embedded, SOCKS) |

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

**Automatic SOCKS port.** Leave a Dynamic forward's local port at `0` and RustConn
picks a free local port each time you connect, shown in the summary as
`D auto (SOCKS)`. This avoids port clashes when the same connection is opened
twice, or when several tunnels run at once. The chosen port is written to the log
at connect time (`Assigned a random local SOCKS proxy port …`); point your browser
or a local command at `socks5://127.0.0.1:<that port>`. A fixed port still works —
set any non-zero value to keep the same SOCKS port every time.

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
| Connection Multiplexing | `ControlMaster=auto` | Reuse a single TCP connection for multiple SSH sessions to the same host. Subsequent connections open instantly without re-authenticating. RustConn adds `ControlPersist=60` so the master connection stays alive for 60 seconds after the last session closes. The shorter persist time (reduced from 10 minutes in 0.18.7) combined with proactive socket cleanup on network change prevents new connections from trying to multiplex through dead master sockets. When RustConn exits, all ControlMaster sockets are automatically closed to prevent stale sockets from lingering in the filesystem |
| Waypipe | `waypipe ssh ...` | Forward Wayland GUI applications (see [Waypipe](#waypipe-wayland-forwarding) below) |
| Verbose | `-v` | Show detailed SSH debug output in the terminal for diagnosing connection issues (auth failures, key negotiation, resets) |
| Multipath TCP | `mptcpize run` | Use multiple network paths simultaneously for seamless mobility (switch between Wi-Fi and Ethernet without dropping the connection) and bandwidth aggregation. See [Multipath TCP (MPTCP)](#multipath-tcp-mptcp) below |

**Configure:**
1. Edit an SSH connection → **Protocol** tab
2. Scroll to the **Session** group
3. Toggle the desired options
4. Click **Save**

All toggles are off by default. They can be combined freely — for example, enabling both Agent Forwarding and Compression at the same time adds `-A -C` to the SSH command.

#### Backspace and Delete Behavior

Network appliances and older Unix systems often disagree with the code RustConn sends when you press Backspace: instead of erasing, they echo `^?` or beep, and the remote side offers no way to fix it. The **Keyboard** group sets what each key sends for that connection.

| Setting | Sends | Use when |
|---------|-------|----------|
| Automatic | Backspace: `^?` (`0x7F`), Delete: `\e[3~` | Default — correct for Linux, BSD and macOS hosts |
| Backspace (^H) | `0x08` | The host expects `Ctrl+H` as its erase character (switches, routers, embedded systems) |
| Delete (^?) | `0x7F` | The host expects `Ctrl+?` — useful when assigned to the Delete key |

**Configure:**
1. Edit an SSH connection → **Protocol** tab
2. Scroll to the **Keyboard** group
3. Pick what **Backspace sends** and what **Delete sends**
4. Click **Save**

The setting belongs to the terminal, not to the `ssh` command, so it takes effect from the next connect onwards without changing the arguments RustConn passes to SSH. It then stays in force for the life of the session: reconnecting in place keeps it, and so does saving Settings → Terminal, which reinstalls the global key bindings over open terminals and used to drop the per-connection choice back to Automatic. Existing connections keep the previous behavior — Automatic.

Telnet and MOSH have the same **Keyboard** group in their own protocol tabs, since all three draw into the same terminal widget. SFTP does not: an SFTP connection opens a file manager or `mc` rather than a terminal, so there is nothing for the setting to apply to. An SFTP connection created from a converted SSH connection keeps whatever value it had stored, it is simply not shown or used.

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

**Note:** Since 0.18.8, `ServerAliveInterval=15` and `ServerAliveCountMax=3` are applied by default to all SSH sessions. You only need to set them in Custom Options if you want a *different* value (e.g. `ServerAliveInterval=60` for a slow satellite link). Setting either option explicitly in Custom Options overrides the default.

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

#### Multipath TCP (MPTCP)

Multipath TCP allows a single TCP connection to use multiple network paths simultaneously. This enables:

- **Seamless mobility** — switch between Wi-Fi and Ethernet (or dock/undock a laptop) without dropping the SSH session
- **Bandwidth aggregation** — combine multiple network interfaces for higher throughput

**System requirements:**

| Component | Minimum version |
|-----------|----------------|
| Linux kernel | 5.6+ with `CONFIG_MPTCP=y` (most modern distros ship with MPTCP enabled) |
| mptcpize (for SSH) | Part of `mptcpd` package (Fedora: `mptcpd`, Debian/Ubuntu: `mptcpize`, Arch: `mptcpd`) |
| Server | Must also support MPTCP (Linux 5.6+) |

**How to check availability:**

```bash
# Check if your kernel supports MPTCP
cat /proc/sys/net/mptcp/enabled
# "1" = available, "0" = disabled, file missing = not supported

# Enable MPTCP if disabled (requires root)
sudo sysctl net.mptcp.enabled=1
```

The connection dialog shows whether MPTCP is available in the toggle subtitle. If it reads "Not available: kernel MPTCP is disabled", enable it via the sysctl above or check that your kernel is compiled with `CONFIG_MPTCP=y`.

**Setup:**

1. Edit a connection (SSH, RDP, or VNC) → **Protocol** tab
2. In the **Session** (SSH) or **Features** (RDP/VNC) group, enable **Multipath TCP**
3. Save and connect

**Behavior:**

- **SSH**: Wraps the SSH command with `mptcpize run`, which forces all TCP sockets created by the process to use MPTCP. If the server does not support MPTCP, the connection falls back to regular TCP transparently — no error, no user action needed. If `mptcpize` is not installed, the SSH command runs without MPTCP wrapping (a warning is shown in the connection dialog).
- **RDP/VNC (embedded mode)**: Creates an MPTCP socket directly via `socket2`. Falls back to a regular TCP socket if the kernel does not support the MPTCP protocol number. External viewers (FreeRDP, TigerVNC) handle their own sockets and ignore this setting.

**CLI:**

```bash
# Create an SSH connection with MPTCP enabled
rustconn-cli add --name "server" --host example.com --protocol ssh --mptcp

# Enable MPTCP on an existing connection
rustconn-cli update "server" --mptcp

# Disable MPTCP on an existing connection
rustconn-cli update "server" --mptcp false
```

**Status indicator:**

When MPTCP is enabled on an embedded RDP or VNC session, the toolbar status label shows "MPTCP" alongside other connection info (e.g., "RTT: 12 ms | GFX + H.264 | MPTCP" for RDP). This confirms the feature is active for the current session.

**SSH config import/export:**

- The MPTCP setting is stored in RustConn's own JSON config only. There is no standard `ssh_config` directive for MPTCP, so it is neither imported from nor exported to `~/.ssh/config`.

**Known limitations:**

- MPTCP is negotiated during the TCP handshake — both client **and** server must support it. If either side does not, the connection uses regular TCP (no error).
- `mptcpize` must be installed for SSH MPTCP to work. Install via your distribution's package manager (package name is typically `mptcpd` or `mptcpize`).
- MPTCP does not help if both interfaces route through the same path (e.g., two Wi-Fi adapters on the same access point).
- The connection dialog cannot verify whether the remote server supports MPTCP — only local kernel availability is checked.

### RDP

#### Session Toolbar

The embedded RDP session provides a floating toolbar with actions like Copy, Paste, Autotype, Ctrl+Alt+Del, Quick Actions, and Scripts. The toolbar auto-hides to keep the full remote desktop visible:

- **Reveal** — hover or click the small arrow indicator (▼) at the top center of the remote desktop view
- **Hide** — the toolbar hides automatically after 2 seconds of inactivity (pointer and keyboard focus have left)
- **Keyboard access** — Tab into the arrow indicator, then press Enter to show the toolbar; focus moves into the toolbar actions
- **Touch** — tap the arrow indicator to toggle the toolbar

The toolbar is fully transparent and pass-through when hidden — it does not block interaction with the remote desktop or window controls. In narrow panels (split view or small windows), secondary actions collapse into an overflow "⋯" menu while Fit Resolution and Ctrl+Alt+Del stay directly visible.

**Turning the toolbar off completely:** switch off **Session Toolbar** in the connection dialog's Features section. The toolbar and its arrow indicator are then absent for that connection — there is nothing left to reveal and nothing to hit by accident, which matters when the window behind the session has controls of its own near the top edge. In a split pane the panel's own corner buttons are suppressed for that session too (see [Split View](#split-view)).

Know what goes with it. **Ctrl+Alt+Del has no keyboard route of its own**, and neither do Fit Resolution, Autotype, Scripts, Quick Actions or Save Files — all of those live only on the toolbar. Copy and Paste survive, because Ctrl+C / Ctrl+V reach the remote session directly, as does clipboard sync. Leave the toolbar on if you ever need to unlock a Windows login screen from inside the session.

The setting is per connection and defaults to on, so existing connections are unaffected. Quick Connect, which has no saved profile, always keeps the toolbar. It can also be set from the CLI for VNC and Web connections (`--vnc-toolbar false`, `--web-toolbar false`); the RDP toolbar has no CLI flag, so use the connection editor for it. RDP's only CLI protocol options are `--rdp-display-mode` and `--rdp-resolution`.

#### Mouse Jiggler

Keeps the remote RDP session awake and prevents the remote desktop from locking by sending periodic input.

- Configure in Connection Dialog → RDP → Features: enable **Mouse Jiggler** and set the interval (10–600 seconds, default 60)
- Auto-starts when the RDP session connects, auto-stops on disconnect
- Each tick sends a small mouse movement (keeps the session from idle-disconnecting) **and** a no-op Scroll Lock keystroke. The keystroke is required because Windows does not reset its workstation lock / screensaver timer on RDP-injected mouse motion alone
- **Embedded (IronRDP) mode only.** The External FreeRDP client runs as a separate process with no input channel from RustConn, so the jiggler cannot drive it — switch to Embedded mode if you need this feature

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

#### External Window Sizing

*New in 0.20.9.* When an RDP session opens in a separate FreeRDP window, its size comes from **External Window** in the connection dialog's Display group:

| External Window | Behaviour |
|-----------------|-----------|
| **Fit to screen** *(default)* | A decorated window covering the monitor (`/size:100%`). |
| **Fullscreen** | Takes over the monitor completely (`/f`). |
| **Custom resolution** | Uses the **Resolution** row below it (`/w:` + `/h:`). That row is only shown for this mode. |
| **All monitors** | Spans every connected monitor (`/multimon`). |

The setting is shown for both client modes, not only External, because an embedded session hands over to the external client whenever the built-in IronRDP client cannot serve the server — a legacy security layer, RemoteApp, remote audio playback, or a failed connection. In that case this is the only setting that decides how large the window opens.

Before 0.20.9 there was no such setting. A separate window was sized from a fixed resolution that the editor collected in a hidden row and defaulted to 1920×1080, so on a 4K display the client opened at roughly a quarter of the screen. **A connection that genuinely needs a fixed resolution has to select *Custom resolution* once** — a stored 1920×1080 cannot be told apart from that old default, so it is not assumed.

From the CLI: `--rdp-display-mode fit|fullscreen|custom|multimon` and `--rdp-resolution WIDTHxHEIGHT`.

#### HiDPI Support

On HiDPI/4K displays the embedded RDP/VNC session's remote resolution is governed by the **Display Scale** setting in the connection dialog:

| Display Scale | Behaviour |
|---------------|-----------|
| **Auto (system)** *(default)* | Requests the widget's *logical* resolution and upscales the framebuffer locally. Uses the least bandwidth; the remote UI is comfortably sized. Best over slow links. |
| **Native (full HiDPI)** | Follows the display's live scale factor, so a 2× screen requests a full-resolution ("retina") remote desktop for a crisp image. Adapts automatically if the window moves to a monitor with a different scale. Uses more bandwidth. |
| **125% – 400%** | Requests a fixed multiple of the logical resolution — a sharper image at a scale you pick by hand, regardless of the monitor. |

For embedded RDP, the chosen scale is also sent to the Windows server as its desktop DPI (MS-RDPEDISP), so remote UI elements render at the correct logical size, and it is re-applied on every dynamic resize.

*Changed in 0.20.9:* the external FreeRDP client receives the same scale, as `/scale-desktop:` with a matching `/scale-device:`. It used to be embedded-only, and the row was hidden whenever the client mode was External — so a session in a separate window on a 4K display had no way to ask for legible text. The row is now shown for both client modes.

When the available area is smaller than the minimum remote desktop resolution (640×480) — for example a very small split panel or a narrow window — the embedded viewer requests an aspect-matched resolution scaled up to that minimum, raises the remote scale so content stays legible, and locally downscales the frame to fully fill the area. The result is that a small or oddly-shaped area is filled without letterboxing rather than clipping the remote view.

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

#### External FreeRDP Keyboard Shortcuts (Right Shift hotkeys)

When you use **External** RDP mode, RustConn launches the FreeRDP SDL client (`sdl-freerdp3` / `sdl-freerdp`). That client has its own built-in shortcuts that use **Right Shift** as the modifier by default. These are handled entirely inside FreeRDP — RustConn does not intercept them:

| Shortcut | Action |
|----------|--------|
| Right Shift + Enter | Toggle fullscreen |
| Right Shift + R | Toggle window resizable |
| Right Shift + G | Toggle keyboard/mouse grab (release input back to the local system) |
| Right Shift + D | **Disconnect the session** |
| Right Shift + M | Minimize the window |

If you press **Right Shift + D** by accident the session closes immediately. The grab toggle is **Right Shift + G** (not "Win+Esc").

**Configuring or disabling the hotkeys**

FreeRDP reads its shortcut configuration from `$XDG_CONFIG_HOME/freerdp/sdl-freerdp.json`. Because the bundled FreeRDP runs **inside the RustConn sandbox**, the path is *not* the one used by the standalone `com.freerdp.FreeRDP` Flatpak. Use the location for your install type:

| Install | Config file path |
|---------|------------------|
| Flatpak | `~/.var/app/io.github.totoshko88.RustConn/config/freerdp/sdl-freerdp.json` |
| System / native | `~/.config/freerdp/sdl-freerdp.json` |

Note: `/etc/FreeRDP/sdl-freerdp.json` is read from *inside* the sandbox filesystem for the Flatpak build, so a file placed in the host's `/etc/FreeRDP/` is **not** visible to it — use the per-user path above.

Disable **all** hotkeys (the safest option if you only need a single release/grab key):

```json
{
  "SDL_KeyModMask": ["KMOD_NONE"]
}
```

Or keep the modifier but move just the disconnect action onto a key you will never press by accident, while leaving grab on Right Shift + G:

```json
{
  "SDL_KeyModMask": ["KMOD_RSHIFT"],
  "SDL_Disconnect": "SDL_SCANCODE_F24"
}
```

Recognised keys (with their defaults): `SDL_KeyModMask` (`KMOD_RSHIFT`), `SDL_Fullscreen` (`SDL_SCANCODE_RETURN`), `SDL_Resizeable` (`SDL_SCANCODE_R`), `SDL_Grab` (`SDL_SCANCODE_G`), `SDL_Disconnect` (`SDL_SCANCODE_D`), `SDL_Minimize` (`SDL_SCANCODE_M`). Modifier names come from [SDL_Keymod](https://wiki.libsdl.org/SDL3/SDL_Keymod) and key names from [SDL_Scancode](https://wiki.libsdl.org/SDL3/SDL_Scancode).

**Quick setup (Flatpak):**

```bash
mkdir -p ~/.var/app/io.github.totoshko88.RustConn/config/freerdp
cat > ~/.var/app/io.github.totoshko88.RustConn/config/freerdp/sdl-freerdp.json <<'EOF'
{
  "SDL_KeyModMask": ["KMOD_NONE"]
}
EOF
```

Create the `freerdp` directory first if it does not exist, make sure the JSON is valid (an invalid file is silently ignored), and reconnect — the file is read each time a FreeRDP process starts, so no RustConn restart is needed.

**Verify it is being read:** start RustConn with `RUST_LOG=debug` (Flatpak: `flatpak run io.github.totoshko88.RustConn` from a terminal with `RUST_LOG=debug` set) and connect. The captured FreeRDP `stderr` is forwarded to the log; pressing the modifier + a hotkey logs a line such as `<KMOD_RSHIFT>+<...> pressed`. If hotkeys are disabled, no such line appears.

### VNC

VNC connections support embedded (vnc-rs) or external (TigerVNC) client modes. Configure encoding (Auto/Tight/ZRLE/Hextile/Raw/CopyRect), compression level, quality level, display scale override, view-only mode, scaling, and clipboard sharing in the VNC protocol tab.

An embedded VNC session carries the same floating toolbar as RDP — Copy, Paste and Ctrl+Alt+Del behind an arrow indicator at the top centre — with the same **Session Toolbar** switch in the Features section to remove it entirely. See [Session Toolbar](#session-toolbar) under RDP for how the reveal works and what becomes unreachable when it is off.

### SPICE

SPICE connections support TLS encryption, CA certificate validation, USB redirection, clipboard sharing, image compression (Auto/Off/GLZ/LZ/QUIC), proxy URL, and shared folders. SPICE opens in an external viewer (remote-viewer / virt-viewer).

**Password (fixed in 0.21.2):**

The connection's **Password Source** now reaches the viewer, so a SPICE session with a stored password connects without asking for it. Before 0.21.2 the password was resolved and then discarded at launch, and `remote-viewer` prompted every time whatever the Password Source was set to — changing it made no difference, which is what made the behaviour confusing rather than merely missing.

The secret is passed in a virt-viewer `.vv` connection file that only your account can read, never on the command line where other processes could see it. The file carries the flag that tells the viewer to delete it after reading.

Two cases still prompt in the viewer, by design: a connection with no stored password, and **Unix Socket Mode** below — the `.vv` format has no field for a socket path, and a local socket needs no password in practice.

**Unix Socket Mode:**

For local VMs managed by libvirt/QEMU, you can connect directly via a unix socket instead of host:port. Enable the "Unix Socket" toggle in the SPICE tab and provide the socket path (e.g. `/run/libvirt/qemu/vm-spice.sock`). The viewer uses `spice+unix://` URI. Jump host is not available in socket mode.

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
| Backspace sends / Delete sends | What each key sends to the remote — see [Backspace and Delete Behavior](#backspace-and-delete-behavior) | Automatic |
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

On Linux, RustConn tries file managers in this order: `dolphin` (KDE), `nautilus` (GNOME), `xdg-open` (fallback). The `SSH_AUTH_SOCK` environment variable is injected into the spawned process so the file manager can access the SSH agent.

On KDE, if `dolphin` is not found (e.g., in Flatpak), `xdg-open` is used — which opens whichever application is registered as the `sftp://` handler. See [SFTP Troubleshooting](#sftp-troubleshooting) if the wrong application opens.

On macOS there is no `xdg-open`, `dolphin` or `nautilus`; RustConn opens locations through `open(1)`, which hands the `sftp://` URI to the default handler registered with Launch Services. The "SFTP via Midnight Commander" mode below is the recommended way to browse SFTP on a Mac, since it does not depend on a `sftp://` handler being installed.

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

#### Open Browser via Tunnel

Right-click an SSH connection and choose **Open Browser via Tunnel** to reach web
services that only that SSH host can see — an internal dashboard, a router admin
page, a service in a DMZ — without configuring a separate Web connection. RustConn
raises a dynamic SOCKS proxy (`ssh -N -D`) to the host on a free local port and
opens a browser on a start page routed through it; every request exits from the
SSH host. This is RustConn's equivalent of Ásbrú's incognito browser.

Where it opens is a global setting — Settings → Interface → Connections → **Open
tunnelled browser in the embedded browser**:

- **On** (default) — the in-app WebKit browser opens in a tab, and the tunnel
  stays up only as long as that tab. Available in builds that ship the embedded
  browser.
- **Off**, or a build without the embedded browser — an external Chromium-based
  browser (Chromium, Chrome, Brave, Vivaldi, Edge) launches with `--proxy-server`
  in an isolated throwaway profile, so a browser you already have open cannot
  hijack the launch and drop the proxy. Which browser, and the fallbacks, are
  described under **External browser command** below. The tunnel is kept alive
  behind the browser and released when RustConn exits.

The **Tunnelled browser start page** entry just below that switch sets where the
browser opens (embedded and external alike). It defaults to
`https://www.google.com` so it is immediately visible the tunnel works — a blank
page would look dead, and the embedded browser rejects a non-web URL anyway. The
presets button beside the entry fills it with one of the common choices — Google,
DuckDuckGo, or `ifconfig.me` (which shows the exit IP, a quick way to confirm the
page really left through the SSH host) — or **Custom** to type your own. Any URL
works; a value without a scheme is treated as `https://`.

For the external (non-embedded) path, **External browser command (tunnelled)**
lets you name the browser to use, the same way a Web connection's **Custom
Browser** command does. Set a command or path — for example `chromium`,
`brave-browser`, or `/opt/foo/chrome`, optionally with extra arguments before the
proxy flags. It must be a Chromium-family browser, since only those take a
command-line `--proxy-server`; a non-Chromium or missing command is reported as a
toast rather than opening un-tunnelled.

Leave the command empty to auto-detect, in this order: your system default
browser (via `xdg-settings`) when it is Chromium-family, then `$BROWSER`, then the
first Chromium binary found on `PATH`. So if your default browser is Chrome or
Brave, that is what opens. When the default is Firefox or another browser that
cannot take a command-line proxy, RustConn uses an installed Chromium binary
instead; if none exists, it opens the embedded browser (which can always tunnel)
and shows a toast suggesting you install a Chromium-based browser.

If the tunnel cannot be raised, RustConn shows a toast and opens nothing — it
never falls back to browsing directly, so traffic meant for the tunnel does not
leak. Firefox is not usable for the external mode because it takes its proxy from
a profile rather than the command line; when it is your default, RustConn uses an
installed Chromium browser or the embedded browser instead, as described above.

#### SFTP Troubleshooting

**Choosing the Default SFTP Client (Linux — KDE / GNOME / other):**

This section is Linux-only. On macOS see the `open(1)` / Midnight Commander note above.

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

Web connections store website URLs and can open them in three browser modes depending on platform and configuration.

**Use cases:**
- Quick access to web-based admin panels (AWS Console, Grafana, Proxmox, etc.)
- Storing credentials for web services alongside SSH/RDP connections
- Organizing all infrastructure access points in one place

**Browser Modes:**

| Mode | Description | Default |
|------|-------------|---------|
| **Embedded** | Built-in WebKitGTK 6.0 browser in a notebook tab (Linux only) | Yes (Linux) |
| **System** | Opens URL in the default system browser via GTK4 `UriLauncher` (portal-aware, works in Flatpak) | Fallback / other platforms |
| **Custom** | Opens URL with a user-defined browser command | Manual |

Configure the browser mode in the connection dialog's protocol tab.

**Creating a Web connection:**
1. Click "New Connection" → select **Web** protocol
2. Enter the full URL in the **URL** field (must start with `http://`, `https://`, or `file://`)
3. Select browser mode (Embedded, System, or Custom)
4. Optionally set username/password — these are stored in the configured secret backend for credential autofill (embedded mode) or copy-to-clipboard (system/custom mode)
5. Click Save

**Connecting:**
- Double-click the connection in the sidebar → opens in the configured browser mode
- In embedded mode, a browser tab appears in the session area with a navigation toolbar
- In system/custom mode, the URL opens externally and the sidebar status briefly shows "connecting" (yellow) then clears

#### Embedded Browser (WebKitGTK 6.0)

The embedded browser provides a full browsing experience inside a RustConn tab on Linux.

**Navigation Toolbar:**
- **Back / Forward / Reload / Home** buttons for standard navigation
- **URL address bar** — shows the current page URL; click to edit, type a new URL and press Enter to navigate (auto-prepends `https://` for bare hostnames); page title shown as tooltip on hover
- **Ctrl+L** — copies the current URL to the clipboard
- **Autofill** button — injects stored credentials into login forms; shows a toast if no form fields are detected
- **Zoom In / Zoom Out** buttons with dynamic tooltip showing current zoom percentage (e.g. "Zoom in (120%)")
- **Menu (⋯)** button with: Copy URL, Open in System Browser, Zoom Reset (100%), Clear Session Data

**Floating Toolbar:**
The toolbar floats over the page rather than taking a strip of its own, so the page gets the full height of the tab. It appears briefly when the page connects and then hides after 2 seconds of inactivity; hover or click the arrow indicator at the top centre to bring it back — the same place RDP and VNC put theirs. The trailing corner would sit better over a web page, whose logo and navigation run across the top centre, but that corner already belongs to the split view's own panel arrow, which covered the toolbar indicator completely in a split pane. If the indicator is in your way on a particular site, switch the toolbar off for that connection (below).

To remove it entirely, switch off **Navigation Toolbar** in the connection dialog's Embedded Browser Settings. The address bar goes with it, but the keyboard routes do not: Alt+← / Alt+→ still navigate, Ctrl+R reloads, Ctrl+L copies the current URL, and Ctrl+Plus / Ctrl+Minus / Ctrl+0 still zoom. The setting applies to embedded mode only — System and Custom browser modes hand the URL to another program, which has its own chrome.

**Responsive Toolbar:**
When the panel is too narrow for the assembled toolbar, secondary actions (Home, Autofill, Zoom In, Zoom Out) move into the "⋯" menu button instead of being clipped, so every action stays reachable at any width. Back, Forward, Reload, the URL bar and the menu itself stay in place. The threshold is measured from the toolbar's own requested width rather than being a fixed pixel count, so it follows your theme's button metrics.

**Loading Progress Bar:**
A thin progress bar appears under the toolbar during page loads, showing real-time load progress. It disappears automatically when the page finishes loading.

**Zoom Controls:**
- **Ctrl+Plus** — zoom in
- **Ctrl+Minus** — zoom out
- **Ctrl+0** — reset zoom to 100%
- Zoom range: 30% to 300%
- **Auto-fit zoom** — when the WebView is narrower than 1024px (split view, small window), zoom is automatically reduced proportionally to prevent horizontal overflow; always enabled by default
- **Zoom persistence** — the zoom level is saved to the connection config (debounced 2 seconds) and restored when the connection is reopened

**Credential Autofill:**
When a connection has stored credentials (username/password), the embedded browser can fill login forms:
- **Autofill button** — click to inject credentials via JavaScript into username/email and password fields; dispatches `input` and `change` events for framework compatibility
- **HTTP Basic Auth** — responds to HTTP 401/Digest challenges automatically using stored credentials
- Shows an informational toast "No login form fields found on this page" if no fields are detected after 3 seconds

**Reconnect Banner:**
When a page fails to load *after* it has already shown content — a broken link you followed, or a later navigation error — a banner appears below the toolbar with the error description and a "Reload" button. Clicking Reload navigates back to the configured URL.

If the **very first** load never paints (a bad host, an unreachable address, a DNS failure, or a SOCKS-tunnel `Name or service not known`), RustConn does not leave a blank tab: it closes the tab and shows the reason as a toast (`Could not open the page: …`). Only the initial load is treated this way, so a working session is never closed by a later failed navigation.

**Persistent Sessions:**
Cookies and session data persist across RustConn restarts, so you stay logged in to web services between sessions. Each connection has isolated storage — no cross-connection data leakage.

**Per-connection JavaScript Toggle:**
Disable JavaScript execution for specific connections in the connection dialog's protocol tab. Useful for lightweight pages or security-sensitive bookmarks.

**Accept Invalid TLS Certificates:**
Enable "Accept Invalid Certs" in the connection dialog's protocol tab to allow self-signed, expired, or hostname-mismatched certificates. Useful for local services like Cockpit, Proxmox, or development environments that use self-signed certificates. This setting applies only to the embedded browser mode.

**Tunnel Through SSH (SOCKS proxy):**
To reach a web service that only a particular SSH host can see — an internal
dashboard, a router admin page, a service in a DMZ — set **Tunnel Through SSH**
to one of your SSH connections. When the Web connection opens, RustConn raises a
dynamic SOCKS proxy over that SSH host on an automatically-chosen free local port
and routes the browser through it, so the page loads as if you were browsing from
the SSH host. The chosen connection's key, jump-host chain and (session-cached)
password are reused, so no extra credentials are needed. If the tunnel cannot be
opened the connection does not fall back to a direct request — it fails with a
message — so traffic you meant to tunnel never leaks.

Which browser modes honour it:

- **Embedded** — fully supported. The tunnel is applied to the browser's network
  session and stays up only while the browser tab is open, coming down with it.
- **Custom**, when the command is a Chromium-based browser (Chromium, Chrome,
  Brave, Vivaldi, Edge, Opera) — supported via `--proxy-server`. The tunnel is
  kept alive behind the launched browser and released when RustConn exits.
- **Custom** with Firefox, or **System** (portal) — *not* tunnelled: Firefox
  takes its proxy from a profile rather than the command line, and the system
  browser is chosen by the desktop portal, which RustConn cannot hand a proxy to.
  In both cases a toast tells you the page opened without the tunnel; use the
  embedded or a Chromium-based Custom browser to tunnel.

**Downloads:**
Files are automatically saved to `~/Downloads/`. A toast notification "Downloaded: {filename}" appears when a download completes.

**Open in System Browser:**
Available via the "⋯" menu → "Open in System Browser" — opens the current page URL in your default system browser using `UriLauncher` (portal-aware, works in Flatpak).

**Keyboard Shortcuts (Embedded Mode):**

| Shortcut | Action |
|----------|--------|
| Ctrl+L | Copy current URL to clipboard |
| Ctrl+Plus | Zoom in |
| Ctrl+Minus | Zoom out |
| Ctrl+0 | Reset zoom |
| Enter (in URL bar) | Navigate to entered URL |

**Known Limitations:**
- WebKitGTK does not support WebCodecs (needed for Selkies/WebRTC streaming)
- No DRM/EME content (Widevine not available in WebKitGTK)
- Embedded mode is Linux-only (requires WebKitGTK 6.0)
- Not available in the snap package: the core24 GNOME platform provides no WebKitGTK 6.0, and bundling it is still pending (issue [#244](https://github.com/totoshko88/RustConn/issues/244)). Web connections there use System or Custom mode; a connection saved with Embedded mode falls back to the system browser.

**Context menu actions:**
- **Copy Username** / **Copy Password** — copies stored credentials to clipboard (auto-clears after 30 seconds)

**CLI:**
```bash
rustconn-cli add --name "AWS Console" --protocol web --host "https://console.aws.amazon.com"
rustconn-cli add --name "Proxmox" --protocol web --host "https://pve.local:8006" --accept-invalid-certs
rustconn-cli add --name "Docs" --protocol web --host "file:///usr/share/doc/index.html" --javascript false
rustconn-cli add --name "Grafana" --protocol web --host "https://grafana.internal" --browser-mode system
```

---

## Sessions & Terminal

### Session Types

| Protocol | Session Type |
|----------|--------------|
| SSH | Embedded VTE terminal tab |
| RDP | Embedded IronRDP or external FreeRDP (bundled in Flatpak) |
| VNC | Embedded vnc-rs or external TigerVNC |
| SPICE | External viewer (remote-viewer / virt-viewer) |
| MOSH | MOSH via VTE terminal (external `mosh` client) |
| Telnet | Embedded VTE terminal tab (external `telnet` client) |
| Serial | Embedded VTE terminal tab (external `picocom` client) |
| Kubernetes | Embedded VTE terminal tab (external `kubectl exec`) |
| ZeroTrust | Provider CLI in terminal |
| Web | Embedded WebKitGTK tab (Linux) or system/custom browser (external) |
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

**External-session tracking:** an external-viewer session gets no notebook tab. Instead it is surfaced in the sidebar with a window emblem next to the connected status, and its right-click menu adds **Disconnect** (closes a RustConn-owned viewer such as TigerVNC/FreeRDP/remote-viewer) and **Stop tracking** (deregisters without closing the viewer). Owned viewers are closed automatically when you quit RustConn. Detaching viewers that RustConn cannot control (Remmina, KRDC, Vinagre) keep running independently: if you close such a viewer window yourself, the sidebar keeps showing the session as connected until you select **Stop tracking**. A double-click on a connection that already runs in an external window shows an "Already running in an external window" hint instead of opening a duplicate — use **Open new session** to force a second one.

### Tab Management

- **Switch** — Click tab or Ctrl+Tab / Ctrl+Shift+Tab
- **Close** — Click X or Ctrl+W / Ctrl+Shift+W
- **Reorder** — Drag tabs
- **Tab Overview** — Click the grid icon (▦) at the right end of the tab bar, or press **Ctrl+Shift+O**, to open a full-screen grid view of all open tabs. Useful when you have many tabs open and need to visually locate a session. Click any thumbnail to switch to it.
- **Tab Switcher** — Press **Ctrl+%** (or open Command Palette with **Ctrl+P** and type `%`) to fuzzy-search across all open tabs by name. Results show protocol type and tab group. Select and press Enter to switch instantly.
- **Pin Tab** — Right-click a tab → **Pin Tab**. Pinned tabs stay at the left edge of the tab bar and are never scrolled out of view. Useful for long-running sessions you need constant access to. Right-click again → **Unpin Tab** to restore normal behavior.

### Split View

Split view works with **any in-process (embedded) session**, not just VTE terminals. That means every terminal-based session (SSH, Telnet, Serial, Kubernetes, Local Shell, and SFTP in mc mode) as well as embedded RDP and VNC remote desktops. The only sessions that cannot be split are those handed off to an external viewer process (see below) — which includes every SPICE session, since SPICE has no embedded client. You can mix terminals and embedded remote desktops in the same split, and an embedded session keeps its live connection when it moves between panels.

Sessions shown through an external viewer (xfreerdp, vncviewer, or an external SPICE viewer) cannot be placed in a split — attempting to split one shows "Split view is not available for external-viewer sessions. Switch this connection to embedded mode to use split." and leaves the layout unchanged.

- **Horizontal Split** — Ctrl+Shift+H splits the current tab horizontally (side by side)
- **Vertical Split** — Ctrl+Shift+S splits the current tab vertically (top and bottom)
- **Close Pane** — Ctrl+Shift+X closes the focused pane; if only one pane remains, the split is dissolved and the session returns to normal tab mode
- **Focus Next Pane** — Ctrl+` cycles focus between panes
- **Select Tab** — click the "Select Tab..." button in an empty pane to pick which session to display; sessions already in other split views show a colored indicator
- **Move between splits** — a session can be moved from one split to another via "Select Tab"; the original split keeps a placeholder in the vacated panel, and the session's own tab shows a "Displayed in Split View" page with a "Go to Split View" button
- **Tab Overview** — split-view tabs render correctly in Tab Overview (Ctrl+Shift+O) with live thumbnails showing the split layout

Embedded viewers adapt to narrow panels: the toolbar collapses its secondary actions into an overflow ("⋯") menu (Fit resolution and Ctrl+Alt+Del stay visible), and the remote desktop rescales to fully fill a small or oddly-shaped panel. The same adaptation applies to a single embedded tab in a small or narrow application window. Keystroke broadcast (Ctrl+Shift+B) applies only to terminals — its toggle appears when a split holds at least two terminal sessions and a terminal panel is focused, and mirroring never targets an embedded remote desktop.

Each occupied panel has a small arrow indicator (◂) at the top-right corner. Hovering or clicking it reveals the panel action buttons: **Remove from Split** (returns the session to its own tab without closing it) and **Close** (terminates the session). The buttons hide automatically after a short delay, keeping the view uncluttered and avoiding overlap with the session toolbar.

A session whose **Session Toolbar** / **Navigation Toolbar** is switched off still shows the split-view panel corner buttons (detach/close) — those are part of the split infrastructure, not the session toolbar. Right-clicking the panel also offers **Remove from Split**, **Remove Split** and **Close Connection**.

**Connection name labels** — enable **Settings ▸ Interface ▸ Window ▸ Show connection name in split panes** to display a compact colored header at the top of each pane showing the connection name and protocol. This makes it easy to identify which pane belongs to which connection at a glance, especially useful with 3 or more panes side by side. The header color matches the panel's indicator color. In compact mode, the header shrinks automatically.

### Detached Session Windows

A session can be moved out of the main window into a window of its own — useful for keeping one session on a second monitor while you keep working in the main window. Moving a session never reconnects it: the same live widget is handed over, so scrollback, monitoring, recording, highlight rules, and SSH tunnels continue uninterrupted.

- **Detach** — Right-click a tab → **Move to New Window**, or press **Ctrl+Shift+M**. The tab leaves the main window and the session appears in its own window, focused and ready for input.
- **Attach** — Click the restore button in the detached window's header bar (**Move to Main Window**), or press **Ctrl+Shift+M** again while that window has focus. The session returns to a tab with the same title, icon, tab group, and color it had before, and the now-empty window closes.
- **Choose a monitor** — With two or more monitors connected, the tab menu also offers **Move to New Window on…** with one entry per monitor (for example "Monitor 2 (DP-1)"). The window opens fullscreen on the chosen monitor, because Wayland does not let an application place a window at specific coordinates. With a single monitor the submenu is not shown.

**Supported sessions:** everything RustConn renders itself — terminal sessions (SSH, Local Shell, Telnet, Serial, Kubernetes, MOSH, Zero Trust, and SFTP in mc mode) plus embedded RDP, embedded VNC, and embedded Web.

**Not offered for:** sessions handed to an external viewer (SPICE, or RDP/VNC in External Window mode), which already run in their own operating-system window, and split tabs — remove the split first (activating the item on a split tab explains this).

A detached window behaves like the main window: copy, paste, terminal search, close, and fullscreen (F11) act on the session in that window and never on a session in the main window, the keyboard passthrough toggle works from there as well, and toasts raised by that session appear in it. Compact-interface and layout-independent accelerator handling apply the same way as in the main window.

**Lifecycle:**
- **Closing a detached window ends its session** — the same teardown as closing its tab: the child process is stopped, SSH tunnels are dropped, and the sidebar status and history entry are updated. Ctrl+W inside a detached window closes only that session.
- Closing or quitting the main window closes detached windows with it, and the "Close RustConn?" confirmation counts detached sessions along with open tabs.
- Minimize-to-tray leaves detached windows and their sessions running.
- Activating a detached session in the sidebar presents its window instead of starting a second session.
- If a detached session ends on its own (remote disconnect, process exit), its window closes and the sidebar and history update exactly as they would for a tab.

Detached sessions are not listed in Tab Overview, the Tab Switcher, or the split-view session picker, since they have no tab; the sidebar keeps showing them as connected.

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

Logging is armed by either of two switches, and the per-connection one wins when
both are set:

- **Globally** — Settings → Terminal page → Logging → "Persist logs". Applies to
  every session and writes into the log directory configured next to it.
- **Per connection** — Connection dialog → Logs tab → "Enable Logging". Applies
  to that connection only, with its own path, timestamp format, size limit and
  retention.

Three logging modes:
- **Activity** — Track session activity changes
- **User Input** — Capture typed commands
- **Terminal Output** — Full transcript

Optional timestamps:
- Enable "Timestamps" to prepend `[HH:MM:SS]` to each line in log files

Per-connection logging options (Connection dialog → Logs tab → Content Options):
- **Log Activity** — Record connection and disconnection events
- **Log Input** — Record keyboard input sent to remote
- **Log Output** — Record terminal output from remote
- **Add Timestamps** — Prepend timestamp to each log line

#### Where the log files go

The **Log Directory** in Settings is the default destination. A relative value is
resolved against the configuration directory, so the out-of-the-box location is
`~/.config/rustconn/logs/`. In Flatpak that is
`~/.var/app/io.github.totoshko88.RustConn/config/rustconn/logs/`.

The per-connection **Log Path** is a template. A relative template lands in the
same log directory; an absolute one is used as written. These variables are
expanded:

| Variable | Expands to |
|----------|-----------|
| `${connection_name}` | Connection name, sanitized for use in a file name |
| `${protocol}` | Protocol id (`ssh`, `rdp`, ...) |
| `${date}` | `YYYY-MM-DD` |
| `${time}` | `HH-MM-SS` |
| `${datetime}` | `YYYY-MM-DD_HH-MM-SS` |
| `${HOME}` or a leading `~` | Home directory |

An unknown `${variable}` is an error: the session starts, but no log is written
and a toast reports why.

In Flatpak, `${HOME}` and `~` resolve to the sandbox home
(`~/.var/app/io.github.totoshko88.RustConn`), not your real home directory. The
sandbox can only write outside it where a filesystem permission exists — of the
usual locations that means `~/Downloads` (granted for SFTP transfers), which has
to be given as an absolute path such as `/home/<user>/Downloads/rustconn/`.

To read the files, use **Session Logs...** in the primary menu (Sessions
submenu), or **Session Log...** in a connection's context menu to open the
directory that connection writes to. Both show the directory path in the dialog
header.

#### Rotation, retention and redaction

- **Max Size (MB)** rotates the current file once it grows past the limit; `0`
  disables rotation. The global logging settings have no size field, so a
  connection without its own configuration never rotates by size.
- **Retention (days)** deletes `*.log` files older than the limit, and only
  inside the log directory RustConn manages. A log written to a path of your own
  is never deleted automatically.
- Lines that look like a credential prompt or a secret (passwords, API keys,
  tokens, private keys, AWS keys, JWTs) are replaced with `[REDACTED]` before
  anything reaches the file.

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

Group names must be unique among siblings sharing the same parent folder. You can have identically named groups under different parents (e.g. `Site A/RDP` and `Site B/RDP`). Moving or renaming a group into a parent that already contains a child with the same name is rejected.

#### Group Operations

- **Rename** — F2 or right-click → Rename
- **Move** — Drag-drop or right-click → Move to Group
- **Delete** — Delete key (shows choice dialog: Keep Connections, Delete All, or Cancel)

#### Drag and Drop in the Sidebar

The drop zone inside a row decides what happens, for both connections and folders:

| Drop target | Result |
|-------------|--------|
| Middle of a folder | The dragged item goes **into** that folder — this is how folders are nested |
| Top / bottom edge of a folder | The dragged item becomes a **sibling** of that folder, placed before or after it |
| A connection | The dragged item joins the folder that holds that connection |

A folder cannot be dropped into itself or into one of its own subfolders — such a drop is refused with an error toast. Import folders mirror a synced source, so dropping anything into them is blocked. Moving a folder carries its whole subtree, including the KeePass entry paths of the connections inside it.

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

> **Tip:** Responses support two kinds of placeholder, both resolved at connection time. Four come from the connection itself — `${password}` (read from the configured **Password Source**, never stored in the response), `${username}`, `${host}` and `${port}` — and the rest are your own **global variables** by name. For this connection a built-in wins over a global of the same name. Use the ➕ button next to the Response field to insert a placeholder without typing.
>
> `${password}` resolves only when the connection actually has a Password Source configured; when it does not, the rule is left untouched and a warning naming the variable goes to the log. A value containing a line break is rejected, because a newline would submit the answer early. At a *login* prompt on Telnet or Serial prefer [Automatic Login](#automatic-login-telnet--serial), which types the credentials without a rule at all; expect rules are the right tool for a later prompt such as `sudo`.

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
- The two **Automatic Login** prompt fields are inherited field by field: a connection's own value wins, otherwise the first non-empty value found walking up the chain is used
- If a connection has its own Expect Rules, group rules are **not** merged — the connection's rules take precedence
- Expect rules and post-login scripts are inherited independently — rules may come from one group and scripts from another
- Inheritance walks up the group hierarchy: child group → parent group → grandparent group
- To override group automation for a specific connection, add at least one rule in the connection's Automation tab

**Example — Sudo password for all servers in a group:**
1. Edit the "Production Servers" group
2. Enable Automation → click **From Template** → select **Sudo Password (SSH)**
3. The template adds a rule: pattern `\[sudo\] password for \w+:` → response `${password}\n`
4. Save the group
5. All SSH connections in "Production Servers" now auto-respond to sudo prompts, each with its own password — `${password}` resolves per connection from that connection's Password Source, so one group rule covers the whole folder

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
| Emoji / Unicode | `🇺🇦`, `🏢`, `🔒`, `🐳`, `👨‍💻`, `1️⃣` | Displayed as text next to the name |
| GTK icon name | `starred-symbolic`, `network-server-symbolic` | Rendered as a symbolic icon |

Multi-codepoint emoji work as well — ZWJ sequences (`👨‍💻`, `❤️‍🔥`), regional-indicator flags, subdivision tag flags, and keycaps (`1️⃣`).

**Set a Custom Icon:**
1. Edit a connection or group
2. Enter an emoji or GTK icon name in the **Icon** field
3. Save

Leave the field empty to use the default icon (folder for groups, protocol-based for connections).

> **Note:** If the active icon theme does not carry the GTK icon name you entered, RustConn falls back to the default icon (folder or protocol) instead of drawing empty space. Under Flatpak this is the usual case for exotic names: the GNOME runtime ships only the Adwaita theme, so a name you found in a host icon browser may not exist inside the sandbox. See the [Adwaita named icons list](https://gnome.pages.gitlab.gnome.org/libadwaita/doc/main/named-icons.html) for names that always resolve.

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

**Close All Ungrouped:** Right-click any tab → **Close All Ungrouped** — closes everything that is *not* in a group, which is the quick way to clear scratch tabs while keeping your labelled sets.

**Monitor Mode Toggle:** Right-click any tab → **Monitor: Off/Activity/Silence** to cycle monitoring mode.

Groups are visual only — they are session-scoped and not persisted across restarts.

Opening a [cluster](#clusters) assigns its member tabs to a group named after the cluster automatically, so the operations above work on a cluster without any setup.

---

## Productivity Tools

### Templates

Templates are connection presets that store protocol settings, authentication defaults, tags, custom properties, and automation tasks. When you create a connection from a template, all configured fields are copied into the new connection — including the template's icon.

**Manage Templates:** Menu → Tools → **Templates...** (or `rustconn-cli template list`)

**Create Template:**
- **From scratch:** Open Manage Templates → Click **Create Template** → configure name, icon (emoji or GTK icon name), protocol, default settings
- **From existing connection:** Right-click a connection → **Create Template from Connection**

**Use Template:**
- **From Connection Wizard (Ctrl+N):** Choose "Custom Command" → template grid shows your templates and predefined ones; click to fill command and name
- **From Quick Connect (Ctrl+Shift+Q):** Select a template from the dropdown — fields pre-fill the form
- **From Manage Templates:** Select a template → click **Create Connection**
- **From CLI:** `rustconn-cli template apply "SSH Template" --name "New Server" --host "10.0.0.5"`

**Template Fields:** Protocol, Host/Port, Username/Domain, Password Source, Tags, Icon, Protocol Config, Custom Properties, Pre/Post Tasks, WoL Config.

**Predefined Templates:** RustConn ships with 21 built-in templates for common CLI tools that don't have dedicated protocol support:

| Category | Templates |
|----------|-----------|
| Remote Desktop | 🖥️ RustDesk, 🔴 AnyDesk, 🌐 Remmina, 📡 WinBox |
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

**Manage Snippets:** Menu → Tools → **Snippets...** (or `rustconn-cli snippet list`)

**Execute Snippet:** running a snippet is always started from the thing you want it to run *on*, never from the app menu. The menu belongs to the main window, so an entry there would fire against whatever session happened to be current — not necessarily the one you are looking at, and not a detached session window at all. Pick whichever of these matches what you have in front of you:

1. **Right-click inside the terminal** — up to five snippets appear inline for one-click use; beyond that the list becomes **Execute Snippet…**, which opens the picker
2. **Right-click a connection in the sidebar** → **Run Snippet...** — connects first if needed, then opens the picker
3. **Menu → Tools → Snippets...** → the ▶ button on a snippet's row

Any of them fills in variable values first if the snippet needs them and they cannot be resolved from [Global Variables](#global-variables).

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

#### Confirm Before Running

Some commands are not the kind you want to fire by accident — `rm -rf`, `mkfs`, a production deploy. Turn on **Confirm before running** in the snippet editor and RustConn asks once more, showing the exact command it is about to send, every time that snippet runs.

The setting is per snippet and off by default, so existing snippets keep running on a single action.

It covers every way a snippet can be started: the picker, the Execute button in the snippet manager, the variable-input dialog, the snippet entries in the terminal's right-click menu, and the Scripts menu of an embedded RDP session. Cancelling in the variable dialog's case keeps the values you typed, so you can correct a variable rather than start over.

**From the CLI:**

```bash
# Mark a snippet as needing confirmation
rustconn-cli snippet add --name "Wipe scratch disk" \
    --command "mkfs.ext4 /dev/sdb" --confirm

# Turn it on or off for an existing snippet
rustconn-cli snippet edit "Wipe scratch disk" --confirm false

# Running it prompts on a terminal; --force is required in a script
rustconn-cli snippet run "Wipe scratch disk" --execute --force
```

Without a terminal on stdin and without `--force`, `snippet run --execute` refuses rather than prompting into the void.

### Clusters

A cluster is a named set of connections you open together — a rack, a Kubernetes node pool, the three web servers behind one load balancer.

**Create a cluster:**
1. Menu → Tools → **Clusters…**
2. Click **New Cluster** → enter a name → tick the member connections → **Save**

You can also select several connections in the sidebar and use **Create Cluster** in the bulk-action bar.

**Open a cluster:** Menu → Tools → **Clusters…** → the ▶ button on the cluster's row. RustConn opens a tab for every member, whatever their protocol.

**Every tab of an open cluster joins a tab group named after the cluster.** The tab reads `[ClusterName] ConnectionName`, so you can see at a glance which tabs belong together, and every [tab group](#tab-grouping) operation applies to the cluster:

- **Close All in Group** on any member closes the whole cluster
- **Close All Ungrouped** closes everything *except* your open clusters
- The tab switcher (**Ctrl+%**) shows the cluster name beside each member

**Close a cluster:** either **Close All in Group** from any member tab, or the ■ button on the cluster's row in Clusters… (**Disconnect all cluster sessions**).

Notes on the automatic group:
- The group name is the cluster's name exactly as written. Renaming a cluster affects the *next* time you open it, not tabs that are already open.
- A cluster whose name matches a group you created by hand merges into it — same name means same group, which is the rule two hand-labelled tabs already follow.
- You can override it per tab: **Remove from Group**, or **Set Group…** to move one member elsewhere. The tab stays part of the cluster for the Disconnect button either way.
- Groups are visual and session-scoped; they are not written to disk.

**Broadcast:** typing into several hosts at once is *not* a cluster property. It is a split-view feature — arrange the members in a split layout and use the broadcast toggle in the header bar. See [Broadcast Input](#broadcast-input).

**Use cases:**
- Rolling out configuration changes across multiple servers
- Running the same diagnostic command on all nodes
- Opening and closing a whole environment as one unit

### Workspace Profiles

Workspace profiles save your current set of open connections (with tab order) as a named snapshot that you can restore later with one click. Unlike session restore (which works automatically on restart), workspaces are named and can be switched manually at any time.

**Save a workspace:**
1. Open the connections you want in the workspace
2. Menu → Tools → **Workspaces...**
3. Click **Save Current** → enter a name → Save

**Open a workspace:**
1. Menu → Tools → **Workspaces...**
2. Select the workspace → click **Open**
3. All connections from the workspace are connected simultaneously

**Use cases:**
- "Production" workspace with monitoring + DB + web servers
- "Development" workspace with staging servers + bastion
- Quick context switching between projects

Workspaces persist in `~/.config/rustconn/workspace_profiles.toml`. If a connection referenced by a workspace is deleted, the entry is automatically removed from the workspace.

### Port Knocking

Port knocking allows you to open firewall ports by sending a specific sequence of TCP/UDP packets before connecting. This works without any external tools — RustConn has a built-in implementation.

**Configure per-connection:**
1. Edit Connection → **Advanced** tab → **Connection Behavior** section
2. Enter the knock sequence in the **Port Knock Sequence** field
3. Format: `7000 8000/tcp 9000/udp` (space or comma separated, /tcp is default)

**How it works:**
- Before each connection, RustConn sends the knock sequence to the target host
- Each knock is a TCP connect attempt or UDP datagram (the SYN itself is the knock)
- After all knocks, a 200ms settle time allows the firewall to install its rule
- Then the normal connection proceeds (port check → connect)

**Timing defaults:**
- Inter-knock delay: 100ms
- Post-knock settle: 200ms

#### fwknop Single Packet Authorization (SPA)

Where a knock *sequence* can be replayed or observed by anyone watching the wire, SPA opens the firewall with a **single encrypted, authenticated UDP packet**. This is the same scheme `fwknopd` expects on the server side, implemented natively in RustConn — no `fwknop` client binary is needed, and it works inside the Flatpak sandbox.

**Configure per-connection:**
1. Edit Connection → **Advanced** tab → **Connection Behavior** section → expand **SPA (fwknop)**
2. Enable **SPA** and fill in the fields below

| Field | Meaning | Default |
|-------|---------|---------|
| **Rijndael Key** | AES-256-CBC passphrase (or base64) — must match the server's `KEY`/`KEY_BASE64`. Stored in your secret backend. | — |
| **HMAC Key** | HMAC-SHA256 key — must match the server's `HMAC_KEY`/`HMAC_KEY_BASE64`. Stored in your secret backend. Strongly recommended. | — |
| **Access** | The access request the server should honor, e.g. `tcp/22` or `tcp/22,tcp/443`. | `tcp/22` |
| **Destination Port** | UDP port the SPA packet is sent to. | `62201` |
| **Allow IP** | Which address the server opens for: **Source IP** (packet's own source, `0.0.0.0` in the request), **Resolve Public** (look up this machine's public IP first, like `fwknop -R`), or **Explicit**. | Source IP |

**How it works:**
- Before connecting, RustConn builds the SPA payload, encrypts it with AES-256-CBC (OpenSSL-compatible `Salted__` / EVP_BytesToKey key derivation), appends an HMAC-SHA256 digest, and sends it as one UDP datagram. A fresh random salt and timestamp make every packet unique.
- SPA runs only when both keys are set; otherwise it is skipped. It can be combined with a knock sequence, but on its own it is the more secure of the two.
- The keys never appear in the config file — only vault references are stored.

### Broadcast Input

Type once and have the keystrokes reach several terminals at the same time.

Broadcast is a property of a **split layout**, not of a cluster: whichever terminals share the active tab's split receive the input. That keeps the target set visible on screen — you are typing into exactly the panels you can see.

**Usage:**
1. Put the terminals side by side in a split layout (see [Split View](#split-view))
2. Focus one of the terminal panels
3. Press **Ctrl+Shift+B**, or click the broadcast toggle in the header bar
4. Type — the keystrokes are mirrored into the other terminal panels of that split
5. Press **Ctrl+Shift+B** again to stop

The toggle only becomes available when the active tab's split holds at least two terminal sessions and a terminal panel has focus. Mirroring never targets an embedded RDP, VNC or SPICE panel — those are not terminals and receive their own input only.

Opening a [cluster](#clusters) is the quick way to get the members on screen; from there, split them and toggle broadcast. Earlier releases had a separate cluster-owned broadcast mode with checkboxes on tabs. It was removed in 0.14.8 in favour of the split-view toggle, which has one visible target set instead of two competing ones. `Cluster` still carries a `broadcast_enabled` flag on disk for CLI compatibility; it does not affect the GUI.

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
| Connection | `network-server-symbolic` | Network (global jump host) + Clients |
| Monitoring | `power-profile-performance-symbolic` | Remote host metrics (General + Visible Metrics) + Terminal Activity Monitor defaults |
| Cloud Sync | `emblem-synchronizing-symbolic` | Sync directory, synced groups, simple sync |

### Terminal page

**Terminal group:** Font (family and size), Scrollback (history buffer lines, keep on reconnect), Color Theme (Dark, Light, Solarized, Monokai, Dracula, plus user-created custom themes), Cursor (shape and blink mode), Behavior (scroll on output/keystroke, hyperlinks, mouse autohide, bell, SFTP via mc, copy on select, close tab on clean exit).

**Close tab on clean exit:** When enabled, tabs are automatically closed when the remote session exits cleanly (exit code 0, e.g. user typed `exit` or `logout`) instead of showing the reconnect overlay. Disabled by default.

**Keep on reconnect:** When enabled (the default), reconnecting a terminal session keeps the previous session's output instead of clearing the terminal, so output from before an idle-timeout disconnect stays readable. A dim `── Reconnected at … ──` rule marks where the new session begins, and the view returns to the bottom. Applies to sessions that reconnect in place — SSH, Telnet, Serial, Kubernetes, Mosh and custom commands — and is bounded by the Scrollback line limit like any other history. Disable it to get a cleared terminal on every reconnect.

**Local Shell group:** Command — custom command to run in Local Shell tabs instead of the default login shell (e.g. `fish`, `bash --norc`, `neofetch && bash`). Leave empty for system default.

**Custom Themes:** Click the **+** button next to the theme dropdown to create a new custom theme. The theme editor lets you set background, foreground, cursor, and all 16 ANSI palette colors. Custom themes are saved to `~/.config/rustconn/custom_themes.json` and appear alongside built-in themes. Edit or delete custom themes with the pencil and trash buttons.

**Logging group:** Enable Logging (global toggle), Log Directory, Retention Days, Logging Modes (activity, user input, terminal output), Timestamps.

### Interface page

**Appearance group:** Theme (System, Light, Dark), Language (UI language selector, restart required), Rendering (see below, restart required), Color tabs by protocol, Sidebar width (260–500 pixels, default 320).

**Rendering** chooses which GTK renderer draws the interface. Leave it on **Automatic** unless the interface is sluggish: RustConn then uses the GPU renderer, except where it is known to behave worse than software rasterisation — X11 sessions whose compositor paints menus blank until you hover them, and macOS running inside a virtual machine, where the virtual GPU offers no accelerated OpenGL and the GPU path becomes both slow and CPU-hungry. **Software (Cairo)** forces software rasterisation everywhere; pick it if the interface lags, scrolls in steps, or responds late to typing in an environment the automatic choice does not recognise. **Hardware (GPU)** forces the GPU renderer, which is the setting for an X11 session with a driver that works fine. A `GSK_RENDERER` environment variable, if you set one, overrides all three. The choice applies on the next start, because GTK reads it while it opens the first window.

**Window group:** Remember size (restore window geometry on startup), Show connection in window title (appends the active connection name so time-tracking tools can attribute usage; off by default for privacy).

**Connections group:** Open a new session on every double-click — off by default, so a double-click on a connection that is already running focuses that session instead of duplicating it (hold Shift or Ctrl, or use right-click → Open new session, to force a second one). Turn it on if you routinely keep several concurrent sessions on the same host: every double-click then starts another session, and the modifier is no longer needed.

**Startup group:** On startup — Do nothing, Local Shell, or connect to a specific saved connection.

**System Tray group:** Show icon, Minimize to tray (hide window instead of closing).

**Session Restore group:** Enabled, Ask first, Max age (1–168 hours).

**Keybindings group:** Customizable keyboard shortcuts for 30+ actions across 6 categories. Record button to capture key combinations. Per-shortcut Reset and Reset All to Defaults.

### Secrets page

**Secret backend group:**
- **Backend** — libsecret (or macOS Keychain), KeePassXC, Bitwarden, 1Password, Passbolt, Pass (passwordstore.org), Encrypted file, Portable encrypted file
- **Version** — whether the backend's client program is installed, and which version
- **Status** — whether that backend can actually store and read a password *right now*. This is the line to read: a backend can be installed and still be unusable because it is locked, not logged in, has no database file chosen, or has no keyring service answering. Shown for every backend. If you select one that is not ready, a banner appears under the header bar after you close Settings, naming the backend and what is missing.
- **Also read from the encrypted file** — look in this computer's encrypted credential file as well as in the selected backend when resolving a password, so entries saved before you switched backend keep working. Reading only: saving always targets the selected backend, and a write it refuses is reported so you can choose where the password goes instead of having it moved for you.
- **Copy Passwords…** — move passwords you already have from one store into another. This is the supported way to migrate after changing backend.
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

**Network group:** *Global Jump Host* (a saved SSH connection) and *Global ProxyJump* (free text, OpenSSH syntax). The outermost of the three bastion tiers — it applies to every connection that inherits, including ungrouped ones, which no group can reach. Set both and they chain: the Jump Host is contacted first and the ProxyJump value is reached through it. A connection set to *Direct* ignores both. See [Network Mode and Where a Jump Host Comes From](#network-mode-and-where-a-jump-host-comes-from).

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

**Persistence (0.20.3+):** The passthrough state is saved when the window closes and restored on the next start. If passthrough was active when you quit, it will be active again when you reopen RustConn — no need to toggle it every session. The setting is stored in `config.toml` under `[ui] keyboard_passthrough = true/false`.

**Automatic focus-based shortcut management (0.18.8+):**

Independently from manual passthrough mode, RustConn automatically suspends single-modifier shortcuts (e.g. `Ctrl+W`) that conflict with terminal input when a terminal has focus, while keeping multi-modifier variants active (e.g. `Ctrl+Shift+W`). This means:
- `Ctrl+W` in a focused terminal → sent to the remote session (shell word-delete)
- `Ctrl+Shift+W` → still closes the tab (application shortcut)
- If you remap "Close Tab" to a non-conflicting combo (e.g. `Ctrl+F4`), it works regardless of terminal focus

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
| Templates and snippets | SSH keys |
| Clusters | Session logs |
| Global variables (names only; secret values are in vault) | Flatpak-installed CLI tools |
| Keybindings | |
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
- Royal TS / Royal TSX (`.rtsz`, `.rtsx` — passwords stay in Royal TS, see below)
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
| Royal TS / Royal TSX | — | `.rtsz` (compressed) or `.rtsx` file | SSH, Telnet, RDP, VNC | Folder hierarchy → groups; usernames inherited from folder credentials; passwords cannot be imported (encrypted in the document) |
| MobaXterm | — | `.mxtsessions` | SSH, RDP, VNC, Telnet, Serial | INI-based sessions |
| SecureCRT | `~/.vandyke/Config/Sessions/` | Directory or `.ini` | SSH, Telnet, RDP, VNC | Folder hierarchy → groups |
| Remote Desktop Manager | — | JSON file | SSH, RDP, VNC, Telnet | Devolutions JSON export; `Group` paths → groups |
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

#### From Royal TS / Royal TSX

1. In Royal TS: **File > Export > Royal TS Document (.rtsz)** — `.rtsx` works as well
2. **File > Import > Royal TS** → select file → Import (folder structure preserved as groups)

SSH, Telnet, RDP (`RoyalRDSConnection`) and VNC connections are imported; every other object
type (web page, file transfer, TeamViewer, ...) is listed as skipped. A Royal TS Terminal
object is imported as SSH or Telnet according to its own connection type, on port 22 or 23
unless the document sets one. Usernames and domains come from the connection, from an
assigned credential, or are inherited from the parent folder. An encrypted or lockdown
document cannot be read at all.

**Passwords cannot be imported.** This is a property of the format, not a missing feature:
Royal TS never writes a password in clear text. A document you protected with an encryption
password has its passwords encrypted under that password, and a document without one has
them encrypted under a key built into Royal TS. Neither is readable from outside the
application, so every affected connection is set to prompt for its password and the import
result says so. Enter each password once on the first connect and let RustConn store it in
your vault, or copy the passwords across through your password manager.

#### From Remote Desktop Manager

1. In RDM: **File > Export** → JSON. Include credentials if you want the passwords to come
   across; otherwise they stay encrypted and unusable.
2. **File > Import > Remote Desktop Manager** → select the file → Import

Folders are taken from the `Group` path of each entry, credentials from the entry itself or
from a linked `Credential` entry, and any entry type RustConn does not support is listed as
skipped together with its RDM type.

RDM lets add-ons contribute their own entry fields and serializes them inconsistently between
data-source types, so the importer accepts any scalar shape a field may arrive in: a
connection type or a port written as a bare integer (`"ConnectionType": 25`, `"Port": 3389`)
as readily as a quoted one, and a flag as `0`/`1` or `"true"`. A single entry the importer
still cannot read is reported in the skipped list instead of aborting the import — under its
name when it has one, and under its position (`Connection #3`) when it does not. A `Port`
value that is not a usable port (zero, negative, fractional, or above 65535) is reported as
a warning and the protocol default is used. A blank `ConnectionType` is reported as skipped
rather than guessed at; an absent one still means RDP, which is RDM's own default.

Besides the usual `{"Connections": [...]}` export, a bare array of entries and a single entry
copied with **Clipboard > Copy** are also accepted. A JSON file with neither a `Connections`
nor a `Folders` array — and no `ConnectionType` marking it as a copied entry — is rejected
with a parse error telling you which RDM export to produce. That includes files that look
plausible, such as an inventory of `{"Name": ..., "Host": ...}` objects: importing one of
those as a single connection would be a worse answer than saying it is not an RDM export.
An export whose list is empty (`{"Connections": []}`) is still valid and imports zero
connections.

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

Groups can define SSH settings (auth method, key path, jump host, proxy jump, agent socket) that child connections inherit. This avoids duplicating key paths across dozens of connections and keeps `ssh_key_path` local-only per device. See [Network Mode and Where a Jump Host Comes From](#network-mode-and-where-a-jump-host-comes-from) for the bastion fields, which have their own three-tier resolution and are not gated on the key source.

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
2. Set SSH Key Path, Auth Method, Jump Host, SSH Proxy Jump, or Agent Socket
3. What decides whether a child connection picks each one up differs by field:

| Group setting | Inherited when |
|---------------|----------------|
| SSH Key Path | the connection's **Key Source** is *Inherit* |
| Auth Method | the connection's **Key Source** is *Inherit* |
| Jump Host / SSH Proxy Jump | the connection's **Network Mode** is *Inherit from group or global*, and it has no jump host of its own — whatever its protocol, and regardless of where it gets its SSH key |
| Agent Socket | the connection has none of its own |

The inheritance chain walks from the connection's immediate group up to the root, returning the first value found.

> **Changed in 0.20.9:** the bastion fields used to be gated on **Key Source** as well, which made a group jump host and a connection-level key file mutually exclusive, and RDP, VNC and SPICE inherited unconditionally with no way to refuse. **Network Mode** is now the single answer for all protocols. If you worked around the old behaviour by setting Key Source to *Inherit* on a connection that has its own key, you can set the key back.

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
| macOS Keychain | macOS (default there) | High — OS-managed via Security.framework |
| KeePassXC | Users who already use KeePassXC | High — AES-256 encrypted database |
| Bitwarden | Teams using Bitwarden | High — cloud-synced, E2E encrypted |
| 1Password | Teams using 1Password | High — cloud-synced, E2E encrypted |
| Passbolt | Self-hosted team password management | High — GPG-based |
| Pass (passwordstore.org) | CLI-oriented users, git-synced passwords | High — GPG-encrypted files |
| KDBX File | Offline/air-gapped environments | High — AES-256, local file only |
| Encrypted-file fallback | Systems with no usable keyring (headless, minimal desktops) | Medium — AES-256-GCM, but key sits on the same disk (obfuscation at rest, not a boundary) |
| Portable encrypted file | Sharing one password set across your own machines, including Linux ↔ macOS | High — AES-256-GCM under an Argon2id passphrase; the key is never written to disk |

Configure your preferred backend in Settings → Secrets. RustConn falls back to the system keyring if the preferred backend is unavailable.

### Portable Encrypted File (cloud-syncable)

Every other backend ties credentials to one machine or one vendor's vault. This
one puts them in a single file that you control the key to, so it can live in
Dropbox, Google Drive, Nextcloud or Syncthing and open on any machine — Linux and
macOS included.

**Setting it up on the first machine**

1. Settings → Secrets → **Backend** → *Portable encrypted file*. The row's
   subtitle then describes the choice, so you can confirm it took effect.
2. **File path** — Browse to a folder your cloud client syncs, and name the file
   (the default name is `credentials-portable.enc`). Leave the field empty to use
   the default location, which the placeholder shows. A leading `~/` is expanded.
3. **Passphrase** — choose one, then repeat it in **Confirm passphrase**
4. **Save passphrase** — optional; see below
5. **Set up the file → Create File** — writes the file now and tells you where.
   Do this before relying on it: it is how a mistyped folder gets caught here
   rather than later, as a failure to save a password.
6. **Existing passwords → Copy Credentials** — re-encrypts the passwords already
   stored on this machine with your passphrase and writes them into the portable
   file. The originals are kept, so nothing is lost either way.

The **File** row above the passphrase reports the file's state throughout —
whether it exists, how many passwords are in it, or that it cannot be read.

**Setting it up on the second machine**

Wait for the file to sync, then point Settings → Secrets at it and enter the same
passphrase. **Create File** checks that the passphrase opens the delivered file
without changing it, so you find out here rather than on the first connection.
There is nothing to import: the file already contains everything.

**Coming from KeePassXC, the system keyring or a vault**

*Copy Credentials* only reads the machine-bound encrypted file. To bring
passwords in from any other store, use Settings → Secrets → **Move between
stores → Copy Passwords…**, pick the store they are in now under *From* and the
portable file under *To*, and the dialog reports how many entries the pair
covers before you confirm.

The transfer copies; it never removes anything from the source. It moves the
passwords of every connection and group set to **Vault**, plus secret variables
that RustConn stores itself — a variable pointing at an entry you maintain
elsewhere (a custom KeePass path or vault entry name) is left alone. Entries that
exist in a vault but were not created by RustConn are not touched either: nothing
in the connection list names them.

The dialog counts entries as it goes and offers **Stop**. Stopping takes effect
after the entry in progress, never in the middle of one, and the report then says
how many entries were never looked at — so a run you stopped is not mistaken for
one that failed. Everything already copied stays where it is, and running the
copy again finishes the rest.

**Choosing the passphrase**

The passphrase field says when what you have typed would not take long to guess.
It is advice, not a rule: the same field is how an existing file is opened, so
nothing is ever refused for being weak, and the warning only appears for a file
being created. Length does most of the work — several unrelated words beat a
short string of symbols and are easier to remember. RustConn has no wordlist, so
it cannot tell you that a common password is common; it can only tell you when a
passphrase is short or repetitive.

**Changing the passphrase**

Settings → Secrets → **Passphrase → Change Passphrase…** asks for the current
passphrase and the new one together, then re-encrypts every password in the file
under the new one and reports how many.

It is all-or-nothing: if one entry cannot be read the change is abandoned and the
file stays as it was, rather than leaving you with a file that opens but is
missing credentials. If a copy of the passphrase is kept on this computer, save
your settings afterwards — until you do, this computer still remembers the old
one and will ask for the new one on the next start.

What changing the passphrase does **not** do is protect what has already left.
Anyone holding the old passphrase and an old copy of the file still reads every
password that was in it at the time. If the old passphrase was exposed, change
the passwords themselves too.

**Saving the passphrase**

The **Save passphrase** selector decides whether you retype it every session:

| Choice | Effect |
|--------|--------|
| Don't save | Asked once per session, on the first connection that needs a credential |
| Encrypted file | Stored on *this* machine, encrypted with this machine's key. The portable file stays portable; only the convenience copy is local |
| System keyring | Stored in GNOME Keyring / KDE Wallet / macOS Keychain |

Choosing "Don't save" removes any copy that was previously stored.

**If the passphrase is not available**, the first connection that needs a stored
password shows an unlock dialog. Enter the passphrase and the connection
continues; it is kept in memory for the rest of the session.

**What to know before relying on it**

- **There is no recovery.** The passphrase is the only key. RustConn cannot reset
  it, and no copy of it exists in the file. This is why the setup asks you to
  type it twice.
- **Use one machine at a time.** Each save rewrites the whole file. If two
  machines both save while offline, the sync client keeps one version and the
  other machine's additions are lost. Adding passwords on one machine at a time
  avoids this entirely.
- **The file is safe to sync, but it is still a file.** Anyone who obtains it can
  attempt to guess the passphrase offline. Argon2id makes that slow and expensive
  — a long passphrase makes it impractical.
- **Keep the machine-bound copy.** "Copy Credentials" does not delete the
  originals on purpose: they are your fallback on this machine if the passphrase
  is ever lost.

`rustconn-cli secret get`, `set` and `delete` work with this backend too. If the
passphrase is not saved locally, the CLI prompts for it; in a script or a shell
with no terminal it reports that it cannot ask, rather than hanging.

**Fallback Behavior:**

This describes **variable** secrets only. Connection and group passwords behave differently — see **Also read from the encrypted file** in the Secrets settings above, where reads may fall back and writes never do.

When the preferred backend (e.g., KeePassXC) cannot be reached — database password not configured, database file locked, or CLI tool not installed — RustConn falls back to the system keyring (libsecret) for both reading and writing a variable's secret. This requires **Also read from the encrypted file** in Settings → Secrets (enabled by default; it was labelled "Enable fallback" up to 0.21.2).

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
- **SSH keepalive** — `ServerAliveInterval=15` + `ServerAliveCountMax=3` applied by default to all SSH sessions, detecting dead connections within ~45 seconds instead of relying on TCP timeout. Override via Custom Options if your server requires different values
- **Network change detection** — `gio::NetworkMonitor` reacts to interface changes immediately (stale socket cleanup + auto-reconnect), preventing hanging sessions after VPN/WiFi switches
- **ControlPersist=60s** — short master socket lifetime reduces the window for stale multiplexed connections after network changes
- Use **SSH Proxy Jump** for connections behind bastion hosts
- Use **Zero Trust providers** to eliminate direct SSH exposure
- Enable **session logging** for audit trails

---

## Troubleshooting & FAQ

### Frequently Asked Questions

**Where are my passwords stored?**
Depending on your configured secret backend: libsecret (desktop keyring), macOS Keychain (on macOS), KeePassXC (database), KDBX file (local encrypted file), Bitwarden/1Password/Passbolt (cloud vault), Pass (GPG-encrypted files), or the app-managed encrypted-file fallback (AES-256-GCM, used when no keyring is available). Connection files themselves never contain actual passwords.

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

**My External RDP session disconnects (or goes fullscreen) when I press a key combo.**
That is a built-in shortcut of the FreeRDP SDL client, which uses **Right Shift** as the modifier (e.g. Right Shift + D disconnects, Right Shift + G releases the keyboard/mouse). You can remap or disable these in `sdl-freerdp.json` — see [External FreeRDP Keyboard Shortcuts](#external-freerdp-keyboard-shortcuts-right-shift-hotkeys) for the exact Flatpak path and ready-to-use JSON.

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
2. **Flatpak only, and the usual cause of "Not logged in":** run `bw` so that it uses the same state directory as the application. A Local Shell tab is a *host* shell, so `bw login` there writes somewhere RustConn does not read. See [BITWARDEN_SETUP.md](BITWARDEN_SETUP.md#where-the-cli-keeps-its-login-state).
3. For self-hosted: `bw config server https://your-server` before logging in — same directory caveat applies
4. Login: `bw login` → Unlock from Settings → Secrets
5. Select Bitwarden in Settings → Secrets and read the **Status** line; it says what is still missing
6. For 2FA (FIDO2, Duo): use API key authentication
7. Enable "Save to system keyring" for auto-unlock

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

RustConn opens the KeePass database directly by file (`.kdbx`); it does not use KeePassXC's browser-integration protocol.

1. Install KeePassXC (for the `keepassxc-cli` helper and the app itself)
2. Select the KeePassXC/KDBX backend and set the KDBX path in Settings → Secrets, then unlock with the database password (optionally cached in the system keyring)
3. Flatpak: `keepassxc-cli` on the host is detected automatically via `flatpak-spawn --host`

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

**Before attaching a log to a bug report, skim it.** RustConn does not log passwords, and the one dependency that did — `sspi`, which printed the encoded CredSSP request containing the RDP password in clear — is filtered out and cannot be re-enabled through `RUST_LOG`. A debug log still describes your setup in detail, though: hostnames and ports, usernames, key file paths, vault entry names, and share paths. Prefer the narrowest filter that shows the problem, which is what the module-specific forms above are for.

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
| Ctrl+Shift+M | Move Session to New Window (and back) |
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
| Ctrl+Shift+R | Pop Pane to Tab (keeps the session, returns it to its own tab) |
| Ctrl+Shift+J | Unsplit (return every session to its own tab) |
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
| Ctrl+Shift+D | Toggle Compact Interface |
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

> **Note:** Quitting with **Ctrl+Q** (or closing the window) while session tabs are open shows a "Close RustConn?" confirmation dialog with the number of open sessions (tabs plus [detached windows](#detached-session-windows)), instead of silently disconnecting everything. This is skipped when minimize-to-tray is enabled (the app keeps running in the tray).

---

## Support

- **GitHub:** https://github.com/totoshko88/RustConn
- **Issues:** https://github.com/totoshko88/RustConn/issues
- **Releases:** https://github.com/totoshko88/RustConn/releases

**Made with ❤️ in Ukraine 🇺🇦**
