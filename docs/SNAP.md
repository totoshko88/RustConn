# RustConn Snap Package

RustConn is available as a Snap package with **strict confinement** for enhanced security.
Both `rustconn` (GUI) and `rustconn-cli` are included.

## Installation

```bash
sudo snap install rustconn
```

## Apps

| App | Command | Description |
|-----|---------|-------------|
| GUI | `rustconn` | GTK4 connection manager |
| CLI | `rustconn.rustconn-cli` | Command-line interface |

```bash
# GUI
rustconn

# CLI
rustconn.rustconn-cli --help
rustconn.rustconn-cli list
rustconn.rustconn-cli add --name myserver --protocol ssh --host 192.168.1.10
```

## Strict Confinement

This snap uses strict confinement with embedded Rust protocol implementations.
External CLIs (Zero Trust, password managers, kubectl) can be downloaded on demand
via the Components dialog (Menu → Components) inside the sandbox — no host access required.

### Automatic Interfaces

These interfaces are connected automatically:

| Interface | Purpose |
|-----------|---------|
| `network` | Network access for connections |
| `network-bind` | Listening on network ports |
| `audio-playback` | RDP audio playback |
| `desktop`, `wayland`, `x11` | GUI access |
| `gsettings` | GNOME settings |
| `home` | Access to home directory |
| `opengl` | GPU rendering |
| `password-manager-service` | D-Bus secret service (GNOME Keyring, KWallet) |

### Manual Interface Connections

These interfaces require manual connection after installation:

```bash
# SSH keys (required for SSH connections)
sudo snap connect rustconn:ssh-keys

# Serial port access (for serial console connections)
sudo snap connect rustconn:serial-port

# Cloud provider credentials
sudo snap connect rustconn:aws-credentials       # ~/.aws (read-write for SSO token cache)
sudo snap connect rustconn:gcloud-credentials     # ~/.config/gcloud (read-only)
sudo snap connect rustconn:azure-credentials      # ~/.azure (read-only)
sudo snap connect rustconn:oci-credentials        # ~/.oci (read-only)

# Kubernetes config
sudo snap connect rustconn:kube-credentials       # ~/.kube (read-only)
```

## Bundled Components

The snap includes all core protocol clients — no separate installation needed:

| Component | Purpose |
|-----------|---------|
| openssh-client | SSH client |
| IronRDP | Embedded RDP client |
| vnc-rs | Embedded VNC client |
| spice-client | Embedded SPICE client |
| inetutils-telnet | Telnet client |
| picocom | Serial console (RS-232/USB) |
| Midnight Commander | SFTP file browser |
| waypipe | Wayland application forwarding over SSH |

## Embedded Protocol Clients

| Protocol | Implementation | Notes |
|----------|----------------|-------|
| SSH | VTE terminal | Always embedded |
| RDP | IronRDP | Embedded; FreeRDP fallback via Components |
| VNC | vnc-rs | Embedded; TigerVNC fallback via Components |
| SPICE | spice-client | Embedded; remote-viewer fallback via Components |
| Telnet | Bundled inetutils | VTE terminal session |
| Serial | Bundled picocom | VTE terminal session; requires `serial-port` interface |
| Kubernetes | Components kubectl | Requires `kube-credentials` |
| SFTP | Bundled mc | Midnight Commander FISH VFS |
| Waypipe | Bundled waypipe | Wayland forwarding over SSH |

### Serial Console

Serial connections use the bundled `picocom` client. Connect the `serial-port` interface:

```bash
sudo snap connect rustconn:serial-port
```

Your user must also be in the `dialout` group for serial device access:
```bash
sudo usermod -aG dialout $USER
# Log out and back in for the change to take effect
```

## External CLIs (On-Demand Download)

External CLIs (Zero Trust providers, password managers, kubectl, FreeRDP, VNC viewer)
are downloaded on demand via the Components dialog (Menu → Components) inside the
sandbox. CLIs install into `$SNAP_USER_DATA/cli/` and are available for connections
automatically — no host access is required.

### Available CLIs

| CLI | Purpose |
|-----|---------|
| `aws` | AWS SSM connections |
| `gcloud` | GCP IAP connections |
| `az` | Azure Bastion connections |
| `oci` | OCI Bastion connections |
| `cloudflared` | Cloudflare Tunnel |
| `tsh` | Teleport |
| `tailscale` | Tailscale |
| `boundary` | HashiCorp Boundary |
| `kubectl` | Kubernetes |
| `bw` | Bitwarden CLI |
| `op` | 1Password CLI |
| `passbolt` | Passbolt CLI |
| `keepassxc-proxy` | KeePassXC proxy |
| `remote-viewer` | SPICE fallback |
| `xfreerdp` | RDP fallback |
| `vncviewer` | VNC fallback |

### Zero Trust CLIs

Download the CLIs you need from the Components dialog, then connect the credentials interfaces:

```bash
# AWS SSM
sudo snap connect rustconn:aws-credentials

# GCP IAP
sudo snap connect rustconn:gcloud-credentials

# Azure Bastion
sudo snap connect rustconn:azure-credentials

# OCI Bastion
sudo snap connect rustconn:oci-credentials

# Kubernetes
sudo snap connect rustconn:kube-credentials
```

For CLI installation instructions, see [INSTALL.md — Zero Trust CLI Tools](INSTALL.md#zero-trust-cli-tools).

### Password Manager CLIs

Download the CLI from the Components dialog. No additional interface connections
are needed for password manager CLIs (they run inside the sandbox).

| Manager | Host package | Notes |
|---------|-------------|-------|
| Bitwarden | `bw` | `npm install -g @bitwarden/cli` or snap: `sudo snap install bw` |
| 1Password | `op` | [1password.com/downloads/command-line](https://1password.com/downloads/command-line/) |
| Passbolt | `go-passbolt-cli` | [passbolt.com](https://www.passbolt.com/) |
| KeePassXC | `keepassxc-proxy` | `keepassxc` package |

## Quick Setup

Connect all commonly used interfaces at once:

```bash
# Essential
sudo snap connect rustconn:ssh-keys

# Serial console
sudo snap connect rustconn:serial-port

# Cloud credentials (connect only what you use)
sudo snap connect rustconn:aws-credentials
sudo snap connect rustconn:gcloud-credentials
sudo snap connect rustconn:azure-credentials
sudo snap connect rustconn:oci-credentials
sudo snap connect rustconn:kube-credentials
```

## Data Locations

Due to snap confinement, RustConn stores data in snap-specific locations:

| Data | Path |
|------|------|
| Connections | `~/snap/rustconn/current/.local/share/rustconn/` |
| Config | `~/snap/rustconn/current/.config/rustconn/` |
| Session logs | `~/snap/rustconn/current/.local/share/rustconn/logs/` |

## Troubleshooting

### SSH connections fail
- Ensure `ssh-keys` interface is connected: `sudo snap connect rustconn:ssh-keys`
- Check that SSH keys are in `~/.ssh/`

### SSH agent does not work (known limitation)

The host SSH agent (`$SSH_AUTH_SOCK`) is **not reachable from the strictly
confined snap**. snapd has no interface that exposes the agent socket
(unlike Flatpak's `--socket=ssh-auth`), so agent-based authentication and
agent forwarding (`ForwardAgent`) cannot work in the snap package.

Workarounds:
- Use key files directly (the `ssh-keys` interface grants read access to
  `~/.ssh/`); passphrases are prompted in the terminal
- If your workflow relies on the SSH agent, prefer the **Flatpak** or
  native package

### Zero Trust / kubectl CLI not found
- Open the Components dialog (Menu → Components) and download the CLI
- Connect credentials: e.g. `sudo snap connect rustconn:aws-credentials`

### Serial port permission denied
- Connect interface: `sudo snap connect rustconn:serial-port`
- Add user to dialout group: `sudo usermod -aG dialout $USER`
- Log out and back in

### Check connected interfaces
```bash
snap connections rustconn
```

## Comparison with Other Packages

| Feature | Snap (strict) | Flatpak | Native (.deb/.rpm) |
|---------|---------------|---------|-------------------|
| Security | High (strict) | High (sandbox) | Medium |
| Setup | Manual interfaces | Automatic | None needed |
| SSH/RDP/VNC/SPICE | Embedded | Embedded | Embedded |
| SSH agent | Not available (no snapd interface) | Host agent via `ssh-auth` socket | Host agent |
| Telnet | Bundled | Bundled | Host CLI |
| Serial | Bundled | Bundled | Host CLI |
| Waypipe | Bundled | Bundled | Host CLI |
| Kubernetes | Components kubectl | Components kubectl (flatpak-spawn) | Host kubectl |
| Zero Trust | Components CLIs | Components CLIs (flatpak-spawn) | Host CLIs |
| Password CLIs | Components CLIs | Components CLIs (flatpak-spawn) | Host CLIs |
| CLI downloads | Components dialog | Components dialog | — |

**Flatpak Components** — Flatpak users can download additional CLI tools (Zero Trust,
password managers, TigerVNC) directly within the sandbox via Menu → Components.
See [User Guide — Flatpak Components](USER_GUIDE.md#flatpak-components) for details.

**Recommendation:**
- **Flatpak:** Recommended for most users. Full functionality with on-demand CLI downloads.
- **Snap:** Good for users who prefer strict confinement; on-demand CLI downloads and manual interface connections.
- **Native:** Full functionality with all host CLIs, no sandboxing overhead.
