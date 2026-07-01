# Requirements Document

## Introduction

RustConn stores RDP/SSH/VNC credentials through pluggable secret backends. The
default Linux backend ("libsecret") does not use the libsecret C library
directly — it shells out to the `secret-tool` CLI, which talks over D-Bus to an
`org.freedesktop.secrets` Secret Service provider. On desktops where no working
Secret Service provider exists (notably KDE Plasma 6 / Fedora atomic KDE such as
uBlue Aurora, where KWallet historically does not expose the Secret Service and
the Plasma 6 `ksecretd` bridge is new and flaky), `secret-tool store` fails. The
user then sees only a generic, non-actionable dialog ("The password could not be
saved to the vault.") while the real cause is logged but never surfaced. This is
GitHub issue #201.

Two further root causes compound the problem: `LibSecretBackend::is_available()`
only checks that the `secret-tool` binary can be spawned — not that a Secret
Service actually answers — producing a false positive; and the startup
availability warning is skipped entirely for the default LibSecret backend, so
the user gets no proactive signal.

This feature makes RustConn credential storage resilient across every supported
desktop environment, packaging format (Flatpak, Snap, native Debian, macOS
`.app`/`.dmg`), platform (Linux, BSD, macOS) and architecture. The work is
delivered as three incremental, independently shippable parts within release
0.17.6:

- **Part A — Diagnostics and graceful fallback** (highest priority; fixes the
  #201 user experience).
- **Part B — First-class application-managed encrypted-file backend** (a durable
  default for environments without a system keyring).
- **Part C — Migration of the system-keyring path from the `secret-tool`
  subprocess to the in-process `oo7` crate** (quality and ecosystem alignment;
  optional; must not regress other platforms).

Cross-cutting requirements (backward compatibility, crate boundaries, i18n, the
packaging matrix, and testing) apply to all three parts.

## Glossary

- **Secret_Service**: The `org.freedesktop.secrets` D-Bus interface that stores
  secrets on Linux/BSD desktops. Provided by GNOME Keyring, KWallet's bridge,
  `gnome-keyring`, `oo7-daemon`, or similar. RustConn is a client of this
  service, not a provider.
- **secret-tool**: The libsecret command-line utility currently spawned as a
  subprocess by RustConn to read/write the Secret_Service.
- **oo7**: A MIT-licensed, Linux/BSD-only Rust crate providing an in-process,
  async, typed-error client for the Secret_Service and native support for the
  Secret_Portal. KDE is migrating to `oo7-daemon` as its Secret_Service provider.
- **Secret_Portal**: The `org.freedesktop.portal.Secret` XDG Desktop Portal
  interface used by sandboxed applications to obtain a master secret. Its only
  implementation backend is `gnome-keyring`; it therefore does not by itself
  provide secret storage on a host that lacks `gnome-keyring`.
- **LibSecret_Backend**: The RustConn `SecretBackend` implementation in
  `rustconn-core/src/secret/libsecret.rs` that targets the Secret_Service.
- **Keyring_Module**: The shared helper in `rustconn-core/src/secret/keyring.rs`
  used by other backends to store master secrets in the Secret_Service.
- **Encrypted_File_Backend**: The new first-class `SecretBackend` introduced in
  Part B that stores credentials in an application-managed AES-256-GCM encrypted
  file, requiring no system keyring.
- **Machine_Key**: The per-install random key material stored at
  `~/.local/share/rustconn/.machine-key` (file permissions `0600`), used by the
  existing AES-256-GCM credential encryption. Works in Flatpak/Snap sandboxes and
  headless environments.
- **Application_Managed_Encryption**: The existing AES-256-GCM encryption
  (`encrypt_credential` / `decrypt_credential_aes` in
  `rustconn-core/src/config/settings.rs`), keyed by the Machine_Key and using
  Argon2id key derivation. Currently used for Bitwarden/1Password/Passbolt master
  secrets via `*_encrypted` fields.
- **KDBX**: The KeePass database file format used by the file-based, desktop-
  environment-independent KeePass backend.
- **SecretBackend_Trait**: The trait in `rustconn-core/src/secret/backend.rs`
  defining `store`, `retrieve`, `delete`, `is_available`, `backend_id`,
  `display_name`.
- **SecretBackendType**: The enum in `rustconn-core/src/config/settings.rs`
  enumerating selectable backends.
- **Secret_Manager**: The `SecretManager` in `rustconn-core/src/secret/manager.rs`
  that registers backends and applies fallback.
- **Credential_Resolver**: The resolver in `rustconn-core/src/secret/resolver.rs`
  that resolves credentials for a connection, honouring `enable_fallback`.
- **Vault_Operations**: The GUI-side credential save/load orchestration in
  `rustconn/src/vault_ops.rs`, including `generate_store_key`.
- **Settings_Secrets_UI**: The Settings → Secrets tab in the `rustconn` crate
  that lists and selects backends.
- **enable_fallback**: The `SecretSettings` flag (default `true`) that authorises
  falling back from the preferred backend to an alternative.
- **Store_Key**: The flat string key under which a connection's credentials are
  stored in a backend, produced by `generate_store_key`.
- **SecretString**: The `secrecy::SecretString` type used for all in-memory
  secret values.
- **Zeroizing**: The `zeroize::Zeroizing` wrapper used for intermediate plaintext
  buffers so they are wiped on drop.

## Requirements

---

## Part A — Diagnostics and Graceful Fallback

### Requirement 1: Surface the real underlying secret-storage error

**User Story:** As a user on a desktop without a working Secret Service, I want
to see what actually went wrong and what to do about it, so that I can recover
instead of facing a dead-end dialog.

#### Acceptance Criteria

1. WHEN a credential store, retrieve, or delete operation through the
   LibSecret_Backend or the Keyring_Module fails, THE Vault_Operations SHALL
   present an error message that states the underlying cause and at least one
   recovery action the user can take.
2. THE Vault_Operations SHALL render the failure message using an
   `adw::AlertDialog` consistent with the GNOME Human Interface Guidelines
   "explain what happened and what to do" pattern.
3. THE Vault_Operations SHALL produce every user-facing string in the failure
   message through `i18n()` or `i18n_f()`.
4. IF a failure message is produced, THEN THE Vault_Operations SHALL exclude all
   secret values (passwords, passphrases, tokens) from the message text.
5. WHEN the underlying cause is the absence of a responding Secret_Service, THE
   Vault_Operations SHALL include a recovery action that directs the user to the
   Settings_Secrets_UI to select an alternative backend.

### Requirement 2: Real availability probe for the keyring path

**User Story:** As a user, I want RustConn to detect that the system keyring is
genuinely usable before relying on it, so that it does not silently choose a
backend that cannot store secrets.

#### Acceptance Criteria

1. WHEN `is_available()` is evaluated for the LibSecret_Backend, THE
   LibSecret_Backend SHALL perform a probe that confirms a Secret_Service
   responds, rather than only confirming that the `secret-tool` binary can be
   spawned.
2. THE LibSecret_Backend SHALL report an available state only WHEN both the
   client mechanism is present and a Secret_Service responds to the probe.
3. IF the `secret-tool` binary or `oo7` client is present but no Secret_Service
   responds, THEN THE LibSecret_Backend SHALL report an unavailable state.
4. WHEN the availability probe is executed, THE LibSecret_Backend SHALL complete
   the probe within the 5-second `has_secret_backend` timeout bound.
5. THE LibSecret_Backend SHALL distinguish the "client present" condition from
   the "Secret_Service responds" condition in the diagnostic information made
   available to the Vault_Operations and the Settings_Secrets_UI.

### Requirement 3: Automatic fallback to the encrypted-file store on keyring failure

**User Story:** As a user whose system keyring fails, I want my credential to be
saved to the application-managed encrypted store instead of being lost, so that I
do not silently lose my password.

#### Acceptance Criteria

1. WHILE `enable_fallback` is enabled, WHEN a credential store operation through
   the system-keyring path fails, THE Secret_Manager SHALL store the credential
   through the Encrypted_File_Backend.
2. WHILE `enable_fallback` is disabled, IF a credential store operation through
   the system-keyring path fails, THEN THE Secret_Manager SHALL report the
   failure to the Vault_Operations without storing the credential elsewhere.
3. WHEN a credential is stored through the Encrypted_File_Backend as a fallback,
   THE Credential_Resolver SHALL retrieve that credential from the
   Encrypted_File_Backend on the next resolution for the same connection.
4. WHEN a fallback store to the Encrypted_File_Backend succeeds, THE
   Vault_Operations SHALL inform the user that the credential was saved to the
   encrypted-file store instead of the system keyring.

### Requirement 4: Proactive surfacing of availability problems

**User Story:** As a user, I want to be warned at startup and in settings when my
selected secret backend cannot work, so that I can fix it before I need to save a
password.

#### Acceptance Criteria

1. WHEN the application starts, THE Startup_Check SHALL evaluate the availability
   of the preferred backend, including WHERE the preferred backend is the default
   LibSecret_Backend.
2. IF the preferred backend reports an unavailable state at startup, THEN THE
   Startup_Check SHALL present a warning that names the affected backend and
   points to the Settings_Secrets_UI.
3. WHEN the Settings_Secrets_UI is displayed, THE Settings_Secrets_UI SHALL show,
   for each listed backend, whether that backend is currently available.
4. THE Startup_Check SHALL produce every user-facing availability string through
   `i18n()` or `i18n_f()`.

---

## Part B — First-Class Application-Managed Encrypted-File Backend

### Requirement 5: Encrypted-file backend implements the SecretBackend trait

**User Story:** As a user on any platform or desktop, I want a credential store
that needs no system keyring, so that I can save passwords reliably in Flatpak,
Snap, headless, and KDE-only environments.

#### Acceptance Criteria

1. THE Encrypted_File_Backend SHALL implement the SecretBackend_Trait methods
   `store`, `retrieve`, `delete`, `is_available`, `backend_id`, and
   `display_name`.
2. THE Encrypted_File_Backend SHALL encrypt stored credentials using the existing
   Application_Managed_Encryption keyed by the Machine_Key.
3. THE Encrypted_File_Backend SHALL report an available state on Linux, BSD, and
   macOS, including within Flatpak and Snap sandboxes and in headless
   environments.
4. THE Encrypted_File_Backend SHALL expose the display name "Encrypted file — no
   system keyring required" through `display_name()`, wrapped in `i18n()` at the
   UI call site.
5. WHEN a credential previously stored through the Encrypted_File_Backend is
   retrieved, THE Encrypted_File_Backend SHALL return credentials equal to the
   credentials that were stored (round-trip property).

### Requirement 6: Per-connection key scheme and storage location

**User Story:** As a user with many connections, I want each connection's
credential addressed independently and stored in a sandbox-safe location, so that
the encrypted-file store coexists with the other backends.

#### Acceptance Criteria

1. THE Encrypted_File_Backend SHALL address each connection's credentials using
   the connection identifier supplied to the SecretBackend_Trait methods.
2. THE Encrypted_File_Backend SHALL store its data within the XDG data directory
   using paths that remain valid inside Flatpak and Snap sandboxes.
3. THE Encrypted_File_Backend SHALL accept the Store_Key produced by
   `generate_store_key` so that the key used at store time matches the key the
   Credential_Resolver uses at resolve time.
4. THE Encrypted_File_Backend SHALL store its data separately from the
   `config.toml` `*_encrypted` fields used for backend master secrets, so that
   the two storage forms do not collide.
5. WHEN the Encrypted_File_Backend deletes a connection's credentials, THE
   Encrypted_File_Backend SHALL remove only that connection's entry and leave
   other connections' entries intact.

### Requirement 7: Integration with selection, resolver, and manager

**User Story:** As a user, I want to choose the encrypted-file backend in
settings, so that RustConn uses it as my credential store.

#### Acceptance Criteria

1. THE SecretBackendType enum SHALL include a variant that selects the
   Encrypted_File_Backend.
2. THE Settings_Secrets_UI SHALL list the Encrypted_File_Backend as a selectable
   backend in the secrets tab.
3. WHEN the user selects the Encrypted_File_Backend in the Settings_Secrets_UI,
   THE Secret_Manager SHALL route store, retrieve, and delete operations to the
   Encrypted_File_Backend.
4. THE Credential_Resolver SHALL resolve credentials from the
   Encrypted_File_Backend WHEN it is the selected backend.

### Requirement 8: Security and threat-model documentation for the encrypted-file backend

**User Story:** As a security-conscious user, I want the encrypted-file backend
to handle secrets safely and to document its protection limits, so that I can
make an informed choice versus a system keyring.

#### Acceptance Criteria

1. THE Encrypted_File_Backend SHALL hold all in-memory secret values as
   SecretString.
2. THE Encrypted_File_Backend SHALL wrap intermediate plaintext buffers in
   Zeroizing so they are wiped on drop.
3. THE Encrypted_File_Backend SHALL exclude all secret values from log output and
   from error messages.
4. WHEN the Encrypted_File_Backend writes its data file, THE
   Encrypted_File_Backend SHALL set the file permissions to `0600`.
5. THE feature documentation SHALL describe the Encrypted_File_Backend threat
   model, stating that the encryption key resides at rest on the same machine as
   the data and contrasting this with a system keyring.
6. WHEN the `Debug` representation of any new type that holds a secret is
   produced, THE type SHALL exclude the secret value from that representation.

---

## Part C — Migrate the System-Keyring Path to the oo7 Crate

### Requirement 9: Replace the secret-tool subprocess with in-process oo7

**User Story:** As a maintainer, I want the keyring path to use a typed,
in-process client, so that error handling is precise and portal support is
native.

#### Acceptance Criteria

1. WHERE the target platform is not macOS, THE LibSecret_Backend SHALL perform
   Secret_Service operations through the in-process `oo7` crate instead of
   spawning the `secret-tool` subprocess.
2. WHERE the target platform is not macOS, THE Keyring_Module SHALL perform its
   store, lookup, and clear operations through the in-process `oo7` crate instead
   of spawning the `secret-tool` subprocess.
3. THE LibSecret_Backend SHALL map `oo7` typed errors onto the existing
   `SecretError` variants so that callers continue to pattern-match the same
   error categories.
4. THE `oo7`-based code paths SHALL be gated behind
   `#[cfg(not(target_os = "macos"))]`.

### Requirement 10: macOS keychain path remains unaffected

**User Story:** As a macOS user, I want my keychain integration to keep working
unchanged, so that the Linux/BSD migration does not regress my platform.

#### Acceptance Criteria

1. WHERE the target platform is macOS, THE Secret_Manager SHALL continue to use
   the `security-framework` MacOsKeychainBackend.
2. THE `oo7` crate SHALL NOT be compiled into the macOS build.

### Requirement 11: Backward compatibility for already-stored secrets

**User Story:** As an existing user, I want secrets I stored before the migration
to remain usable, so that the upgrade does not lose my credentials.

#### Acceptance Criteria

1. WHEN the `oo7`-based LibSecret_Backend retrieves a credential that was
   previously stored through the `secret-tool` path, THE LibSecret_Backend SHALL
   return the equivalent credential.
2. WHERE the `oo7` migration helper is applicable, THE LibSecret_Backend SHALL
   provide a migration path for existing Secret_Service entries.
3. THE LibSecret_Backend SHALL preserve the existing entry attributes and labels
   so that entries written before and after migration address the same secrets.

### Requirement 12: Update the packaging matrix to drop bundled secret-tool

**User Story:** As a maintainer, I want packaging manifests to reflect the
removed `secret-tool` dependency, so that the bundles stay minimal and correct.

#### Acceptance Criteria

1. WHEN Part C is delivered, THE Flatpak manifest
   `packaging/flathub/io.github.totoshko88.RustConn.yml` SHALL no longer bundle
   `secret-tool`.
2. WHEN Part C is delivered, THE Snap and Debian packaging definitions SHALL no
   longer declare the `secret-tool` dependency.
3. THE Flatpak manifest SHALL retain the D-Bus permissions required for the
   `oo7` client to reach the Secret_Service and the Secret_Portal.
4. THE removal of bundled `secret-tool` SHALL occur only as part of Part C and
   SHALL NOT be applied in Part A or Part B.

### Requirement 13: Supply-chain checks remain green

**User Story:** As a maintainer, I want dependency and licence policy checks to
pass after adding `oo7`, so that the supply chain stays compliant.

#### Acceptance Criteria

1. WHEN `cargo deny` runs after the `oo7` dependency is added, THE supply-chain
   check SHALL report no new policy violations.
2. THE `oo7` dependency SHALL be recorded with a pinned version in the dependency
   manifest.

---

## Cross-Cutting Requirements

### Requirement 14: Backward compatibility and no silent data loss

**User Story:** As an existing user, I want all my previously stored credentials
to keep working or be migrated, so that no upgrade loses data silently.

#### Acceptance Criteria

1. THE feature SHALL keep existing libsecret entries, KDBX databases, and
   `config.toml` `*_encrypted` fields readable after upgrade.
2. IF a credential cannot be stored in the selected backend, THEN the
   Secret_Manager SHALL either store it through an authorised fallback or report
   the failure to the user, and SHALL NOT discard the credential silently.
3. THE feature SHALL preserve the existing serialized form of `SecretSettings`
   so that older configurations load without modification.

### Requirement 15: Crate boundary and safety constraints

**User Story:** As a maintainer, I want all secret logic to stay in the GUI-free
core crate, so that the architecture and safety guarantees hold.

#### Acceptance Criteria

1. THE secret storage logic SHALL reside in `rustconn-core` and SHALL NOT import
   `gtk4`, `adw`, or `vte4`.
2. THE GUI wiring for secret storage SHALL reside in the `rustconn` crate.
3. THE feature SHALL NOT introduce `unsafe` code in any crate other than
   `rustconn-pty-sys`.

### Requirement 16: Internationalisation of new strings

**User Story:** As a non-English user, I want all new messages translated, so
that the diagnostics and settings are usable in my language.

#### Acceptance Criteria

1. THE feature SHALL route every new user-facing string through `i18n()` or
   `i18n_f()` with `{}` placeholders.
2. WHEN new user-facing strings are added, THE build process SHALL regenerate the
   POT template via `po/update-pot.sh`.
3. THE feature SHALL provide updated message catalogue entries for the 16
   supported languages (be, cs, da, de, es, fr, it, kk, nl, pl, pt, sk, sv, uk,
   uz, zh-cn).

### Requirement 17: Packaging matrix coverage per part

**User Story:** As a maintainer, I want each part to state its packaging impact,
so that Flatpak, Snap, Debian, and macOS bundles stay correct as parts ship.

#### Acceptance Criteria

1. THE Part A and Part B deliverables SHALL function within the existing Flatpak,
   Snap, Debian, and macOS packaging without removing the bundled `secret-tool`.
2. THE Part B deliverable SHALL store the Encrypted_File_Backend data within
   paths writable under the Flatpak and Snap sandbox permissions already granted.
3. THE feature SHALL document, per part, which packaging permissions or manifest
   entries are added, retained, or removed.

### Requirement 18: Testing coverage

**User Story:** As a maintainer, I want property and leak tests for the new code,
so that regressions and secret leaks are caught automatically.

#### Acceptance Criteria

1. THE rustconn-core property-test suite SHALL include a round-trip property for
   the Encrypted_File_Backend asserting that retrieving a stored credential
   yields an equivalent credential.
2. THE rustconn-core test suite SHALL include, for each new type that holds a
   secret, a test asserting that the `Debug` representation excludes the secret
   value.
3. THE rustconn-core test suite SHALL include a test asserting that the keyring
   availability probe distinguishes "client present" from "Secret_Service
   responds".
