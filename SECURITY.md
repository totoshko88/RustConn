# Security Policy

## Reporting a Vulnerability

Please report security issues privately via GitHub Security Advisories
(<https://github.com/totoshko88/RustConn/security/advisories/new>) rather than
in public issues. We aim to acknowledge reports within a few days.

## Release Artifact Provenance

The `.deb`, `.rpm`, AppImage and Flatpak bundle attached to a GitHub release from
0.20.0 onwards carry a signed [SLSA build provenance](https://slsa.dev/) attestation,
issued through Sigstore to the release workflow itself:

```bash
gh attestation verify <file> --repo totoshko88/RustConn
```

**What a pass proves.** The file's digest and filename match a statement GitHub
holds, signed with a short-lived certificate bound to a specific workflow run in
this repository, naming the commit it was built from. An artifact rebuilt elsewhere,
tampered with after the run, or renamed will not verify.

**What it does not prove.** Nothing about the *contents* being free of defects or
malicious code — provenance answers "who built this, from what source", not "is this
source trustworthy". It also does not cover the snap (signed by the Snap Store) or
packages installed from Flathub, OBS, AUR, nixpkgs or Homebrew, which are signed by
whichever repository serves them.

See [docs/INSTALL.md](docs/INSTALL.md#verifying-a-file-downloaded-from-a-github-release)
for the offline variant and further detail.

## Credential Storage — Threat Model

RustConn supports several backends for storing connection credentials. They
differ in the level of protection they provide; choose the one that matches
your threat model.

### Recommended: keyring / vault backends

For real secrets, use one of the integrated secret backends:

- **System keyring** (libsecret / GNOME Keyring / KWallet via `secret-tool`)
- **Vault managers**: Bitwarden, KeePassXC, 1Password, Passbolt

These keep secrets encrypted at rest under a key that is **not** stored next to
the data, and (for the system keyring) unlocked together with your login
session. This is the appropriate choice when defending against an attacker who
can read your files.

### Machine-key encryption — obfuscation at rest, not strong protection

When no vault/keyring backend is configured, the `*_encrypted` fields in the
configuration are encrypted with AES-256-GCM using a **machine key** stored at
`~/.local/share/rustconn/.machine-key` (file mode `0600`).

**What this protects against:** casual disk inspection, shoulder-surfing the
plaintext config, and secrets leaking into backups or synced dotfiles.

**What this does NOT protect against:** an attacker who can read files as the
**same user** — the decryption key sits next to the data by design, so anyone
with read access to your home directory can decrypt these fields. Treat
machine-key encryption as **obfuscation at rest**, not as a security boundary.

> If you store sensitive credentials, use a keyring or vault backend instead of
> relying on machine-key encryption.

## Known Issues

### Passbolt: resource password passed as a command-line argument

The Passbolt backend shells out to the community `go-passbolt-cli` tool. Since
0.25.0 the GPG account passphrase is passed through the child's environment
(`USERPASSWORD`), which only the same user can read. The *resource* password
is still passed as the `--password` argument when RustConn creates or updates
a Passbolt entry, because the CLI offers no stdin or environment input for it.
While that command runs, the password is visible in `/proc/<pid>/cmdline`.

- **Scope:** only during the short-lived `passbolt create/update resource`
  call, which is killed if it outlives its timeout. The password is never
  written to logs or to disk by RustConn, and the CLI's stderr is redacted.
- **Upstream limitation:** we will switch once `go-passbolt-cli` accepts the
  resource password from stdin or the environment.
- **Mitigation:** on a multi-user host, mount `/proc` with `hidepid=2`, or
  prefer another vault backend for saving passwords from RustConn.
