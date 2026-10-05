//! Vault credential operations
//!
//! Functions for saving, loading, renaming, deleting, and copying credentials
//! in the configured secret backend (KeePass, libsecret, Bitwarden, 1Password,
//! Passbolt, Pass). Extracted from `state.rs` to reduce module complexity.

/// Returns the name to show a user for `backend`, translated.
///
/// One helper rather than `i18n(backend.display_name())` at each call site,
/// because the reason it is safe needs saying once: `po/update-pot.sh` extracts
/// translatable strings by matching `i18n("…")` on a *literal*, so this call
/// contributes nothing to the catalogue. It does not need to — the only two
/// backend names that are words rather than product names, "Encrypted file" and
/// "Portable encrypted file", are extracted from the literals in
/// `secrets_tab::backend_choices`, and gettext looks up whatever string it is
/// handed at runtime. A name added to `display_name()` and nowhere else would
/// appear untranslated; the test in `secrets_tab` is what keeps the two in step.
pub fn backend_display_name(backend: rustconn_core::config::SecretBackendType) -> String {
    crate::i18n::i18n(backend.display_name())
}

/// Reports that the portable credential file could not be written because it is
/// locked, or because the passphrase in force does not open it.
///
/// `wrong_passphrase` picks between the two: they need different next steps, and
/// telling someone to "enter the passphrase" when they already have one entered
/// and wrong is the kind of advice that gets a bug report.
///
/// The suggested action opens Settings ▸ Secrets rather than the unlock dialog
/// directly. The unlock dialog hands its result to `AppState::unlock_portable_store`,
/// and this function is reached from `spawn_blocking_with_callback` on paths that
/// hold no state handle — routing through Settings keeps one place responsible
/// for recording a verified passphrase instead of adding a second.
fn show_portable_locked_error(wrong_passphrase: bool) {
    use gtk4::prelude::*;
    use libadwaita as adw;
    use libadwaita::prelude::*;

    gtk4::glib::idle_add_local_once(move || {
        let Some(window) = gtk4::gio::Application::default()
            .and_then(|app| app.downcast_ref::<gtk4::Application>().cloned())
            .and_then(|app| app.active_window())
        else {
            tracing::warn!("Could not show portable store lock dialog: no active window");
            return;
        };

        let (heading, body) = if wrong_passphrase {
            (
                crate::i18n::i18n("Passphrase Does Not Open the Portable File"),
                crate::i18n::i18n(
                    "The password was not saved. The passphrase in use does not decrypt the portable credential file. Open Settings, then Secrets, and enter the passphrase the file was created with.",
                ),
            )
        } else {
            (
                crate::i18n::i18n("Portable File Is Locked"),
                crate::i18n::i18n(
                    "The password was not saved because the portable credential file has not been unlocked in this session. Open Settings, then Secrets, and enter its passphrase.",
                ),
            )
        };

        let dialog = adw::AlertDialog::new(Some(&heading), Some(&body));
        dialog.add_response("close", &crate::i18n::i18n("Close"));
        dialog.add_response("settings", &crate::i18n::i18n("Open Settings"));
        dialog.set_response_appearance("settings", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("settings"));
        dialog.set_close_response("close");

        let window_for_action = window.clone();
        dialog.connect_response(None, move |_, response| {
            if response == "settings" {
                let _ = gtk4::prelude::WidgetExt::activate_action(
                    &window_for_action,
                    "win.settings",
                    None,
                );
            }
        });

        dialog.present(Some(&window));
    });
}

/// Confirms that a credential went to `destination` after the user chose it there.
///
/// This is a result notification for an action the user just approved, so it is a
/// toast rather than a dialog, and `Info` rather than `Warning` — nothing is
/// degraded or unexpected at this point.
///
/// It replaces a toast that fired after a *silent* relocation and read
/// "Saved to the encrypted file store because the system keyring was
/// unavailable." Two things were wrong with it. It said "system keyring"
/// whatever the failing backend was, so a locked Bitwarden produced a sentence
/// about a keyring the user had not selected; and it reported a decision the
/// user was never asked about, at which point the password was in a store the
/// connect path does not read — `resolve_credentials_blocking` queries the
/// selected backend alone. Naming the destination is now the *smaller* half of
/// the fix; the larger half is that there is a choice to name.
fn show_saved_to_fallback_toast(destination: rustconn_core::config::SecretBackendType) {
    use gtk4::prelude::*;

    gtk4::glib::idle_add_local_once(move || {
        let Some(window) = gtk4::gio::Application::default()
            .and_then(|app| app.downcast_ref::<gtk4::Application>().cloned())
            .and_then(|app| app.active_window())
        else {
            tracing::warn!("Could not show vault fallback toast: no active window");
            return;
        };

        let message = crate::i18n::i18n_f(
            "Password saved to {}.",
            &[&backend_display_name(destination)],
        );
        crate::toast::show_toast_on_window(&window, &message, crate::toast::ToastType::Info);
    });
}

/// Whether the encrypted file takes part as the fallback store.
///
/// One predicate for both directions, and that is the point rather than a
/// convenience: the destination a refused write is *offered* has to be a store the
/// connect path will actually *look in*, and the two answering differently is the
/// exact shape of the bug this release is about. So the offer in
/// [`show_vault_store_failed_dialog`] and the read in
/// [`retrieve_from_encrypted_file_fallback`] are governed from here.
///
/// Two conditions. The user has to have left **Also read from the encrypted
/// file** on, or a credential written there would not be found again; and the
/// encrypted file must not already *be* the selected backend, which on the write
/// side would offer to retry the failure and on the read side would query one
/// store twice.
///
/// Both conditions are the same test `SecretManager::build_from_settings` applies
/// before appending `EncryptedFileBackend` to its chain, which is deliberate: the
/// paths that go through the manager and the paths that go through
/// [`dispatch_vault_op_for`] have to reach the same store or the setting means two
/// things depending on the password source.
pub fn encrypted_file_fallback_enabled(
    secret_settings: &rustconn_core::config::SecretSettings,
) -> bool {
    secret_settings.enable_fallback
        && !matches!(
            secret_settings.preferred_backend,
            rustconn_core::config::SecretBackendType::EncryptedFile
        )
}

/// Reads a credential out of the encrypted file after the selected backend missed.
///
/// The read half of what the **Also read from the encrypted file** switch
/// promises, and what makes the "Save to This Computer" destination in
/// [`show_vault_store_failed_dialog`] reachable at all: that write goes to the
/// encrypted file under the *selected backend's* lookup key, so the read has to
/// use the same keys. They are passed in rather than derived here for that
/// reason — a key computed independently is a key that can disagree, and a
/// password saved to a store nothing queries is what the dialog was added to stop
/// happening silently.
///
/// Call this after a miss and never after an error. A backend that could not be
/// read has not said the password is absent, so answering it with a password from
/// somewhere else is the defect fixed on the KeePass resolve path in this same
/// release. An empty password is not a hit, matching the selected-backend loop:
/// older releases left such entries behind and accepting one ends the search
/// before the key holding the secret is tried.
///
/// A failure to read the fallback is logged and treated as a miss. The caller is
/// already on its way to a password prompt, and a broken store the user did not
/// choose must not turn that into an error dialog about it.
pub fn retrieve_from_encrypted_file_fallback(
    secret_settings: &rustconn_core::config::SecretSettings,
    lookup_keys: &[String],
) -> Option<rustconn_core::models::Credentials> {
    use secrecy::ExposeSecret;

    if !encrypted_file_fallback_enabled(secret_settings) {
        return None;
    }

    for lookup_key in lookup_keys {
        match dispatch_vault_op_for(
            secret_settings,
            rustconn_core::config::SecretBackendType::EncryptedFile,
            lookup_key,
            VaultOp::Retrieve,
        ) {
            Ok(Some(creds))
                if creds
                    .password
                    .as_ref()
                    .is_some_and(|password| !password.expose_secret().is_empty()) =>
            {
                tracing::warn!(
                    %lookup_key,
                    preferred = ?secret_settings.preferred_backend,
                    "credential read from the encrypted-file fallback, not the selected backend"
                );
                return Some(creds);
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(
                    %lookup_key,
                    error = %e,
                    "encrypted-file fallback could not be read; treating it as a miss"
                );
            }
        }
    }

    None
}

/// Asks where a credential should go after the chosen backend refused it.
///
/// The chosen backend is named, and so is the reason it gave. `on_fallback`, when
/// present, writes the credential to the encrypted file on this computer; it is
/// `None` when [`encrypted_file_fallback_enabled`] says that is not on offer, and
/// then the dialog is a plain report with no destination to choose.
///
/// This is a dialog and not a toast because the project's HIG notes put a failed
/// save — where an unfinished user action is at stake — in the dialog column, and
/// because the thing being decided is *where a password lives*. It replaces a
/// silent relocation: the write used to walk the backend chain on any primary
/// error and report the result in a toast, which meant a locked vault quietly
/// moved the password into a store the connect path never queries. The password
/// was then simultaneously saved and, from the connection's point of view,
/// missing — the reported symptom being "Vault entry not found. You will be
/// prompted for a password" for a password that was on disk the whole time.
///
/// The credential is captured in `on_fallback` and therefore stays in memory for
/// as long as the dialog is open. That is deliberate and it is the narrow cost of
/// asking: the alternative is to decide for the user, which is what this replaces.
fn show_vault_store_failed_dialog(
    backend: rustconn_core::config::SecretBackendType,
    err: &rustconn_core::error::SecretError,
    on_fallback: Option<Box<dyn FnOnce()>>,
) {
    use gtk4::prelude::*;
    use libadwaita as adw;
    use libadwaita::prelude::*;

    // A locked portable store needs its own advice — "pick another backend" is
    // the wrong next step when the store is fine and merely closed.
    if matches!(
        err,
        rustconn_core::error::SecretError::PassphraseRequired
            | rustconn_core::error::SecretError::IncorrectPassphrase
    ) {
        show_portable_locked_error(matches!(
            err,
            rustconn_core::error::SecretError::IncorrectPassphrase
        ));
        return;
    }

    // The SecretError Display string carries operation context and backend
    // diagnostics only — never a secret value — so it is safe to show verbatim.
    let cause = err.to_string();
    let backend_name = backend_display_name(backend);

    gtk4::glib::idle_add_local_once(move || {
        let Some(window) = gtk4::gio::Application::default()
            .and_then(|app| app.downcast_ref::<gtk4::Application>().cloned())
            .and_then(|app| app.active_window())
        else {
            tracing::warn!("Could not show vault store failure dialog: no active window");
            return;
        };

        let heading = crate::i18n::i18n_f("{} did not accept this password", &[&backend_name]);

        // What to do next depends on which backend refused. The keyring cases
        // keep the advice the old `show_vault_save_error` carried, including the
        // snap interface hint (#249) — that text was right, it was just applied
        // to every backend, so a locked Bitwarden was answered with instructions
        // about a keyring the user had not selected.
        let recovery = match backend {
            rustconn_core::config::SecretBackendType::LibSecret
            | rustconn_core::config::SecretBackendType::MacOsKeychain => {
                if rustconn_core::snap::is_snap() {
                    crate::i18n::i18n(
                        "The system keyring is not accessible. Run: sudo snap connect rustconn:password-manager-service — or open Settings, then Secrets, and choose another backend.",
                    )
                } else {
                    crate::i18n::i18n(
                        "No system keyring is responding. Open Settings, then Secrets, and choose another backend such as Encrypted file or KeePassXC.",
                    )
                }
            }
            _ => crate::i18n::i18n_f(
                "Fix {} and save again, or choose a different backend in Settings, then Secrets.",
                &[&backend_name],
            ),
        };

        let body = if on_fallback.is_some() {
            let offer = crate::i18n::i18n(
                "You can also save it to this computer's encrypted file instead — it will still be found when you connect.",
            );
            crate::i18n::i18n_f(
                "{}\n\nThe password has not been saved. {}\n\n{}",
                &[&cause, &recovery, &offer],
            )
        } else {
            crate::i18n::i18n_f(
                "{}\n\nThe password has not been saved. {}",
                &[&cause, &recovery],
            )
        };

        let dialog = adw::AlertDialog::new(Some(&heading), Some(&body));
        dialog.add_response("close", &crate::i18n::i18n("Cancel"));
        dialog.add_response("settings", &crate::i18n::i18n("Open Settings"));
        if on_fallback.is_some() {
            dialog.add_response("fallback", &crate::i18n::i18n("Save to This Computer"));
        }
        // Unconditional, and deliberately the *only* suggested response: saving
        // somewhere the user did not originally choose is a real decision, so it
        // is offered without being pushed. The safe next step is to go and fix
        // the backend they did choose, whether or not there is another
        // destination on the dialog.
        dialog.set_response_appearance("settings", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("settings"));
        dialog.set_close_response("close");

        let window_for_action = window.clone();
        // `connect_response` wants an `Fn`, and running the action consumes it, so
        // the `FnOnce` lives in a `Cell` and is taken out on use. `Cell` rather
        // than `RefCell` because `Cell::take` is exactly this operation and cannot
        // panic; a second response cannot fire for one dialog, but neither type
        // system nor reviewer should have to take that on trust.
        let on_fallback = std::cell::Cell::new(on_fallback);
        dialog.connect_response(None, move |_, response| match response {
            "settings" => {
                let _ = gtk4::prelude::WidgetExt::activate_action(
                    &window_for_action,
                    "win.settings",
                    None,
                );
            }
            "fallback" => {
                if let Some(action) = on_fallback.take() {
                    action();
                }
            }
            _ => {}
        });

        dialog.present(Some(&window));
    });
}

/// How long a "Load from vault" button waits for a `Bitwarden` auto-unlock
/// before reporting timeout.
///
/// 30 seconds covers the worst case where the user has to type the master
/// password in an interactive prompt; below that, slow GPG/keyring backends
/// would falsely time out. Shared by the connection and group dialogs so the
/// two load buttons cannot drift apart.
pub(crate) const BITWARDEN_UNLOCK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// How long a "Load from vault" button waits for a single vault retrieve
/// before reporting timeout.
///
/// 10 seconds is the standard project-wide vault budget — see `secrets-guide.md`.
pub(crate) const VAULT_RETRIEVE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// How long one vault operation may take before the GUI stops waiting for it.
///
/// Two budgets, because the backends are two different kinds of thing and until
/// now the smaller number was applied to both. A keyring, a KDBX database and the
/// two file stores answer locally — a D-Bus round trip, a read, an Argon2id
/// derivation — so ten seconds there already means something is wrong. A
/// CLI-backed backend is child processes and network: one Bitwarden store is
/// `bw list folders`, then `bw list items`, then `bw create item` or
/// `bw edit item`, and each of those is a fresh `node` plus a round trip to the
/// vault server.
///
/// Measured on the reporter's machine in issue
/// [#312](https://github.com/totoshko88/RustConn/issues/312) against
/// `bitwarden.eu`: 2.9 s for the folder list, 5.0 s for the item search, and
/// `create item` still in flight when the ten-second budget expired at 10.008 s.
/// The consequence was worse than a save that failed, because dropping that future
/// does not stop the child — `tokio::process` does not kill on drop — so `bw` ran
/// to completion, the item landed in the vault, and RustConn told the user the
/// write had been refused and offered to put the password somewhere else.
///
/// Forty-five seconds is about three times that measured worst case. It is not the
/// only thing bounding a hung CLI: every `bw` invocation carries its own 30 s
/// ceiling inside `rustconn-core`. And none of these calls run on the GTK thread —
/// [`store_primary_blocking`] is reached through `spawn_blocking_with_callback`,
/// and the connect-path resolution runs off the main loop — so a long wait here
/// costs a slow save, not a frozen window.
///
/// The credential-transfer loop keeps its own, deliberately smaller
/// [`TRANSFER_OP_TIMEOUT`]: that budget is per entry across a batch of forty, and
/// changing it is a different trade-off from the one made here.
const fn vault_op_timeout(
    backend: rustconn_core::config::SecretBackendType,
) -> std::time::Duration {
    use rustconn_core::config::SecretBackendType;

    match backend {
        // Shells out to a CLI that talks to a remote vault.
        SecretBackendType::Bitwarden
        | SecretBackendType::OnePassword
        | SecretBackendType::Passbolt
        | SecretBackendType::Pass => std::time::Duration::from_secs(45),
        // Answers from this machine.
        SecretBackendType::LibSecret
        | SecretBackendType::MacOsKeychain
        | SecretBackendType::KeePassXc
        | SecretBackendType::KdbxFile
        | SecretBackendType::EncryptedFile
        | SecretBackendType::PortableEncryptedFile => std::time::Duration::from_secs(10),
    }
}

/// The message for a vault operation that ran out of its budget.
///
/// One function so the wording and the number cannot drift apart, and so the
/// number shown is the one that was actually applied — the previous messages said
/// "after 10s" as a literal, which would have started lying the moment
/// [`vault_op_timeout`] returned anything else.
fn vault_op_timed_out(operation: &str, budget: std::time::Duration) -> String {
    format!("Vault {operation} timed out after {}s", budget.as_secs())
}

/// Stores credentials in the selected backend, and only in the selected backend.
///
/// `allow_fallback` is passed as `false` to [`SecretManager::store_reported`], so
/// the primary backend's own error comes back unchanged and nothing else in the
/// chain is written to. That is the point: this used to pass
/// `secret_settings.enable_fallback`, which is `true` by default, so any primary
/// failure — a locked vault, an unresponsive keyring — walked the chain and put
/// the password in the encrypted file instead. Nobody was asked, and the connect
/// path does not read that store when another backend is selected, so the
/// password was saved and missing at the same time.
///
/// Where a refused write goes now is the user's call:
/// [`show_vault_store_failed_dialog`] asks, and the encrypted-file destination is
/// reached through [`dispatch_vault_op_for`] naming it explicitly.
///
/// The read side is unchanged and still walks the chain — a password saved before
/// the user switched backend has to keep resolving. `enable_fallback` now governs
/// only that.
///
/// # Errors
///
/// Returns the backend's own [`SecretError`] if the store exceeds
/// [`vault_op_timeout`] for the selected backend, or the backend rejects the
/// write. The typed error is preserved rather than flattened to a string because
/// the caller has to tell a locked portable store apart from an unresponsive
/// keyring — the two need different advice, and a formatted string cannot be
/// matched on.
///
/// [`SecretManager`]: rustconn_core::secret::SecretManager
/// [`SecretManager::store_reported`]: rustconn_core::secret::SecretManager::store_reported
/// [`SecretError`]: rustconn_core::error::SecretError
fn store_primary_blocking(
    secret_settings: &rustconn_core::config::SecretSettings,
    lookup_key: &str,
    creds: &rustconn_core::models::Credentials,
) -> Result<(), rustconn_core::error::SecretError> {
    use rustconn_core::error::SecretError;

    let manager = rustconn_core::secret::SecretManager::build_from_settings(secret_settings);
    // The budget follows the backend the manager will actually write to: a hung
    // keyring must not block the callback for as long as a Bitwarden round trip
    // legitimately needs.
    let budget = vault_op_timeout(secret_settings.preferred_backend);

    crate::async_utils::with_runtime(|rt| {
        rt.block_on(async {
            tokio::time::timeout(budget, manager.store_reported(lookup_key, creds, false))
                .await
                .map_err(|_| SecretError::StoreFailed(vault_op_timed_out("store", budget)))?
        })
    })
    .map_err(SecretError::StoreFailed)
    .and_then(|r| r)
    .map(|_| ())
}

/// Writes a credential to the machine-bound encrypted file, naming it explicitly.
///
/// The destination is passed to [`dispatch_vault_op_for`] rather than reached by
/// falling off the end of the backend chain, so this can only ever write where
/// the caller said. Used by the "Save to This Computer" response of
/// [`show_vault_store_failed_dialog`].
///
/// # Errors
///
/// Returns a human-readable error when the encrypted file rejects the write.
fn store_in_encrypted_file_blocking(
    secret_settings: &rustconn_core::config::SecretSettings,
    lookup_key: &str,
    creds: &rustconn_core::models::Credentials,
) -> Result<(), String> {
    dispatch_vault_op_for(
        secret_settings,
        rustconn_core::config::SecretBackendType::EncryptedFile,
        lookup_key,
        VaultOp::Store(creds),
    )
    .map(|_| ())
}

/// Saves a connection password to the configured vault backend.
///
/// Dispatches to KeePass (hierarchical) or generic backend (flat key)
/// based on the current settings. Password is taken as `&SecretString`
/// so plaintext copies do not leak via call-site `String`s — see
/// `secrets-guide.md`.
#[expect(
    clippy::too_many_arguments,
    reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
)]
pub fn save_password_to_vault(
    settings: &rustconn_core::config::AppSettings,
    groups: &[rustconn_core::models::ConnectionGroup],
    conn: Option<&rustconn_core::models::Connection>,
    conn_name: &str,
    conn_host: &str,
    protocol: rustconn_core::models::ProtocolType,
    username: &str,
    password: &secrecy::SecretString,
    conn_id: uuid::Uuid,
) {
    use secrecy::ExposeSecret;
    let protocol_str = protocol.as_str().to_lowercase();

    // A password is about to exist where the connect path may have recorded that
    // none did, so drop that record before the write is even attempted (#307).
    // Here rather than at the seven call sites: this function is the choke point,
    // so a new caller cannot forget it. In the prologue rather than after the
    // store because the write happens on a worker whose completion this function
    // does not observe, and because a forget that turns out to have been
    // unnecessary costs one vault lookup.
    crate::vault_miss_cache::forget(conn_id);

    if settings.secrets.kdbx_enabled
        && matches!(
            settings.secrets.preferred_backend,
            rustconn_core::config::SecretBackendType::KeePassXc
                | rustconn_core::config::SecretBackendType::KdbxFile
        )
    {
        // KeePass backend — use hierarchical path
        if let Some(kdbx_path) = settings.secrets.kdbx_path.clone() {
            // Which of the two KDBX rows the user picked, so a failure can name
            // it. The branch condition guarantees this is `KeePassXc` or
            // `KdbxFile`; it is read rather than hardcoded so the message follows
            // the selection.
            let kdbx_backend = settings.secrets.preferred_backend;
            let key_file = settings.secrets.kdbx_key_file.clone();
            let db_password = settings.secrets.kdbx_password.clone();
            let yubikey_slot = settings.secrets.kdbx_yubikey_slot.clone();
            let read_only = settings.secrets.kdbx_read_only;
            let entry_name = if let Some(c) = conn {
                let entry_path =
                    rustconn_core::secret::KeePassHierarchy::build_entry_path(c, groups);
                let base_path = entry_path.strip_prefix("RustConn/").unwrap_or(&entry_path);
                format!("{base_path} ({protocol_str})")
            } else {
                format!("{conn_name} ({protocol_str})")
            };
            let username = username.to_string();
            let url = format!("{}://{}", protocol_str, conn_host);
            // The closure below is `move` and runs on another thread, so it needs
            // an owned secret. `SecretString` is `SecretBox<str>` and `str` is not
            // `Clone`, so the secret cannot simply be cloned into it; an `Arc`
            // shares it without a second plaintext copy, which is the same shape
            // `db_password` already uses in the bulk-transfer path below. Cloning
            // the `SecretString` instead would duplicate the `Box<str>`, i.e. make
            // exactly the second plaintext this avoids.
            //
            // The argument has to stay a `to_string()` of a `&str`. That path is
            // `Vec::with_capacity(len)`, so capacity equals length and the
            // `into_boxed_str()` inside `SecretString::from` cannot reallocate.
            // Build the `String` any other way — `format!`, or pushing — and the
            // shrink-to-fit realloc becomes live, freeing the original buffer
            // *unzeroed* and making this weaker than the `Zeroizing<String>` it
            // replaced.
            let pwd = std::sync::Arc::new(secrecy::SecretString::from(
                password.expose_secret().to_string(),
            ));

            crate::utils::spawn_blocking_with_callback(
                move || {
                    let kdbx = std::path::Path::new(&kdbx_path);
                    let key = key_file.as_ref().map(std::path::Path::new);
                    rustconn_core::secret::KeePassStatus::save_password_to_kdbx(
                        kdbx,
                        db_password.as_ref(),
                        key,
                        &entry_name,
                        &username,
                        &pwd,
                        Some(&url),
                        yubikey_slot.as_deref(),
                        read_only,
                    )
                },
                move |result| {
                    if let Err(e) = result {
                        tracing::error!(%conn_id, error = %e, "KDBX refused the password");
                        // No fallback offer here. The KDBX database is a file the
                        // user chose and manages, and a write that failed against
                        // it is a problem with that file — silently or otherwise
                        // putting the password somewhere else would split their
                        // database, which is the opposite of what picking a
                        // database backend asks for.
                        show_vault_store_failed_dialog(kdbx_backend, &e, None);
                    } else {
                        tracing::info!(%conn_id, "Password saved to the KDBX database");
                    }
                },
            );
        }
    } else {
        // Generic backend — dispatch via consolidated helper.
        // Use the same key format that the resolver expects for each backend,
        // so that store and resolve are consistent.
        let backend_type = select_backend_for_load(&settings.secrets);
        // For LibSecret, include group path to prevent name collisions (issue #264).
        // A connection always gets the "RustConn/" prefix, grouped or not, which is
        // what `generate_store_key_for_connection` is for; quick-connect has no
        // connection to take a group from, so it keeps the flat legacy format.
        let lookup_key = match conn {
            Some(c) => generate_store_key_for_connection(
                conn_name,
                conn_host,
                &protocol_str,
                backend_type,
                c.group_id,
                groups,
            ),
            None => generate_store_key(conn_name, conn_host, &protocol_str, backend_type),
        };
        tracing::debug!(
            %lookup_key,
            ?backend_type,
            conn_name,
            conn_host,
            protocol_str,
            "save_password_to_vault: storing with key"
        );
        // One `Arc<Credentials>` shared by the write and by the possible second
        // write after the dialog, so asking the user where the password should go
        // costs no additional plaintext copy. The one copy is `password.clone()`,
        // which the previous version made too; what is new is that it is not made
        // twice when a retry happens.
        let creds = std::sync::Arc::new(rustconn_core::models::Credentials {
            username: Some(username.to_string()),
            password: Some(password.clone()),
            key_passphrase: None,
            domain: None,
        });
        let secret_settings = settings.secrets.clone();
        let creds_worker = std::sync::Arc::clone(&creds);
        let settings_worker = secret_settings.clone();
        let key_worker = lookup_key.clone();

        crate::utils::spawn_blocking_with_callback(
            move || store_primary_blocking(&settings_worker, &key_worker, &creds_worker),
            move |result: Result<(), rustconn_core::error::SecretError>| match result {
                Ok(()) => {
                    tracing::info!(
                        ?backend_type,
                        %conn_id,
                        "Password saved to the selected vault backend"
                    );
                }
                Err(e) => {
                    // `warn`, not `error`: the write did not happen, but the user
                    // is about to be asked what to do about it, so this is not the
                    // end of the story.
                    tracing::warn!(
                        ?backend_type,
                        %conn_id,
                        error = %e,
                        "Selected vault backend refused the password"
                    );
                    let on_fallback = encrypted_file_fallback_enabled(&secret_settings).then(|| {
                        let settings_retry = secret_settings.clone();
                        let key_retry = lookup_key.clone();
                        let creds_retry = std::sync::Arc::clone(&creds);
                        Box::new(move || {
                            crate::utils::spawn_blocking_with_callback(
                                move || {
                                    store_in_encrypted_file_blocking(
                                        &settings_retry,
                                        &key_retry,
                                        &creds_retry,
                                    )
                                },
                                move |retry: Result<(), String>| match retry {
                                    Ok(()) => {
                                        tracing::info!(
                                            %conn_id,
                                            "Password saved to the encrypted file at the user's request"
                                        );
                                        show_saved_to_fallback_toast(
                                            rustconn_core::config::SecretBackendType::EncryptedFile,
                                        );
                                    }
                                    Err(msg) => {
                                        tracing::error!(
                                            %conn_id,
                                            error = %msg,
                                            "Encrypted-file write refused after the user chose it"
                                        );
                                        show_vault_store_failed_dialog(
                                            rustconn_core::config::SecretBackendType::EncryptedFile,
                                            &rustconn_core::error::SecretError::StoreFailed(msg),
                                            None,
                                        );
                                    }
                                },
                            );
                        }) as Box<dyn FnOnce()>
                    });
                    show_vault_store_failed_dialog(backend_type, &e, on_fallback);
                }
            },
        );
    }
}

/// Saves a group password to the configured vault backend.
///
/// Password is taken as `&SecretString` so plaintext copies do not leak
/// via call-site `String`s.
pub fn save_group_password_to_vault(
    settings: &rustconn_core::config::AppSettings,
    group_path: &str,
    lookup_key: &str,
    username: &str,
    password: &secrecy::SecretString,
) {
    use secrecy::ExposeSecret;

    if settings.secrets.kdbx_enabled
        && matches!(
            settings.secrets.preferred_backend,
            rustconn_core::config::SecretBackendType::KeePassXc
                | rustconn_core::config::SecretBackendType::KdbxFile
        )
    {
        if let Some(kdbx_path) = settings.secrets.kdbx_path.clone() {
            // See the connection path: read rather than hardcoded so a failure
            // names the row the user actually picked.
            let kdbx_backend = settings.secrets.preferred_backend;
            let key_file = settings.secrets.kdbx_key_file.clone();
            let db_password = settings.secrets.kdbx_password.clone();
            let yubikey_slot = settings.secrets.kdbx_yubikey_slot.clone();
            let read_only = settings.secrets.kdbx_read_only;
            let entry_name = group_path
                .strip_prefix("RustConn/")
                .unwrap_or(group_path)
                .to_string();
            let username_val = username.to_string();
            // Owned for the `move` closure, shared rather than copied — see the
            // note in `save_password_to_vault` above.
            let password_val = std::sync::Arc::new(secrecy::SecretString::from(
                password.expose_secret().to_string(),
            ));

            crate::utils::spawn_blocking_with_callback(
                move || {
                    let kdbx = std::path::Path::new(&kdbx_path);
                    let key = key_file.as_ref().map(std::path::Path::new);
                    rustconn_core::secret::KeePassStatus::save_password_to_kdbx(
                        kdbx,
                        db_password.as_ref(),
                        key,
                        &entry_name,
                        &username_val,
                        &password_val,
                        None,
                        yubikey_slot.as_deref(),
                        read_only,
                    )
                },
                move |result| {
                    if let Err(e) = result {
                        tracing::error!(error = %e, "KDBX refused the group password");
                        show_vault_store_failed_dialog(kdbx_backend, &e, None);
                    } else {
                        tracing::info!("Group password saved to the KDBX database");
                    }
                },
            );
        }
    } else {
        // Same shape as the connection path above: the selected backend alone is
        // written to, and a refusal asks the user where the password should go
        // rather than relocating it silently.
        let backend_type = select_backend_for_load(&settings.secrets);
        let lookup_key = lookup_key.to_string();
        let creds = std::sync::Arc::new(rustconn_core::models::Credentials {
            username: Some(username.to_string()),
            password: Some(password.clone()),
            key_passphrase: None,
            domain: None,
        });
        let secret_settings = settings.secrets.clone();
        let creds_worker = std::sync::Arc::clone(&creds);
        let settings_worker = secret_settings.clone();
        let key_worker = lookup_key.clone();

        crate::utils::spawn_blocking_with_callback(
            move || store_primary_blocking(&settings_worker, &key_worker, &creds_worker),
            move |result: Result<(), rustconn_core::error::SecretError>| match result {
                Ok(()) => {
                    tracing::info!(
                        ?backend_type,
                        "Group password saved to the selected backend"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        ?backend_type,
                        error = %e,
                        "Selected vault backend refused the group password"
                    );
                    let on_fallback = encrypted_file_fallback_enabled(&secret_settings).then(|| {
                        let settings_retry = secret_settings.clone();
                        let key_retry = lookup_key.clone();
                        let creds_retry = std::sync::Arc::clone(&creds);
                        Box::new(move || {
                            crate::utils::spawn_blocking_with_callback(
                                move || {
                                    store_in_encrypted_file_blocking(
                                        &settings_retry,
                                        &key_retry,
                                        &creds_retry,
                                    )
                                },
                                move |retry: Result<(), String>| match retry {
                                    Ok(()) => {
                                        tracing::info!(
                                            "Group password saved to the encrypted file at the user's request"
                                        );
                                        show_saved_to_fallback_toast(
                                            rustconn_core::config::SecretBackendType::EncryptedFile,
                                        );
                                    }
                                    Err(msg) => {
                                        tracing::error!(
                                            error = %msg,
                                            "Encrypted-file write refused after the user chose it"
                                        );
                                        show_vault_store_failed_dialog(
                                            rustconn_core::config::SecretBackendType::EncryptedFile,
                                            &rustconn_core::error::SecretError::StoreFailed(msg),
                                            None,
                                        );
                                    }
                                },
                            );
                        }) as Box<dyn FnOnce()>
                    });
                    show_vault_store_failed_dialog(backend_type, &e, on_fallback);
                }
            },
        );
    }
}

/// Renames a credential in the configured vault backend when a connection
/// is renamed.
///
/// Thin wrapper over [`migrate_vault_credential_for_edit`]; `protocol_str` is
/// accepted for call-site compatibility but the protocol is derived from
/// `updated_conn` so that a caller cannot pass one that disagrees with it.
///
/// # Errors
///
/// Returns a human-readable error string if the backend rejects the migration.
pub fn rename_vault_credential(
    settings: &rustconn_core::config::AppSettings,
    groups: &[rustconn_core::models::ConnectionGroup],
    updated_conn: &rustconn_core::models::Connection,
    old_name: &str,
    _protocol_str: &str,
) -> Result<(), String> {
    let mut old_conn = updated_conn.clone();
    old_conn.name = old_name.to_string();
    migrate_vault_credential_for_edit(settings, groups, groups, &old_conn, updated_conn)
}

/// Where a connection's credential currently lives and where it must end up.
///
/// `old_keys` is ordered most-current-format first; the migration walks it and
/// uses the first key that yields a credential, so entries written by earlier
/// releases are picked up rather than orphaned.
#[derive(Debug, PartialEq, Eq)]
struct VaultKeyMigration {
    old_keys: Vec<String>,
    new_key: String,
    is_keepass: bool,
}

/// Computes the key migration for a connection edit, or `None` when the edit
/// cannot have changed the lookup key.
///
/// Kept separate from the I/O so the key derivation — the part that has been
/// wrong in several different ways — is unit-testable without a live vault.
fn plan_vault_key_migration(
    settings: &rustconn_core::config::AppSettings,
    old_groups: &[rustconn_core::models::ConnectionGroup],
    new_groups: &[rustconn_core::models::ConnectionGroup],
    old_conn: &rustconn_core::models::Connection,
    new_conn: &rustconn_core::models::Connection,
) -> Option<VaultKeyMigration> {
    use rustconn_core::config::SecretBackendType;

    let old_protocol = old_conn
        .protocol_config
        .protocol_type()
        .as_str()
        .to_lowercase();
    let new_protocol = new_conn
        .protocol_config
        .protocol_type()
        .as_str()
        .to_lowercase();

    let is_keepass = settings.secrets.kdbx_enabled
        && matches!(
            settings.secrets.preferred_backend,
            SecretBackendType::KeePassXc | SecretBackendType::KdbxFile
        );

    let (old_keys, new_key) = if is_keepass {
        let old_base =
            rustconn_core::secret::KeePassHierarchy::build_entry_path(old_conn, old_groups);
        let new_base =
            rustconn_core::secret::KeePassHierarchy::build_entry_path(new_conn, new_groups);
        (
            vec![format!("{old_base} ({old_protocol})")],
            format!("{new_base} ({new_protocol})"),
        )
    } else {
        let backend_type = select_backend_for_load(&settings.secrets);
        let old_keys = vault_keys_for_connection(old_groups, old_conn, &old_protocol, backend_type);
        let new_key = vault_keys_for_connection(new_groups, new_conn, &new_protocol, backend_type)
            .into_iter()
            .next()?;
        (old_keys, new_key)
    };

    if old_keys.first() == Some(&new_key) {
        return None;
    }

    Some(VaultKeyMigration {
        old_keys,
        new_key,
        is_keepass,
    })
}

/// Returns whether an edit moved the connection's vault lookup key.
///
/// Lets a caller that has just written a freshly typed password under the new
/// key decide whether the entry under the previous key is now stale. Returns
/// `false` when the key is unchanged, where deleting the old entry would delete
/// the credential that was just saved.
#[must_use]
pub fn vault_key_changed_by_edit(
    settings: &rustconn_core::config::AppSettings,
    old_groups: &[rustconn_core::models::ConnectionGroup],
    new_groups: &[rustconn_core::models::ConnectionGroup],
    old_conn: &rustconn_core::models::Connection,
    new_conn: &rustconn_core::models::Connection,
) -> bool {
    plan_vault_key_migration(settings, old_groups, new_groups, old_conn, new_conn).is_some()
}

/// Migrates a connection's vault credential after an edit that changed its
/// lookup key.
///
/// Covers a rename, a move to another group, a protocol change, or any
/// combination of the three in a single save — the connection edit dialog can
/// change all three at once, and until 0.19.19 that path performed no migration
/// at all, so editing the name in the configuration panel silently stranded the
/// credential under the old key (issue #263).
///
/// `old_groups` and `new_groups` are separate so a group rename, which changes
/// the path without changing the connection, can reuse this. Pass the same slice
/// twice when the hierarchy is unchanged.
///
/// The `SecretBackend` trait has no rename operation, so for every backend
/// except KeePass the move is retrieve → store under the new key → delete the
/// old. The delete only runs once the store has succeeded, so a failure leaves
/// the credential readable under the old key rather than losing it.
///
/// # Errors
///
/// Returns a human-readable error string if the backend rejects the migration.
pub fn migrate_vault_credential_for_edit(
    settings: &rustconn_core::config::AppSettings,
    old_groups: &[rustconn_core::models::ConnectionGroup],
    new_groups: &[rustconn_core::models::ConnectionGroup],
    old_conn: &rustconn_core::models::Connection,
    new_conn: &rustconn_core::models::Connection,
) -> Result<(), String> {
    let Some(plan) = plan_vault_key_migration(settings, old_groups, new_groups, old_conn, new_conn)
    else {
        return Ok(());
    };

    // The credential is moving to a new key, so any recorded "no entry here" for
    // this connection was about the old one and must not be trusted (#307).
    crate::vault_miss_cache::forget(new_conn.id);

    if plan.is_keepass {
        let Some(kdbx_path) = settings.secrets.kdbx_path.as_ref() else {
            return Ok(());
        };
        let old_key = plan.old_keys.first().map_or("", String::as_str);
        tracing::info!(%old_key, new_key = %plan.new_key, "Migrating KeePass entry after edit");
        let key_file = settings.secrets.kdbx_key_file.clone();
        return rustconn_core::secret::KeePassStatus::rename_entry_in_kdbx(
            std::path::Path::new(kdbx_path),
            settings.secrets.kdbx_password.as_ref(),
            key_file.as_ref().map(std::path::Path::new),
            old_key,
            &plan.new_key,
            settings.secrets.kdbx_yubikey_slot.as_deref(),
            settings.secrets.kdbx_read_only,
        )
        .map_err(|e| format!("{e}"));
    }

    tracing::info!(new_key = %plan.new_key, "Migrating vault entry after edit");
    let secret_settings = settings.secrets.clone();
    for old_key in &plan.old_keys {
        if *old_key == plan.new_key {
            continue;
        }
        if let Ok(Some(creds)) = dispatch_vault_op(&secret_settings, old_key, VaultOp::Retrieve) {
            // The vault lock is taken per operation, not across this triple, so
            // a save under the new key can land between the retrieve and the
            // store (a Group Sync migration runs after the new tree is already
            // live). A credential already under the new key is the newer one:
            // keep it and only drop the stale copy.
            if matches!(
                dispatch_vault_op(&secret_settings, &plan.new_key, VaultOp::Retrieve),
                Ok(Some(_))
            ) {
                tracing::info!(%old_key, new_key = %plan.new_key, "New vault key already holds a credential; keeping it");
                let _ = dispatch_vault_op(&secret_settings, old_key, VaultOp::Delete);
                return Ok(());
            }
            dispatch_vault_op(&secret_settings, &plan.new_key, VaultOp::Store(&creds))?;
            let _ = dispatch_vault_op(&secret_settings, old_key, VaultOp::Delete);
            return Ok(());
        }
    }
    Ok(())
}

/// Renames a vault credential when a connection is moved to a different group.
///
/// For KeePass backends, the entry path includes the group hierarchy, so moving
/// a connection changes the lookup key. This function renames the old entry to
/// the new path so the password remains accessible.
///
/// LibSecret and the macOS Keychain also embed the group path in their key
/// (since 0.19.18, issue #264), so they are migrated the same way — via
/// retrieve/store/delete, since the backend trait has no rename. Bitwarden,
/// 1Password, Passbolt, pass and the encrypted file use `rustconn/{name}`,
/// which a group move does not change, so they need no migration.
///
/// Thin wrapper over [`migrate_vault_credential_for_edit`]; `protocol_str` is
/// accepted for call-site compatibility but the protocol is derived from the
/// connections themselves.
///
/// # Errors
///
/// Returns a human-readable error string if the backend rejects the migration.
pub fn rename_vault_credential_for_move(
    settings: &rustconn_core::config::AppSettings,
    groups: &[rustconn_core::models::ConnectionGroup],
    old_conn: &rustconn_core::models::Connection,
    new_conn: &rustconn_core::models::Connection,
    _protocol_str: &str,
) -> Result<(), String> {
    migrate_vault_credential_for_edit(settings, groups, groups, old_conn, new_conn)
}

/// Migrates all KeePass vault entries affected by a group rename or move.
///
/// When a group is renamed or moved to a different parent, the hierarchical
/// KeePass entry paths change for:
/// 1. The group's own credential (if `password_source == Vault`)
/// 2. All connections in the group (and descendant groups) with `password_source == Vault`
///
/// LibSecret and the macOS Keychain embed the group path in the connection key
/// too (since 0.19.18, issue #264), so their connection entries are migrated as
/// well. Their group credentials are keyed by group UUID, which a rename does
/// not change. Bitwarden, 1Password, Passbolt, pass and the encrypted file use
/// flat `rustconn/{name}` keys and need no migration.
pub fn migrate_vault_entries_on_group_change(
    settings: &rustconn_core::config::AppSettings,
    old_groups: &[rustconn_core::models::ConnectionGroup],
    new_groups: &[rustconn_core::models::ConnectionGroup],
    connections: &[rustconn_core::models::Connection],
    changed_group_id: uuid::Uuid,
) {
    use rustconn_core::config::SecretBackendType;

    let is_keepass = settings.secrets.kdbx_enabled
        && matches!(
            settings.secrets.preferred_backend,
            SecretBackendType::KeePassXc | SecretBackendType::KdbxFile
        );

    if is_keepass {
        let Some(kdbx_path) = settings.secrets.kdbx_path.clone() else {
            return;
        };
        migrate_keepass_entries_on_group_change(
            settings,
            old_groups,
            new_groups,
            connections,
            changed_group_id,
            kdbx_path,
        );
        return;
    }

    let backend_type = select_backend_for_load(&settings.secrets);
    if matches!(
        backend_type,
        SecretBackendType::LibSecret | SecretBackendType::MacOsKeychain
    ) {
        migrate_keyring_entries_on_group_change(
            settings,
            old_groups,
            new_groups,
            connections,
            changed_group_id,
            backend_type,
        );
    }
}

/// Migrates every vault entry a Group Sync import moved, on one background
/// thread, one entry after another.
///
/// A sync can rename and move groups and connections in a single pass, so the
/// old keys come from `old_groups` and the pre-sync connections and the new
/// keys from `new_groups` and the applied connections — the split
/// [`migrate_vault_credential_for_edit`] makes for an edit. Each pair in
/// `connections` is `(before, after)`; a pair whose key did not move is
/// dropped before anything is spawned. For KeePass, the group credentials of
/// `changed_group_ids` and their descendants are renamed as well, as
/// [`migrate_vault_entries_on_group_change`] does for an edit.
///
/// One thread rather than one per entry: the KDBX writers rewrite the whole
/// database, and two running at once would lose one of the renames. A failed
/// migration — a read-only database included — is logged; it never undoes the
/// sync, and the credential stays readable under its old key.
pub fn migrate_vault_entries_after_group_sync(
    settings: &rustconn_core::config::AppSettings,
    old_groups: Vec<rustconn_core::models::ConnectionGroup>,
    new_groups: Vec<rustconn_core::models::ConnectionGroup>,
    connections: Vec<(
        rustconn_core::models::Connection,
        rustconn_core::models::Connection,
    )>,
    changed_group_ids: &[uuid::Uuid],
) {
    use rustconn_core::config::SecretBackendType;
    use rustconn_core::secret::KeePassHierarchy;

    let connections: Vec<_> = connections
        .into_iter()
        .filter(|(old, new)| {
            new.password_source == rustconn_core::models::PasswordSource::Vault
                && plan_vault_key_migration(settings, &old_groups, &new_groups, old, new).is_some()
        })
        .collect();

    let is_keepass = settings.secrets.kdbx_enabled
        && matches!(
            settings.secrets.preferred_backend,
            SecretBackendType::KeePassXc | SecretBackendType::KdbxFile
        );
    let mut group_renames: Vec<(String, String)> = Vec::new();
    if is_keepass {
        let mut affected: std::collections::HashSet<uuid::Uuid> = std::collections::HashSet::new();
        for gid in changed_group_ids {
            affected.extend(rustconn_core::models::collect_descendant_group_ids(
                *gid,
                &new_groups,
            ));
        }
        for new_group in new_groups.iter().filter(|g| affected.contains(&g.id)) {
            if new_group.password_source != Some(rustconn_core::models::PasswordSource::Vault) {
                continue;
            }
            let Some(old_group) = old_groups.iter().find(|g| g.id == new_group.id) else {
                continue;
            };
            let old_path = KeePassHierarchy::build_group_entry_path(old_group, &old_groups);
            let new_path = KeePassHierarchy::build_group_entry_path(new_group, &new_groups);
            if old_path != new_path {
                group_renames.push((old_path, new_path));
            }
        }
    }

    if connections.is_empty() && group_renames.is_empty() {
        return;
    }

    let settings = settings.clone();
    crate::utils::spawn_blocking_with_callback(
        move || {
            let mut errors: Vec<String> = Vec::new();
            for (old_conn, new_conn) in &connections {
                if let Err(e) = migrate_vault_credential_for_edit(
                    &settings,
                    &old_groups,
                    &new_groups,
                    old_conn,
                    new_conn,
                ) {
                    errors.push(format!("{}: {e}", new_conn.name));
                }
            }
            if let Some(kdbx_path) = settings.secrets.kdbx_path.as_ref() {
                let key_file = settings.secrets.kdbx_key_file.clone();
                for (old_key, new_key) in &group_renames {
                    tracing::info!(%old_key, %new_key, "Migrating KeePass group entry after Group Sync");
                    if let Err(e) = rustconn_core::secret::KeePassStatus::rename_entry_in_kdbx(
                        std::path::Path::new(kdbx_path),
                        settings.secrets.kdbx_password.as_ref(),
                        key_file.as_ref().map(std::path::Path::new),
                        old_key,
                        new_key,
                        settings.secrets.kdbx_yubikey_slot.as_deref(),
                        settings.secrets.kdbx_read_only,
                    ) {
                        errors.push(format!("{old_key} → {new_key}: {e}"));
                    }
                }
            }
            if errors.is_empty() {
                Ok(())
            } else {
                Err(errors.join("; "))
            }
        },
        |result: Result<(), String>| {
            if let Err(e) = result {
                tracing::warn!(
                    error = %e,
                    "Group Sync: some vault entries could not follow a rename or move"
                );
            }
        },
    );
}

/// Migrates keyring connection entries whose key embeds a renamed group path.
///
/// The `SecretBackend` trait has no rename, so each entry is moved by
/// retrieve → store under the new key → delete the old key. Group credentials
/// are keyed by group UUID and are deliberately left alone.
fn migrate_keyring_entries_on_group_change(
    settings: &rustconn_core::config::AppSettings,
    old_groups: &[rustconn_core::models::ConnectionGroup],
    new_groups: &[rustconn_core::models::ConnectionGroup],
    connections: &[rustconn_core::models::Connection],
    changed_group_id: uuid::Uuid,
    backend_type: rustconn_core::config::SecretBackendType,
) {
    let affected_group_ids =
        rustconn_core::models::collect_descendant_group_ids(changed_group_id, new_groups);

    let mut moves: Vec<(Vec<String>, String)> = Vec::new();
    let mut moved_ids: Vec<uuid::Uuid> = Vec::new();
    for conn in connections {
        if conn.password_source != rustconn_core::models::PasswordSource::Vault {
            continue;
        }
        let Some(group_id) = conn.group_id else {
            continue;
        };
        if !affected_group_ids.contains(&group_id) {
            continue;
        }

        let protocol_str = conn.protocol_config.protocol_type().as_str().to_lowercase();
        let old_keys = vault_keys_for_connection(old_groups, conn, &protocol_str, backend_type);
        let new_keys = vault_keys_for_connection(new_groups, conn, &protocol_str, backend_type);
        let Some(new_key) = new_keys.first().cloned() else {
            continue;
        };
        if old_keys.first() == Some(&new_key) {
            continue;
        }
        moves.push((old_keys, new_key));
        moved_ids.push(conn.id);
    }

    if moves.is_empty() {
        return;
    }

    // Each of these credentials is about to live under a different key, so a
    // recorded "no entry" was about the old one (#307). Done before the spawn
    // rather than inside it because the ids are already gathered here and the
    // closure would have to carry them for no reason; the cache itself is
    // thread-safe either way.
    for id in moved_ids {
        crate::vault_miss_cache::forget(id);
    }

    let secret_settings = settings.secrets.clone();
    crate::utils::spawn_blocking_with_callback(
        move || {
            for (old_keys, new_key) in &moves {
                tracing::info!(%new_key, "Migrating keyring entry after group change");
                for old_key in old_keys {
                    if old_key == new_key {
                        continue;
                    }
                    if let Ok(Some(creds)) =
                        dispatch_vault_op(&secret_settings, old_key, VaultOp::Retrieve)
                    {
                        if let Err(e) =
                            dispatch_vault_op(&secret_settings, new_key, VaultOp::Store(&creds))
                        {
                            tracing::error!(
                                %new_key,
                                error = %e,
                                "Failed to store keyring entry under new group path"
                            );
                        } else {
                            let _ = dispatch_vault_op(&secret_settings, old_key, VaultOp::Delete);
                        }
                        break;
                    }
                }
            }
            Ok::<(), String>(())
        },
        |result: Result<(), String>| {
            if let Err(e) = result {
                tracing::error!(error = %e, "Failed to migrate keyring entries after group change");
            }
        },
    );
}

/// Migrates KDBX entry paths for a renamed or re-parented group.
fn migrate_keepass_entries_on_group_change(
    settings: &rustconn_core::config::AppSettings,
    old_groups: &[rustconn_core::models::ConnectionGroup],
    new_groups: &[rustconn_core::models::ConnectionGroup],
    connections: &[rustconn_core::models::Connection],
    changed_group_id: uuid::Uuid,
    kdbx_path: std::path::PathBuf,
) {
    // Collect all group IDs in the subtree rooted at changed_group_id
    let affected_group_ids =
        rustconn_core::models::collect_descendant_group_ids(changed_group_id, new_groups);

    // Build rename pairs: (old_key, new_key)
    let mut rename_pairs: Vec<(String, String)> = Vec::new();

    // 1. Migrate group credentials
    for &gid in &affected_group_ids {
        let old_group = old_groups.iter().find(|g| g.id == gid);
        let new_group = new_groups.iter().find(|g| g.id == gid);
        if let (Some(og), Some(ng)) = (old_group, new_group)
            && ng.password_source == Some(rustconn_core::models::PasswordSource::Vault)
        {
            let old_path =
                rustconn_core::secret::KeePassHierarchy::build_group_entry_path(og, old_groups);
            let new_path =
                rustconn_core::secret::KeePassHierarchy::build_group_entry_path(ng, new_groups);
            if old_path != new_path {
                rename_pairs.push((old_path, new_path));
            }
        }
    }

    // 2. Migrate connection credentials
    for conn in connections {
        if conn.password_source != rustconn_core::models::PasswordSource::Vault {
            continue;
        }
        let Some(group_id) = conn.group_id else {
            continue;
        };
        if !affected_group_ids.contains(&group_id) {
            continue;
        }

        let old_path = rustconn_core::secret::KeePassHierarchy::build_entry_path(conn, old_groups);
        let new_path = rustconn_core::secret::KeePassHierarchy::build_entry_path(conn, new_groups);

        if old_path != new_path {
            let protocol_str = conn.protocol_config.protocol_type().as_str().to_lowercase();
            let old_key = format!("{old_path} ({protocol_str})");
            let new_key = format!("{new_path} ({protocol_str})");
            rename_pairs.push((old_key, new_key));
        }
    }

    if rename_pairs.is_empty() {
        return;
    }

    let key_file = settings.secrets.kdbx_key_file.clone();
    let db_password = settings.secrets.kdbx_password.clone();
    let yubikey_slot = settings.secrets.kdbx_yubikey_slot.clone();
    let read_only = settings.secrets.kdbx_read_only;

    crate::utils::spawn_blocking_with_callback(
        move || {
            let kdbx = std::path::Path::new(&kdbx_path);
            let key = key_file.as_ref().map(std::path::Path::new);
            let mut errors = Vec::new();

            for (old_key, new_key) in &rename_pairs {
                tracing::info!(%old_key, %new_key, "Migrating KeePass entry after group change");
                if let Err(e) = rustconn_core::secret::KeePassStatus::rename_entry_in_kdbx(
                    kdbx,
                    db_password.as_ref(),
                    key,
                    old_key,
                    new_key,
                    yubikey_slot.as_deref(),
                    read_only,
                ) {
                    errors.push(format!("{old_key} → {new_key}: {e}"));
                }
            }

            if errors.is_empty() {
                Ok(())
            } else {
                Err(errors.join("; "))
            }
        },
        |result| {
            if let Err(e) = result {
                tracing::error!(error = %e, "Failed to migrate vault entries after group change");
            }
        },
    );
}

/// Saves a secret variable value to the configured vault backend.
///
/// Respects `preferred_backend` from secret settings, using the same
/// backend selection logic as connection passwords. Password is taken
/// as `&SecretString` so plaintext copies do not leak via call-site
/// `String`s.
///
/// # Errors
///
/// Returns an error string if the configured backend is unreachable, the
/// KeePass database cannot be written to, or no fallback backend is
/// available when the primary backend fails.
pub fn save_variable_to_vault(
    settings: &rustconn_core::config::SecretSettings,
    var_name: &str,
    password: &secrecy::SecretString,
) -> Result<(), String> {
    use rustconn_core::config::SecretBackendType;

    let lookup_key = rustconn_core::variable_secret_key(var_name);
    let backend_type = select_backend_for_load(settings);

    tracing::debug!(?backend_type, var_name, "Saving secret variable to vault");

    let creds = rustconn_core::models::Credentials {
        username: None,
        password: Some(password.clone()),
        key_passphrase: None,
        domain: None,
    };

    match backend_type {
        SecretBackendType::KdbxFile | SecretBackendType::KeePassXc => {
            if let Some(kdbx_path) = settings.kdbx_path.as_ref() {
                let key_file = settings.kdbx_key_file.clone();
                let kdbx = std::path::Path::new(kdbx_path);
                let key = key_file.as_ref().map(std::path::Path::new);
                // No intermediate plaintext at all now that the callee takes a
                // `&SecretString`: this call is synchronous, so the caller's secret
                // can simply be borrowed. It used to copy into a `Zeroizing<String>`
                // because the signature asked for `&str`.
                let result = rustconn_core::secret::KeePassStatus::save_password_to_kdbx(
                    kdbx,
                    settings.kdbx_password.as_ref(),
                    key,
                    &lookup_key,
                    "",
                    password,
                    None,
                    settings.kdbx_yubikey_slot.as_deref(),
                    settings.kdbx_read_only,
                );

                match result {
                    Ok(()) => Ok(()),
                    // Read-only is the user's refusal, not an outage: writing the
                    // keyring instead would violate it and leave a copy that
                    // shadows the database's own value.
                    Err(e @ rustconn_core::error::SecretError::ReadOnly(_)) => Err(format!("{e}")),
                    // If KeePass save failed and fallback is enabled, try LibSecret
                    Err(_) if settings.enable_fallback => {
                        tracing::info!(var_name, "KeePass save failed, falling back to LibSecret");
                        dispatch_vault_op(settings, &lookup_key, VaultOp::Store(&creds))?;
                        Ok(())
                    }
                    Err(e) => Err(format!("{e}")),
                }
            } else if settings.enable_fallback {
                tracing::info!(
                    var_name,
                    "KeePass not configured, falling back to LibSecret"
                );
                dispatch_vault_op(settings, &lookup_key, VaultOp::Store(&creds))?;
                Ok(())
            } else {
                Err("KeePass enabled but no database file configured".to_string())
            }
        }
        _ => {
            dispatch_vault_op(settings, &lookup_key, VaultOp::Store(&creds))?;
            Ok(())
        }
    }
}

/// Loads a secret variable value from the configured vault backend,
/// optionally using a custom KeePass entry path or vault entry name.
///
/// When `kdbx_entry_path` is `Some(path)`, the KeePass backend looks up
/// the entry at that exact path (the function prepends `RustConn/` prefix
/// is NOT added — the path is used as-is for direct entry lookup).
/// This allows referencing existing entries in the user's KeePass database.
///
/// When `vault_entry_name` is `Some(name)`, non-KeePass backends
/// (Bitwarden, 1Password, Passbolt, Pass) search for an existing entry
/// by exact name instead of the default `rustconn/var/{name}` key.
/// This allows reusing credentials already stored in the vault.
pub fn load_variable_from_vault_with_path(
    settings: &rustconn_core::config::SecretSettings,
    var_name: &str,
    kdbx_entry_path: Option<&str>,
    vault_entry_name: Option<&str>,
) -> Result<Option<zeroize::Zeroizing<String>>, String> {
    use rustconn_core::config::SecretBackendType;
    use secrecy::ExposeSecret;

    let default_key = rustconn_core::variable_secret_key(var_name);
    // Filter out empty/whitespace-only custom paths — treat them as "no custom path".
    let effective_custom_path = kdbx_entry_path.filter(|p| !p.trim().is_empty());
    let lookup_key = effective_custom_path.unwrap_or(&default_key);
    let backend_type = select_backend_for_load(settings);

    tracing::debug!(
        ?backend_type,
        var_name,
        lookup_key,
        "Loading secret variable from vault"
    );

    match backend_type {
        SecretBackendType::KdbxFile | SecretBackendType::KeePassXc => {
            if let Some(kdbx_path) = settings.kdbx_path.as_ref() {
                let key_file = settings.kdbx_key_file.clone();
                let kdbx = std::path::Path::new(kdbx_path);
                let key = key_file.as_ref().map(std::path::Path::new);

                // Custom path → exact lookup (no RustConn/ prefix, no fallbacks)
                // Default path → standard lookup with RustConn/ prefix and fallbacks
                let kdbx_result = if effective_custom_path.is_some() {
                    rustconn_core::secret::KeePassStatus::get_password_from_kdbx_exact(
                        kdbx,
                        settings.kdbx_password.as_ref(),
                        key,
                        lookup_key,
                        settings.kdbx_yubikey_slot.as_deref(),
                    )
                } else {
                    rustconn_core::secret::KeePassStatus::get_password_from_kdbx_with_key(
                        kdbx,
                        settings.kdbx_password.as_ref(),
                        key,
                        lookup_key,
                        None,
                        settings.kdbx_yubikey_slot.as_deref(),
                    )
                }
                .map(|opt| opt.map(|s| zeroize::Zeroizing::new(s.expose_secret().to_string())))
                .map_err(|e| format!("{e}"));

                // If KeePass returned Ok(None) or Err and fallback is enabled,
                // try LibSecret as a fallback (the variable may have been saved
                // there via the "Variable Not Configured" dialog).
                match &kdbx_result {
                    Ok(Some(_)) => kdbx_result,
                    Ok(None) | Err(_) if settings.enable_fallback => {
                        tracing::debug!(
                            var_name,
                            "KeePass lookup returned nothing, trying LibSecret fallback"
                        );
                        let fallback = dispatch_vault_op(settings, &default_key, VaultOp::Retrieve);
                        match fallback {
                            Ok(Some(creds)) if creds.expose_password().is_some() => Ok(creds
                                .expose_password()
                                .map(|p| zeroize::Zeroizing::new(p.to_string()))),
                            _ => kdbx_result,
                        }
                    }
                    _ => kdbx_result,
                }
            } else {
                Err("KeePass enabled but no database file configured".to_string())
            }
        }
        _ => {
            // For non-KeePass backends: if vault_entry_name is set, search by
            // exact name in the vault (Bitwarden, 1Password, etc.) instead of
            // the default rustconn/var/{name} key.
            let effective_entry_name = vault_entry_name.filter(|n| !n.trim().is_empty());

            if let Some(entry_name) = effective_entry_name {
                // Direct lookup by exact vault entry name
                retrieve_by_vault_entry_name(settings, entry_name)
            } else {
                let creds = dispatch_vault_op(settings, &default_key, VaultOp::Retrieve)?;
                Ok(creds.and_then(|c| {
                    c.expose_password()
                        .map(|p| zeroize::Zeroizing::new(p.to_string()))
                }))
            }
        }
    }
}

/// Retrieves a password from a vault entry matched by exact name.
///
/// Used when a variable has a custom `vault_entry_name` — searches
/// for an existing entry in Bitwarden/1Password/Passbolt/Pass by
/// its exact name (without `RustConn:` prefix or `rustconn/var/` key).
///
/// # Errors
/// Returns an error string if vault operations fail or time out.
fn retrieve_by_vault_entry_name(
    settings: &rustconn_core::config::SecretSettings,
    entry_name: &str,
) -> Result<Option<zeroize::Zeroizing<String>>, String> {
    use rustconn_core::config::SecretBackendType;
    use rustconn_core::secret::SecretBackend;
    use secrecy::ExposeSecret;

    let backend_type = select_backend_for_load(settings);
    // The Bitwarden arm below opens with `auto_unlock`, which is itself allowed
    // 30 s elsewhere in this module, and then runs a `bw list items` on top of it.
    // Ten seconds could not cover the first step alone.
    let budget = vault_op_timeout(backend_type);

    crate::async_utils::with_runtime(|rt| {
        rt.block_on(async {
            tokio::time::timeout(budget, async {
                match backend_type {
                    SecretBackendType::Bitwarden => {
                        let bw = rustconn_core::secret::auto_unlock(settings)
                            .await
                            .map_err(|e| format!("{e}"))?;
                        let password = bw
                            .find_password_by_exact_name(entry_name)
                            .await
                            .map_err(|e| format!("{e}"))?;
                        Ok(
                            password
                                .map(|p| zeroize::Zeroizing::new(p.expose_secret().to_string())),
                        )
                    }
                    SecretBackendType::OnePassword => {
                        // 1Password: use `op item get "{name}" --fields password`
                        let backend =
                            rustconn_core::secret::OnePasswordBackend::from_secret_settings(
                                settings,
                            );
                        let creds = backend
                            .retrieve(entry_name)
                            .await
                            .map_err(|e| format!("{e}"))?;
                        Ok(creds.and_then(|c| {
                            c.expose_password()
                                .map(|p| zeroize::Zeroizing::new(p.to_string()))
                        }))
                    }
                    SecretBackendType::Pass => {
                        // Pass: entry_name is the pass path (e.g. "work/ad-creds")
                        let backend =
                            rustconn_core::secret::PassBackend::from_secret_settings(settings);
                        let creds = backend
                            .retrieve(entry_name)
                            .await
                            .map_err(|e| format!("{e}"))?;
                        Ok(creds.and_then(|c| {
                            c.expose_password()
                                .map(|p| zeroize::Zeroizing::new(p.to_string()))
                        }))
                    }
                    SecretBackendType::Passbolt => {
                        let backend =
                            rustconn_core::secret::PassboltBackend::from_secret_settings(settings);
                        let creds = backend
                            .retrieve(entry_name)
                            .await
                            .map_err(|e| format!("{e}"))?;
                        Ok(creds.and_then(|c| {
                            c.expose_password()
                                .map(|p| zeroize::Zeroizing::new(p.to_string()))
                        }))
                    }
                    #[cfg(target_os = "macos")]
                    SecretBackendType::MacOsKeychain => {
                        let backend = rustconn_core::secret::MacOsKeychainBackend::new();
                        let creds = backend
                            .retrieve(entry_name)
                            .await
                            .map_err(|e| format!("{e}"))?;
                        Ok(creds.and_then(|c| {
                            c.expose_password()
                                .map(|p| zeroize::Zeroizing::new(p.to_string()))
                        }))
                    }
                    SecretBackendType::EncryptedFile => {
                        // Application-managed encrypted file: retrieve by the
                        // flat entry name (same key scheme as the other
                        // app-managed backends).
                        let backend = rustconn_core::secret::EncryptedFileBackend::new();
                        let creds = backend
                            .retrieve(entry_name)
                            .await
                            .map_err(|e| format!("{e}"))?;
                        Ok(creds.and_then(|c| {
                            c.expose_password()
                                .map(|p| zeroize::Zeroizing::new(p.to_string()))
                        }))
                    }
                    SecretBackendType::PortableEncryptedFile => {
                        // Passphrase-based portable file: needs the session
                        // passphrase from settings to unlock.
                        let backend = portable_backend_from_settings(settings);
                        let creds = backend
                            .retrieve(entry_name)
                            .await
                            .map_err(|e| format!("{e}"))?;
                        Ok(creds.and_then(|c| {
                            c.expose_password()
                                .map(|p| zeroize::Zeroizing::new(p.to_string()))
                        }))
                    }
                    _ => {
                        // System keyring — lookup by entry_name as attribute.
                        // macOS uses the Keychain; LibSecretBackend (oo7) is
                        // not compiled there (R10.1, R10.2).
                        #[cfg(target_os = "macos")]
                        let backend = rustconn_core::secret::MacOsKeychainBackend::new();
                        #[cfg(not(target_os = "macos"))]
                        let backend = rustconn_core::secret::LibSecretBackend::new("rustconn");
                        let creds = backend
                            .retrieve(entry_name)
                            .await
                            .map_err(|e| format!("{e}"))?;
                        Ok(creds.and_then(|c| {
                            c.expose_password()
                                .map(|p| zeroize::Zeroizing::new(p.to_string()))
                        }))
                    }
                }
            })
            .await
            .map_err(|_| vault_op_timed_out("retrieve by entry name", budget))?
        })
    })?
}

/// Returns global variables with secret values restored from vault.
///
/// Non-secret variables are returned as-is. Secret variables with empty
/// values have their values loaded from the configured vault backend.
/// Vault load failures are logged but do not prevent other variables
/// from being returned.
///
/// When a variable has a custom `kdbx_entry_path`, that path is used
/// for KeePass lookup instead of the default `rustconn/var/{name}`.
pub fn resolve_global_variables(
    settings: &rustconn_core::config::AppSettings,
) -> Vec<rustconn_core::Variable> {
    use zeroize::Zeroize;

    let mut vars = settings.global_variables.clone();
    for var in &mut vars {
        if var.is_secret && var.value.is_empty() {
            match load_variable_from_vault_with_path(
                &settings.secrets,
                &var.name,
                var.kdbx_entry_path.as_deref(),
                var.vault_entry_name.as_deref(),
            ) {
                Ok(Some(pwd)) => {
                    // Zeroize the previous value before overwriting — clone_from
                    // frees the old buffer via the allocator (no zeroize), so a
                    // stale secret could linger in freed heap memory.
                    var.value.zeroize();
                    var.value.clone_from(&pwd);
                }
                Ok(None) => {
                    tracing::debug!(var_name = %var.name, "No secret found in vault for variable");
                }
                Err(e) => {
                    tracing::warn!(var_name = %var.name, error = %e, "Failed to load secret variable from vault");
                }
            }
        }
    }
    vars
}

/// Builds every vault lookup key a connection's credentials may live under on a
/// non-KeePass backend, most-current format first.
///
/// The first entry is the key the current code stores under; the rest are
/// formats written by earlier releases, kept so that cleanup and fallback
/// retrieval still find them.
///
/// For LibSecret and the macOS Keychain the current key embeds the group path
/// (`RustConn/{group}/{name} ({protocol})`) so same-named connections in
/// different groups no longer collide (issue #264). Deleting by the flat
/// `{name} ({protocol})` key alone therefore matched nothing and silently left
/// the keyring item behind (issue #263) — the attribute search in
/// `LibSecretBackend::delete_value` finds zero items and reports success.
///
/// The legacy name-keyed formats are deliberately included even though two
/// same-named connections in different groups map onto the same legacy key, so
/// cleaning up one can remove an entry the other still resolves through. That is
/// acceptable because a shared legacy key means the two connections were already
/// sharing a single stored password — which is precisely the collision reported
/// in issue #264 — so there is no distinct credential to lose. Once either
/// connection has been resolved on 0.19.18 or later, its credential lives under
/// its own group-scoped key and is unaffected.
fn vault_keys_for_connection(
    groups: &[rustconn_core::models::ConnectionGroup],
    connection: &rustconn_core::models::Connection,
    protocol_str: &str,
    backend_type: rustconn_core::config::SecretBackendType,
) -> Vec<String> {
    use rustconn_core::config::SecretBackendType;

    let mut keys = Vec::new();

    if matches!(
        backend_type,
        SecretBackendType::LibSecret | SecretBackendType::MacOsKeychain
    ) {
        // Primary: exactly what `save_password_to_vault` writes. The ungrouped
        // case — the `RustConn/` prefix with no group segment — is decided inside
        // `generate_store_key_for_connection`; this call site used to make it
        // itself, which is how the two sites that made it differently went
        // unnoticed (issue #316).
        keys.push(generate_store_key_for_connection(
            &connection.name,
            &connection.host,
            protocol_str,
            backend_type,
            connection.group_id,
            groups,
        ));
        // The resolver builds the same shape but additionally runs the name
        // through `sanitize_imported_value`, so an imported name with trailing
        // escape sequences can land under a second key. Read-time migration
        // writes that variant, so cleanup has to cover it too.
        keys.push(
            rustconn_core::secret::CredentialResolver::generate_keyring_key_with_hierarchy(
                connection, groups,
            ),
        );
        // Pre-0.19.18 flat key, before the group path was part of the key.
        keys.push(rustconn_core::secret::CredentialResolver::generate_keyring_key(connection));
        // Pre-0.19.19 macOS Keychain key: the store path wrote the flat
        // `rustconn/{name}` format while the resolver looked for the
        // hierarchical one, so entries can still exist under it.
        let identifier = if connection.name.trim().is_empty() {
            &connection.host
        } else {
            &connection.name
        };
        keys.push(format!("rustconn/{identifier}"));
    } else {
        keys.push(generate_store_key(
            &connection.name,
            &connection.host,
            protocol_str,
            backend_type,
        ));
    }

    // Oldest format: the connection UUID.
    keys.push(connection.id.to_string());

    keys.dedup();
    keys
}

/// Deletes a connection's vault credentials from the configured backend.
///
/// For KeePass backends, deletes the hierarchical entry. For flat backends,
/// every key format from [`vault_keys_for_connection`] is deleted so that
/// entries written by earlier releases do not survive the connection.
///
/// This runs on permanent deletion — either when the undo window for a deleted
/// connection closes, or when the trash is emptied — never on the soft-delete
/// itself, so that Undo restores a connection with its password intact.
pub fn delete_vault_credential(
    settings: &rustconn_core::config::AppSettings,
    groups: &[rustconn_core::models::ConnectionGroup],
    connection: &rustconn_core::models::Connection,
) -> Result<(), String> {
    use rustconn_core::config::SecretBackendType;

    let protocol_str = connection
        .protocol_config
        .protocol_type()
        .as_str()
        .to_lowercase();
    let backend_type = select_backend_for_load(&settings.secrets);

    tracing::debug!(
        ?backend_type,
        connection_name = %connection.name,
        protocol = %protocol_str,
        "Deleting vault credential for connection"
    );

    match backend_type {
        SecretBackendType::KdbxFile | SecretBackendType::KeePassXc => {
            if let Some(kdbx_path) = settings.secrets.kdbx_path.as_ref() {
                // Built by the same helper the Trash guard compares with.
                let full_entry_path = kdbx_connection_entry(groups, connection, &protocol_str);
                let key_file = settings.secrets.kdbx_key_file.clone();
                let kdbx = std::path::Path::new(kdbx_path);
                let key = key_file.as_ref().map(std::path::Path::new);
                // Actually remove the entry via keepassxc-cli (previously this
                // only overwrote it with an empty password as a best-effort,
                // leaving the entry behind in the database).
                rustconn_core::secret::KeePassStatus::delete_entry_from_kdbx(
                    kdbx,
                    settings.secrets.kdbx_password.as_ref(),
                    key,
                    &full_entry_path,
                    settings.secrets.kdbx_yubikey_slot.as_deref(),
                    settings.secrets.kdbx_read_only,
                )
                .map_err(|e| format!("{e}"))
            } else {
                Ok(()) // No KDBX configured, nothing to clean
            }
        }
        _ => {
            // Delete every key format this connection may have been stored
            // under. Best-effort per key: a backend that does not hold one of
            // the legacy keys must not abort cleanup of the others, but a
            // failure on the primary key is still reported.
            let keys = vault_keys_for_connection(groups, connection, &protocol_str, backend_type);
            let mut primary_result = Ok(());
            for (index, key) in keys.iter().enumerate() {
                let result = dispatch_vault_op(&settings.secrets, key, VaultOp::Delete).map(|_| ());
                if let Err(ref e) = result {
                    tracing::debug!(
                        lookup_key = %key,
                        error = %e,
                        "Vault credential delete failed for one key format"
                    );
                }
                if index == 0 {
                    primary_result = result;
                }
            }
            primary_result
        }
    }
}

/// The KDBX entry [`delete_vault_credential`] removes for `connection`: the
/// full hierarchical path, `RustConn/` prefix included, plus the protocol
/// suffix — e.g. `"RustConn/Group/Name (rdp)"`, the format
/// `delete_entry_from_kdbx` (and the CLI) expect.
fn kdbx_connection_entry(
    groups: &[rustconn_core::models::ConnectionGroup],
    connection: &rustconn_core::models::Connection,
    protocol_str: &str,
) -> String {
    let entry_path = rustconn_core::secret::KeePassHierarchy::build_entry_path(connection, groups);
    format!("{entry_path} ({protocol_str})")
}

/// Every key [`delete_vault_credential`] removes for `connection` on
/// `backend_type`, the key the credential is stored under today first.
fn vault_keys_deleted_for(
    groups: &[rustconn_core::models::ConnectionGroup],
    connection: &rustconn_core::models::Connection,
    backend_type: rustconn_core::config::SecretBackendType,
) -> Vec<String> {
    use rustconn_core::config::SecretBackendType;

    let protocol_str = connection
        .protocol_config
        .protocol_type()
        .as_str()
        .to_lowercase();
    if matches!(
        backend_type,
        SecretBackendType::KeePassXc | SecretBackendType::KdbxFile
    ) {
        return vec![kdbx_connection_entry(groups, connection, &protocol_str)];
    }
    vault_keys_for_connection(groups, connection, &protocol_str, backend_type)
}

/// The key `connection`'s credential is stored under today: the first of the
/// keys [`delete_vault_credential`] would remove for it.
fn current_vault_key(
    groups: &[rustconn_core::models::ConnectionGroup],
    connection: &rustconn_core::models::Connection,
    backend_type: rustconn_core::config::SecretBackendType,
) -> Option<String> {
    vault_keys_deleted_for(groups, connection, backend_type)
        .into_iter()
        .next()
}

/// Finds a live connection whose credential would go with `trashed`'s if the
/// latter were deleted now.
///
/// A vault key is built from a connection's name, group path and protocol —
/// never from its id — so a trashed connection and a live one that agree on
/// those three resolve to the same entry. Group Sync produces exactly that
/// pair whenever it recreates a connection: the old copy goes to the Trash,
/// and the new one keeps the name and lands in the same group. Emptying the
/// Trash then deleted the live copy's password together with the old one's.
///
/// Each live connection's current key is compared with every key deleting
/// `trashed` removes, using the builders [`delete_vault_credential`] uses, so
/// `groups` must be the hierarchy that delete is called with. Any live
/// connection counts, whatever its password source: keeping a shared entry
/// costs an orphan at worst, deleting it costs a password.
#[must_use]
pub fn live_connection_sharing_vault_key<'a>(
    settings: &rustconn_core::config::AppSettings,
    groups: &[rustconn_core::models::ConnectionGroup],
    trashed: &rustconn_core::models::Connection,
    live: &'a [rustconn_core::models::Connection],
) -> Option<&'a rustconn_core::models::Connection> {
    let backend_type = select_backend_for_load(&settings.secrets);
    let doomed = vault_keys_deleted_for(groups, trashed, backend_type);
    // ponytail: rebuilds every live key for each trashed connection, so a purge
    // is O(trashed × live); fine for a Trash of dozens against hundreds of
    // connections — collect the live keys into a HashSet once per purge if a
    // profile ever shows it.
    live.iter()
        .filter(|candidate| candidate.id != trashed.id)
        .find(|candidate| {
            current_vault_key(groups, candidate, backend_type)
                .is_some_and(|key| doomed.contains(&key))
        })
}

/// Deletes a trashed connection's vault credential, unless a live connection
/// still resolves to the same entry (see [`live_connection_sharing_vault_key`]).
///
/// Used by every permanent delete — the Undo window closing and emptying the
/// Trash — in place of a bare [`delete_vault_credential`].
///
/// # Errors
///
/// Returns the error of [`delete_vault_credential`]. Keeping a shared entry is
/// not an error.
pub fn delete_trashed_vault_credential(
    settings: &rustconn_core::config::AppSettings,
    groups: &[rustconn_core::models::ConnectionGroup],
    trashed: &rustconn_core::models::Connection,
    live: &[rustconn_core::models::Connection],
) -> Result<(), String> {
    if let Some(twin) = live_connection_sharing_vault_key(settings, groups, trashed, live) {
        tracing::debug!(
            connection_id = %trashed.id,
            connection_name = %trashed.name,
            live_connection_id = %twin.id,
            "Kept a trashed connection's vault credential: a live connection uses the same key"
        );
        return Ok(());
    }
    delete_vault_credential(settings, groups, trashed)
}

/// Deletes a group's vault credentials from the configured backend.
///
/// Similar to [`delete_vault_credential`] but for group-level passwords.
pub fn delete_group_vault_credential(
    settings: &rustconn_core::config::AppSettings,
    groups: &[rustconn_core::models::ConnectionGroup],
    group: &rustconn_core::models::ConnectionGroup,
) -> Result<(), String> {
    use rustconn_core::config::SecretBackendType;

    let backend_type = select_backend_for_load(&settings.secrets);

    tracing::debug!(
        ?backend_type,
        group_name = %group.name,
        "Deleting vault credential for group"
    );

    match backend_type {
        SecretBackendType::KdbxFile | SecretBackendType::KeePassXc => {
            if let Some(kdbx_path) = settings.secrets.kdbx_path.as_ref() {
                let group_path =
                    rustconn_core::secret::KeePassHierarchy::build_group_entry_path(group, groups);
                let key_file = settings.secrets.kdbx_key_file.clone();
                let kdbx = std::path::Path::new(kdbx_path);
                let key = key_file.as_ref().map(std::path::Path::new);
                // Actually remove the entry. Overwriting it with an empty
                // username/password (the previous behaviour) left a visible
                // orphan entry in the user's database after the group was gone.
                rustconn_core::secret::KeePassStatus::delete_entry_from_kdbx(
                    kdbx,
                    settings.secrets.kdbx_password.as_ref(),
                    key,
                    &group_path,
                    settings.secrets.kdbx_yubikey_slot.as_deref(),
                    settings.secrets.kdbx_read_only,
                )
                .map_err(|e| format!("{e}"))
            } else {
                Ok(())
            }
        }
        _ => {
            let lookup_key = group.id.to_string();
            dispatch_vault_op(&settings.secrets, &lookup_key, VaultOp::Delete)?;
            Ok(())
        }
    }
}

/// Copies vault credentials from one connection to another.
///
/// Retrieves credentials under the old connection's key and stores them
/// under the new connection's key. Used during clipboard paste to duplicate
/// credentials for the copied connection.
pub fn copy_vault_credential(
    settings: &rustconn_core::config::AppSettings,
    groups: &[rustconn_core::models::ConnectionGroup],
    old_conn: &rustconn_core::models::Connection,
    new_conn: &rustconn_core::models::Connection,
) -> Result<(), String> {
    use rustconn_core::config::SecretBackendType;

    let protocol_str = old_conn
        .protocol_config
        .protocol_type()
        .as_str()
        .to_lowercase();
    let backend_type = select_backend_for_load(&settings.secrets);

    tracing::debug!(
        ?backend_type,
        old_name = %old_conn.name,
        new_name = %new_conn.name,
        "Copying vault credential for pasted connection"
    );

    match backend_type {
        SecretBackendType::KdbxFile | SecretBackendType::KeePassXc => {
            if let Some(kdbx_path) = settings.secrets.kdbx_path.as_ref() {
                let key_file = settings.secrets.kdbx_key_file.clone();
                let kdbx = std::path::Path::new(kdbx_path);
                let key = key_file.as_ref().map(std::path::Path::new);

                // Read from old entry
                let old_entry_path =
                    rustconn_core::secret::KeePassHierarchy::build_entry_path(old_conn, groups);
                let old_base = old_entry_path
                    .strip_prefix("RustConn/")
                    .unwrap_or(&old_entry_path);
                let old_entry_name = format!("{old_base} ({protocol_str})");

                let password_opt =
                    rustconn_core::secret::KeePassStatus::get_password_from_kdbx_with_key(
                        kdbx,
                        settings.secrets.kdbx_password.as_ref(),
                        key,
                        &old_entry_name,
                        None,
                        settings.secrets.kdbx_yubikey_slot.as_deref(),
                    )
                    .map_err(|e| format!("{e}"))?;

                if let Some(pwd) = password_opt {
                    // Write to new entry
                    let new_entry_path =
                        rustconn_core::secret::KeePassHierarchy::build_entry_path(new_conn, groups);
                    let new_base = new_entry_path
                        .strip_prefix("RustConn/")
                        .unwrap_or(&new_entry_path);
                    let new_entry_name = format!("{new_base} ({protocol_str})");
                    let username = new_conn.username.as_deref().unwrap_or("");
                    let url = format!("{}://{}", protocol_str, new_conn.host);
                    rustconn_core::secret::KeePassStatus::save_password_to_kdbx(
                        kdbx,
                        settings.secrets.kdbx_password.as_ref(),
                        key,
                        &new_entry_name,
                        username,
                        &pwd,
                        Some(&url),
                        settings.secrets.kdbx_yubikey_slot.as_deref(),
                        settings.secrets.kdbx_read_only,
                    )
                    .map_err(|e| format!("{e}"))?;
                }
                Ok(())
            } else {
                Ok(())
            }
        }
        _ => {
            // Both keys must account for the group path on LibSecret/Keychain,
            // otherwise a paste into another group reads nothing and writes the
            // copy where the resolver will never look for it (issue #264).
            let old_keys = vault_keys_for_connection(groups, old_conn, &protocol_str, backend_type);
            let new_keys = vault_keys_for_connection(groups, new_conn, &protocol_str, backend_type);

            for old_key in &old_keys {
                if let Some(creds) =
                    dispatch_vault_op(&settings.secrets, old_key, VaultOp::Retrieve)?
                {
                    let new_key = new_keys.first().map_or(old_key.as_str(), String::as_str);
                    dispatch_vault_op(&settings.secrets, new_key, VaultOp::Store(&creds))?;
                    break;
                }
            }
            Ok(())
        }
    }
}

/// Operation to perform on a vault backend.
///
/// Used by [`dispatch_vault_op`] to consolidate the repeated
/// `match backend_type { … }` dispatch blocks throughout this module.
pub enum VaultOp<'a> {
    /// Store credentials under the given key.
    Store(&'a rustconn_core::models::Credentials),
    /// Retrieve credentials for the given key.
    Retrieve,
    /// Delete credentials for the given key.
    Delete,
}

/// Dispatches a single vault operation to the configured non-KeePass backend.
///
/// This helper eliminates the repeated `match backend_type` blocks that were
/// duplicated across `save_password_to_vault`, `save_group_password_to_vault`,
/// `rename_vault_credential`, `resolve_credentials_blocking` (Inherit branch),
/// and credential cleanup on delete.
///
/// For KeePass backends, callers must handle KDBX operations directly because
/// they use a different API (`save_password_to_kdbx` / `get_password_from_kdbx`).
///
/// # Errors
///
/// Returns a human-readable error string if the backend is unavailable or the
/// operation fails.
///
/// # See also
///
/// - [`CredentialResolver::resolve_inherited_credentials`] — async equivalent
///   in `rustconn-core`
pub fn dispatch_vault_op(
    secret_settings: &rustconn_core::config::SecretSettings,
    lookup_key: &str,
    op: VaultOp<'_>,
) -> Result<Option<rustconn_core::models::Credentials>, String> {
    dispatch_vault_op_for(
        secret_settings,
        select_backend_for_load(secret_settings),
        lookup_key,
        op,
    )
}

/// Constructs one backend of the named type from `secret_settings`.
///
/// Split out of [`dispatch_vault_op_for`] so a caller that performs many
/// operations against the same backend can build it **once**. That is not a
/// micro-optimisation for the portable file: its `store` derives the data key
/// from the passphrase with Argon2id, and the derivation is cached *per backend
/// instance*, so a fresh instance per credential means a full ~0.5 s KDF pass
/// each time. Its `write_lock` is per instance too, so separate instances
/// serialise nothing against each other.
///
/// `KeePassXc` / `KdbxFile` deliberately resolve to the system keyring here,
/// because KDBX proper is not a [`SecretBackend`] — it goes through
/// `KeePassStatus` and the `keepassxc-cli` binary. Callers that mean the database
/// itself must intercept those two variants before calling this.
///
/// # Errors
///
/// Returns a human-readable error when the backend cannot be constructed, which
/// today only happens for Bitwarden, whose construction includes an unlock.
fn build_single_backend(
    secret_settings: &rustconn_core::config::SecretSettings,
    backend_type: rustconn_core::config::SecretBackendType,
    rt: &tokio::runtime::Runtime,
) -> Result<std::sync::Arc<dyn rustconn_core::secret::SecretBackend>, String> {
    use rustconn_core::config::SecretBackendType;
    use rustconn_core::secret::SecretBackend;

    let backend: std::sync::Arc<dyn SecretBackend> = match backend_type {
        SecretBackendType::Bitwarden => std::sync::Arc::new(rt.block_on(async {
            tokio::time::timeout(
                std::time::Duration::from_secs(30),
                rustconn_core::secret::auto_unlock(secret_settings),
            )
            .await
            .map_err(|_| "Bitwarden auto-unlock timed out after 30s".to_string())?
            .map_err(|e| format!("{e}"))
        })?),
        // `from_secret_settings` (and `auto_unlock` above) carry the read-only and
        // root-search toggles; building these by hand dropped them, so every
        // write routed through here ignored read-only mode.
        SecretBackendType::OnePassword => std::sync::Arc::new(
            rustconn_core::secret::OnePasswordBackend::from_secret_settings(secret_settings),
        ),
        SecretBackendType::Passbolt => std::sync::Arc::new(
            rustconn_core::secret::PassboltBackend::from_secret_settings(secret_settings),
        ),
        SecretBackendType::Pass => std::sync::Arc::new(
            rustconn_core::secret::PassBackend::from_secret_settings(secret_settings),
        ),
        #[cfg(target_os = "macos")]
        SecretBackendType::MacOsKeychain => {
            std::sync::Arc::new(rustconn_core::secret::MacOsKeychainBackend::new())
        }
        #[cfg(not(target_os = "macos"))]
        SecretBackendType::MacOsKeychain => {
            std::sync::Arc::new(rustconn_core::secret::LibSecretBackend::new("rustconn"))
        }
        SecretBackendType::LibSecret
        | SecretBackendType::KeePassXc
        | SecretBackendType::KdbxFile => {
            // macOS uses the system Keychain; LibSecretBackend (oo7) is not
            // compiled there (R10.1, R10.2). Non-macOS keeps libsecret.
            #[cfg(target_os = "macos")]
            {
                std::sync::Arc::new(rustconn_core::secret::MacOsKeychainBackend::new())
            }
            #[cfg(not(target_os = "macos"))]
            {
                std::sync::Arc::new(rustconn_core::secret::LibSecretBackend::new("rustconn"))
            }
        }
        SecretBackendType::EncryptedFile => {
            // Application-managed encrypted file; addressed by the flat
            // lookup key, same as the other app-managed backends.
            std::sync::Arc::new(rustconn_core::secret::EncryptedFileBackend::new())
        }
        SecretBackendType::PortableEncryptedFile => {
            // Passphrase-based portable encrypted file.
            std::sync::Arc::new(portable_backend_from_settings(secret_settings))
        }
    };
    Ok(backend)
}

/// Serialises every vault backend operation across the process.
///
/// The connect path resolves credentials on a background thread
/// ([`spawn_blocking_with_callback`]), so opening several RDP connections in
/// quick succession fires several `dispatch_vault_op_for` calls at once. Without
/// a lock they collide on the one backend underneath — a CLI backend runs
/// `bw`/`op`/`pass` twice, a `keepassxc-cli` database opens twice, Secret Service
/// gets two concurrent D-Bus round trips — and the loser stalls long enough to
/// hit [`vault_op_timeout`]. A timed-out or empty resolve then collapses to
/// [`CredentialResolutionResult::NotNeeded`] and the connection falls through to
/// the password prompt, which is exactly the "asks for a password even though it
/// is saved" race the RDP reporter hit.
///
/// A plain [`std::sync::Mutex`]: these calls never run on the GTK thread, so
/// blocking one background resolve behind another is a short serial wait, not a
/// frozen window. It does not dedupe identical requests — the app-level
/// `password_cache` is what keeps the second connect from looking up at all once
/// the first has warmed it — it only stops two lookups from running *at the same
/// time*. A poisoned lock (a panic mid-operation) must not wedge every future
/// vault call, so the guard is recovered with `into_inner()`.
static VAULT_OP_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Dispatches a single vault operation to an explicitly named backend.
///
/// [`dispatch_vault_op`] always addresses whichever backend the settings prefer,
/// which is right for saving and deleting a credential. A credential *transfer*
/// has to address two backends in one operation, neither of which need be the
/// preferred one, so the type is a parameter here.
///
/// `secret_settings` is still needed in full: it carries the per-backend
/// configuration (Bitwarden unlock, the 1Password token, the Passbolt server and
/// passphrase, the `pass` store directory, the portable file's path and
/// passphrase) that constructing the backend requires.
///
/// `KeePassXc` / `KdbxFile` are **not** handled here. They resolve to the system
/// keyring, which is correct for `dispatch_vault_op`'s callers — KDBX proper goes
/// through `KeePassStatus` — but would be a silent substitution for a transfer
/// that names KeePass explicitly. [`TransferPort`] intercepts those two variants
/// for that reason.
///
/// # Errors
///
/// Returns a human-readable error string if the backend is unavailable or the
/// operation fails.
pub fn dispatch_vault_op_for(
    secret_settings: &rustconn_core::config::SecretSettings,
    backend_type: rustconn_core::config::SecretBackendType,
    lookup_key: &str,
    op: VaultOp<'_>,
) -> Result<Option<rustconn_core::models::Credentials>, String> {
    // One budget for whichever operation follows, derived from the backend that
    // was named rather than assumed to be a local one.
    let budget = vault_op_timeout(backend_type);

    // Serialise against any other in-flight vault op — see VAULT_OP_LOCK. Held
    // for the whole build-plus-operation so a concurrent call cannot slip its
    // backend construction (a Bitwarden unlock, an Argon2id derivation) between
    // this one's lock and its work. Recover a poisoned guard rather than
    // propagating the panic and wedging the backend for the session.
    let _serial = VAULT_OP_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    crate::async_utils::with_runtime(|rt| {
        let backend = build_single_backend(secret_settings, backend_type, rt)?;

        match op {
            VaultOp::Store(creds) => {
                tracing::debug!(
                    %lookup_key,
                    ?backend_type,
                    "dispatch_vault_op: storing credentials"
                );
                rt.block_on(async {
                    tokio::time::timeout(budget, backend.store(lookup_key, creds))
                        .await
                        .map_err(|_| vault_op_timed_out("store", budget))?
                        .map_err(|e| format!("{e}"))
                })?;
                tracing::debug!(%lookup_key, "dispatch_vault_op: store succeeded");
                Ok(None)
            }
            VaultOp::Retrieve => {
                tracing::debug!(
                    %lookup_key,
                    ?backend_type,
                    "dispatch_vault_op: retrieving credentials"
                );
                let result = rt.block_on(async {
                    tokio::time::timeout(budget, backend.retrieve(lookup_key))
                        .await
                        .map_err(|_| vault_op_timed_out("retrieve", budget))?
                        .map_err(|e| format!("{e}"))
                })?;
                tracing::debug!(
                    %lookup_key,
                    found = result.is_some(),
                    "dispatch_vault_op: retrieve completed"
                );
                Ok(result)
            }
            VaultOp::Delete => {
                rt.block_on(async {
                    tokio::time::timeout(budget, backend.delete(lookup_key))
                        .await
                        .map_err(|_| vault_op_timed_out("delete", budget))?
                        .map_err(|e| format!("{e}"))
                })?;
                Ok(None)
            }
        }
    })
    .and_then(|r| r)
}

// ─────────────────────────────────────────────────────────────────────────────
// Credential transfer between backends
// ─────────────────────────────────────────────────────────────────────────────

/// How long one entry's read or write may take before the transfer moves on.
///
/// Ten seconds, matching what the rest of this module allows a single vault
/// operation. It is per entry rather than per batch: forty entries are allowed
/// forty of these, because the alternative — one budget for the whole run — makes
/// the last entries fail for the first entries' slowness.
const TRANSFER_OP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// The message for an operation that ran out of time.
///
/// One function so the two arms of each operation cannot drift into reporting
/// different wordings, and so the duration is stated once.
fn timed_out(operation: &str) -> String {
    format!(
        "the {operation} timed out after {}s",
        TRANSFER_OP_TIMEOUT.as_secs()
    )
}

/// One side of a transfer, opened once for the whole batch.
///
/// The KDBX arms run their `keepassxc-cli` call on a blocking task under a
/// [`TRANSFER_OP_TIMEOUT`], which the backend arms already had and they did not.
/// This bounds *the transfer*, not the child process: `std::process::Command` has
/// no timeout, so a `keepassxc-cli` that never returns — a database on an
/// unreachable mount, a hardware key waiting for a touch — leaves its blocking
/// task parked until the process exits. That is a leaked thread against a loop
/// that would otherwise stop dead on one entry and never reach the other thirty
/// nine, which is the worse of the two.
///
/// Two consequences of that abandoned task are worth stating rather than
/// discovering. Its `Zeroizing` plaintext password — and its handle on the shared
/// database password — are released when the closure returns, so a task that never
/// returns holds both for as long as the child lives: the timeout bounds the
/// *transfer*, not the exposure. And on the
/// write side the abandoned `keepassxc-cli` is still writing while the loop moves
/// on, so two of them can touch one `.kdbx`; `save_password_to_kdbx` removes an
/// existing entry before adding it, so a timed-out write can leave its entry
/// deleted in the destination. The source is untouched either way and running the
/// transfer again repairs it, which is why this is a documented edge and not a
/// refusal to proceed.
///
/// Two reasons this is not just a [`SecretBackendType`] passed to
/// [`dispatch_vault_op_for`] per entry. The portable store caches its
/// passphrase-derived data key *per backend instance*, so a fresh instance per
/// credential pays a full Argon2id derivation each time — roughly half a second
/// — and its write mutex is per instance too, so separate instances serialise
/// nothing. And the checks that decide whether a side is usable at all belong
/// before the loop, not once per entry: a locked KeePass database should say so
/// once, not fail forty times.
enum TransferPort {
    /// A [`SecretBackend`](rustconn_core::secret::SecretBackend) instance, reused
    /// for every entry.
    Backend(std::sync::Arc<dyn rustconn_core::secret::SecretBackend>),
    /// The KeePass database, which is not a `SecretBackend`: it is reached through
    /// `KeePassStatus` and the `keepassxc-cli` binary.
    Kdbx {
        /// Database file.
        path: std::path::PathBuf,
        /// Database password, when the database uses one.
        ///
        /// Behind an `Arc` because each entry's call moves its inputs into a
        /// blocking task, and `SecretString` is `SecretBox<str>`: cloning it
        /// copies the plaintext into a fresh allocation. That would be one
        /// transient plaintext copy of the database master password per entry,
        /// and — since a timed-out task keeps running — several live at once.
        /// `Arc::clone` copies a pointer and leaves one copy for the whole batch.
        db_password: Option<std::sync::Arc<secrecy::SecretString>>,
        /// Key file, when the database uses one.
        key_file: Option<std::path::PathBuf>,
        /// `YubiKey` Challenge-Response slot (`slot[:serial]`), when the database
        /// uses one as a second factor. Passed through to `keepassxc-cli` as `-y`.
        yubikey_slot: Option<String>,
        /// `SecretSettings::kdbx_read_only`. A transfer *into* a read-only
        /// database is refused per entry by the writer itself.
        read_only: bool,
    },
}

impl TransferPort {
    /// Opens one side of a transfer, refusing up front what cannot work.
    ///
    /// # Errors
    /// Returns a human-readable reason the side cannot be used: no database
    /// configured, the KeePass integration switched off, a database whose
    /// password is not available, or a backend that could not be constructed.
    fn open(
        settings: &rustconn_core::config::AppSettings,
        backend_type: rustconn_core::config::SecretBackendType,
        rt: &tokio::runtime::Runtime,
    ) -> Result<Self, String> {
        use rustconn_core::config::SecretBackendType;

        if !matches!(
            backend_type,
            SecretBackendType::KeePassXc | SecretBackendType::KdbxFile
        ) {
            return build_single_backend(&settings.secrets, backend_type, rt).map(Self::Backend);
        }

        // Everything below is a precondition every *reader* of a KDBX entry
        // already applies. Skipping them would let the transfer write into a
        // database that nothing will look at, or fail once per entry on a
        // database it was never going to be able to open.
        let Some(path) = settings.secrets.kdbx_path.clone() else {
            return Err("no KeePass database is configured".to_string());
        };
        if !settings.secrets.kdbx_enabled {
            // `select_backend_for_load` and the resolver both require this flag
            // before they will touch a KDBX entry; without it they use the
            // keyring instead. Writing here anyway produces entries that are
            // never read.
            return Err(
                "KDBX integration is switched off, so entries in the database would not be used"
                    .to_string(),
            );
        }
        if settings.secrets.kdbx_use_password && settings.secrets.kdbx_password.is_none() {
            // `keepassxc-cli` would get an empty stdin and fail to unlock, once
            // per entry. The resolver reports this as a lockout and prompts; here
            // the honest answer is to refuse before starting.
            return Err(
                "the KeePass database password is not available — set it in Settings ▸ Secrets"
                    .to_string(),
            );
        }

        Ok(Self::Kdbx {
            path,
            // One plaintext copy for the batch; see the field's documentation.
            db_password: settings
                .secrets
                .kdbx_password
                .clone()
                .map(std::sync::Arc::new),
            key_file: settings.secrets.kdbx_key_file.clone(),
            yubikey_slot: settings.secrets.kdbx_yubikey_slot.clone(),
            read_only: settings.secrets.kdbx_read_only,
        })
    }

    /// Reads one credential, trying `item.source_keys` in order.
    ///
    /// # Errors
    /// Returns the backend's failure. A key that simply holds nothing is
    /// `Ok(None)`, not an error.
    fn retrieve(
        &self,
        item: &CredentialTransferItem,
        rt: &tokio::runtime::Runtime,
    ) -> Result<Option<rustconn_core::models::Credentials>, String> {
        let mut last_error = None;
        for key in &item.source_keys {
            let outcome = match self {
                Self::Backend(backend) => rt.block_on(async {
                    tokio::time::timeout(TRANSFER_OP_TIMEOUT, backend.retrieve(key))
                        .await
                        .map_err(|_| timed_out("read"))?
                        .map_err(|e| format!("{e}"))
                }),
                Self::Kdbx {
                    path,
                    db_password,
                    key_file,
                    yubikey_slot,
                    read_only: _,
                } => {
                    let path = path.clone();
                    let db_password = db_password.as_ref().map(std::sync::Arc::clone);
                    let key_file = key_file.clone();
                    let yubikey_slot = yubikey_slot.clone();
                    let entry = key.clone();
                    let username = item.username.clone();
                    rt.block_on(async move {
                        tokio::time::timeout(
                            TRANSFER_OP_TIMEOUT,
                            tokio::task::spawn_blocking(move || {
                                rustconn_core::secret::KeePassStatus::get_password_from_kdbx_with_key(
                                    &path,
                                    db_password.as_deref(),
                                    key_file.as_deref(),
                                    &entry,
                                    None,
                                    yubikey_slot.as_deref(),
                                )
                            }),
                        )
                        .await
                        .map_err(|_| timed_out("read"))?
                        .map_err(|e| format!("the read could not be run: {e}"))?
                        .map_err(|e| format!("{e}"))
                        // The KDBX read yields a password and nothing else, which
                        // is why the item carries a username of its own.
                        .map(|password| {
                            password.map(|password| rustconn_core::models::Credentials {
                                username,
                                password: Some(password),
                                key_passphrase: None,
                                domain: None,
                            })
                        })
                    })
                }
            };
            match outcome {
                Ok(Some(creds)) => return Ok(Some(creds)),
                Ok(None) => {}
                Err(e) => last_error = Some(e),
            }
        }
        // Every key missed. A store that could not be opened at all failed on the
        // first key too, so a recorded error outweighs "not found": reporting a
        // miss would tell the user their passwords are not there when the truth is
        // that nothing could look.
        last_error.map_or(Ok(None), Err)
    }

    /// Writes one credential under `item.destination_key`.
    ///
    /// Returns which credential fields this side could not hold, so a partial
    /// write is not reported as a clean copy.
    ///
    /// # Errors
    /// Returns the backend's failure.
    fn store(
        &self,
        item: &CredentialTransferItem,
        creds: &rustconn_core::models::Credentials,
        rt: &tokio::runtime::Runtime,
    ) -> Result<Vec<&'static str>, String> {
        use secrecy::ExposeSecret;

        match self {
            Self::Backend(backend) => rt
                .block_on(async {
                    tokio::time::timeout(
                        TRANSFER_OP_TIMEOUT,
                        backend.store(&item.destination_key, creds),
                    )
                    .await
                    .map_err(|_| timed_out("write"))?
                    .map_err(|e| format!("{e}"))
                })
                .map(|()| Vec::new()),
            Self::Kdbx {
                path,
                db_password,
                key_file,
                yubikey_slot,
                read_only,
            } => {
                let read_only = *read_only;
                let Some(password) = creds.password.as_ref() else {
                    return Err("the entry has no password to write".to_string());
                };
                // Owned for the `spawn_blocking` closure, shared rather than
                // copied — the same `Arc` shape `db_password` uses just below.
                let entry_password = std::sync::Arc::new(secrecy::SecretString::from(
                    password.expose_secret().to_string(),
                ));
                let path = path.clone();
                let db_password = db_password.as_ref().map(std::sync::Arc::clone);
                let key_file = key_file.clone();
                let yubikey_slot = yubikey_slot.clone();
                let entry = item.destination_key.clone();
                let username = creds.username.clone().unwrap_or_default();
                rt.block_on(async move {
                    tokio::time::timeout(
                        TRANSFER_OP_TIMEOUT,
                        tokio::task::spawn_blocking(move || {
                            rustconn_core::secret::KeePassStatus::save_password_to_kdbx(
                                &path,
                                db_password.as_deref(),
                                key_file.as_deref(),
                                &entry,
                                &username,
                                &entry_password,
                                None,
                                yubikey_slot.as_deref(),
                                read_only,
                            )
                        }),
                    )
                    .await
                    .map_err(|_| timed_out("write"))?
                    .map_err(|e| format!("the write could not be run: {e}"))?
                    .map_err(|e| format!("{e}"))
                })?;

                // `save_password_to_kdbx` writes a title, username, password and
                // URL. A key passphrase or a domain has nowhere to go, and the
                // caller has to know that rather than being told the entry copied
                // cleanly.
                let mut dropped = Vec::new();
                if creds.key_passphrase.is_some() {
                    dropped.push("key passphrase");
                }
                if creds.domain.is_some() {
                    dropped.push("domain");
                }
                Ok(dropped)
            }
        }
    }
}

/// One credential to move, resolved to the key each side files it under.
///
/// The two sides need separate keys because the shape is per backend, not per
/// credential: the system keyring uses `RustConn/{group}/{name} ({protocol})`,
/// KDBX uses an entry path, and the remaining backends use `rustconn/{name}`.
/// Copying a key verbatim between two backends of different shape would write a
/// credential that the resolver never looks for again.
#[derive(Debug, Clone)]
pub struct CredentialTransferItem {
    /// What to call this entry when reporting — a connection, group or variable
    /// name. Never a secret.
    pub label: String,
    /// Keys to try in the source, most-current format first, so credentials
    /// written by earlier releases are picked up rather than reported missing.
    pub source_keys: Vec<String>,
    /// Key to write in the destination.
    pub destination_key: String,
    /// Username to pair with the password when the source cannot supply one.
    ///
    /// The KDBX read path returns a password and nothing else, so without this a
    /// KeePassXC-to-anywhere transfer would drop every username.
    pub username: Option<String>,
}

/// Outcome of a credential transfer.
#[derive(Debug, Clone, Default)]
pub struct CredentialTransferReport {
    /// Credentials read from the source and written to the destination.
    pub transferred: usize,
    /// Entries the source held nothing for.
    ///
    /// Not a failure, and reported separately for that reason: a connection set
    /// to use the vault does not have to have a password saved yet, and counting
    /// those as errors would make a healthy transfer look broken.
    pub missing: usize,
    /// Entries whose password arrived but which lost a field the destination
    /// cannot hold, as `(label, field list)`.
    ///
    /// Separate from both counters above because it is neither: the entry works,
    /// but not identically. Writing a KeePass database drops a key passphrase and
    /// a domain, which libsecret and both file backends do store.
    pub incomplete: Vec<(String, String)>,
    /// Entries that could not be transferred, as `(label, error description)`.
    pub failures: Vec<(String, String)>,
    /// Whether the run stopped early because the user asked it to.
    ///
    /// Reported so the counts can be read correctly: "Copied: 7" out of forty
    /// planned entries is a complete answer to a cancelled run and an alarming
    /// one to a finished run, and nothing else in the report distinguishes them.
    pub cancelled: bool,
}

impl CredentialTransferReport {
    /// Whether every entry that had a credential arrived complete.
    ///
    /// A cancelled run is never complete, even when everything it did reach
    /// arrived cleanly: the entries it never looked at are the point.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        !self.cancelled && self.failures.is_empty() && self.incomplete.is_empty()
    }
}

/// The dialog's handles on a running transfer.
///
/// Both halves exist because a transfer over a CLI-backed vault is a process spawn
/// per entry: forty entries is minutes of a dialog that previously said "Copying…"
/// and nothing else, with no way to stop it.
pub struct TransferControl {
    /// Sent the number of entries finished, after each one.
    ///
    /// Unbounded, so the worker never blocks on a main loop that is busy, and
    /// `try_send` is used rather than `send` — a dropped progress tick costs a
    /// stale count for a moment, where a blocked worker costs the transfer.
    pub progress: async_channel::Sender<usize>,
    /// Set by the dialog to stop the run.
    ///
    /// Checked before each entry, never mid-entry: a credential is written by a
    /// single backend call, and abandoning one in flight would leave the
    /// destination in a state the report could not describe. So Cancel means "stop
    /// after the one you are on", and the report says how far it got.
    pub cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// What a transfer would move, and what stands in its way.
#[derive(Debug, Clone, Default)]
pub struct CredentialTransferPlan {
    /// The credentials to copy, one entry each.
    pub items: Vec<CredentialTransferItem>,
    /// Entries that would overwrite one another in the destination, grouped by
    /// the key they share. See [`find_destination_collisions`] for why this can
    /// happen and why the answer is to refuse rather than to pick a winner.
    pub collisions: Vec<Vec<String>>,
}

/// Builds the list of credentials a transfer from `source` to `destination`
/// would move.
///
/// The `SecretBackend` trait cannot enumerate: six of the eight backends offer no
/// way to list what they hold, so the set of credentials is taken from what
/// RustConn knows it stored — every connection and group set to use the vault,
/// plus every secret variable. A vault may well contain entries RustConn never
/// created; those are not touched, and cannot be, because nothing can name them.
///
/// Variables that point at a pre-existing external entry (`kdbx_entry_path` or
/// `vault_entry_name`) are skipped on purpose. Those are references to entries the
/// user maintains elsewhere, not credentials RustConn owns, and copying one into
/// another backend would duplicate someone else's secret under a name the
/// resolver would not look for anyway.
#[must_use]
pub fn plan_credential_transfer(
    settings: &rustconn_core::config::AppSettings,
    connections: &[rustconn_core::models::Connection],
    groups: &[rustconn_core::models::ConnectionGroup],
    source: rustconn_core::config::SecretBackendType,
    destination: rustconn_core::config::SecretBackendType,
) -> CredentialTransferPlan {
    use rustconn_core::models::PasswordSource;

    let mut items = Vec::new();

    for conn in connections {
        if conn.password_source != PasswordSource::Vault {
            continue;
        }
        let protocol = conn.protocol_config.protocol_type().as_str().to_lowercase();
        let source_keys = transfer_keys_for_connection(groups, conn, &protocol, source);
        let Some(destination_key) =
            transfer_keys_for_connection(groups, conn, &protocol, destination)
                .into_iter()
                .next()
        else {
            continue;
        };
        items.push(CredentialTransferItem {
            label: conn.name.clone(),
            source_keys,
            destination_key,
            username: conn.username.clone(),
        });
    }

    for group in groups {
        if group.password_source != Some(PasswordSource::Vault) {
            continue;
        }
        items.push(CredentialTransferItem {
            label: group.name.clone(),
            source_keys: vec![transfer_key_for_group(groups, group, source)],
            destination_key: transfer_key_for_group(groups, group, destination),
            username: group.username.clone(),
        });
    }

    for variable in &settings.global_variables {
        if !variable.is_secret
            || variable.kdbx_entry_path.is_some()
            || variable.vault_entry_name.is_some()
        {
            continue;
        }
        let key = rustconn_core::variables::variable_secret_key(&variable.name);
        items.push(CredentialTransferItem {
            label: variable.name.clone(),
            source_keys: vec![key.clone()],
            destination_key: key,
            username: None,
        });
    }

    let collisions = find_destination_collisions(&items);
    CredentialTransferPlan { items, collisions }
}

/// Groups of entries that would land on the same key in the destination.
///
/// The flat key shape the six non-keyring backends use is `rustconn/{name}` with
/// no group in it, while the keyring and KDBX shapes both carry the group path.
/// Two connections called `web` in `Prod` and `Staging` therefore have distinct
/// keys in a keyring or a database and *one* key in the portable file — so a
/// keyring-to-portable transfer would read two different passwords, write both to
/// the same entry, and count two successes. The user then has a connection that
/// authenticates with another connection's password, silently.
///
/// This is a property of the key shape, not of the transfer, and predates it (the
/// group was added to the keyring key for issue #264 and to nothing else). But
/// the transfer is the first thing that walks every entry at once, so it is the
/// first thing that can see the collision — and refusing is the only honest
/// response, since there is no key for the second entry to go to.
fn find_destination_collisions(items: &[CredentialTransferItem]) -> Vec<Vec<String>> {
    let mut by_key: std::collections::HashMap<&str, Vec<&str>> = std::collections::HashMap::new();
    for item in items {
        by_key
            .entry(item.destination_key.as_str())
            .or_default()
            .push(item.label.as_str());
    }

    let mut collisions: Vec<Vec<String>> = by_key
        .into_values()
        .filter(|labels| labels.len() > 1)
        .map(|labels| {
            let mut owned: Vec<String> = labels.into_iter().map(str::to_owned).collect();
            owned.sort_unstable();
            owned
        })
        .collect();
    // Deterministic order so the dialog and the log do not reshuffle per run.
    collisions.sort_unstable();
    collisions
}

/// Keys a connection's credential can live under in `backend_type`, current
/// format first.
fn transfer_keys_for_connection(
    groups: &[rustconn_core::models::ConnectionGroup],
    connection: &rustconn_core::models::Connection,
    protocol_str: &str,
    backend_type: rustconn_core::config::SecretBackendType,
) -> Vec<String> {
    use rustconn_core::config::SecretBackendType;

    if matches!(
        backend_type,
        SecretBackendType::KeePassXc | SecretBackendType::KdbxFile
    ) {
        // The same string `save_password_to_vault` writes: the hierarchical entry
        // path without the `RustConn/` prefix, which both KDBX helpers add back,
        // plus the protocol suffix.
        let entry_path =
            rustconn_core::secret::KeePassHierarchy::build_entry_path(connection, groups);
        let base = entry_path.strip_prefix("RustConn/").unwrap_or(&entry_path);
        return vec![format!("{base} ({protocol_str})")];
    }

    vault_keys_for_connection(groups, connection, protocol_str, backend_type)
}

/// Key a group's credential lives under in `backend_type`.
fn transfer_key_for_group(
    groups: &[rustconn_core::models::ConnectionGroup],
    group: &rustconn_core::models::ConnectionGroup,
    backend_type: rustconn_core::config::SecretBackendType,
) -> String {
    use rustconn_core::config::SecretBackendType;

    if matches!(
        backend_type,
        SecretBackendType::KeePassXc | SecretBackendType::KdbxFile
    ) {
        // The entry name without the `RustConn/` root — the KDBX helpers add it
        // back (see issue #327). Shared with the group load path.
        return rustconn_core::secret::KeePassHierarchy::build_group_entry_name(group, groups);
    }

    // Every other backend keys group credentials by the group's UUID, which is
    // why a group rename does not have to migrate them.
    group.id.to_string()
}

/// Copies each planned credential from `source` into `destination`.
///
/// Blocking: every entry costs at least one backend round trip, and a KDBX or
/// Bitwarden side costs a process spawn per entry. Call this from
/// `spawn_blocking`, never on the GTK main thread.
///
/// The source is never modified. Deleting from it is not offered at all: for a
/// shared vault such as Bitwarden or a Passbolt server the entries may not be
/// RustConn's to remove, and for the machine-bound file the originals are this
/// machine's fallback — the same reason the portable-file wizard keeps them.
pub fn run_credential_transfer(
    settings: &rustconn_core::config::AppSettings,
    source: rustconn_core::config::SecretBackendType,
    destination: rustconn_core::config::SecretBackendType,
    items: &[CredentialTransferItem],
    control: &TransferControl,
) -> Result<CredentialTransferReport, String> {
    crate::async_utils::with_runtime(|rt| {
        // Both sides once, before the loop: see `TransferPort`.
        let source_port = TransferPort::open(settings, source, rt)
            .map_err(|e| format!("cannot read from the source: {e}"))?;
        let destination_port = TransferPort::open(settings, destination, rt)
            .map_err(|e| format!("cannot write to the destination: {e}"))?;

        let mut report = CredentialTransferReport::default();
        for (finished, item) in items.iter().enumerate() {
            // Between entries, not within one: see `TransferControl::cancel`.
            if control.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                report.cancelled = true;
                tracing::info!(
                    finished,
                    planned = items.len(),
                    "credential transfer cancelled"
                );
                break;
            }
            match source_port.retrieve(item, rt) {
                Ok(Some(creds)) => match destination_port.store(item, &creds, rt) {
                    Ok(dropped) if dropped.is_empty() => report.transferred += 1,
                    Ok(dropped) => {
                        report.transferred += 1;
                        report
                            .incomplete
                            .push((item.label.clone(), dropped.join(", ")));
                    }
                    Err(e) => {
                        // The reason goes to the log here and nowhere else. It is
                        // backend output, and a backend that takes a password on
                        // its command line can quote it back; the dialog shows the
                        // names, which is what the user acts on. Logging it at all
                        // is a judgement that a diagnosable failure is worth more
                        // than the residual risk, now that the one backend known
                        // to echo its argv scrubs it first.
                        tracing::warn!(
                            entry = %item.label,
                            error = %e,
                            "credential transfer could not write an entry"
                        );
                        report.failures.push((item.label.clone(), e));
                    }
                },
                Ok(None) => report.missing += 1,
                Err(e) => {
                    tracing::warn!(
                        entry = %item.label,
                        error = %e,
                        "credential transfer could not read an entry"
                    );
                    report.failures.push((item.label.clone(), e));
                }
            }
            // After the entry, so the count is entries *done* rather than started.
            // A full queue is ignored: the dialog showing a count one behind for a
            // moment is better than a worker waiting on the main loop.
            let _ = control.progress.try_send(finished + 1);
        }

        tracing::info!(
            ?source,
            ?destination,
            transferred = report.transferred,
            missing = report.missing,
            incomplete = report.incomplete.len(),
            failed = report.failures.len(),
            cancelled = report.cancelled,
            "credential transfer finished"
        );
        Ok(report)
    })
    .and_then(|r| r)
}

/// Selects the appropriate storage backend for variable secrets.
///
/// Mirrors `CredentialResolver::select_storage_backend` logic.
/// Also used by connection password load/save and variable vault operations.
/// Builds a portable-file backend from settings, unlocked when possible.
///
/// The passphrase comes from `SecretSettings::portable_passphrase`, the
/// session-only field that the unlock dialog, the startup restore and the
/// Settings page all write to. When it is absent the backend stays locked and
/// its operations report [`rustconn_core::error::SecretError::PassphraseRequired`],
/// which the connection path turns into an unlock prompt.
///
/// Exists so the path resolution and the unlock live in one place: four call
/// sites each had their own copy, and every one of them had forgotten to set the
/// passphrase at some point in this feature's history.
#[must_use]
pub fn portable_backend_from_settings(
    settings: &rustconn_core::config::SecretSettings,
) -> rustconn_core::secret::PortableEncryptedFileBackend {
    let path =
        rustconn_core::secret::resolve_portable_store_path(settings.portable_file_path.as_deref());
    let backend = rustconn_core::secret::PortableEncryptedFileBackend::with_path(path);
    if let Some(ref passphrase) = settings.portable_passphrase {
        backend.set_passphrase(passphrase.clone());
    }
    backend
}

pub fn select_backend_for_load(
    secrets: &rustconn_core::config::SecretSettings,
) -> rustconn_core::config::SecretBackendType {
    use rustconn_core::config::SecretBackendType;

    match secrets.preferred_backend {
        SecretBackendType::Bitwarden => SecretBackendType::Bitwarden,
        SecretBackendType::OnePassword => SecretBackendType::OnePassword,
        SecretBackendType::Passbolt => SecretBackendType::Passbolt,
        SecretBackendType::Pass => SecretBackendType::Pass,
        SecretBackendType::MacOsKeychain => SecretBackendType::MacOsKeychain,
        SecretBackendType::KeePassXc | SecretBackendType::KdbxFile => {
            if secrets.kdbx_enabled && secrets.kdbx_path.is_some() {
                SecretBackendType::KdbxFile
            } else if secrets.enable_fallback {
                SecretBackendType::LibSecret
            } else {
                secrets.preferred_backend
            }
        }
        SecretBackendType::LibSecret => SecretBackendType::LibSecret,
        // EncryptedFile is a flat-key backend; identity mapping mirrors the
        // other app-managed backends above. (Allowed flat-key wiring for 2.4.)
        SecretBackendType::EncryptedFile => SecretBackendType::EncryptedFile,
        // PortableEncryptedFile is also a flat-key backend (passphrase-based).
        SecretBackendType::PortableEncryptedFile => SecretBackendType::PortableEncryptedFile,
    }
}

/// Generates the correct store key for a connection based on the backend type.
///
/// LibSecret uses hierarchical `"RustConn/{group_path}/{name} ({protocol})"` format
/// when `group_path` is provided (matching
/// [`CredentialResolver::generate_keyring_key_with_hierarchy`]), or falls back to
/// `"{name} ({protocol})"` for backward compatibility when no group path is given.
/// All other backends use `"rustconn/{name}"` (matching
/// [`CredentialResolver::generate_lookup_key`]).
///
/// When `conn_name` is empty, falls back to `conn_host` for non-LibSecret
/// backends, matching the resolver's `generate_lookup_key` behavior.
///
/// This ensures that the key used at store time matches the primary key the
/// resolver tries at resolve time, eliminating the need for fallback lookups.
pub fn generate_store_key(
    conn_name: &str,
    conn_host: &str,
    protocol_str: &str,
    backend_type: rustconn_core::config::SecretBackendType,
) -> String {
    generate_store_key_with_group(conn_name, conn_host, protocol_str, backend_type, None)
}

/// Generates a store key that includes the group path for keyring backends.
///
/// Prefer [`generate_store_key_for_connection`] whenever a connection's group is
/// what decides the key. This function's `group_path` has three meanings, not
/// two, and getting the third wrong is issue
/// [#316](https://github.com/totoshko88/RustConn/issues/316): `Some("Prod")` is a
/// grouped connection, `Some("")` is an ungrouped *connection*, and `None` is the
/// pre-0.19.18 flat key that belongs to no connection at all. `Some("")` and
/// `None` differ by the `RustConn/` prefix, which is exactly what an ungrouped
/// connection's entry is written with.
///
/// `group_path` is the `/`-separated group hierarchy (e.g. `"Production/Web"`).
/// When provided and the backend is LibSecret or the macOS Keychain, the key is
/// `"RustConn/{group_path}/{name} ({protocol})"`.
///
/// The macOS Keychain is included because `CredentialResolver` resolves it
/// through the same hierarchical keyring path as LibSecret
/// (`resolve_from_keyring_hierarchical`). Storing it under the flat
/// `rustconn/{name}` key, as this function did until 0.19.19, meant the resolver
/// never looked where the credential had been written.
pub fn generate_store_key_with_group(
    conn_name: &str,
    conn_host: &str,
    protocol_str: &str,
    backend_type: rustconn_core::config::SecretBackendType,
    group_path: Option<&str>,
) -> String {
    use rustconn_core::config::SecretBackendType;

    if matches!(
        backend_type,
        SecretBackendType::LibSecret | SecretBackendType::MacOsKeychain
    ) {
        let name = conn_name.trim().replace('/', "-");
        let suffix = format!("{name} ({protocol_str})");
        match group_path {
            Some(path) if !path.is_empty() => format!("RustConn/{path}/{suffix}"),
            Some(_) => format!("RustConn/{suffix}"),
            None => suffix,
        }
    } else {
        // All other backends: "rustconn/{identifier}" — matches generate_lookup_key
        // Falls back to host when name is empty, same as CredentialResolver
        let identifier = if conn_name.trim().is_empty() {
            conn_host
        } else {
            conn_name
        };
        format!("rustconn/{identifier}")
    }
}

/// Generates the vault key for a connection, from the group it is in.
///
/// The form to use wherever a connection decides the key, and the reason it
/// exists is issue [#316](https://github.com/totoshko88/RustConn/issues/316).
/// Four call sites built the `Option<&str>` group path for
/// [`generate_store_key_with_group`] themselves, and they did not agree about the
/// ungrouped case: saving and deleting passed `Some("")`, while resolving and the
/// connection dialog's 📂 load and ✓ test buttons wrote
/// `connection.group_id.map(…)`, which is `None` when there is no group. With
/// LibSecret that meant a password on an ungrouped connection was written to
/// `RustConn/{name} ({protocol})` and looked for at `{name} ({protocol})`. The
/// Secret Service matches attributes exactly, so the two never met: connecting
/// prompted for a password that was sitting in the keyring, and both buttons
/// reported it missing. Putting the connection into a group fixed it, because
/// then the two keys differed *and* the resolver's second, legacy key happened to
/// cover the miss.
///
/// `group_id` of `None` means the connection is ungrouped — it does not mean the
/// key should drop the `RustConn/` prefix. That decision is made here now, once,
/// instead of at each caller.
pub fn generate_store_key_for_connection(
    conn_name: &str,
    conn_host: &str,
    protocol_str: &str,
    backend_type: rustconn_core::config::SecretBackendType,
    group_id: Option<uuid::Uuid>,
    groups: &[rustconn_core::models::ConnectionGroup],
) -> String {
    let group_path = group_id
        .map(|gid| {
            rustconn_core::secret::KeePassHierarchy::resolve_group_path(gid, groups).join("/")
        })
        .unwrap_or_default();
    generate_store_key_with_group(
        conn_name,
        conn_host,
        protocol_str,
        backend_type,
        Some(&group_path),
    )
}

#[cfg(test)]
mod tests {
    use rustconn_core::config::{SecretBackendType, SecretSettings};
    use rustconn_core::models::{Connection, ConnectionGroup};

    use super::*;

    fn default_secret_settings(backend: SecretBackendType) -> SecretSettings {
        SecretSettings {
            preferred_backend: backend,
            kdbx_enabled: false,
            kdbx_path: None,
            kdbx_key_file: None,
            kdbx_password: None,
            enable_fallback: false,
            ..Default::default()
        }
    }

    // ── vault_op_timeout ─────────────────────────────────────────────

    /// The two tiers exist because a CLI-backed backend is child processes and a
    /// network round trip while the rest answer from this machine. Asserted as a
    /// relation rather than as two literals, so retuning either number does not
    /// break the test — only collapsing them back into one would.
    #[test]
    fn a_cli_backed_backend_gets_a_longer_budget_than_a_local_one() {
        let cli = [
            SecretBackendType::Bitwarden,
            SecretBackendType::OnePassword,
            SecretBackendType::Passbolt,
            SecretBackendType::Pass,
        ];
        let local = [
            SecretBackendType::LibSecret,
            SecretBackendType::MacOsKeychain,
            SecretBackendType::KeePassXc,
            SecretBackendType::KdbxFile,
            SecretBackendType::EncryptedFile,
            SecretBackendType::PortableEncryptedFile,
        ];

        for remote in cli {
            for near in local {
                assert!(
                    vault_op_timeout(remote) > vault_op_timeout(near),
                    "{remote:?} must outlast {near:?}"
                );
            }
        }
    }

    /// The measurement the budget was chosen from: one Bitwarden store was three
    /// `bw` invocations totalling over ten seconds on the reporter's machine in
    /// issue #312, and the old ten-second budget expired mid-write while `bw`
    /// carried on and completed it. Anything at or below that measurement
    /// reintroduces the bug where a successful store is reported as refused.
    #[test]
    fn the_bitwarden_budget_clears_the_measured_worst_case() {
        let budget = vault_op_timeout(SecretBackendType::Bitwarden);
        assert!(
            budget > std::time::Duration::from_secs(10),
            "10s is the budget that expired mid-write; got {budget:?}"
        );
    }

    /// The message has to state the budget that was applied. It used to say "10s"
    /// as a literal, which would start lying the moment the budget was no longer
    /// ten seconds — which is exactly what this release did.
    #[test]
    fn the_timeout_message_names_the_budget_that_was_applied() {
        let budget = vault_op_timeout(SecretBackendType::Bitwarden);
        let message = vault_op_timed_out("store", budget);

        assert!(message.contains("store"), "operation missing: {message}");
        assert!(
            message.contains(&budget.as_secs().to_string()),
            "budget missing: {message}"
        );
        assert!(
            !message.contains("10s"),
            "still reporting the old hardcoded budget: {message}"
        );
    }

    // ── select_backend_for_load ──────────────────────────────────────

    #[test]
    fn select_backend_bitwarden() {
        let s = default_secret_settings(SecretBackendType::Bitwarden);
        assert_eq!(select_backend_for_load(&s), SecretBackendType::Bitwarden);
    }

    #[test]
    fn select_backend_onepassword() {
        let s = default_secret_settings(SecretBackendType::OnePassword);
        assert_eq!(select_backend_for_load(&s), SecretBackendType::OnePassword);
    }

    #[test]
    fn select_backend_passbolt() {
        let s = default_secret_settings(SecretBackendType::Passbolt);
        assert_eq!(select_backend_for_load(&s), SecretBackendType::Passbolt);
    }

    #[test]
    fn select_backend_pass() {
        let s = default_secret_settings(SecretBackendType::Pass);
        assert_eq!(select_backend_for_load(&s), SecretBackendType::Pass);
    }

    #[test]
    fn select_backend_libsecret() {
        let s = default_secret_settings(SecretBackendType::LibSecret);
        assert_eq!(select_backend_for_load(&s), SecretBackendType::LibSecret);
    }

    /// The direct-dispatch path (migrations, deletes, copies, transfers) must
    /// build CLI backends with the persisted read-only toggle, or those writes
    /// ignore read-only mode while the main save path honours it.
    #[test]
    fn single_backend_carries_the_read_only_toggle() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        for (backend_type, settings) in [
            (
                SecretBackendType::OnePassword,
                SecretSettings {
                    onepassword_read_only: true,
                    ..default_secret_settings(SecretBackendType::OnePassword)
                },
            ),
            (
                SecretBackendType::Passbolt,
                SecretSettings {
                    passbolt_read_only: true,
                    ..default_secret_settings(SecretBackendType::Passbolt)
                },
            ),
            (
                SecretBackendType::Pass,
                SecretSettings {
                    pass_read_only: true,
                    ..default_secret_settings(SecretBackendType::Pass)
                },
            ),
        ] {
            let backend = build_single_backend(&settings, backend_type, &rt).unwrap();
            assert!(
                backend.is_read_only(),
                "{backend_type:?} lost its read-only toggle"
            );
        }
    }

    #[test]
    fn select_backend_keepass_with_kdbx_enabled() {
        let s = SecretSettings {
            preferred_backend: SecretBackendType::KeePassXc,
            kdbx_enabled: true,
            kdbx_path: Some(std::path::PathBuf::from("/tmp/test.kdbx")),
            ..Default::default()
        };
        assert_eq!(select_backend_for_load(&s), SecretBackendType::KdbxFile);
    }

    #[test]
    fn select_backend_keepass_without_kdbx_falls_back() {
        let s = SecretSettings {
            preferred_backend: SecretBackendType::KeePassXc,
            kdbx_enabled: false,
            kdbx_path: None,
            enable_fallback: true,
            ..Default::default()
        };
        assert_eq!(select_backend_for_load(&s), SecretBackendType::LibSecret);
    }

    #[test]
    fn select_backend_keepass_no_fallback() {
        let s = SecretSettings {
            preferred_backend: SecretBackendType::KeePassXc,
            kdbx_enabled: false,
            kdbx_path: None,
            enable_fallback: false,
            ..Default::default()
        };
        assert_eq!(select_backend_for_load(&s), SecretBackendType::KeePassXc);
    }

    // ── generate_store_key ───────────────────────────────────────────

    #[test]
    fn store_key_libsecret_format() {
        let key = generate_store_key("My Server", "10.0.0.1", "ssh", SecretBackendType::LibSecret);
        assert_eq!(key, "My Server (ssh)");
    }

    #[test]
    fn store_key_libsecret_strips_slashes() {
        let key = generate_store_key(
            "Prod/Web-01",
            "10.0.0.1",
            "ssh",
            SecretBackendType::LibSecret,
        );
        assert_eq!(key, "Prod-Web-01 (ssh)");
    }

    #[test]
    fn store_key_bitwarden_format() {
        let key = generate_store_key("My Server", "10.0.0.1", "ssh", SecretBackendType::Bitwarden);
        assert_eq!(key, "rustconn/My Server");
    }

    #[test]
    fn store_key_empty_name_falls_back_to_host() {
        let key = generate_store_key("", "10.0.0.1", "rdp", SecretBackendType::Bitwarden);
        assert_eq!(key, "rustconn/10.0.0.1");
    }

    #[test]
    fn store_key_whitespace_name_falls_back_to_host() {
        let key = generate_store_key("   ", "10.0.0.1", "rdp", SecretBackendType::OnePassword);
        assert_eq!(key, "rustconn/10.0.0.1");
    }

    #[test]
    fn store_key_pass_format() {
        let key = generate_store_key("DB Server", "db.local", "ssh", SecretBackendType::Pass);
        assert_eq!(key, "rustconn/DB Server");
    }

    /// Issue #316, as the invariant that was missing.
    ///
    /// Nothing compared the key the GUI writes against the key the resolver
    /// reads, so the two drifted apart for exactly one case — a connection in no
    /// group — and every existing test used a grouped one. This asserts they are
    /// the same string, which is the only thing the Secret Service cares about:
    /// it matches attributes exactly, so a one-character difference is a miss.
    #[test]
    fn an_ungrouped_connection_is_stored_where_the_resolver_looks() {
        let conn = ssh_connection("admin", None);
        let ours = generate_store_key_for_connection(
            &conn.name,
            &conn.host,
            "ssh",
            SecretBackendType::LibSecret,
            conn.group_id,
            &[],
        );
        assert_eq!(ours, "RustConn/admin (ssh)");
        assert_eq!(
            ours,
            rustconn_core::secret::CredentialResolver::generate_keyring_key_with_hierarchy(
                &conn,
                &[]
            ),
            "the store key and the resolver's lookup key must be one string"
        );
    }

    #[test]
    fn a_grouped_connection_still_is() {
        // The case that worked, so the fix cannot have been a swap.
        let group = ConnectionGroup::new("oracle".to_string());
        let conn = ssh_connection("admin", Some(group.id));
        let groups = std::slice::from_ref(&group);
        let ours = generate_store_key_for_connection(
            &conn.name,
            &conn.host,
            "ssh",
            SecretBackendType::LibSecret,
            conn.group_id,
            groups,
        );
        assert_eq!(ours, "RustConn/oracle/admin (ssh)");
        assert_eq!(
            ours,
            rustconn_core::secret::CredentialResolver::generate_keyring_key_with_hierarchy(
                &conn, groups
            )
        );
    }

    /// The distinction the bug turned on, pinned so it cannot be "simplified".
    ///
    /// `Some("")` is an ungrouped connection and keeps the prefix; `None` is the
    /// pre-0.19.18 flat key and belongs to no connection. They are not
    /// interchangeable, and reading them as if they were is issue #316.
    #[test]
    fn an_empty_group_path_is_not_the_same_as_no_group_path() {
        let with_prefix = generate_store_key_with_group(
            "admin",
            "10.0.0.1",
            "ssh",
            SecretBackendType::LibSecret,
            Some(""),
        );
        let legacy = generate_store_key_with_group(
            "admin",
            "10.0.0.1",
            "ssh",
            SecretBackendType::LibSecret,
            None,
        );
        assert_eq!(with_prefix, "RustConn/admin (ssh)");
        assert_eq!(legacy, "admin (ssh)");
        assert_ne!(with_prefix, legacy);
    }

    /// Quick-connect keeps the flat key: it has no connection, so it has no group
    /// and no entry under a prefixed key either.
    #[test]
    fn a_connectionless_save_still_uses_the_flat_key() {
        let key = generate_store_key("admin", "10.0.0.1", "ssh", SecretBackendType::LibSecret);
        assert_eq!(key, "admin (ssh)");
    }

    #[test]
    fn store_key_macos_keychain_is_hierarchical_like_libsecret() {
        // The resolver reads the Keychain through the hierarchical keyring path,
        // so the store key must match it rather than the flat "rustconn/{name}".
        let key = generate_store_key_with_group(
            "admin",
            "10.0.0.1",
            "ssh",
            SecretBackendType::MacOsKeychain,
            Some("oracle"),
        );
        assert_eq!(key, "RustConn/oracle/admin (ssh)");
    }

    // ── vault_keys_for_connection ────────────────────────────────────

    fn ssh_connection(name: &str, group_id: Option<uuid::Uuid>) -> Connection {
        let mut conn = Connection::new(
            name.to_string(),
            "10.0.0.1".to_string(),
            22,
            rustconn_core::models::ProtocolConfig::Ssh(rustconn_core::models::SshConfig::default()),
        );
        conn.group_id = group_id;
        conn
    }

    #[test]
    fn keyring_keys_lead_with_the_group_scoped_key() {
        // Deleting by the flat key alone left the keyring item behind (#263),
        // because the item is stored under the group-scoped key (#264).
        let group = ConnectionGroup::new("oracle".to_string());
        let conn = ssh_connection("admin", Some(group.id));
        let keys = vault_keys_for_connection(
            std::slice::from_ref(&group),
            &conn,
            "ssh",
            SecretBackendType::LibSecret,
        );

        assert_eq!(
            keys.first().map(String::as_str),
            Some("RustConn/oracle/admin (ssh)")
        );
        assert!(
            keys.iter().any(|k| k == "admin (ssh)"),
            "legacy flat key must stay in the cleanup set: {keys:?}"
        );
        assert!(
            keys.iter().any(|k| *k == conn.id.to_string()),
            "UUID key must stay in the cleanup set: {keys:?}"
        );
    }

    #[test]
    fn keyring_keys_differ_between_groups_for_the_same_name() {
        // The collision from issue #264: two "admin" connections in different
        // groups must never share a primary key.
        let oracle = ConnectionGroup::new("oracle".to_string());
        let pve = ConnectionGroup::new("pve".to_string());
        let groups = vec![oracle.clone(), pve.clone()];

        let in_oracle = ssh_connection("admin", Some(oracle.id));
        let in_pve = ssh_connection("admin", Some(pve.id));

        let oracle_key =
            vault_keys_for_connection(&groups, &in_oracle, "ssh", SecretBackendType::LibSecret);
        let pve_key =
            vault_keys_for_connection(&groups, &in_pve, "ssh", SecretBackendType::LibSecret);

        assert_eq!(
            oracle_key.first().map(String::as_str),
            Some("RustConn/oracle/admin (ssh)")
        );
        assert_eq!(
            pve_key.first().map(String::as_str),
            Some("RustConn/pve/admin (ssh)")
        );
        assert_ne!(oracle_key.first(), pve_key.first());
    }

    #[test]
    fn flat_backend_keys_use_the_rustconn_prefix() {
        let conn = ssh_connection("admin", None);
        let keys = vault_keys_for_connection(&[], &conn, "ssh", SecretBackendType::Bitwarden);
        assert_eq!(keys.first().map(String::as_str), Some("rustconn/admin"));
    }

    // ── plan_vault_key_migration ─────────────────────────────────────

    fn app_settings(backend: SecretBackendType) -> rustconn_core::config::AppSettings {
        rustconn_core::config::AppSettings {
            secrets: default_secret_settings(backend),
            ..Default::default()
        }
    }

    fn keepass_app_settings() -> rustconn_core::config::AppSettings {
        rustconn_core::config::AppSettings {
            secrets: SecretSettings {
                preferred_backend: SecretBackendType::KeePassXc,
                kdbx_enabled: true,
                kdbx_path: Some("/tmp/does-not-need-to-exist.kdbx".into()),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn plan_migration_is_none_when_the_edit_changed_nothing_in_the_key() {
        let settings = app_settings(SecretBackendType::LibSecret);
        let conn = ssh_connection("admin", None);
        // A description or tag edit leaves the key alone.
        let mut edited = conn.clone();
        edited.description = Some("touched".to_string());

        assert!(plan_vault_key_migration(&settings, &[], &[], &conn, &edited).is_none());
    }

    #[test]
    fn plan_migration_detects_a_rename() {
        let settings = app_settings(SecretBackendType::LibSecret);
        let group = ConnectionGroup::new("oracle".to_string());
        let old = ssh_connection("admin", Some(group.id));
        let mut new = old.clone();
        new.name = "dba".to_string();

        let groups = std::slice::from_ref(&group);
        let plan = plan_vault_key_migration(&settings, groups, groups, &old, &new).unwrap();

        assert_eq!(plan.new_key, "RustConn/oracle/dba (ssh)");
        assert_eq!(
            plan.old_keys.first().map(String::as_str),
            Some("RustConn/oracle/admin (ssh)")
        );
        assert!(!plan.is_keepass);
    }

    #[test]
    fn plan_migration_detects_a_group_change() {
        let settings = app_settings(SecretBackendType::LibSecret);
        let oracle = ConnectionGroup::new("oracle".to_string());
        let pve = ConnectionGroup::new("pve".to_string());
        let groups = vec![oracle.clone(), pve.clone()];

        let old = ssh_connection("admin", Some(oracle.id));
        let mut new = old.clone();
        new.group_id = Some(pve.id);

        let plan = plan_vault_key_migration(&settings, &groups, &groups, &old, &new).unwrap();

        assert_eq!(plan.new_key, "RustConn/pve/admin (ssh)");
        assert_eq!(
            plan.old_keys.first().map(String::as_str),
            Some("RustConn/oracle/admin (ssh)")
        );
    }

    #[test]
    fn plan_migration_detects_a_protocol_change() {
        // The protocol is part of the key suffix, and neither pre-0.19.19 helper
        // could express a protocol change — both used one protocol string for the
        // old and the new key.
        let settings = app_settings(SecretBackendType::LibSecret);
        let old = ssh_connection("admin", None);
        let mut new = old.clone();
        new.protocol_config =
            rustconn_core::models::ProtocolConfig::Rdp(rustconn_core::models::RdpConfig::default());

        let plan = plan_vault_key_migration(&settings, &[], &[], &old, &new).unwrap();

        assert_eq!(plan.new_key, "RustConn/admin (rdp)");
        assert_eq!(
            plan.old_keys.first().map(String::as_str),
            Some("RustConn/admin (ssh)")
        );
    }

    #[test]
    fn plan_migration_handles_name_group_and_protocol_at_once() {
        // The connection edit dialog rebuilds the whole connection, so a single
        // save can change all three.
        let settings = app_settings(SecretBackendType::LibSecret);
        let oracle = ConnectionGroup::new("oracle".to_string());
        let pve = ConnectionGroup::new("pve".to_string());
        let groups = vec![oracle.clone(), pve.clone()];

        let old = ssh_connection("admin", Some(oracle.id));
        let mut new = old.clone();
        new.name = "dba".to_string();
        new.group_id = Some(pve.id);
        new.protocol_config =
            rustconn_core::models::ProtocolConfig::Rdp(rustconn_core::models::RdpConfig::default());

        let plan = plan_vault_key_migration(&settings, &groups, &groups, &old, &new).unwrap();

        assert_eq!(plan.new_key, "RustConn/pve/dba (rdp)");
        assert_eq!(
            plan.old_keys.first().map(String::as_str),
            Some("RustConn/oracle/admin (ssh)")
        );
    }

    #[test]
    fn plan_migration_keepass_uses_hierarchical_entry_paths() {
        let settings = keepass_app_settings();
        let group = ConnectionGroup::new("oracle".to_string());
        let old = ssh_connection("admin", Some(group.id));
        let mut new = old.clone();
        new.name = "dba".to_string();

        let groups = std::slice::from_ref(&group);
        let plan = plan_vault_key_migration(&settings, groups, groups, &old, &new).unwrap();

        assert!(plan.is_keepass);
        assert_eq!(plan.new_key, "RustConn/oracle/dba (ssh)");
        assert_eq!(
            plan.old_keys.first().map(String::as_str),
            Some("RustConn/oracle/admin (ssh)")
        );
    }

    #[test]
    fn plan_migration_flat_backend_ignores_a_group_change() {
        // Bitwarden and friends key on the name alone, so moving between groups
        // cannot move the entry.
        let settings = app_settings(SecretBackendType::Bitwarden);
        let oracle = ConnectionGroup::new("oracle".to_string());
        let pve = ConnectionGroup::new("pve".to_string());
        let groups = vec![oracle.clone(), pve.clone()];

        let old = ssh_connection("admin", Some(oracle.id));
        let mut new = old.clone();
        new.group_id = Some(pve.id);

        assert!(plan_vault_key_migration(&settings, &groups, &groups, &old, &new).is_none());
    }

    #[test]
    fn plan_migration_flat_backend_detects_a_rename() {
        let settings = app_settings(SecretBackendType::Bitwarden);
        let old = ssh_connection("admin", None);
        let mut new = old.clone();
        new.name = "dba".to_string();

        let plan = plan_vault_key_migration(&settings, &[], &[], &old, &new).unwrap();

        assert_eq!(plan.new_key, "rustconn/dba");
        assert_eq!(
            plan.old_keys.first().map(String::as_str),
            Some("rustconn/admin")
        );
    }

    // ── credential transfer ──────────────────────────────────────────
    //
    // The transfer's correctness is entirely a claim about *agreement*: the key it
    // writes has to be the key the save path would have written and the resolver
    // will later look for. Nothing in the type system enforces that, so these
    // tests pin it per backend shape.

    /// A vault-backed connection, since the plan only includes those.
    fn vault_connection(name: &str, group_id: Option<uuid::Uuid>) -> Connection {
        let mut conn = ssh_connection(name, group_id);
        conn.password_source = rustconn_core::models::PasswordSource::Vault;
        conn
    }

    #[test]
    fn transfer_destination_key_matches_what_the_save_path_writes() {
        let group = ConnectionGroup::new("Prod".to_string());
        let conn = vault_connection("web", Some(group.id));
        let groups = std::slice::from_ref(&group);

        for backend in [
            SecretBackendType::LibSecret,
            SecretBackendType::MacOsKeychain,
            SecretBackendType::Bitwarden,
            SecretBackendType::OnePassword,
            SecretBackendType::Passbolt,
            SecretBackendType::Pass,
            SecretBackendType::EncryptedFile,
            SecretBackendType::PortableEncryptedFile,
        ] {
            let transfer = transfer_keys_for_connection(groups, &conn, "ssh", backend);
            let expected = vault_keys_for_connection(groups, &conn, "ssh", backend)[0].clone();
            assert_eq!(
                transfer.first(),
                Some(&expected),
                "{backend:?}: the transfer would write a key nothing reads"
            );
        }
    }

    /// KDBX is the one shape that does not come from `vault_keys_for_connection`,
    /// because the database is addressed by entry path rather than by lookup key.
    #[test]
    fn transfer_kdbx_key_matches_the_kdbx_save_path() {
        let group = ConnectionGroup::new("Prod".to_string());
        let conn = vault_connection("web", Some(group.id));
        let groups = std::slice::from_ref(&group);

        let keys = transfer_keys_for_connection(groups, &conn, "ssh", SecretBackendType::KdbxFile);

        // Exactly what `save_password_to_vault`'s KDBX arm builds: the entry path
        // with the `RustConn/` prefix stripped, plus the protocol suffix.
        assert_eq!(keys, vec!["Prod/web (ssh)".to_string()]);
    }

    #[test]
    fn transfer_group_key_is_the_uuid_off_kdbx_and_a_path_on_it() {
        let group = ConnectionGroup::new("Prod".to_string());
        let groups = std::slice::from_ref(&group);

        assert_eq!(
            transfer_key_for_group(groups, &group, SecretBackendType::LibSecret),
            group.id.to_string(),
            "every non-KDBX backend keys group credentials by UUID"
        );
        assert_eq!(
            transfer_key_for_group(groups, &group, SecretBackendType::KdbxFile),
            "Groups/Prod",
            "KDBX keys them by entry path, prefix stripped"
        );
    }

    /// The defect this guard exists for: the flat key shape carries no group, so
    /// two same-named connections in different groups map onto one destination
    /// entry. Writing both would leave one connection authenticating with the
    /// other's password, and the report would call it two successes.
    #[test]
    fn plan_refuses_when_two_entries_would_share_a_destination_key() {
        let prod = ConnectionGroup::new("Prod".to_string());
        let staging = ConnectionGroup::new("Staging".to_string());
        let groups = vec![prod.clone(), staging.clone()];
        let connections = vec![
            vault_connection("web", Some(prod.id)),
            vault_connection("web", Some(staging.id)),
        ];
        let settings = rustconn_core::config::AppSettings::default();

        let plan = plan_credential_transfer(
            &settings,
            &connections,
            &groups,
            SecretBackendType::LibSecret,
            SecretBackendType::PortableEncryptedFile,
        );

        assert_eq!(plan.items.len(), 2);
        assert_eq!(
            plan.collisions,
            vec![vec!["web".to_string(), "web".to_string()]],
            "the flat destination key must be reported as shared, not silently reused"
        );

        // The same pair is fine when the destination keeps groups apart.
        let keyring_plan = plan_credential_transfer(
            &settings,
            &connections,
            &groups,
            SecretBackendType::PortableEncryptedFile,
            SecretBackendType::LibSecret,
        );
        assert!(
            keyring_plan.collisions.is_empty(),
            "the keyring key carries the group path, so there is no collision"
        );
    }

    #[test]
    fn plan_skips_variables_that_point_at_an_external_entry() {
        let settings = rustconn_core::config::AppSettings {
            global_variables: vec![
                rustconn_core::variables::Variable {
                    name: "ours".to_string(),
                    value: String::new(),
                    is_secret: true,
                    description: None,
                    kdbx_entry_path: None,
                    vault_entry_name: None,
                },
                rustconn_core::variables::Variable {
                    name: "theirs".to_string(),
                    value: String::new(),
                    is_secret: true,
                    description: None,
                    kdbx_entry_path: Some("Internet/MyRouter".to_string()),
                    vault_entry_name: None,
                },
                rustconn_core::variables::Variable {
                    name: "plain".to_string(),
                    value: "not a secret".to_string(),
                    is_secret: false,
                    description: None,
                    kdbx_entry_path: None,
                    vault_entry_name: None,
                },
            ],
            ..rustconn_core::config::AppSettings::default()
        };

        let plan = plan_credential_transfer(
            &settings,
            &[],
            &[],
            SecretBackendType::LibSecret,
            SecretBackendType::PortableEncryptedFile,
        );

        let labels: Vec<&str> = plan.items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(
            labels,
            vec!["ours"],
            "only secret variables RustConn itself stores are ours to copy"
        );
    }

    // ── Trash guard: a live twin keeps the shared entry ──────────────
    //
    // Group Sync recreates a connection it cannot match and trashes the old
    // copy, so the Trash can hold a predecessor with the name, the group and
    // the protocol of a live connection — everything a vault key is built
    // from. Deleting the predecessor's credential deleted the live password.

    #[test]
    fn a_trashed_twin_of_a_live_connection_keeps_its_keyring_entry() {
        let settings = app_settings(SecretBackendType::LibSecret);
        let group = ConnectionGroup::new("production-servers".to_string());
        let groups = std::slice::from_ref(&group);
        let trashed = vault_connection("bastion", Some(group.id));
        let live = vec![vault_connection("bastion", Some(group.id))];

        let twin = live_connection_sharing_vault_key(&settings, groups, &trashed, &live);
        assert_eq!(twin.map(|c| c.id), Some(live[0].id));
    }

    #[test]
    fn a_trashed_connection_with_a_key_of_its_own_is_deleted() {
        let settings = app_settings(SecretBackendType::LibSecret);
        let prod = ConnectionGroup::new("Prod".to_string());
        let staging = ConnectionGroup::new("Staging".to_string());
        let groups = vec![prod.clone(), staging.clone()];
        let trashed = vault_connection("web", Some(prod.id));
        // The same name in another group, another name in the same group:
        // neither shares the group-scoped keyring key.
        let live = vec![
            vault_connection("web", Some(staging.id)),
            vault_connection("db", Some(prod.id)),
        ];

        let twin = live_connection_sharing_vault_key(&settings, &groups, &trashed, &live);
        assert!(twin.is_none());
    }

    #[test]
    fn a_trashed_twin_keeps_its_keepass_entry() {
        let settings = keepass_app_settings();
        let group = ConnectionGroup::new("production-servers".to_string());
        let groups = std::slice::from_ref(&group);
        let trashed = vault_connection("bastion", Some(group.id));
        let twin = vault_connection("bastion", Some(group.id));
        let mut over_rdp = vault_connection("bastion", Some(group.id));
        over_rdp.protocol_config =
            rustconn_core::models::ProtocolConfig::Rdp(rustconn_core::models::RdpConfig::default());

        // The entry the delete removes is the one the twin resolves to.
        assert_eq!(
            kdbx_connection_entry(groups, &trashed, "ssh"),
            "RustConn/production-servers/bastion (ssh)"
        );
        let live = std::slice::from_ref(&twin);
        assert!(live_connection_sharing_vault_key(&settings, groups, &trashed, live).is_some());

        // Another protocol is another entry ("… (rdp)"), so nothing is kept.
        let live = std::slice::from_ref(&over_rdp);
        assert!(live_connection_sharing_vault_key(&settings, groups, &trashed, live).is_none());
    }

    /// Bitwarden and the other flat backends key on the name alone, so a
    /// same-named live connection in any group resolves to the trashed one's
    /// entry.
    #[test]
    fn a_flat_key_is_shared_across_groups() {
        let settings = app_settings(SecretBackendType::Bitwarden);
        let prod = ConnectionGroup::new("Prod".to_string());
        let staging = ConnectionGroup::new("Staging".to_string());
        let groups = vec![prod.clone(), staging.clone()];
        let trashed = vault_connection("web", Some(prod.id));
        let live = vec![vault_connection("web", Some(staging.id))];

        let twin = live_connection_sharing_vault_key(&settings, &groups, &trashed, &live);
        assert!(twin.is_some());
    }

    /// A connection never protects itself: a caller that passes every
    /// connection, the trashed one included, still gets the entry deleted.
    #[test]
    fn a_trashed_connection_is_not_its_own_twin() {
        let settings = app_settings(SecretBackendType::LibSecret);
        let trashed = vault_connection("web", None);
        let live = std::slice::from_ref(&trashed);
        assert!(live_connection_sharing_vault_key(&settings, &[], &trashed, live).is_none());
    }
}
