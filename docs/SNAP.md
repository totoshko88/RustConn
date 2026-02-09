# RustConn Snap Package

RustConn is available as a Snap package with **strict confinement** for enhanced security.

## Installation

```bash
sudo snap install rustconn
```

## Strict Confinement

This snap uses strict confinement with embedded Rust protocol implementations. External CLIs (Zero Trust, password managers, SPICE) must be installed on the host system.

### Automatic Interfaces

These interfaces are connected automatically:

- `network` - Network access for connections
- `network-bind` - Listening on network ports
- `audio-playback` - RDP audio playback
- `desktop`, `wayland`, `x11` - GUI access
- `gsettings` - GNOME settings
- `home` - Access to your home directory

### SSH Access

SSH functionality requires the `ssh-keys` interface:

```bash
sudo snap connect rustconn:ssh-keys
sudo snap connect rustconn:ssh-public-keys
```

This grants access to:
- `~/.ssh/` directory (read/write)
- `~/.ssh/config` (read)
- `~/.ssh/known_hosts` (read/write)
- SSH agent socket via `SSH_AUTH_SOCK`

## Embedded Protocol Clients

RustConn uses embedded Rust implementations for protocols:

| Protocol | Implementation | Notes |
|----------|----------------|-------|
| SSH | VTE terminal | Always embedded |
| RDP | IronRDP | Embedded Rust client |
| VNC | vnc-rs | Embedded Rust client |
| Telnet | External `telnet` | VTE terminal session |
| SPICE | — | External only (see below) |

No external protocol clients (xfreerdp, vncviewer) are needed for SSH, RDP, and VNC. Telnet requires the `telnet` client on the host.

## External CLIs (Host-Installed)

For Zero Trust connections, SPICE, and password managers, install CLIs on your host system and connect the appropriate interfaces.

### SPICE Client

SPICE requires external `remote-viewer`:

```bash
# Install on host
sudo apt install virt-viewer  # Debian/Ubuntu
sudo dnf install virt-viewer  # Fedora

# Connect interface
sudo snap connect rustconn:host-spice-client
```

### Zero Trust CLIs

Install the CLIs you need on your host, then connect interfaces:

#### AWS CLI (for AWS SSM)
```bash
# Install: https://docs.aws.amazon.com/cli/latest/userguide/getting-started-install.html
curl "https://awscli.amazonaws.com/awscli-exe-linux-x86_64.zip" -o "awscliv2.zip"
unzip awscliv2.zip && sudo ./aws/install

# Connect interfaces
sudo snap connect rustconn:host-aws-cli
sudo snap connect rustconn:aws-credentials
```

#### Google Cloud CLI (for GCP IAP)
```bash
# Install: https://cloud.google.com/sdk/docs/install
sudo snap install google-cloud-cli --classic
# Or: https://cloud.google.com/sdk/docs/install#deb

# Connect interfaces
sudo snap connect rustconn:host-gcloud-cli
sudo snap connect rustconn:gcloud-credentials
```

#### Azure CLI (for Azure Bastion)
```bash
# Install: https://docs.microsoft.com/en-us/cli/azure/install-azure-cli-linux
curl -sL https://aka.ms/InstallAzureCLIDeb | sudo bash

# Connect interfaces
sudo snap connect rustconn:host-az-cli
sudo snap connect rustconn:azure-credentials
```

#### OCI CLI (for OCI Bastion)
```bash
# Install: https://docs.oracle.com/en-us/iaas/Content/API/SDKDocs/cliinstall.htm
bash -c "$(curl -L https://raw.githubusercontent.com/oracle/oci-cli/master/scripts/install/install.sh)"

# Connect interfaces
sudo snap connect rustconn:host-oci-cli
sudo snap connect rustconn:oci-credentials
```

#### Teleport (tsh)
```bash
# Install: https://goteleport.com/docs/installation/
curl https://goteleport.com/static/install.sh | bash -s

# Connect interface
sudo snap connect rustconn:host-tsh-cli
```

#### Tailscale
```bash
# Install: https://tailscale.com/download/linux
curl -fsSL https://tailscale.com/install.sh | sh

# Connect interface
sudo snap connect rustconn:host-tailscale-cli
```

#### Cloudflare Tunnel
```bash
# Install: https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/downloads/
wget https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-amd64.deb
sudo dpkg -i cloudflared-linux-amd64.deb

# Connect interface
sudo snap connect rustconn:host-cloudflared-cli
```

#### HashiCorp Boundary
```bash
# Install: https://developer.hashicorp.com/boundary/tutorials/oss-getting-started/oss-getting-started-install
wget -O- https://apt.releases.hashicorp.com/gpg | sudo gpg --dearmor -o /usr/share/keyrings/hashicorp-archive-keyring.gpg
echo "deb [signed-by=/usr/share/keyrings/hashicorp-archive-keyring.gpg] https://apt.releases.hashicorp.com $(lsb_release -cs) main" | sudo tee /etc/apt/sources.list.d/hashicorp.list
sudo apt update && sudo apt install boundary

# Connect interface
sudo snap connect rustconn:host-boundary-cli
```

### Password Manager CLIs

#### Bitwarden CLI
```bash
# Install: https://bitwarden.com/help/cli/
sudo snap install bw
# Or: npm install -g @bitwarden/cli

# Connect interfaces
sudo snap connect rustconn:host-bw-cli
sudo snap connect rustconn:bitwarden-config
```

#### 1Password CLI
```bash
# Install: https://developer.1password.com/docs/cli/get-started/
curl -sS https://downloads.1password.com/linux/keys/1password.asc | sudo gpg --dearmor --output /usr/share/keyrings/1password-archive-keyring.gpg
echo "deb [arch=amd64 signed-by=/usr/share/keyrings/1password-archive-keyring.gpg] https://downloads.1password.com/linux/debian/amd64 stable main" | sudo tee /etc/apt/sources.list.d/1password.list
sudo apt update && sudo apt install 1password-cli

# Connect interfaces
sudo snap connect rustconn:host-op-cli
sudo snap connect rustconn:onepassword-config
```

### KeePass Databases

To access KeePass databases stored in common locations:

```bash
sudo snap connect rustconn:keepass-databases
```

This grants read access to:
- `~/Documents/`
- `~/Dropbox/`
- `~/OneDrive/`

**Alternative:** Store your KeePass database in `~/snap/rustconn/current/` which is always accessible.

## Quick Setup

Connect all commonly used interfaces at once:

```bash
# Essential
sudo snap connect rustconn:ssh-keys
sudo snap connect rustconn:ssh-public-keys

# Zero Trust (connect only what you use)
sudo snap connect rustconn:host-aws-cli
sudo snap connect rustconn:aws-credentials
sudo snap connect rustconn:host-gcloud-cli
sudo snap connect rustconn:gcloud-credentials

# SPICE
sudo snap connect rustconn:host-spice-client

# Password managers
sudo snap connect rustconn:keepass-databases
```

## Data Locations

Due to snap confinement, RustConn stores data in snap-specific locations:

- **Connections:** `~/snap/rustconn/current/.local/share/rustconn/`
- **Config:** `~/snap/rustconn/current/.config/rustconn/`
- **SSH keys:** `~/snap/rustconn/current/.ssh/` (managed by snap)
- **Logs:** `~/snap/rustconn/current/.local/share/rustconn/logs/`

## Troubleshooting

### SSH connections fail
- Ensure `ssh-keys` interface is connected
- Check that SSH keys are in `~/.ssh/` or `~/snap/rustconn/current/.ssh/`
- Verify SSH agent is running: `echo $SSH_AUTH_SOCK`

### Zero Trust CLI not found
- Install the CLI on your host system (see instructions above)
- Connect the appropriate `host-*-cli` interface
- Verify CLI is in `/usr/bin/` or `/usr/local/bin/`

### SPICE connection fails
- Install `virt-viewer` package on host
- Connect `host-spice-client` interface

### Permission denied errors
- Check which interfaces are connected: `snap connections rustconn`
- Connect missing interfaces as needed

## Comparison with Other Packages

| Feature | Snap (strict) | Flatpak | Native (.deb/.rpm) |
|---------|---------------|---------|-------------------|
| Security | High | High | Medium |
| Setup | Manual interfaces | Automatic | None needed |
| SSH/RDP/VNC | ✅ Embedded | ✅ Embedded | ✅ Embedded |
| Telnet | Host CLI | Host CLI | ✅ Host CLI |
| SPICE | Host CLI | ❌ | ✅ Host CLI |
| Zero Trust | Host CLIs | ❌ | ✅ Host CLIs |
| Package Size | ~50 MB | ~50 MB | ~30 MB |

**Recommendation:**
- **Snap:** Best for users who want embedded clients with strict security
- **Flatpak:** Best for embedded-only usage (no Zero Trust, no SPICE)
- **Native:** Best for full functionality with all external CLIs

## Support

- **Issues:** https://github.com/totoshko88/RustConn/issues
- **Discussions:** https://github.com/totoshko88/RustConn/discussions

## License

GPL-3.0+ - See LICENSE file for details
