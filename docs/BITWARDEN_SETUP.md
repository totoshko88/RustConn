# Bitwarden Secret Backend Setup Guide

This guide covers configuring Bitwarden CLI as a secret backend in RustConn for storing and retrieving connection passwords.

## Prerequisites

- Bitwarden account (cloud or self-hosted)
- Bitwarden CLI (`bw`) installed and accessible
- RustConn v0.9.1 or later

## Step 1: Install Bitwarden CLI

### Flatpak (recommended for Flatpak users)

The easiest way to install `bw` inside the Flatpak sandbox:

1. Open RustConn
2. Go to Menu → **Flatpak Components...**
3. Find **Bitwarden CLI** in the Password Manager section
4. Click **Install**
5. Wait for download and verification to complete

The CLI is installed to `~/.var/app/io.github.totoshko88.RustConn/cli/bitwarden/bw` and is automatically detected by RustConn.

> **Important:** Installing `bw` on the host system (e.g. via `npm install -g @bitwarden/cli`) does NOT make it available inside the Flatpak sandbox. You must install it through Flatpak Components or place the binary manually in the path above.

### Native package (non-Flatpak)

Choose one method:

```bash
# npm (Node.js required)
npm install -g @bitwarden/cli

# Snap
sudo snap install bw

# Direct download
# See https://bitwarden.com/help/cli/
```

Verify installation:

```bash
bw --version
```

## Step 2: Log in to Bitwarden

### Standard login (email + password)

```bash
bw login
```

Follow the interactive prompts for email, master password, and 2FA if enabled.

### Self-hosted server

If you use a self-hosted Bitwarden instance, configure the server URL **before** logging in:

```bash
bw config server https://your-bitwarden-server.example.com
bw login
```

### API key login (for FIDO2, Duo, or automation)

If your account uses 2FA methods not supported by the CLI (FIDO2, Duo), use API key authentication:

1. Log in to Bitwarden web vault
2. Go to **Settings → Security → Keys → API Key**
3. Note your Client ID and Client Secret
4. Log in via CLI:

```bash
bw login --apikey
```

Enter Client ID and Client Secret when prompted.

### Flatpak users

If you installed `bw` via Flatpak Components, you need to run it from within the sandbox. Open a **Local Shell** tab in RustConn (Ctrl+T or set startup action to Local Shell), then run the login commands there. The Flatpak Components installer adds the `bw` path to the Local Shell PATH automatically.

Alternatively, run directly:

```bash
~/.var/app/io.github.totoshko88.RustConn/cli/bitwarden/bw login
```

## Step 3: Unlock the vault

After login, unlock the vault to get a session key:

```bash
bw unlock
```

Copy the `BW_SESSION` value from the output. You can export it in your shell:

```bash
export BW_SESSION="your-session-key-here"
```

> **Note:** You do not need to manually manage the session key. RustConn handles session management automatically through Settings → Secrets.

## Step 4: Configure RustConn

1. Open **Settings** (Ctrl+,)
2. Go to the **Secrets** page
3. Set **Preferred Backend** to **Bitwarden**
4. Check the vault status indicator — it should show the vault state

### Unlock from RustConn UI

Instead of unlocking from the terminal, you can unlock directly in Settings → Secrets:

1. Enter your master password in the Bitwarden section
2. Click **Unlock**
3. The status indicator should turn green

### Save master password (optional, recommended)

To enable automatic vault unlock on startup, choose one option:

- **Save to system keyring** (recommended) — stores the master password in GNOME Keyring / KDE Wallet via `secret-tool`. Requires `libsecret-tools` package (bundled in Flatpak).
- **Save password (encrypted)** — stores the master password encrypted with AES-256-GCM + Argon2id in RustConn settings, tied to a machine-specific key.

Only one option can be active at a time.

### API key authentication (optional)

For accounts with FIDO2/Duo 2FA:

1. Enable **Use API key authentication** in the Bitwarden section
2. Enter **Client ID** and **Client Secret**
3. RustConn will use API key login when the vault session expires

### Enable fallback (optional)

Enable **Enable Fallback** to use libsecret (GNOME Keyring) as a fallback when Bitwarden is unavailable. This ensures passwords can still be resolved if the vault is locked.

## Step 5: Store connection passwords

1. Edit a connection (Ctrl+E or double-click → Edit)
2. In the password field, set the source to **Vault**
3. Enter the password and click **Save**
4. The password is stored in your Bitwarden vault under a "RustConn" folder

To load an existing password from the vault, click the folder icon next to the password field.

## Troubleshooting

### "Failed to run bw: No such file or directory"

The `bw` binary is not found in PATH.

- **Flatpak users:** Install `bw` via Menu → Flatpak Components. Host-installed `bw` is not accessible inside the sandbox.
- **Native users:** Verify `bw` is installed: `which bw`. If installed in a non-standard location, ensure it is in your PATH.

### "Bitwarden vault is locked"

The vault needs to be unlocked before RustConn can access passwords.

1. Open Settings → Secrets
2. Enter master password and click **Unlock**
3. Or enable "Save to system keyring" for automatic unlock on startup

### "secret-tool not found, cannot use system keyring"

The `libsecret-tools` package is not installed.

- **Flatpak:** `secret-tool` is bundled — this should not happen. Report a bug.
- **Debian/Ubuntu:** `sudo apt install libsecret-tools`
- **Fedora:** `sudo dnf install libsecret`
- **Arch:** `sudo pacman -S libsecret`

### Vault shows "unlocked" in UI but operations fail

This can happen when the UI session state and the backend session state are out of sync. Try:

1. Click **Lock** in Settings → Secrets
2. Click **Unlock** again with your master password
3. If the problem persists, restart RustConn

### Self-hosted server not connecting

Ensure you configured the server URL before logging in:

```bash
bw config server https://your-server.example.com
bw login
```

If you logged in before configuring the server, log out and reconfigure:

```bash
bw logout
bw config server https://your-server.example.com
bw login
```

### Auto-unlock fails after restart

If auto-unlock from keyring fails on startup:

1. Check that a Secret Service provider is running (GNOME Keyring, KDE Wallet)
2. Verify `secret-tool` works: `secret-tool search service rustconn`
3. Re-save the master password: Settings → Secrets → toggle "Save to system keyring" off and on
4. If using encrypted settings storage instead of keyring, the password is tied to the machine — it will not work after OS reinstall or major system changes

### API key login fails

1. Verify Client ID format: `user.xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`
2. Verify Client Secret is correct (regenerate in web vault if needed)
3. Check network connectivity to Bitwarden server
4. For self-hosted: ensure the server URL is configured correctly

## Architecture Notes

RustConn stores connection passwords in Bitwarden as individual vault items:

- **Folder:** `RustConn` (created automatically)
- **Item name:** `RustConn: <connection-name>`
- **Username:** connection username
- **Password:** connection password
- **URI:** `rustconn://connection/<connection-uuid>`
- **Notes:** additional credential fields (domain, key passphrase) as JSON

The `SecretManager` tries backends in priority order. If Bitwarden is unavailable and fallback is enabled, libsecret (GNOME Keyring) is used as a fallback.
