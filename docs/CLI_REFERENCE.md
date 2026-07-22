# RustConn CLI Reference

**Version 0.18.11** | Command-line interface for RustConn connection management

The `rustconn-cli` binary provides headless connection management from the terminal. It shares the same configuration files as the GUI (`~/.config/rustconn/`), so changes made in either tool are immediately visible to the other. The default build is the minimal headless path; desktop/client-launch and secret-management commands are enabled with optional features.

For the main user guide, see [USER_GUIDE.md](USER_GUIDE.md).

---

## Installation

| Install Method | How to Run |
|----------------|------------|
| Native (deb/rpm/AUR) | `rustconn-cli` is installed alongside `rustconn` |
| Flatpak | `flatpak run --command=rustconn-cli io.github.totoshko88.RustConn [command]` |

For Flatpak, create a shell alias to save typing (see [Flatpak Usage](#flatpak-usage) below).

## Build Feature Sets

| Feature set | Includes |
|-------------|----------|
| Default / `--no-default-features` | Headless management commands: config, list/show, add/update/delete, import/export, groups, tags, templates, clusters, variables, sync metadata, stats, shell completions |
| `client-launch` | Optional launch paths such as `connect` and desktop file-manager integration |
| `secret-management` | Optional `secret` commands and system keyring support |
| `full` | `client-launch` + `secret-management` |

---

## GUI Startup Flags

The GUI binary (`rustconn`) accepts startup flags:

```bash
rustconn --shell                        # Open local shell on startup
rustconn --connect "My Server"          # Connect by name (case-insensitive)
rustconn --connect 550e8400-...         # Connect by UUID
rustconn file.rdp                       # Open and connect from an .rdp file
rustconn file.vv                        # Open a virt-viewer .vv file
rustconn --version                      # Print version (also -V)
rustconn --help                         # Print usage (also -h)
```

These flags override the startup action configured in Settings.

---

## Global Options

Every `rustconn-cli` command accepts these flags:

| Flag | Short | Description |
|------|-------|-------------|
| `--config <PATH>` | `-c` | Path to configuration directory (overrides default and `RUSTCONN_CONFIG_DIR`) |
| `--verbose` | `-v` | Increase log verbosity (`-v` info, `-vv` debug, `-vvv` trace) |
| `--quiet` | `-q` | Suppress all output except errors |
| `--no-color` | — | Disable colored output (also respects `NO_COLOR` env var) |

---

## Environment Variables

| Variable | Description |
|----------|-------------|
| `RUSTCONN_CONFIG_DIR` | Override the default configuration directory (`~/.config/rustconn/`) |
| `NO_COLOR` | Disable colored output when set to any value (see [no-color.org](https://no-color.org)) |
| `RUST_LOG` | Override log level filter (e.g. `RUST_LOG=debug rustconn-cli list`) |

---

## Connection Lookup

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

---

## Output Formats

Commands that list or display data support three output formats:

| Format | Flag | Description |
|--------|------|-------------|
| `table` | `--format table` | Human-readable table (default in terminal) |
| `json` | `--format json` | Machine-readable JSON (default when piped) |
| `csv` | `--format csv` | Comma-separated values |

When stdout is not a terminal (piped or redirected), the format automatically switches from `table` to `json` for scripting convenience. Long table output is paged through `less` when available.

Commands supporting `--format`: `list`, `show`, `test`, `stats`, `group list`, `group show`, `snippet list`, `template list`, `cluster list`, `var list`, `smart-folder list`, `dynamic-folder list`, `recording list`, `sync list`, `history list`, `tag list`, `monitor metrics`.

---

## Exit Codes

| Code | Meaning |
|------|---------|
| `0` | Success |
| `1` | General error (configuration, validation, I/O, export, import) |
| `2` | Connection failure (test failed, connection not found, connection error) |

---

## Commands Reference

### list — List connections

```bash
rustconn-cli list [-f table|json|csv] [-p protocol] [-g group] [-t tag]
```

```bash
rustconn-cli list                                    # All connections (table)
rustconn-cli list --format json                      # JSON output
rustconn-cli list --format csv                       # CSV output
rustconn-cli list --protocol ssh                     # Filter by protocol
rustconn-cli list --group "Production"               # Filter by group name
rustconn-cli list --tag "web"                        # Filter by tag
rustconn-cli list --group "Production" --tag "web"   # Combine filters
```

### add — Add a new connection

```bash
rustconn-cli add -n <name> -H <host> [-P protocol] [-p port] [-u user] [-k key] [options...]
```

Options:

| Flag | Short | Description |
|------|-------|-------------|
| `--name` | `-n` | Connection name (required) |
| `--host` | `-H` | Hostname, IP, or device path (required) |
| `--port` | `-p` | Port number (defaults: SSH=22, RDP=3389, VNC=5900, Telnet=23) |
| `--protocol` | `-P` | Protocol: `ssh`, `rdp`, `vnc`, `spice`, `sftp`, `telnet`, `serial`, `mosh`, `kubernetes`/`k8s`, `zerotrust`/`zt` (default: `ssh`) |
| `--user` | `-u` | Username |
| `--key` | `-k` | Path to SSH private key file |
| `--auth-method` | — | SSH auth: `password`, `publickey`, `keyboard-interactive`, `agent`, `security-key` |
| `--device` | — | Serial device path (alias for `--host` with serial protocol) |
| `--baud-rate` | — | Serial baud rate (default: 115200) |
| `--icon` | — | Custom icon (emoji or GTK icon name, e.g. `"🏢"`, `"starred-symbolic"`) |
| `--ssh-agent-socket` | — | Custom SSH agent socket path |
| `--jump-host` | — | SSH jump host (connection name or UUID, for SSH/SFTP/RDP/VNC/SPICE) |
| `--keep-alive-interval` | — | SSH keep-alive interval in seconds (`ServerAliveInterval`) |
| `--keep-alive-count` | — | SSH keep-alive count max (`ServerAliveCountMax`) |
| `--ssh-verbose` | — | Enable SSH verbose/debug output (`-v` flag) |
| `--mptcp` | — | Enable Multipath TCP (SSH: `-o TCPMultipath=yes`; RDP/VNC embedded: MPTCP socket). Requires Linux 5.6+ kernel, OpenSSH 9.9+ for SSH. Falls back to regular TCP transparently |
| `--ignore-certificate` | — | Accept any RDP server certificate (skip TOFU verification) |
| `--tags` | — | Comma-separated tags (e.g. `"production,linux,critical"`) |
| `--description` | — | Description text for the connection |
| `--group` | — | Group name (creates the group if missing) |
| `--domain` | — | Windows domain for RDP/SPICE authentication |
| `--window-mode` | — | Window mode: `embedded`, `external`, or `fullscreen` (RDP/VNC only) |
| `--skip-port-check` | — | Skip pre-connect TCP port check |
| `--x11-forwarding` | — | Enable X11 forwarding (`-X` flag, SSH/SFTP only) |
| `--agent-forwarding` | — | Enable SSH agent forwarding (`-A` flag, SSH/SFTP only) |
| `--compression` | — | Enable compression (`-C` flag, SSH/SFTP only) |
| `--startup-command` | — | Command to execute on SSH connection startup |
| `--proxy-command` | — | SSH ProxyCommand (e.g. `"ncat --proxy 127.0.0.1:9050 --proxy-type socks5 %h %p"`) |
| `--ssh-option` | — | Custom SSH option `K=V` (repeatable, e.g. `--ssh-option StrictHostKeyChecking=no`) |
| `--local-forward` | — | Local port forwarding `L:H:P` (repeatable, e.g. `8080:localhost:80`) |
| `--remote-forward` | — | Remote port forwarding `R:H:P` (repeatable, e.g. `9090:localhost:3000`) |
| `--dynamic-forward` | — | Dynamic SOCKS forwarding port (repeatable, e.g. `1080`) |
| `--gateway` | — | RDP gateway hostname |
| `--gateway-port` | — | RDP gateway port (default: 443) |
| `--gateway-username` | — | RDP gateway username (if different from connection user) |
| `--remote-app-program` | — | RemoteApp program path or alias (RDP only) |
| `--remote-app-args` | — | RemoteApp command-line arguments (RDP only) |
| `--remote-app-name` | — | RemoteApp display name (RDP only) |
| `--resolution` | — | RDP resolution (e.g. `1920x1080`) |
| `--color-depth` | — | RDP color depth: 8, 15, 16, 24, or 32 |
| `--disable-nla` | — | Disable Network Level Authentication (RDP only) |
| `--keyboard-layout` | — | RDP keyboard layout override (Windows KLID) |
| `--audio-redirect` | — | Enable audio redirection (RDP only) |
| `--shared-folder` | — | Shared folder `NAME:PATH` (repeatable, RDP only) |
| `--vnc-client-mode` | — | VNC client mode: `embedded` (default) or `external` |
| `--vnc-performance` | — | VNC performance mode: `quality`, `balanced` (default), or `speed` |
| `--vnc-encoding` | — | VNC encoding: `tight`, `zrle`, or `hextile` |
| `--vnc-compression` | — | VNC compression level (0-9) |
| `--vnc-quality` | — | VNC quality level (0-9) |
| `--vnc-view-only` | — | VNC view-only mode (no keyboard/mouse input) |
| `--vnc-no-scaling` | — | Disable VNC scaling |
| `--vnc-no-clipboard` | — | Disable VNC clipboard sharing |
| `--vnc-custom-arg` | — | Custom VNC client argument (repeatable) |
| `--spice-tls` | — | Enable SPICE TLS encryption |
| `--spice-ca-cert` | — | SPICE CA certificate path for TLS verification |
| `--spice-skip-cert-verify` | — | Skip SPICE certificate verification (insecure) |
| `--spice-usb-redirection` | — | Enable SPICE USB redirection |
| `--spice-no-clipboard` | — | Disable SPICE clipboard sharing |
| `--spice-image-compression` | — | SPICE image compression: `auto`, `off`, `glz`, `lz`, `quic` |
| `--spice-proxy` | — | SPICE proxy URL (e.g. `http://proxy:3128`) |
| `--spice-shared-folder` | — | SPICE shared folder `NAME:PATH` (repeatable) |
| `--mosh-ssh-port` | — | SSH port for MOSH initial handshake |
| `--mosh-port-range` | — | MOSH UDP port range (e.g. `60000:60010`) |
| `--mosh-server-binary` | — | Path to remote mosh-server binary |
| `--mosh-predict` | — | MOSH prediction mode: `adaptive` (default), `always`, `never` |
| `--mosh-custom-arg` | — | Custom MOSH client argument (repeatable) |
| `--serial-data-bits` | — | Serial data bits: `5`, `6`, `7`, `8` (default) |
| `--serial-stop-bits` | — | Serial stop bits: `1` (default), `2` |
| `--serial-parity` | — | Serial parity: `none` (default), `odd`, `even` |
| `--serial-flow-control` | — | Serial flow control: `none` (default), `hardware`, `software` |
| `--serial-custom-arg` | — | Custom serial client argument (repeatable) |
| `--browser-mode` | — | Web browser mode: `embedded` (default on Linux), `system`, `custom` |
| `--javascript` | — | Enable/disable JavaScript: `true` (default) or `false`; bare `--javascript` = `false` |
| `--user-agent` | — | Custom user agent string (max 512 Unicode characters) |
| `--accept-invalid-certs` | — | Accept invalid TLS certificates: `true`/`false`; bare flag = `true` |
| `--private-mode` | — | Open in private/incognito mode (custom browser) |
| `--zoom-level` | — | Zoom level for embedded browser (0.3–3.0, default: 1.0) |
| `--provider` | — | Zero Trust provider (see [Zero Trust examples](#zero-trust-examples) below) |

**Protocol examples:**

```bash
# SSH (default protocol)
rustconn-cli add -n "Server" -H 10.0.0.1 -P ssh -u admin -k ~/.ssh/id_rsa

# SFTP
rustconn-cli add -n "Files" -H 10.0.0.1 -P sftp -u admin -k ~/.ssh/id_rsa

# RDP
rustconn-cli add -n "Windows" -H 10.0.0.1 -P rdp -u Administrator

# VNC
rustconn-cli add -n "Desktop" -H 10.0.0.1 -P vnc

# SPICE
rustconn-cli add -n "VM" -H 10.0.0.1 -P spice

# Telnet
rustconn-cli add -n "Switch" -H 10.0.0.1 -P telnet

# Serial
rustconn-cli add -n "Router" -H /dev/ttyUSB0 -P serial --baud-rate 9600

# MOSH
rustconn-cli add -n "Mobile" -H 10.0.0.1 -P mosh -u admin

# VNC with advanced settings
rustconn-cli add -n "Desktop" -H 10.0.0.1 -P vnc --vnc-performance speed \
    --vnc-encoding zrle --vnc-compression 9 --vnc-quality 3

# SPICE with TLS and USB redirection
rustconn-cli add -n "VM" -H 10.0.0.1 -P spice --spice-tls \
    --spice-ca-cert /etc/pki/ca.pem --spice-usb-redirection

# MOSH with custom port range and prediction
rustconn-cli add -n "Mobile" -H 10.0.0.1 -P mosh -u admin \
    --mosh-port-range 60000:60010 --mosh-predict always

# Serial with full configuration
rustconn-cli add -n "Router" -H /dev/ttyUSB0 -P serial --baud-rate 9600 \
    --serial-data-bits 8 --serial-parity none --serial-flow-control hardware

# Web (embedded browser with self-signed cert)
rustconn-cli add -n "Proxmox" -H "https://pve.local:8006" -P web --accept-invalid-certs

# Web (system browser, JavaScript disabled)
rustconn-cli add -n "Docs" -H "file:///usr/share/doc/index.html" -P web \
    --browser-mode system --javascript false

# Web (custom user agent, specific zoom)
rustconn-cli add -n "Grafana" -H "https://grafana.internal" -P web \
    --user-agent "RustConn/0.19 Dashboard Monitor" --zoom-level 0.8

# Kubernetes
rustconn-cli add -n "Pod" -H pod-name -P k8s
```

**Zero Trust examples:**

```bash
# AWS SSM
rustconn-cli add -n "EC2" -H i-0123456789 -P zt --provider aws_ssm --aws-region eu-west-1

# GCP IAP
rustconn-cli add -n "GCE" -H instance-1 -P zt --provider gcp_iap --gcp-zone us-central1-a

# Azure Bastion
rustconn-cli add -n "AzVM" -H /subscriptions/.../vm -P zt \
    --provider azure_bastion --resource-group myRG --bastion-name myBastion

# Cloudflare Access
rustconn-cli add -n "CF" -H app.example.com -P zt --provider cloudflare_access -u admin

# Teleport
rustconn-cli add -n "Tele" -H node-1 -P zt --provider teleport

# Tailscale SSH
rustconn-cli add -n "Tail" -H myhost -P zt --provider tailscale_ssh

# HashiCorp Boundary
rustconn-cli add -n "Bound" -H target-host -P zt --provider boundary --boundary-target ttcp_1234

# Hoop.dev
rustconn-cli add -n "Hoop" -H gateway -P zt --provider hoop_dev --hoop-connection-name myconn

# Generic (custom command template)
rustconn-cli add -n "Custom" -H host -P zt --provider generic \
    --custom-command "ssh -o ProxyCommand='...' {host}"
```

Zero Trust provider flags:

| Flag | Providers | Description |
|------|-----------|-------------|
| `--provider` | all | Provider name (required for `-P zt`). One of: `aws_ssm`, `gcp_iap`, `azure_bastion`, `azure_ssh`, `oci_bastion`, `cloudflare_access`, `teleport`, `tailscale_ssh`, `boundary`, `hoop_dev`, `generic` |
| `--aws-profile` | `aws_ssm` | AWS CLI profile |
| `--aws-region` | `aws_ssm` | AWS region |
| `--gcp-zone` | `gcp_iap` | GCP zone |
| `--gcp-project` | `gcp_iap` | GCP project |
| `--resource-group` | `azure_bastion`, `azure_ssh` | Azure resource group |
| `--bastion-name` | `azure_bastion` | Azure Bastion host name |
| `--vm-name` | `azure_ssh` | Azure VM name |
| `--bastion-id` | `oci_bastion` | OCI Bastion OCID |
| `--target-resource-id` | `oci_bastion` | OCI target resource OCID |
| `--target-private-ip` | `oci_bastion` | OCI target private IP |
| `--teleport-cluster` | `teleport` | Teleport cluster name |
| `--boundary-target` | `boundary` | Boundary target ID |
| `--boundary-addr` | `boundary` | Boundary controller URL |
| `--hoop-connection-name` | `hoop_dev` | Hoop.dev connection name |
| `--hoop-gateway-url` | `hoop_dev` | Hoop.dev gateway URL |
| `--hoop-grpc-url` | `hoop_dev` | Hoop.dev gRPC URL |
| `--custom-command` | `generic` | Command template with `{host}`, `{user}`, `{port}` placeholders |

### connect — Initiate a connection

Available only when `rustconn-cli` is built with `--features client-launch` or
`--features full`.

```bash
rustconn-cli connect "Server" [--dry-run]
```

```bash
rustconn-cli connect "My Server"
rustconn-cli connect "My Server" --dry-run   # Show command without executing
```

The `--dry-run` flag prints the exact command that would be executed (e.g. `ssh -p 22 admin@192.168.1.10`), useful for debugging or scripting. For SFTP connections, `connect` prints a hint to use `rustconn-cli sftp` instead.

### show — Show connection details

```bash
rustconn-cli show "Server" [-f table|json|csv]
```

Displays all fields for a connection including protocol-specific configuration (SSH auth method, key path, RDP resolution, serial device settings, monitoring config, Zero Trust provider details).

```bash
rustconn-cli show "My Server"
rustconn-cli show "My Server" --format json          # Full structured JSON (no secrets)
rustconn-cli show "My Server" --format csv           # Key-value CSV pairs
rustconn-cli show 550e8400-e29b-41d4-a716-446655440000   # By UUID
```

The JSON output includes: `id`, `name`, `host`, `port`, `protocol`, `username`, `description`, `group_id`, `group_name`, `tags`, `icon`, `is_pinned`, `created_at`, `updated_at`, `last_connected`, `password_source` (type only, not the value), `domain`, `window_mode`, `skip_port_check`, `session_recording_enabled`, `is_dynamic`, plus protocol-specific fields (`auth_method`, `key_path`, `jump_host`, `resolution`, `device`, `baud_rate`, `provider`, etc.).

When piped (non-TTY stdout), output defaults to JSON automatically.

### update — Update an existing connection

```bash
rustconn-cli update "Server" [--new-name "New Name"] [--host H] [--port P] [--user U] [options...]
```

```bash
rustconn-cli update "My Server" --host 192.168.1.20 --port 2222
rustconn-cli update "My Server" --new-name "Renamed Server"
rustconn-cli update "My Server" --auth-method security-key --key ~/.ssh/id_ed25519_sk
rustconn-cli update "My Server" --icon "⭐"
rustconn-cli update "My Server" --jump-host "Bastion"
rustconn-cli update "My Server" --keep-alive-interval 60 --keep-alive-count 3
rustconn-cli update "My Server" --ssh-verbose
rustconn-cli update "My Server" --mptcp
rustconn-cli update "My Server" --x11-forwarding --agent-forwarding --compression
rustconn-cli update "My Server" --local-forward 8080:localhost:80 --dynamic-forward 1080
rustconn-cli update "My Server" --proxy-command "ncat --proxy 127.0.0.1:9050 --proxy-type socks5 %h %p"
rustconn-cli update "My Server" --ssh-option StrictHostKeyChecking=no
rustconn-cli update "My Server" --tags "production,web" --group "Production"
rustconn-cli update "My Server" --add-tag critical --remove-tag staging
rustconn-cli update "Windows" --ignore-certificate
rustconn-cli update "Windows" --gateway gw.example.com --gateway-port 443
rustconn-cli update "Windows" --resolution 1920x1080 --color-depth 32
rustconn-cli update "Windows" --shared-folder docs:/home/user/Documents
rustconn-cli update "Windows" --remote-app-program "||calc" --remote-app-name "Calculator"
rustconn-cli update "Desktop" --vnc-performance speed --vnc-compression 9 --vnc-view-only
rustconn-cli update "Desktop" --vnc-client-mode external --vnc-custom-arg "-FullScreen"
rustconn-cli update "VM" --spice-tls --spice-ca-cert /etc/pki/ca.pem --spice-usb-redirection
rustconn-cli update "VM" --spice-image-compression glz --spice-shared-folder share:/mnt/data
rustconn-cli update "Mobile" --mosh-port-range 60000:60010 --mosh-predict always
rustconn-cli update "Mobile" --mosh-server-binary /usr/local/bin/mosh-server
rustconn-cli update "Router" --serial-data-bits 7 --serial-parity even --serial-flow-control hardware
```

All flags from `add` are available (except `--protocol`), plus:
- `--new-name` to rename
- `--add-tag` / `--remove-tag` for incremental tag editing
- `--skip-port-check=false` to clear the flag

Only specified fields are changed; unspecified fields remain unchanged.

### delete — Delete a connection

```bash
rustconn-cli delete "Server" [--force]
```

```bash
rustconn-cli delete "My Server"              # Prompts for confirmation
rustconn-cli delete "My Server" --force      # Skip confirmation
```

In non-interactive mode (piped stdin), confirmation is automatically assumed.

### duplicate — Duplicate a connection

```bash
rustconn-cli duplicate "Server" [--new-name "Server Copy"]
```

```bash
rustconn-cli duplicate "My Server"                          # Creates "My Server (copy)"
rustconn-cli duplicate "My Server" --new-name "Staging"     # Custom name
```

The duplicate gets a new UUID, fresh timestamps, and no `last_connected` value.

### test — Test connectivity

```bash
rustconn-cli test "Server" [--timeout 10] [-f table|json|csv]
rustconn-cli test all [--timeout 10] [-f table|json|csv]
```

```bash
rustconn-cli test "My Server"                # Test single connection
rustconn-cli test all                        # Test all connections
rustconn-cli test all --timeout 5            # Custom timeout (seconds, default: 10)
rustconn-cli test all --format json          # Structured JSON with pass_rate and latency
rustconn-cli test "My Server" --format csv   # CSV output
```

Output shows colored pass/fail indicators with latency measurements. When testing all connections, a summary with pass rate is printed at the end. Exit code is `2` if any test fails.

JSON output for `test all` includes:
```json
{
  "total": 5,
  "passed": 4,
  "failed": 1,
  "pass_rate": 80.0,
  "results": [
    {"connection_id": "...", "connection_name": "Server", "success": true, "latency_ms": 42},
    ...
  ]
}
```

When piped (non-TTY stdout), output defaults to JSON automatically.

### sftp — Open SFTP session

```bash
rustconn-cli sftp "Server" [--mc] [--cli]
```

Three modes are available:

```bash
rustconn-cli sftp "My Server"                # Open in file manager (Dolphin/Nautilus/xdg-open)
rustconn-cli sftp "My Server" --cli          # Interactive sftp CLI session
rustconn-cli sftp "My Server" --mc           # Open in Midnight Commander
```

The command automatically manages SSH agent keys before connecting. Only SSH connections are supported; other protocols return an error.

### export — Export connections

```bash
rustconn-cli export -f <format> -o <path> [--csv-delimiter <delim>] [--csv-fields <fields>]
```

```bash
rustconn-cli export -f native -o backup.rcn
rustconn-cli export -f ansible -o inventory.yml
rustconn-cli export -f ssh-config -o config
rustconn-cli export -f remmina -o ~/remmina-export/
rustconn-cli export -f royal-ts -o connections.rtsz
rustconn-cli export -f moba-xterm -o sessions.mxtsessions
rustconn-cli export -f asbru -o asbru.yml
rustconn-cli export -f secure-crt -o ~/securecrt-sessions/
rustconn-cli export -f csv -o connections.csv
rustconn-cli export -f csv -o connections.csv --csv-delimiter semicolon
rustconn-cli export -f csv -o connections.csv --csv-fields "name,host,port,protocol"
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
| `secure-crt` | SecureCRT session format (`.ini` directory) |
| `csv` | CSV format (`.csv`) |

**CSV options** (only valid with `--format csv`):

| Flag | Description |
|------|-------------|
| `--csv-delimiter` | Delimiter: `comma` (default), `semicolon`, `tab` |
| `--csv-fields` | Comma-separated list of fields to include |

### import — Import connections

```bash
rustconn-cli import -f <format> [<file>] [--auto] [--dry-run]
```

```bash
rustconn-cli import -f ssh-config ~/.ssh/config
rustconn-cli import -f remmina ~/remmina/
rustconn-cli import -f native backup.rcn
rustconn-cli import -f ansible inventory.yml
rustconn-cli import -f royal-ts connections.rtsz
rustconn-cli import -f moba-xterm sessions.mxtsessions
rustconn-cli import -f rdp session.rdp
rustconn-cli import -f virt-viewer vm.vv
rustconn-cli import -f libvirt domain.xml
rustconn-cli import -f secure-crt ~/securecrt-sessions/
rustconn-cli import -f csv connections.csv
rustconn-cli import -f ssh-config --auto             # Auto-detect sources
rustconn-cli import -f ssh-config --dry-run file     # Preview without saving
```

| Flag | Description |
|------|-------------|
| `--auto` | Auto-detect available import sources (Asbru-CM, Remmina, SSH config) and import all found |
| `--dry-run` | Show what would be imported without saving changes |

The `--auto` flag scans default locations (`~/.config/asbru-cm/`, `~/.local/share/remmina/`, `~/.ssh/config`) and imports from all detected sources. Duplicates (same name + host) are skipped.

Additional import formats: `rdp` (Microsoft RDP files), `rdm` (Remote Desktop Manager), `virt-viewer` (`.vv` files), `libvirt` (GNOME Boxes / virsh XML), `secure-crt` (SecureCRT `.ini` session directory). Passwords are never included in import/export files — re-enter them after importing.

### wol — Wake-on-LAN

```bash
rustconn-cli wol "Server"
rustconn-cli wol AA:BB:CC:DD:EE:FF
```

```bash
rustconn-cli wol AA:BB:CC:DD:EE:FF                                  # Direct MAC address
rustconn-cli wol "Server Name"                                       # Uses MAC from connection config
rustconn-cli wol AA:BB:CC:DD:EE:FF --broadcast 192.168.1.255 --port 9
```

Three packets are sent with retry. Default broadcast address is `255.255.255.255`, default port is `9`.

### sync — Sync from external inventory

```bash
rustconn-cli sync inventory <file> --source <name> [--remove-stale] [--dry-run]
```

Synchronize connections from an external inventory source (JSON or YAML). Useful for keeping RustConn in sync with a CMDB, NetBox, or Ansible inventory.

```bash
rustconn-cli sync inventory inventory.json --source netbox
rustconn-cli sync inventory inventory.yml --source ansible --remove-stale
rustconn-cli sync inventory inventory.json --source netbox --dry-run
```

| Flag | Description |
|------|-------------|
| `--source` | Source identifier for tagging synced connections (required) |
| `--remove-stale` | Remove connections from this source that are no longer in the inventory |
| `--dry-run` | Show what would change without saving |

The sync report shows added, updated, removed, and skipped counts.

### sync — Cloud Sync operations

Manage Cloud Sync: export Master groups, import from cloud, check sync status.

| Subcommand | Description |
|------------|-------------|
| `sync status` | Show sync directory, device name, and per-group sync status |
| `sync list` | List all synced groups with mode (Master/Import) and last sync time |
| `sync export <group>` | Export a Master group to its sync file |
| `sync import <file>` | Import a `.rcn` sync file as an Import group |
| `sync now` | Export all Master groups and import all Import groups |

```bash
rustconn-cli sync status
rustconn-cli sync list
rustconn-cli sync list --format json
rustconn-cli sync export "Production Servers"
rustconn-cli sync import ~/CloudSync/rustconn/qa-servers.rcn
rustconn-cli sync now
```

### snippet — Manage command snippets

Snippets are reusable command templates with variable substitution. Variables use `${var}` syntax.

| Subcommand | Description |
|------------|-------------|
| `snippet list` | List all snippets (`--format`, `--category`, `--tag`) |
| `snippet show <name>` | Show snippet details and variables |
| `snippet add` | Create a snippet (`--name`, `--command`, `--description`, `--category`, `--tags`) |
| `snippet edit <name>` | Edit a snippet (`--new-name`, `--command`, `--description`, `--category`, `--tags`) |
| `snippet delete <name>` | Delete a snippet |
| `snippet run <name>` | Execute with variable substitution (`--var key=value`, `--execute`) |

```bash
rustconn-cli snippet list
rustconn-cli snippet list --category deploy --tag production
rustconn-cli snippet add --name "Restart" --command "sudo systemctl restart \${service}"
rustconn-cli snippet edit "Restart" --command "sudo systemctl restart \${service} --no-block"
rustconn-cli snippet run "Restart" --var service=nginx             # Preview only
rustconn-cli snippet run "Restart" --var service=nginx --execute   # Actually run
rustconn-cli snippet delete "Old Snippet"
```

The `run` subcommand without `--execute` only prints the expanded command (safe preview). With `--execute`, it runs the command via `sh -c`. Variables referencing Global Variables (`${VARIABLE}`) are automatically resolved before execution; if all variables are resolved, the snippet executes immediately without prompting.

### group — Manage connection groups

| Subcommand | Description |
|------------|-------------|
| `group list` | List all groups (`--format`) |
| `group show <name>` | Show group details, child groups, and connections (`--format`) |
| `group create` | Create a group (`--name`, `--parent`, `--description`, `--icon`) |
| `group edit <name>` | Edit group properties (`--new-name`, `--parent`, `--description`, `--icon`, `--ssh-key-path`, `--ssh-auth-method`, `--ssh-proxy-jump`, `--ssh-agent-socket`, `--add-expect-rule`, `--clear-expect-rules`, `--add-post-login-script`, `--clear-post-login-scripts`) |
| `group delete <name>` | Delete a group |
| `group add-connection` | Add connection to group (`-g group -c connection`) |
| `group remove-connection` | Remove connection from group (`-g group -c connection`) |

```bash
rustconn-cli group list
rustconn-cli group show "Production"                     # Table view
rustconn-cli group show "Production" --format json       # JSON with child groups and connections
rustconn-cli group create --name "Staging"
rustconn-cli group create --name "EU" --parent "Production" --icon "🇪🇺"
rustconn-cli group edit "Staging" --new-name "Staging EU" --icon "🇪🇺" --parent "Production"
rustconn-cli group edit "Production" --ssh-auth-method publickey --ssh-key-path ~/.ssh/id_ed25519
rustconn-cli group edit "Production" --add-expect-rule '{"pattern":"\\[sudo\\] password for \\w+:","response":"${password}\\n","priority":10}'
rustconn-cli group edit "Production" --add-post-login-script "cd /app" --add-post-login-script "source .env"
rustconn-cli group edit "Production" --clear-expect-rules --clear-post-login-scripts
rustconn-cli group add-connection -g "Production" -c "Web-01"
rustconn-cli group remove-connection -g "Production" -c "Web-01"
rustconn-cli group delete "Old Group"
```

Groups support hierarchical nesting via `--parent`.

### template — Manage connection templates

Templates define reusable connection presets.

| Subcommand | Description |
|------------|-------------|
| `template list` | List all templates (`--format`, `--protocol`) |
| `template show <name>` | Show template details |
| `template create` | Create a template (`--name`, `--protocol`, `--host`, `--port`, `--user`, `--description`) |
| `template edit <name>` | Edit a template (`--new-name`, `--host`, `--port`, `--user`, `--description`) |
| `template delete <name>` | Delete a template |
| `template apply <template>` | Create connection from template (`--name`, `--host`, `--port`, `--user`) |

```bash
rustconn-cli template list
rustconn-cli template create --name "SSH Bastion" --protocol ssh --port 2222 --user ops
rustconn-cli template edit "SSH Bastion" --port 2200 --user admin
rustconn-cli template apply "SSH Bastion" --name "Prod Bastion" --host bastion.example.com
rustconn-cli template delete "Old Template"
```

### cluster — Manage connection clusters

Clusters group connections for broadcast command execution (send the same input to all sessions simultaneously).

| Subcommand | Description |
|------------|-------------|
| `cluster list` | List all clusters (`--format`) |
| `cluster show <name>` | Show cluster and its connections |
| `cluster create` | Create a cluster (`--name`, `--connections`, `--broadcast`) |
| `cluster edit <name>` | Edit a cluster (`--new-name`, `--broadcast true/false`) |
| `cluster delete <name>` | Delete a cluster |
| `cluster add-connection` | Add connection (`-C cluster -c connection`) |
| `cluster remove-connection` | Remove connection (`-C cluster -c connection`) |

```bash
rustconn-cli cluster list
rustconn-cli cluster create --name "DB Cluster" --broadcast
rustconn-cli cluster create --name "Mixed" --connections "DB-01,DB-02,Web-01"
rustconn-cli cluster edit "DB Cluster" --new-name "Database Cluster"
rustconn-cli cluster edit "DB Cluster" --broadcast false
rustconn-cli cluster add-connection -C "DB Cluster" -c "DB-01"
rustconn-cli cluster delete "Old Cluster"
```

### var — Manage global variables

Global variables can be referenced in snippets and connection templates. Secret variables are masked in output.

| Subcommand | Description |
|------------|-------------|
| `var list` | List all variables (`--format`) |
| `var show <name>` | Show variable value |
| `var set <name> <value>` | Set a variable (`--secret`, `--description`) |
| `var delete <name>` | Delete a variable |

```bash
rustconn-cli var list
rustconn-cli var set my_var "my_value"
rustconn-cli var set api_key "secret123" --secret                # Masked in output
rustconn-cli var set deploy_env "staging" --description "Current deploy target"
rustconn-cli var delete "my_var"
```

### secret — Manage secret backends

Available only when `rustconn-cli` is built with `--features secret-management`
or `--features full`.

Manage credentials stored in secret backends (system keyring, KeePass, Bitwarden, 1Password, Passbolt, Pass).

| Subcommand | Description |
|------------|-------------|
| `secret status` | Show available backends and their configuration status |
| `secret get <connection>` | Retrieve credentials (`--backend`) |
| `secret set <connection>` | Store credentials (`--user`, `--password`, `--password-stdin`, `--backend`) |
| `secret delete <connection>` | Delete credentials (`--backend`) |
| `secret verify-keepass` | Verify KeePass database (`--database`, `--key-file`) |

```bash
rustconn-cli secret status
rustconn-cli secret get "My Server" --backend keepass
rustconn-cli secret set "My Server" --user admin --backend keyring          # Interactive password prompt
echo "s3cret" | rustconn-cli secret set "My Server" --user admin --password-stdin
rustconn-cli secret delete "My Server"
rustconn-cli secret verify-keepass --database ~/vault.kdbx
```

Backend aliases:

| Backend | Aliases |
|---------|---------|
| System keyring (libsecret) | `keyring`, `libsecret` |
| KeePass KDBX | `keepass`, `kdbx`, `keepassxc` |
| Bitwarden | `bitwarden`, `bw` |
| 1Password | `1password`, `onepassword`, `op` |
| Passbolt | `passbolt` |
| Pass (passwordstore.org) | `pass` |

> **Security note:** Prefer `--password-stdin` (pipe the password via stdin) or the interactive password prompt (omit both flags) over `--password`, which is deprecated because the value is visible in process listings (`/proc/cmdline`).

### smart-folder — Manage smart folders

Smart folders dynamically group connections based on filter criteria.

| Subcommand | Description |
|------------|-------------|
| `smart-folder list` | List all smart folders (`--format`) |
| `smart-folder show <name>` | Show matching connections |
| `smart-folder create` | Create a smart folder (`--name`, `--icon`, `--protocol`, `--host-pattern`, `--tags`) |
| `smart-folder edit <name>` | Edit a smart folder (`--new-name`, `--icon`, `--protocol`, `--host-pattern`, `--tags`; use `"none"` to clear a filter) |
| `smart-folder delete <name>` | Delete a smart folder |

```bash
rustconn-cli smart-folder list
rustconn-cli smart-folder create --name "Production SSH" --protocol ssh --host-pattern "*.prod.*"
rustconn-cli smart-folder create --name "Tagged Web" --tags "web,frontend"
rustconn-cli smart-folder create --name "Rockets" --icon "🚀" --protocol ssh
rustconn-cli smart-folder edit "Production SSH" --host-pattern "*.production.*"
rustconn-cli smart-folder edit "Tagged Web" --tags "none"          # Clear tag filter
rustconn-cli smart-folder edit "Rockets" --icon "none"             # Clear custom icon
rustconn-cli smart-folder show "Production SSH"
rustconn-cli smart-folder delete "Old Folder"
```

### dynamic-folder — Manage dynamic folders

Dynamic folders generate connections by executing a user-defined script.

| Subcommand | Description |
|------------|-------------|
| `dynamic-folder list` | List groups with dynamic folder configuration (`--format`) |
| `dynamic-folder show <name>` | Show configuration and generated connections |
| `dynamic-folder refresh <name>` | Execute the script and update connections |
| `dynamic-folder set <name>` | Create or update dynamic folder (`--script`, `--workdir`, `--timeout`, `--refresh-interval`) |
| `dynamic-folder remove <name>` | Remove dynamic folder and its generated connections |

```bash
rustconn-cli dynamic-folder list
rustconn-cli dynamic-folder list --format json
rustconn-cli dynamic-folder show "AWS Servers"
rustconn-cli dynamic-folder set "AWS Servers" --script 'aws ec2 describe-instances ...' --timeout 60
rustconn-cli dynamic-folder set "K8s Nodes" --script 'kubectl get nodes -o json | jq ...' --refresh-interval 300
rustconn-cli dynamic-folder refresh "AWS Servers"
rustconn-cli dynamic-folder remove "AWS Servers"
```

### recording — Manage session recordings

| Subcommand | Description |
|------------|-------------|
| `recording list` | List all recordings with metadata (`--format`) |
| `recording delete <name>` | Delete a recording (`--force`) |
| `recording import <data_file> <timing_file>` | Import external scriptreplay files |

```bash
rustconn-cli recording list
rustconn-cli recording list --format json
rustconn-cli recording delete "My Session" --force
rustconn-cli recording import session.data session.timing
```

### completions — Generate shell completions

```bash
rustconn-cli completions <shell>
```

```bash
# Bash
rustconn-cli completions bash > ~/.local/share/bash-completion/completions/rustconn-cli

# Zsh
rustconn-cli completions zsh > ~/.local/share/zsh/site-functions/_rustconn-cli

# Fish
rustconn-cli completions fish > ~/.config/fish/completions/rustconn-cli.fish
```

Supported shells: `bash`, `zsh`, `fish`, `elvish`, `powershell`.

### stats — Show connection statistics

```bash
rustconn-cli stats [-f table|json|csv]
```

Shows a summary: total connections by protocol, groups, templates, clusters, snippets, variables, recently used connections (last 7 days), and ever-connected count.

```bash
rustconn-cli stats                   # Human-readable table
rustconn-cli stats --format json     # Structured JSON for scripting
rustconn-cli stats --format csv      # CSV key-value pairs
```

JSON output:
```json
{
  "connections": 42,
  "groups": 5,
  "templates": 3,
  "clusters": 2,
  "snippets": 8,
  "variables": 4,
  "by_protocol": {"ssh": 30, "rdp": 8, "vnc": 4},
  "recently_used_7d": 12,
  "ever_connected": 35
}
```

When piped (non-TTY stdout), output defaults to JSON automatically.

### history — View connection history

```bash
rustconn-cli history list [-f table|json|csv] [-l limit] [-c connection]
rustconn-cli history show <id>
rustconn-cli history clear [--force]
```

| Subcommand | Description |
|------------|-------------|
| `list` | List recent connection history entries |
| `show <id>` | Show details of a specific history entry (by UUID) |
| `clear` | Clear all connection history (prompts for confirmation) |

```bash
rustconn-cli history list                            # Last 20 entries
rustconn-cli history list --limit 50                 # Last 50 entries
rustconn-cli history list --connection "Prod"        # Filter by name
rustconn-cli history list --format json              # JSON output
rustconn-cli history show 550e8400-e29b-...          # Entry details
rustconn-cli history clear --force                   # Clear without prompt
```

### pin / unpin — Pin connections to favorites

```bash
rustconn-cli pin <name>
rustconn-cli unpin <name>
```

Pinned connections appear at the top of the sidebar in the GUI. Pin order is preserved (new pins go to the end).

```bash
rustconn-cli pin "Production DB"                     # Pin to favorites
rustconn-cli unpin "Production DB"                   # Remove from favorites
```

### tag — Manage connection tags

```bash
rustconn-cli tag list [-f table|json|csv]
rustconn-cli tag add -c <connection> -t <tag>
rustconn-cli tag remove -c <connection> -t <tag>
```

| Subcommand | Description |
|------------|-------------|
| `list` | List all tags with usage counts |
| `add` | Add a tag to a connection |
| `remove` | Remove a tag from a connection |

```bash
rustconn-cli tag list                                # All tags with counts
rustconn-cli tag list --format json                  # JSON: {"linux": 5, "prod": 3}
rustconn-cli tag add -c "Server" -t "production"     # Add tag
rustconn-cli tag remove -c "Server" -t "staging"     # Remove tag
```

### move — Move connection to a group

```bash
rustconn-cli move <name> --group <group>
```

Moves a connection to the specified group. Creates the group if it doesn't exist.

```bash
rustconn-cli move "Old Server" --group "Archive"     # Move to existing group
rustconn-cli move "New VM" --group "Cloud"           # Creates "Cloud" if missing
```

### monitor — Manage per-connection monitoring

```bash
rustconn-cli monitor enable <name> [--interval <secs>]
rustconn-cli monitor disable <name>
rustconn-cli monitor metrics <name> [-f table|json|csv]
```

| Subcommand | Description |
|------------|-------------|
| `enable` | Enable monitoring for a connection (optional interval override) |
| `disable` | Disable monitoring for a connection |
| `metrics` | Show monitoring configuration for a connection |

```bash
rustconn-cli monitor enable "Server" --interval 30   # Enable with 30s interval
rustconn-cli monitor disable "Server"                # Disable monitoring
rustconn-cli monitor metrics "Server"                # Show current config
rustconn-cli monitor metrics "Server" --format json  # JSON output
```

---

## Shell Completions

Install tab completions for your shell:

**Bash:**
```bash
rustconn-cli completions bash > ~/.local/share/bash-completion/completions/rustconn-cli
```

**Zsh:**
```bash
rustconn-cli completions zsh > ~/.local/share/zsh/site-functions/_rustconn-cli
```

**Fish:**
```bash
rustconn-cli completions fish > ~/.config/fish/completions/rustconn-cli.fish
```

**Man page:**
```bash
rustconn-cli man-page > ~/.local/share/man/man1/rustconn-cli.1
```

---

## Flatpak Usage

When running RustConn as a Flatpak, the CLI requires the full Flatpak invocation prefix:

```bash
flatpak run --command=rustconn-cli io.github.totoshko88.RustConn list
flatpak run --command=rustconn-cli io.github.totoshko88.RustConn add -n "Server" -H 10.0.0.1
```

Create a shell alias to simplify this:

```bash
# Add to ~/.bashrc, ~/.zshrc, or ~/.config/fish/config.fish
alias rcli='flatpak run --command=rustconn-cli io.github.totoshko88.RustConn'
```

Then use it like the native binary:

```bash
rcli list
rcli add -n "Server" -H 10.0.0.1
```

For shell completions in Flatpak:

```bash
flatpak run --command=rustconn-cli io.github.totoshko88.RustConn completions bash \
    > ~/.local/share/bash-completion/completions/rcli
```

> **Note:** The Flatpak config directory is `~/.var/app/io.github.totoshko88.RustConn/config/rustconn/` instead of `~/.config/rustconn/`.

---

## Scripting Examples

```bash
# Backup all connections to JSON
rustconn-cli list --format json > connections-backup.json

# Test all connections and fail CI if any are unreachable
rustconn-cli test all --timeout 5 || echo "Some connections failed"

# Get structured test results for monitoring
rustconn-cli test all --format json | jq '.results[] | select(.success == false)'

# Show connection details as JSON (useful for AI agents and automation)
rustconn-cli show "Production DB" --format json | jq '{host, port, protocol, username}'

# Get statistics as JSON
rustconn-cli stats --format json | jq '.by_protocol'

# List SSH connections as CSV for processing
rustconn-cli list --format csv --protocol ssh | tail -n +2 | while IFS=, read -r name host port _; do
    echo "Checking $name at $host:$port"
done

# Dry-run a connection to see the command (requires `client-launch` or `full`)
rustconn-cli connect "Production DB" --dry-run

# Get all connections in a group with their details
rustconn-cli group show "Production" --format json | jq '.connections[]'

# Pipe-friendly: auto-JSON when not in terminal
rustconn-cli list | jq '.[].name'                    # Auto-JSON, extract names
rustconn-cli show "Server" | jq '.tags'              # Auto-JSON, extract tags

# Sync from inventory with dry-run first
rustconn-cli sync inventory inventory.yml --source ansible --dry-run
rustconn-cli sync inventory inventory.yml --source ansible --remove-stale
```
