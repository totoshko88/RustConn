# Implementation Plan

## Overview

Resilient secret storage for release 0.17.6, delivered as three parts:
Part A (diagnostics + graceful fallback), Part B (application-managed
encrypted-file backend), Part C (migrate the system-keyring path to `oo7`).

Implementation order respects the A -> B -> C sequence with one pragmatic
exception: the encrypted-file backend (Part B core, tasks 2.x) is built before
the Part A auto-fallback wiring (tasks 3.x), because the fallback target must
exist first. Diagnostics that need no new backend (tasks 1.x) land Part A value
immediately. All work stays inside `rustconn-core` (logic) and `rustconn` (GUI),
honours the crate boundary and `unsafe`-forbid rules, and routes new strings
through `i18n`.

## Tasks

### Part A — Diagnostics and graceful fallback (diagnostics first)

- [x] 1.1 Add `BackendAvailability` enum and `availability()` trait method
  - Define `BackendAvailability { Available, ClientMissing, ServiceUnavailable }` in `rustconn-core/src/secret/backend.rs`
  - Add `async fn availability(&self) -> BackendAvailability` to `SecretBackend` with a default impl deriving from `is_available()`
  - Re-export the enum from `secret/mod.rs`
  - _Requirements: 2.5, 4.3_

- [x] 1.2 Implement a real Secret Service probe in `LibSecretBackend`
  - Override `availability()` to classify ClientMissing / ServiceUnavailable / Available via a read-only `secret-tool lookup` of a sentinel attribute
  - Reimplement `is_available()` as `availability().await == Available`
  - Add a `// ponytail:` note that Part C replaces the stderr heuristic with oo7's typed error
  - _Requirements: 2.1, 2.2, 2.3, 2.4, 2.5_

- [x] 1.3 Unit-test the availability classifier
  - Pure classifier fn over (spawned, exit_code, stderr) returning `BackendAvailability`
  - Test distinguishes ClientMissing, ServiceUnavailable, Available
  - _Requirements: 18.3_

- [x] 1.4 Actionable save-failure dialog in `vault_ops`
  - Replace `show_vault_save_error_toast()` with `show_vault_save_error(&SecretError)` using `adw::AlertDialog`
  - Body = cause (SecretError Display, no secrets) + recovery action; add a "settings" response opening Settings -> Secrets
  - Wrap all new strings in `i18n()` / `i18n_f()`
  - _Requirements: 1.1, 1.2, 1.3, 1.4, 1.5, 16.1_

- [x] 1.5 Proactive availability surfacing at startup and in settings
  - In `app.rs`, remove the early `return` for the default LibSecret backend so it is also checked
  - Produce distinct `ServiceUnavailable` vs `ClientMissing` warnings pointing to Settings -> Secrets, via `availability()`
  - In `secrets_tab`, show a per-backend availability indicator derived from `availability()`
  - _Requirements: 4.1, 4.2, 4.3, 4.4, 16.1_

### Part B — Application-managed encrypted-file backend

- [x] 2.1 Extract shared local crypto into `secret/local_crypto.rs`
  - Move `encrypt_credential`, `decrypt_credential_aes`, `derive_settings_key`, `get_machine_key`, and the `RCSC` constants out of `config/settings.rs` as `pub(crate)` fns
  - Have `settings.rs` delegate to the new module; preserve the on-disk `RCSC` format and machine-key file location byte-for-byte
  - Return intermediate plaintext as `Zeroizing<Vec<u8>>`
  - _Requirements: 5.2, 8.2, 14.1, 14.3, 15.1, 15.3_

- [x] 2.2 Add a regression test for the preserved crypto format
  - Decrypt a fixed pre-refactor `RCSC` blob (committed as a test fixture) to prove backward compatibility
  - _Requirements: 14.1_

- [x] 2.3 Implement `EncryptedFileBackend` in `secret/encrypted_file.rs`
  - `store`/`retrieve`/`delete`/`is_available`/`backend_id`/`display_name` per the `SecretBackend` trait
  - Per-entry AES-256-GCM blobs in a JSON map at `dirs::data_dir()/rustconn/credentials.enc`; write-temp + rename; chmod `0600`
  - Address entries by the supplied `connection_id` (the flat `generate_store_key` value); delete removes only that key
  - `SecretString` for in-memory secrets; `Zeroizing` intermediates; no secrets in logs/errors; manual or derived non-secret `Debug`
  - Use `tokio::task::spawn_blocking` for file I/O
  - _Requirements: 5.1, 5.2, 5.3, 5.4, 6.1, 6.2, 6.3, 6.4, 6.5, 8.1, 8.2, 8.3, 8.4, 15.1_

- [x] 2.4 Add the `SecretBackendType::EncryptedFile` variant
  - Append the variant (serialized `"encrypted_file"`) so existing configs round-trip unchanged
  - _Requirements: 7.1, 14.3_

- [x] 2.5 Wire `EncryptedFile` into manager, resolver, and store-key generation
  - `SecretManager::build_from_settings`: construct `EncryptedFileBackend` for the new variant
  - `resolver.rs`, `generate_store_key`, `select_backend_for_load`: treat `EncryptedFile` as a flat-key backend
  - _Requirements: 7.3, 7.4, 6.3_

- [x] 2.6 Add `EncryptedFile` to the Settings -> Secrets backend selector
  - Append "Encrypted file (no system keyring)" to the `StringList` and extend the index/enum mapping
  - Wrap the label via `i18n()`
  - _Requirements: 7.2, 5.4, 16.1_

- [x] 2.7 Tests for the encrypted-file backend
  - Round-trip property test (store then retrieve yields equivalent credentials) in `rustconn-core` property tests
  - Debug-leak tests for `EncryptedFileBackend` and `StoredCredentials`
  - Delete-isolation test (deleting one entry leaves others intact)
  - _Requirements: 18.1, 18.2, 5.5, 6.5_

- [x] 2.8 Document the encrypted-file threat model
  - Add a section to `docs/ZERO_TRUST.md` describing at-rest key location and the trade-off vs a system keyring
  - _Requirements: 8.5, 17.3_

### Part A (continued) — Auto-fallback wiring (depends on Part B core)

- [x] 3.1 Add `StoreOutcome` and `store_reported` to `SecretManager`
  - Define `StoreOutcome { Primary, Fallback { backend_id } }`
  - `store_reported(id, creds, allow_fallback)` tries the preferred backend, then the encrypted-file fallback when authorised; reports which stored
  - Reimplement `store()` to delegate and discard the outcome (unchanged contract)
  - When `allow_fallback` is false and the primary fails, return the original error unchanged
  - _Requirements: 3.1, 3.2, 14.2_

- [x] 3.2 Register `EncryptedFileBackend` as the terminal fallback
  - In `build_from_settings`, append `EncryptedFileBackend` as the last fallback when `enable_fallback` is set (replacing the useless self-LibSecret fallback)
  - Ensure the resolver retrieves fallback-stored credentials on the next resolution
  - _Requirements: 3.1, 3.3_

- [x] 3.3 Surface fallback result in the GUI
  - In `vault_ops`, call `store_reported`; on `Fallback`, show an `adw::Toast` stating the credential was saved to the encrypted file because the keyring was unavailable
  - Wrap strings via `i18n()`
  - _Requirements: 3.4, 16.1_

- [x] 3.4 Fallback unit test
  - Stub primary backend that always errors + real `EncryptedFileBackend`; assert outcome `Fallback { "encrypted_file" }` and the credential is retrievable
  - _Requirements: 3.1, 3.3_

### Part C — Migrate the system-keyring path to oo7

- [x] 4.1 Add the `oo7` dependency (Linux/BSD only)
  - Add `oo7` to `[workspace.dependencies]` with a pinned version and `features = ["tokio", "native_crypto", "tracing"]`, `default-features = false`
  - Reference it from `rustconn-core` under `[target.'cfg(not(target_os = "macos"))'.dependencies]`
  - _Requirements: 9.4, 10.2, 13.2_

- [x] 4.2 Replace `secret-tool` in `LibSecretBackend` with `oo7::dbus::Service`
  - `#[cfg(not(target_os = "macos"))]` implementations of store/retrieve/delete via `Collection::create_item` / `search_items` / `Item::delete`
  - Preserve existing attributes (`application=rustconn`, `connection_id`, `key`) and labels for backward compatibility
  - Convert the `availability()` probe to `oo7::dbus::Service::new()` typed result (removes the stderr heuristic)
  - _Requirements: 9.1, 9.4, 11.1, 11.3, 2.1_

- [x] 4.3 Replace `secret-tool` in `keyring.rs` with `oo7`
  - `#[cfg(not(target_os = "macos"))]` store/lookup/clear via `oo7`, preserving the `application`/`key` attributes
  - _Requirements: 9.2, 9.4, 11.1_

- [x] 4.4 Map `oo7` errors onto `SecretError`
  - service unreachable -> `BackendUnavailable`; create/update -> `StoreFailed`; search -> `RetrieveFailed`; delete -> `DeleteFailed`; other -> `LibSecret`
  - _Requirements: 9.3_

- [x] 4.5 Route macOS off `LibSecretBackend`
  - In `build_from_settings`, replace every `LibSecretBackend::default_app()` arm with `MacOsKeychainBackend` under `#[cfg(target_os = "macos")]`
  - Gate `LibSecretBackend` and `keyring.rs` oo7 code fully behind `cfg(not(macos))` so `oo7` never compiles on macOS
  - _Requirements: 10.1, 10.2_

- [x] 4.6 Remove bundled libsecret/secret-tool from Flatpak manifests
  - Delete the `libsecret` build module from `packaging/flathub/...yml`, `packaging/flatpak/...yml`, and `...local.yml`
  - Retain `--talk-name=org.freedesktop.secrets`, `org.kde.kwalletd5`, `org.kde.kwalletd6`
  - _Requirements: 12.1, 12.3, 12.4_

- [x] 4.7 Update Snap/Debian packaging prose
  - Update description text referencing libsecret; confirm no actual `secret-tool` runtime dependency remains
  - _Requirements: 12.2, 12.4_

- [x] 4.8 Run `cargo deny` and pin the dependency
  - Run the supply-chain check after adding `oo7`; resolve any new advisories/licence/ban findings
  - _Requirements: 13.1, 13.2_

### Cross-cutting finalization

- [x] 5.1 Regenerate translations
  - Run `bash po/update-pot.sh`; `msgmerge --update` the 16 language catalogues; add `uk` translations and stubs for the rest
  - _Requirements: 16.2, 16.3_

- [x] 5.2 Quality gate
  - Run the `rust-quality-check` sub-agent (fmt + clippy 0 warnings + `cargo test --workspace`) and fix findings
  - Verify no `gtk4`/`adw`/`vte4` imports entered `rustconn-core`, and no new `unsafe`
  - _Requirements: 15.1, 15.2, 15.3, 18.1, 18.2, 18.3_

- [x] 5.3 Update CHANGELOG for 0.17.6
  - Summarize Parts A/B/C and the #201 fix under the 0.17.6 entry
  - _Requirements: 14.1_

## Task Dependency Graph

```json
{
  "waves": [
    { "wave": 1, "tasks": ["1.1", "2.1"], "dependsOn": [] },
    { "wave": 2, "tasks": ["1.2", "1.4", "2.2", "2.3"], "dependsOn": ["1.1", "2.1"] },
    { "wave": 3, "tasks": ["1.3", "1.5", "2.4", "2.7"], "dependsOn": ["1.2", "2.3"] },
    { "wave": 4, "tasks": ["2.5", "2.8"], "dependsOn": ["2.4"] },
    { "wave": 5, "tasks": ["2.6", "3.1"], "dependsOn": ["2.5", "2.3"] },
    { "wave": 6, "tasks": ["3.2", "3.4"], "dependsOn": ["3.1", "2.5"] },
    { "wave": 7, "tasks": ["3.3"], "dependsOn": ["3.2"] },
    { "wave": 8, "tasks": ["4.1"], "dependsOn": [] },
    { "wave": 9, "tasks": ["4.2", "4.8"], "dependsOn": ["4.1"] },
    { "wave": 10, "tasks": ["4.3", "4.5"], "dependsOn": ["4.2"] },
    { "wave": 11, "tasks": ["4.4", "4.6"], "dependsOn": ["4.3", "4.5"] },
    { "wave": 12, "tasks": ["4.7"], "dependsOn": ["4.6"] },
    { "wave": 13, "tasks": ["5.1"], "dependsOn": ["1.4", "1.5", "2.6", "3.3"] },
    { "wave": 14, "tasks": ["5.2"], "dependsOn": ["5.1", "4.7", "4.8"] },
    { "wave": 15, "tasks": ["5.3"], "dependsOn": ["5.2"] }
  ]
}
```

Notes on the graph: tasks 1.1–1.5 (Part A diagnostics) and 2.1–2.8 (Part B) are
independent and can proceed in parallel. Tasks 3.x (Part A auto-fallback) require
2.3 and 2.5. Part C (4.x) is independent of A/B at the code level but is
sequenced last per the A->B->C release plan; 4.2 depends on 1.2 only in that it
supersedes the probe implementation. Finalization (5.x) runs after all feature
tasks.

## Notes

- Each part is independently shippable; Part A diagnostics (1.x) can land before
  Part B, but the Part A auto-fallback (3.x) needs the Part B backend.
- Only Part C touches packaging manifests and adds a dependency; Parts A and B
  keep Flatpak/Snap/Debian/macOS bundles unchanged.
- All secret logic stays in `rustconn-core` (no `gtk4`/`adw`/`vte4`); GUI wiring
  stays in `rustconn`. No new `unsafe` anywhere.
- The `oo7` path is `cfg(not(target_os = "macos"))`; macOS keeps
  `security-framework` and never compiles `oo7`.
- Run tests only at task 2.2, 2.7, 3.4 (targeted) and the 5.2 quality gate; avoid
  re-running the full suite mid-part per the project test-run rules.
- After new i18n strings (1.4, 1.5, 2.6, 3.3), task 5.1 regenerates the POT and
  the 16 catalogues.
