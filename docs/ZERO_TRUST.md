# RustConn Zero Trust Providers

**Version 0.15.4** | Identity-aware proxy integrations for RustConn

RustConn supports connecting through identity-aware proxy services (Zero Trust). Instead of direct SSH/RDP to a host, the connection is tunneled through a provider's CLI tool that handles authentication and authorization.

For the main user guide, see [USER_GUIDE.md](USER_GUIDE.md). For CLI usage, see [CLI_REFERENCE.md](CLI_REFERENCE.md).

---

## Setup

1. Create or edit a connection (Ctrl+N or Ctrl+E)
2. Go to the **Zero Trust** tab
3. Select your provider from the dropdown
4. Fill in the provider-specific fields
5. Optionally add custom CLI arguments in the **Advanced** section

The Zero Trust tab is available for SSH connections. RustConn constructs the appropriate CLI command and runs it in a VTE terminal.

When selecting a provider, RustConn checks if the required CLI tool is available on PATH. If not found, a warning is displayed with instructions to install the tool via the **Components** dialog (Settings → Components).

> **Sandbox note (Flatpak & Snap):** In both sandboxed distributions, host binaries are not directly accessible. CLI tools are downloaded on demand into the app's writable data directory via the Components dialog. Cloud credential directories (`~/.aws`, `~/.config/gcloud`, `~/.azure`, `~/.oci`, `~/.kube`) are accessed read-only through sandbox permissions:
> - **Flatpak:** `--filesystem=home/.aws` etc. in the manifest (granted at install).
> - **Snap:** `personal-files` plugs (manual connection required): `sudo snap connect rustconn:aws-credentials`, `sudo snap connect rustconn:gcloud-credentials`, etc.
>
> RustConn automatically redirects writable config to a sandbox-internal directory and bootstraps credentials from the host mount on first use.

---

## Providers

### AWS Session Manager

Connects via `aws ssm start-session`. Requires the AWS CLI and Session Manager plugin.

| Field | Description | Example |
|-------|-------------|---------|
| Instance ID | EC2 instance ID | `i-0abc123def456` |
| AWS Profile | Named profile from `~/.aws/credentials` | `default`, `production` |
| Region | AWS region | `us-east-1` |

**Prerequisites:** `aws` CLI, `session-manager-plugin`, configured AWS credentials.

**macOS installation:**
```bash
brew install awscli
brew install --cask session-manager-plugin
```

The Session Manager plugin is **not** bundled with the AWS CLI on macOS and must be installed separately. Without it, `aws ssm start-session` will fail silently or report that the plugin is not found.

### GCP IAP Tunnel

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

### Azure Bastion

Connects via `az network bastion ssh`. Requires the Azure CLI with bastion extension.

| Field | Description | Example |
|-------|-------------|---------|
| Target Resource ID | Full ARM resource ID of the target VM | `/subscriptions/.../vm-name` |
| Resource Group | Resource group containing the Bastion | `my-rg` |
| Bastion Name | Name of the Bastion host | `my-bastion` |

**Prerequisites:** `az` CLI, `az extension add --name bastion`, authenticated (`az login`).

**Flatpak:** The Flatpak sandbox mounts `~/.azure/` as read-only to share your host credentials. RustConn automatically redirects Azure CLI's writable config to `~/.var/app/io.github.totoshko88.RustConn/config/azure/` via the `AZURE_CONFIG_DIR` environment variable. On first use, credential files (`azureProfile.json`, `msal_token_cache.json`, etc.) are bootstrapped from the host mount.

### Azure SSH (AAD)

Connects via `az ssh vm` using Azure Active Directory authentication. No SSH keys needed.

| Field | Description | Example |
|-------|-------------|---------|
| VM Name | Azure VM name | `my-vm` |
| Resource Group | Resource group containing the VM | `my-rg` |

**Prerequisites:** `az` CLI, `az extension add --name ssh`, AAD-enabled VM, authenticated.

**Flatpak:** Same as Azure Bastion — `AZURE_CONFIG_DIR` is redirected automatically.

### OCI Bastion

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

### Cloudflare Access

Connects through Cloudflare Zero Trust tunnel.

| Field | Description | Example |
|-------|-------------|---------|
| Hostname | Cloudflare Access hostname | `ssh.example.com` |

**Prerequisites:** `cloudflared` installed, Cloudflare Access application configured for the hostname.

**Flatpak:** Cloudflare Access SSH uses browser-based authentication with short-lived tokens — no persistent config directory is needed for the SSH proxy use case. Install `cloudflared` via Flatpak Components.

### Teleport

Connects via Gravitational Teleport.

| Field | Description | Example |
|-------|-------------|---------|
| Node Name | Teleport node name | `web-01` |
| Cluster | Teleport cluster name (optional) | `production` |

**Prerequisites:** `tsh` CLI, authenticated (`tsh login`).

**Flatpak:** The host `~/.tsh/` directory is not mounted. RustConn redirects Teleport's config directory to `~/.var/app/io.github.totoshko88.RustConn/config/tsh/` via the `TELEPORT_HOME` environment variable. Run `tsh login` from a RustConn Local Shell after installing Teleport via Flatpak Components.

### Tailscale SSH

Connects via Tailscale's built-in SSH.

| Field | Description | Example |
|-------|-------------|---------|
| Tailscale Host | Machine name or Tailscale IP | `my-server` or `100.64.0.1` |

**Prerequisites:** `tailscale` installed and connected (`tailscale up`), SSH enabled on the target node.

### HashiCorp Boundary

Connects via HashiCorp Boundary proxy.

| Field | Description | Example |
|-------|-------------|---------|
| Target ID | Boundary target identifier | `ttcp_1234567890` |
| Controller Address | Boundary controller URL | `https://boundary.example.com` |

**Prerequisites:** `boundary` CLI, authenticated (`boundary authenticate`).

**Flatpak:** Boundary uses system keyring via D-Bus for credential storage, which works natively in the Flatpak sandbox. Install Boundary via Flatpak Components.

### Hoop.dev

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

**Flatpak:** The host `~/.hoop/` directory is **not** mounted by default (the permission was rejected by Flathub lint). To share Hoop.dev authentication tokens and configuration with the sandbox, grant access manually after installation:
```bash
flatpak override --user --filesystem=home/.hoop:ro io.github.totoshko88.RustConn
```
Alternatively, authenticate from a RustConn Local Shell, or pass credentials via the `HOOP_*` environment variables described above. Install `hoop` via Flatpak Components if not available on the host.

### Generic Command

For providers not listed above. Enter a custom command template that RustConn will execute.

| Field | Description | Example |
|-------|-------------|---------|
| Command Template | Full command to execute | `my-proxy connect my-host` |

The command template is run through a shell (`sh -c`), so standard shell syntax works (pipes, environment variables, quoting). Any **Additional CLI arguments** from the Advanced section are appended to the template before execution.

> **Note:** The command template is **not** processed for RustConn placeholders such as `${host}`, `${user}`, or `${port}` — it is passed to the shell verbatim. If you reference `${host}` it will expand as an empty shell variable, not the connection's host. Enter the literal host/user/port values directly in the command.

#### Running Local Programs

Generic Command can be used to launch **any program** on your local computer inside a RustConn terminal tab — not just network tunnels. This is useful for CLI tools that provide their own interactive TUI or shell session.

**Example — Kiro CLI:**

| Field | Value |
|-------|-------|
| Name | kiro-cli |
| Command Template | `kiro-cli` |

**Example — local Docker shell:**

| Field | Value |
|-------|-------|
| Name | Docker dev container |
| Command Template | `docker exec -it my-container /bin/bash` |

**Example — tmux session:**

| Field | Value |
|-------|-------|
| Name | tmux main |
| Command Template | `tmux new-session -A -s main` |

#### Flatpak Compatibility

When RustConn runs as a Flatpak, Generic commands are **automatically executed on the host** via `flatpak-spawn --host` with a proper PTY allocation (same mechanism as Local Shell). No manual `flatpak-spawn` prefixes are needed — simply enter the command as you would on a native install.

This means any program installed on your system (e.g., `kiro-cli`, `docker`, `tmux`, `htop`) works transparently regardless of whether RustConn is installed as a native package or Flatpak.

---

## Custom Arguments

All providers support an **Additional CLI arguments** field in the Advanced section. These arguments are appended to the generated command. Use this for provider-specific flags not covered by the UI fields.

---

## Secret Storage: Encrypted-File Backend (Threat Model)

RustConn stores connection credentials in a system keyring whenever one is available — GNOME Keyring / KDE Wallet via the Secret Service on Linux, or the Keychain on macOS. When no working keyring is present (headless servers, minimal desktops, some sandboxed environments), RustConn falls back to an **application-managed encrypted file** so that credentials still work everywhere. This section documents how that fallback stores data at rest and, importantly, how its security compares to a real keyring.

### Where credentials live

Credentials are written to `credentials.enc` in the app data directory (`dirs::data_dir()/rustconn/credentials.enc`). The file is a JSON map of `{ connection_id: base64(blob) }`, with each connection's secret independently encrypted. The file is written atomically (write to a temp file, then rename) and set to mode `0600` (owner read/write only) on Unix.

### How encryption works

Each entry is sealed with **AES-256-GCM**. The encryption key is derived with **Argon2id** (16 MiB memory, 2 iterations, 1 thread, version `0x13`) from a machine-specific key. Each blob is self-describing:

```
magic "RCSC" + version + 16-byte salt + 12-byte nonce + ciphertext + 16-byte GCM tag
```

The salt and nonce are unique per entry, and the GCM tag provides authenticated encryption (tampering is detected on decrypt).

### Where the at-rest key lives

The machine-specific key comes from an app-specific key file at `dirs::data_dir()/rustconn/.machine-key`, created with mode `0600`. This path works inside Flatpak and Snap sandboxes, where `/etc/machine-id` is not always reachable. When a host machine ID is available, it is mixed in via HKDF-SHA256. In all cases the resulting key material lives **on the same disk as `credentials.enc`**, readable by any process running as the same user.

### The trade-off versus a system keyring

This is the key point, and we want to be honest about it:

- A **system keyring** (GNOME Keyring, KDE Wallet, macOS Keychain) can keep the master key tied to your login session and protected in a separately-encrypted store, so the secrets are not trivially readable just because someone can read your files.
- The **encrypted-file backend** keeps its key on the same disk as the data it protects. It defends against **casual at-rest disclosure** — a stolen disk image, a leaked backup, a copied config directory, or another user account on the same machine — because without the key file the ciphertext is useless. It does **not** defend against a **compromised user account or malware running as you**: anything that can read your data directory can read both `credentials.enc` and `.machine-key` and decrypt your secrets.

In short, the encrypted-file backend is the resilient fallback that guarantees credentials work on systems with no usable keyring. It is **not** a stronger alternative to a keyring — where a working keyring (or KeePassXC with its own master password) is available, prefer it.
