# Bitwarden Secret Backend Setup Guide

This guide covers configuring Bitwarden CLI as a secret backend in RustConn for storing and retrieving connection passwords.

## Prerequisites

- Bitwarden account (cloud or self-hosted)
- Bitwarden CLI (`bw`) installed and accessible
- RustConn v0.9.1 or later

## Step 1: Install Bitwarden CLI

### Flatpak / Snap (recommended for sandboxed installs)

The easiest way to install `bw` inside the sandbox:

1. Open RustConn
2. Go to Menu → **Components...**
3. Find **Bitwarden CLI** in the Password Manager section
4. Click **Install**
5. Wait for download and verification to complete

The CLI is installed to `~/.var/app/io.github.totoshko88.RustConn/cli/bitwarden/bw` (Flatpak) or `~/snap/rustconn/current/cli/bitwarden/bw` (Snap) and is automatically detected by RustConn.

> **Important:** Installing `bw` on the host system (e.g. via `npm install -g @bitwarden/cli`) does NOT make it available inside the sandbox. You must install it through the Components dialog or place the binary manually in the path above.

### Native package (non-sandboxed)

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

## Where the CLI keeps its login state

Read this before running any `bw` command in a terminal. Under Flatpak it is the difference between a login RustConn can see and one it cannot.

`bw` keeps *everything* about your session — which account is logged in, the server URL set by `bw config server`, the encrypted vault cache — in a single `data.json`. It picks the directory for that file in this order:

1. `$BITWARDENCLI_APPDATA_DIR`, if set
2. `$XDG_CONFIG_HOME/Bitwarden CLI`, if `XDG_CONFIG_HOME` is set
3. `$HOME/.config/Bitwarden CLI`

Flatpak sets `XDG_CONFIG_HOME` to a per-application directory, so rule 2 applies to every `bw` RustConn runs itself:

```
~/.var/app/io.github.totoshko88.RustConn/config/Bitwarden CLI/data.json
```

A shell on your host has no `XDG_CONFIG_HOME`, so rule 3 applies there instead and `bw` reads a different file:

```
~/.config/Bitwarden CLI/data.json
```

**A Local Shell tab in the Flatpak build is a host shell**, not a shell inside the sandbox — RustConn runs it through `flatpak-spawn --host` so that you get your real shell with your own dotfiles and tools, and `flatpak-spawn` does not carry the sandbox environment across. Running the sandbox `bw` binary by its absolute path from that tab therefore still writes to the host copy of `data.json`. The binary is the right one; the state it writes is somewhere RustConn does not look.

That is the single most common reason Bitwarden appears broken under Flatpak: `bw login` succeeds in a terminal, `bw unlock` succeeds in the same terminal, and Settings → Secrets still reports **Not logged in**. Both are telling the truth about different files.

You have two ways to make them agree, and Step 2 uses them:

- run `bw` **inside** the sandbox, where rule 2 already points at the right directory, or
- set `BITWARDENCLI_APPDATA_DIR` to that directory explicitly, which overrides the rules on both sides of the boundary.

Snap and native installs are not affected. Under Snap the Local Shell runs inside the same confinement as the application and so inherits the same environment; a native install has no boundary to cross in the first place.

## Step 2: Log in to Bitwarden

The commands in this step are the ones Bitwarden documents, and they are correct for a native install. **On Flatpak, read the [Flatpak](#flatpak) subsection below before running any of them** — the command is the same, but where you run it decides whether RustConn can see the result.

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

### Flatpak

`bw` installed via the Components dialog lives inside the sandbox and is not on your host `PATH`, so the plain `bw login` above will not find it. Use one of the two forms below — both put the login where RustConn reads it, for the reasons in [Where the CLI keeps its login state](#where-the-cli-keeps-its-login-state).

**Run it inside the sandbox.** Nothing to remember, nothing to keep in sync:

```bash
flatpak run --command=sh io.github.totoshko88.RustConn -c \
  '"$HOME/.var/app/io.github.totoshko88.RustConn/cli/bitwarden/bw" login'
```

Use your own terminal application for this, not a Local Shell tab in RustConn. Substitute `login --apikey`, or `config server https://…` followed by `login`, as needed — the same command works for all of them.

**Or point `bw` at the sandbox state directory.** This form works anywhere, including a Local Shell tab:

```bash
export BITWARDENCLI_APPDATA_DIR="$HOME/.var/app/io.github.totoshko88.RustConn/config/Bitwarden CLI"
"$HOME/.var/app/io.github.totoshko88.RustConn/cli/bitwarden/bw" login
```

The `export` has to be repeated in every new shell, and forgetting it silently sends the login to the host copy again. Add it to your shell profile if you expect to do this more than once.

Verify it landed correctly before going near Settings → Secrets:

```bash
BITWARDENCLI_APPDATA_DIR="$HOME/.var/app/io.github.totoshko88.RustConn/config/Bitwarden CLI" \
  "$HOME/.var/app/io.github.totoshko88.RustConn/cli/bitwarden/bw" status
```

`"status":"locked"` with your email in the output means logged in and ready to unlock. `"status":"unauthenticated"` means the login went somewhere else.

### Snap

A Local Shell tab runs inside the same confinement as the application, so it shares the state directory and needs none of the above. Open one (Ctrl+T, or set the startup action to Local Shell) and run the login commands there. RustConn puts the Components install directory on that shell's `PATH`, so a bare `bw` normally resolves; if your shell profile rebuilds `PATH` and it does not, use the absolute path:

```bash
~/snap/rustconn/current/cli/bitwarden/bw login
```

## Step 3: Unlock the vault

Logging in is not enough to read a password — the vault also has to be unlocked, which produces a session key. **Do this from RustConn, not from a terminal:** Settings → Secrets → Bitwarden, enter your master password, click **Unlock**. RustConn holds the resulting key for the rest of the run and reuses it for every vault operation. Combined with **Save master password** below, it unlocks on startup without asking.

Unlocking in a terminal works, but the session key it gives you cannot reach RustConn:

```bash
bw unlock
```

The key it prints belongs to that shell. RustConn reads `BW_SESSION` from its *own* environment, which is fixed when the application starts, so an `export BW_SESSION=…` in a Local Shell tab — or in any shell opened after RustConn — has no effect. The only way to hand over a key that way is to export it before launching, in the environment the application inherits:

```bash
BW_SESSION="your-session-key-here" flatpak run io.github.totoshko88.RustConn
```

That is worth knowing about, not worth doing routinely. A terminal `bw unlock` is still useful for one thing: confirming that the master password and the login state are good, before you go looking for a fault in RustConn.

## Step 4: Configure RustConn

1. Open **Settings** (Ctrl+,)
2. Go to the **Secrets** page
3. Set **Backend** to **Bitwarden**
4. Read the **Status** line

**Status** is the one to read, and it answers the question the **Version** line above it does not. Version says whether `bw` is installed; Status says whether RustConn can actually store and read a password with it right now, using the same state directory the application itself uses. Expect one of:

| Status | Meaning |
|--------|---------|
| **Unlocked** | Ready. |
| **Locked** | Logged in, but the vault needs your master password — use **Unlock** below. |
| **Not logged in** | No account in the state directory RustConn reads. Go back to [Step 2](#step-2-log-in-to-bitwarden); under Flatpak this is almost always the state-directory split. |
| **Not installed** | No `bw` binary found. Install it via Menu → Components. |

If you select Bitwarden while Status is anything but Unlocked or Locked, a banner appears under the header bar after you close Settings, naming the backend and what is missing. It is not a refusal — you can configure ahead of a login — but the choice is no longer accepted in silence.

Unlocking happens here too — that is [Step 3](#step-3-unlock-the-vault). The status shown reflects the state directory RustConn itself uses, which under Flatpak is not the one a host terminal reports; if it says **Not logged in** after a login you believe succeeded, see [Unlock reports "Not logged in"](#unlock-reports-not-logged-in-but-bw-unlock-works-in-a-terminal-flatpak) in Troubleshooting.

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

### Also read from the encrypted file (optional)

With **Also read from the encrypted file** on, RustConn looks in this computer's encrypted credential file as well as in Bitwarden when resolving a password. That is what keeps passwords saved before you switched to Bitwarden working, so leave it on unless you specifically want Bitwarden to be the only place consulted.

It governs **reading only**. Saving a password always targets Bitwarden and nothing else: if the vault refuses the write — locked, not logged in, server unreachable — RustConn tells you so and asks whether to put the password in the encrypted file instead. It will not quietly choose for you, because a password written to a store the connection does not read is indistinguishable from a password that was never saved.

> **Earlier releases behaved differently.** Up to and including 0.21.2 this switch was labelled "Enable fallback", its description named libsecret (which had not been the fallback store for some time), and a save that Bitwarden refused was redirected to the encrypted file with only a transient notice. If you have passwords that Bitwarden does not have but that connections somehow still find, that is where they are; **Copy Passwords…** in Settings ▸ Secrets moves them into the vault.

### Keep the CLI running (optional, faster)

By default RustConn starts a fresh `bw` process for every vault operation and lets it exit afterwards. Each of those processes has to load and decrypt the vault from scratch, which is why a Bitwarden lookup can take several seconds.

With **Keep the CLI running (faster)** on, RustConn instead starts one `bw serve` process when the vault is unlocked and sends every lookup to it. The difference is large:

| Operation | Spawn per call (default) | Kept running |
|-----------|--------------------------|--------------|
| Unlock | ~13 s | ~1 s |
| Retrieve a password from the vault root (56-item vault) | a full vault load per call | under a second |

The switch is **off by default** because of what makes it fast. That one `bw serve` process holds the unlocked vault in its memory for as long as the vault stays unlocked, and it answers requests over a local loopback socket on this computer. With the switch off, the decrypted vault exists only for the few seconds a single `bw` call runs. Turn it on if you trust the local machine and want the speed; leave it off if you would rather not have an unlocked vault resident in a long-lived process.

What you can rely on either way:

- **A lookup never fails for want of it.** If the installed CLI is too old to provide `bw serve`, or the process stops answering, RustConn falls back to spawning `bw` per call on its own, and the lookup goes through at the old speed.
- **The vault does not outlive the session.** Locking the vault or logging out shuts the `bw serve` process down, taking the unlocked vault in its memory with it.
- **Secrets stay off the command line.** When the CLI is kept running, the master password and the items being saved travel in the request body sent to `bw serve`, not as command-line arguments, so they do not show up in a process listing.

## Step 5: Store connection passwords

1. Edit a connection (Ctrl+E or double-click → Edit)
2. In the password field, set the source to **Vault**
3. Enter the password and click **Save**
4. The password is stored in your Bitwarden vault under a "RustConn" folder

To load an existing password from the vault, click the folder icon next to the password field.

## Troubleshooting

### "Failed to run bw: No such file or directory"

The `bw` binary is not found in PATH.

- **Sandbox users (Flatpak/Snap):** Install `bw` via Menu → Components. Host-installed `bw` is not accessible inside the sandbox.
- **Native users:** Verify `bw` is installed: `which bw`. If installed in a non-standard location, ensure it is in your PATH.

### Unlock reports "Not logged in", but `bw unlock` works in a terminal (Flatpak)

The **Unlock** button in Settings → Secrets appears to do nothing, or the label beside it turns to **Not logged in**. With `RUST_LOG=debug` the log line reads:

```
WARN rustconn::dialogs::settings::secrets_tab: Bitwarden GUI: unlock failed raw_stderr=You are not logged in.
```

Meanwhile the same `bw unlock` in a terminal — including a Local Shell tab — reports `Your vault is now unlocked!`.

Settings → Secrets shows **Status: Not logged in** for the same reason.

Both are correct. They are reading different `data.json` files: the login went to the host copy, and RustConn reads the sandbox copy. [Where the CLI keeps its login state](#where-the-cli-keeps-its-login-state) explains why, and Step 2's [Flatpak](#flatpak) section has the login commands that write to the file RustConn reads. Confirm the state directory with the `bw status` command given there — `"status":"unauthenticated"` against the sandbox directory is this problem.

Redo the login using either form in Step 2; there is nothing to migrate. The stray host state at `~/.config/Bitwarden CLI/` is then simply unused. Leave it alone if you also run `bw` on the host for other purposes, since it is that install's state as well.

### "Bitwarden vault is locked"

The vault needs to be unlocked before RustConn can access passwords.

1. Open Settings → Secrets
2. Enter master password and click **Unlock**
3. Or enable "Save to system keyring" for automatic unlock on startup

If **Unlock** reports `Not logged in` rather than `Invalid password`, the vault is not locked — nothing is logged in to it. See the entry above.

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

Under Flatpak, `bw config server` is subject to the same state-directory split as `bw login` — the URL lives in the same `data.json`. Run it the way Step 2's [Flatpak](#flatpak) section shows, or RustConn will keep talking to `bitwarden.com` while your terminal insists the server is configured.

### Auto-unlock fails after restart

If auto-unlock from keyring fails on startup:

1. Check that a Secret Service provider is running (GNOME Keyring, KDE Wallet)
2. Verify `secret-tool` works: `secret-tool search application rustconn`
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
- **Lookup key:** `rustconn/<connection-name>` (falls back to the host when the name is empty)
- **Item name:** `RustConn: <lookup-key>` (e.g. `RustConn: rustconn/my-server`)
- **Username:** connection username
- **Password:** connection password
- **URI:** `rustconn://<lookup-key>` (used as a search marker, exact-match type)
- **Notes:** the connection's domain field, stored as a plain string (empty when no domain is set)

> **Note:** Only the domain is persisted in the notes field. The key passphrase is not stored in Bitwarden — it is handled separately by the SSH key workflow.

When resolving a password, RustConn tries Bitwarden first and then — if **Also read from the encrypted file** is on — this computer's encrypted credential file, so entries saved before you switched backend keep working. Saving never falls back on its own: a write Bitwarden refuses is reported and you choose where it goes.
