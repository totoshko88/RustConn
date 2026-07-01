//! Vault credential operations
//!
//! Functions for saving, loading, renaming, deleting, and copying credentials
//! in the configured secret backend (KeePass, libsecret, Bitwarden, 1Password,
//! Passbolt, Pass). Extracted from `state.rs` to reduce module complexity.

/// Shows an actionable, blocking dialog when saving a credential fails.
///
/// A silently lost credential is critical, so this uses an `adw::AlertDialog`
/// following the GNOME HIG "explain what happened and what to do" pattern
/// (R1.1, R1.2) rather than a transient toast. The body states the underlying
/// cause — the [`SecretError`] `Display` string, which carries only operation
/// context / backend stderr and never any secret value (R1.4) — together with
/// a recovery action. A "settings" response opens Settings, where the user can
/// pick a working backend in the Secrets section (R1.5). Every string is routed
/// through `i18n()` / `i18n_f()` (R1.3, R16.1).
///
/// Uses `glib::idle_add_local_once` so the dialog is presented on the GTK main
/// thread. Falls back to a log message if no active window is found.
///
/// [`SecretError`]: rustconn_core::error::SecretError
fn show_vault_save_error(err: &rustconn_core::error::SecretError) {
    use gtk4::prelude::*;
    use libadwaita as adw;
    use libadwaita::prelude::*;

    // The cause is the SecretError Display string. Its variants embed only
    // operation context and backend diagnostics (e.g. secret-tool stderr),
    // never secret values, so it is safe to surface verbatim (R1.4).
    let cause = err.to_string();

    gtk4::glib::idle_add_local_once(move || {
        let Some(app) = gtk4::gio::Application::default() else {
            tracing::warn!("Could not show vault save error dialog: no default application");
            return;
        };
        let Some(gtk_app) = app.downcast_ref::<gtk4::Application>() else {
            tracing::warn!("Could not show vault save error dialog: not a GtkApplication");
            return;
        };
        let Some(window) = gtk_app.active_window() else {
            tracing::warn!("Could not show vault save error dialog: no active window");
            return;
        };

        // Recovery action: point the user at Settings -> Secrets to choose a
        // backend that works on their desktop (R1.5).
        let recovery = crate::i18n::i18n(
            "No system keyring is responding. Open Settings, then Secrets, \
             and choose another backend such as Encrypted file or KeePassXC.",
        );
        // Body = cause followed by the recovery action (R1.1).
        let body = crate::i18n::i18n_f("{}\n\n{}", &[&cause, &recovery]);

        let dialog =
            adw::AlertDialog::new(Some(&crate::i18n::i18n("Password not saved")), Some(&body));
        dialog.add_response("close", &crate::i18n::i18n("Close"));
        dialog.add_response("settings", &crate::i18n::i18n("Open Settings"));
        // The recovery action is the primary, suggested action.
        dialog.set_response_appearance("settings", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("settings"));
        dialog.set_close_response("close");

        let window_for_action = window.clone();
        dialog.connect_response(None, move |_, response| {
            if response == "settings" {
                // Activate the existing window action that opens the Settings
                // dialog; the user navigates to the Secrets section there.
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

/// Shows a transient toast when a credential was saved to the encrypted-file
/// fallback because the preferred backend (system keyring) was unavailable.
///
/// The credential *was* saved, so this is a non-blocking result notification
/// (GNOME HIG: transient result → `adw::Toast`, not an error dialog). It is
/// shown at `Warning` priority because the user's preferred backend is degraded
/// and worth their attention. The message is routed through `i18n()` (R3.4,
/// R16.1). The backend identifier is intentionally not surfaced in the UI — the
/// user-facing distinction is only "preferred vs encrypted-file fallback".
///
/// Uses `glib::idle_add_local_once` so the toast is presented on the GTK main
/// thread, mirroring [`show_vault_save_error`]. Falls back to a log message if
/// no active window is found.
fn show_vault_fallback_toast() {
    use gtk4::prelude::*;

    gtk4::glib::idle_add_local_once(move || {
        let Some(app) = gtk4::gio::Application::default() else {
            tracing::warn!("Could not show vault fallback toast: no default application");
            return;
        };
        let Some(gtk_app) = app.downcast_ref::<gtk4::Application>() else {
            tracing::warn!("Could not show vault fallback toast: not a GtkApplication");
            return;
        };
        let Some(window) = gtk_app.active_window() else {
            tracing::warn!("Could not show vault fallback toast: no active window");
            return;
        };

        let message = crate::i18n::i18n(
            "Saved to the encrypted file store because the system keyring was unavailable.",
        );
        crate::toast::show_toast_on_window(&window, &message, crate::toast::ToastType::Warning);
    });
}

/// Stores credentials through the [`SecretManager`] fallback chain, reporting
/// which backend accepted the write.
///
/// Unlike [`dispatch_vault_op`], which targets a single backend, this routes the
/// generic (non-KeePass) save path through [`SecretManager::store_reported`] so a
/// failing preferred backend gracefully falls back to the encrypted-file store
/// when `enable_fallback` is set (issue #201, R3.1). The returned [`StoreOutcome`]
/// lets the caller surface a toast when the fallback was used (R3.4).
///
/// # Errors
///
/// Returns a human-readable error string if the store times out after 10s or
/// every backend in the chain rejects the write.
///
/// [`SecretManager`]: rustconn_core::secret::SecretManager
/// [`SecretManager::store_reported`]: rustconn_core::secret::SecretManager::store_reported
/// [`StoreOutcome`]: rustconn_core::secret::StoreOutcome
fn store_reported_blocking(
    secret_settings: &rustconn_core::config::SecretSettings,
    lookup_key: &str,
    creds: &rustconn_core::models::Credentials,
) -> Result<rustconn_core::secret::StoreOutcome, String> {
    let manager = rustconn_core::secret::SecretManager::build_from_settings(secret_settings);
    let allow_fallback = secret_settings.enable_fallback;

    crate::async_utils::with_runtime(|rt| {
        rt.block_on(async {
            // Vault operations wait 10 seconds — a hung keyring must not block the
            // GTK callback indefinitely (matches dispatch_vault_op's timeout).
            tokio::time::timeout(
                std::time::Duration::from_secs(10),
                manager.store_reported(lookup_key, creds, allow_fallback),
            )
            .await
            .map_err(|_| "Vault store timed out after 10s".to_string())?
            .map_err(|e| format!("{e}"))
        })
    })
    .and_then(|r| r)
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

    if settings.secrets.kdbx_enabled
        && matches!(
            settings.secrets.preferred_backend,
            rustconn_core::config::SecretBackendType::KeePassXc
                | rustconn_core::config::SecretBackendType::KdbxFile
        )
    {
        // KeePass backend — use hierarchical path
        if let Some(kdbx_path) = settings.secrets.kdbx_path.clone() {
            let key_file = settings.secrets.kdbx_key_file.clone();
            let db_password = settings.secrets.kdbx_password.clone();
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
            // Wrap intermediate plaintext copy in Zeroizing so it is
            // wiped from memory on drop (M-PUBLIC-DEBUG / SecretString).
            let pwd = zeroize::Zeroizing::new(password.expose_secret().to_string());

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
                        pwd.as_str(),
                        Some(&url),
                    )
                },
                move |result| {
                    if let Err(e) = result {
                        tracing::error!("Failed to save password to vault: {e}");
                        show_vault_save_error(&e);
                    } else {
                        tracing::info!("Password saved to vault for connection {conn_id}");
                    }
                },
            );
        }
    } else {
        // Generic backend — dispatch via consolidated helper.
        // Use the same key format that the resolver expects for each backend,
        // so that store and resolve are consistent.
        let backend_type = select_backend_for_load(&settings.secrets);
        let lookup_key = generate_store_key(conn_name, conn_host, &protocol_str, backend_type);
        tracing::debug!(
            %lookup_key,
            ?backend_type,
            conn_name,
            conn_host,
            protocol_str,
            "save_password_to_vault: storing with key"
        );
        let username = username.to_string();
        // Re-wrap into a fresh SecretString for the spawn_blocking move closure.
        let secret = password.clone();
        let secret_settings = settings.secrets.clone();

        crate::utils::spawn_blocking_with_callback(
            move || {
                let creds = rustconn_core::models::Credentials {
                    username: Some(username),
                    password: Some(secret),
                    key_passphrase: None,
                    domain: None,
                };
                store_reported_blocking(&secret_settings, &lookup_key, &creds)
            },
            move |result: Result<rustconn_core::secret::StoreOutcome, String>| match result {
                Ok(rustconn_core::secret::StoreOutcome::Primary) => {
                    tracing::info!("Password saved to vault for connection {conn_id}");
                }
                Ok(rustconn_core::secret::StoreOutcome::Fallback { backend_id }) => {
                    tracing::warn!(
                        backend = %backend_id,
                        %conn_id,
                        "Password saved to encrypted-file fallback backend"
                    );
                    show_vault_fallback_toast();
                }
                Err(e) => {
                    tracing::error!("Failed to save password to vault: {e}");
                    show_vault_save_error(&rustconn_core::error::SecretError::StoreFailed(e));
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
            let key_file = settings.secrets.kdbx_key_file.clone();
            let db_password = settings.secrets.kdbx_password.clone();
            let entry_name = group_path
                .strip_prefix("RustConn/")
                .unwrap_or(group_path)
                .to_string();
            let username_val = username.to_string();
            // Wrap intermediate plaintext copy in Zeroizing.
            let password_val = zeroize::Zeroizing::new(password.expose_secret().to_string());

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
                        password_val.as_str(),
                        None,
                    )
                },
                move |result| {
                    if let Err(e) = result {
                        tracing::error!("Failed to save group password to vault: {e}");
                        show_vault_save_error(&e);
                    } else {
                        tracing::info!("Group password saved to vault");
                    }
                },
            );
        }
    } else {
        let lookup_key = lookup_key.to_string();
        let username_val = username.to_string();
        let secret = password.clone();
        let secret_settings = settings.secrets.clone();

        crate::utils::spawn_blocking_with_callback(
            move || {
                let creds = rustconn_core::models::Credentials {
                    username: Some(username_val),
                    password: Some(secret),
                    key_passphrase: None,
                    domain: None,
                };
                store_reported_blocking(&secret_settings, &lookup_key, &creds)
            },
            move |result: Result<rustconn_core::secret::StoreOutcome, String>| match result {
                Ok(rustconn_core::secret::StoreOutcome::Primary) => {
                    tracing::info!("Group password saved to vault");
                }
                Ok(rustconn_core::secret::StoreOutcome::Fallback { backend_id }) => {
                    tracing::warn!(
                        backend = %backend_id,
                        "Group password saved to encrypted-file fallback backend"
                    );
                    show_vault_fallback_toast();
                }
                Err(e) => {
                    tracing::error!("Failed to save group password to vault: {e}");
                    show_vault_save_error(&rustconn_core::error::SecretError::StoreFailed(e));
                }
            },
        );
    }
}

/// Renames a credential in the configured vault backend when a connection
/// is renamed.
pub fn rename_vault_credential(
    settings: &rustconn_core::config::AppSettings,
    groups: &[rustconn_core::models::ConnectionGroup],
    updated_conn: &rustconn_core::models::Connection,
    old_name: &str,
    protocol_str: &str,
) -> Result<(), String> {
    if settings.secrets.kdbx_enabled
        && matches!(
            settings.secrets.preferred_backend,
            rustconn_core::config::SecretBackendType::KeePassXc
                | rustconn_core::config::SecretBackendType::KdbxFile
        )
    {
        // KeePass — rename hierarchical entry
        let mut old_conn = updated_conn.clone();
        old_conn.name = old_name.to_string();
        let old_base = rustconn_core::secret::KeePassHierarchy::build_entry_path(&old_conn, groups);
        let new_base =
            rustconn_core::secret::KeePassHierarchy::build_entry_path(updated_conn, groups);
        let old_key = format!("{old_base} ({protocol_str})");
        let new_key = format!("{new_base} ({protocol_str})");

        if old_key == new_key {
            return Ok(());
        }

        if let Some(kdbx_path) = settings.secrets.kdbx_path.as_ref() {
            let key_file = settings.secrets.kdbx_key_file.clone();
            rustconn_core::secret::KeePassStatus::rename_entry_in_kdbx(
                std::path::Path::new(kdbx_path),
                settings.secrets.kdbx_password.as_ref(),
                key_file.as_ref().map(std::path::Path::new),
                &old_key,
                &new_key,
            )
            .map_err(|e| format!("{e}"))
        } else {
            Ok(())
        }
    } else {
        // Non-KeePass backend — rename flat key using the correct format per backend
        use rustconn_core::config::SecretBackendType;

        let backend_type = select_backend_for_load(&settings.secrets);

        // Build old/new keys based on backend key format
        let (old_key, new_key) = match backend_type {
            SecretBackendType::LibSecret | SecretBackendType::MacOsKeychain => {
                // LibSecret/Keychain uses "{name} ({protocol})" format
                let old_key = format!("{} ({protocol_str})", old_name.replace('/', "-"));
                let new_key = format!("{} ({protocol_str})", updated_conn.name.replace('/', "-"));
                (old_key, new_key)
            }
            SecretBackendType::Bitwarden
            | SecretBackendType::OnePassword
            | SecretBackendType::Passbolt
            | SecretBackendType::Pass => {
                // These backends use "rustconn/{name}" format
                let old_identifier = if old_name.trim().is_empty() {
                    &updated_conn.host
                } else {
                    old_name
                };
                let new_identifier = if updated_conn.name.trim().is_empty() {
                    &updated_conn.host
                } else {
                    &updated_conn.name
                };
                let old_key = format!("rustconn/{old_identifier}");
                let new_key = format!("rustconn/{new_identifier}");
                (old_key, new_key)
            }
            SecretBackendType::EncryptedFile => {
                // EncryptedFile uses the same "rustconn/{name}" flat key format.
                // (Flat-key; correct, not a 2.5 placeholder.)
                let old_identifier = if old_name.trim().is_empty() {
                    &updated_conn.host
                } else {
                    old_name
                };
                let new_identifier = if updated_conn.name.trim().is_empty() {
                    &updated_conn.host
                } else {
                    &updated_conn.name
                };
                let old_key = format!("rustconn/{old_identifier}");
                let new_key = format!("rustconn/{new_identifier}");
                (old_key, new_key)
            }
            SecretBackendType::KeePassXc | SecretBackendType::KdbxFile => {
                // Should not reach here — handled above
                return Ok(());
            }
        };

        if old_key == new_key {
            return Ok(());
        }

        let secret_settings = settings.secrets.clone();
        if let Ok(Some(creds)) = dispatch_vault_op(&secret_settings, &old_key, VaultOp::Retrieve) {
            dispatch_vault_op(&secret_settings, &new_key, VaultOp::Store(&creds))?;
            let _ = dispatch_vault_op(&secret_settings, &old_key, VaultOp::Delete);
        }
        Ok(())
    }
}

/// Renames a vault credential when a connection is moved to a different group.
///
/// For KeePass backends, the entry path includes the group hierarchy, so moving
/// a connection changes the lookup key. This function renames the old entry to
/// the new path so the password remains accessible.
///
/// For non-KeePass backends (libsecret, Bitwarden, etc.), the lookup key uses
/// `name (protocol)` without group info, so no rename is needed.
pub fn rename_vault_credential_for_move(
    settings: &rustconn_core::config::AppSettings,
    groups: &[rustconn_core::models::ConnectionGroup],
    old_conn: &rustconn_core::models::Connection,
    new_conn: &rustconn_core::models::Connection,
    protocol_str: &str,
) -> Result<(), String> {
    // Only KeePass backends use group hierarchy in the entry path
    if settings.secrets.kdbx_enabled
        && matches!(
            settings.secrets.preferred_backend,
            rustconn_core::config::SecretBackendType::KeePassXc
                | rustconn_core::config::SecretBackendType::KdbxFile
        )
    {
        let old_base = rustconn_core::secret::KeePassHierarchy::build_entry_path(old_conn, groups);
        let new_base = rustconn_core::secret::KeePassHierarchy::build_entry_path(new_conn, groups);
        let old_key = format!("{old_base} ({protocol_str})");
        let new_key = format!("{new_base} ({protocol_str})");

        if old_key == new_key {
            return Ok(());
        }

        tracing::info!(
            %old_key, %new_key,
            "Migrating KeePass entry after group move"
        );

        if let Some(kdbx_path) = settings.secrets.kdbx_path.as_ref() {
            let key_file = settings.secrets.kdbx_key_file.clone();
            rustconn_core::secret::KeePassStatus::rename_entry_in_kdbx(
                std::path::Path::new(kdbx_path),
                settings.secrets.kdbx_password.as_ref(),
                key_file.as_ref().map(std::path::Path::new),
                &old_key,
                &new_key,
            )
            .map_err(|e| format!("{e}"))
        } else {
            Ok(())
        }
    } else {
        // Non-KeePass backends use flat keys without group info — no rename needed
        Ok(())
    }
}

/// Migrates all KeePass vault entries affected by a group rename or move.
///
/// When a group is renamed or moved to a different parent, the hierarchical
/// KeePass entry paths change for:
/// 1. The group's own credential (if `password_source == Vault`)
/// 2. All connections in the group (and descendant groups) with `password_source == Vault`
///
/// Non-KeePass backends use flat keys without group hierarchy, so no migration
/// is needed for them.
pub fn migrate_vault_entries_on_group_change(
    settings: &rustconn_core::config::AppSettings,
    old_groups: &[rustconn_core::models::ConnectionGroup],
    new_groups: &[rustconn_core::models::ConnectionGroup],
    connections: &[rustconn_core::models::Connection],
    changed_group_id: uuid::Uuid,
) {
    // Only KeePass backends use group hierarchy in entry paths
    if !settings.secrets.kdbx_enabled
        || !matches!(
            settings.secrets.preferred_backend,
            rustconn_core::config::SecretBackendType::KeePassXc
                | rustconn_core::config::SecretBackendType::KdbxFile
        )
    {
        return;
    }

    let Some(kdbx_path) = settings.secrets.kdbx_path.clone() else {
        return;
    };

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
    use secrecy::ExposeSecret;

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
                // Wrap intermediate plaintext copy in Zeroizing for the FFI call.
                let pwd = zeroize::Zeroizing::new(password.expose_secret().to_string());
                let result = rustconn_core::secret::KeePassStatus::save_password_to_kdbx(
                    kdbx,
                    settings.kdbx_password.as_ref(),
                    key,
                    &lookup_key,
                    "",
                    pwd.as_str(),
                    None,
                )
                .map_err(|e| format!("{e}"));

                // If KeePass save failed and fallback is enabled, try LibSecret
                if result.is_err() && settings.enable_fallback {
                    tracing::info!(var_name, "KeePass save failed, falling back to LibSecret");
                    dispatch_vault_op(settings, &lookup_key, VaultOp::Store(&creds))?;
                    Ok(())
                } else {
                    result
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
) -> Result<Option<String>, String> {
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
                    )
                } else {
                    rustconn_core::secret::KeePassStatus::get_password_from_kdbx_with_key(
                        kdbx,
                        settings.kdbx_password.as_ref(),
                        key,
                        lookup_key,
                        None,
                    )
                }
                .map(|opt| {
                    opt.map(|s| {
                        let z = zeroize::Zeroizing::new(s.expose_secret().to_string());
                        // Return the zeroized string content; the Zeroizing wrapper
                        // ensures the original is wiped when `z` drops at end of scope.
                        String::from(z.as_str())
                    })
                })
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
                            Ok(Some(creds)) if creds.expose_password().is_some() => {
                                Ok(creds.expose_password().map(String::from))
                            }
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
                Ok(creds.and_then(|c| c.expose_password().map(String::from)))
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
) -> Result<Option<String>, String> {
    use rustconn_core::config::SecretBackendType;
    use rustconn_core::secret::SecretBackend;
    use secrecy::ExposeSecret;

    let backend_type = select_backend_for_load(settings);

    crate::async_utils::with_runtime(|rt| {
        rt.block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                match backend_type {
                    SecretBackendType::Bitwarden => {
                        let bw = rustconn_core::secret::auto_unlock(settings)
                            .await
                            .map_err(|e| format!("{e}"))?;
                        let password = bw
                            .find_password_by_exact_name(entry_name)
                            .await
                            .map_err(|e| format!("{e}"))?;
                        Ok(password.map(|p| {
                            let z = zeroize::Zeroizing::new(p.expose_secret().to_string());
                            String::from(z.as_str())
                        }))
                    }
                    SecretBackendType::OnePassword => {
                        // 1Password: use `op item get "{name}" --fields password`
                        let mut backend = rustconn_core::secret::OnePasswordBackend::new();
                        if let Some(ref token) = settings.onepassword_service_account_token {
                            backend.set_service_account_token(token.clone());
                        }
                        let creds = backend
                            .retrieve(entry_name)
                            .await
                            .map_err(|e| format!("{e}"))?;
                        Ok(creds.and_then(|c| {
                            c.expose_password().map(|p| {
                                let z = zeroize::Zeroizing::new(p.to_string());
                                String::from(z.as_str())
                            })
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
                            c.expose_password().map(|p| {
                                let z = zeroize::Zeroizing::new(p.to_string());
                                String::from(z.as_str())
                            })
                        }))
                    }
                    SecretBackendType::Passbolt => {
                        let mut backend = rustconn_core::secret::PassboltBackend::new();
                        if let Some(ref url) = settings.passbolt_server_url {
                            backend = backend.with_server_address(url.clone());
                        }
                        if let Some(ref passphrase) = settings.passbolt_passphrase {
                            backend = backend.with_user_password(passphrase.clone());
                        }
                        let creds = backend
                            .retrieve(entry_name)
                            .await
                            .map_err(|e| format!("{e}"))?;
                        Ok(creds.and_then(|c| {
                            c.expose_password().map(|p| {
                                let z = zeroize::Zeroizing::new(p.to_string());
                                String::from(z.as_str())
                            })
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
                            c.expose_password().map(|p| {
                                let z = zeroize::Zeroizing::new(p.to_string());
                                String::from(z.as_str())
                            })
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
                            c.expose_password().map(|p| {
                                let z = zeroize::Zeroizing::new(p.to_string());
                                String::from(z.as_str())
                            })
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
                            c.expose_password().map(|p| {
                                let z = zeroize::Zeroizing::new(p.to_string());
                                String::from(z.as_str())
                            })
                        }))
                    }
                }
            })
            .await
            .map_err(|_| "Vault retrieve by entry name timed out after 10s".to_string())?
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
    let mut vars = settings.global_variables.clone();
    for var in &mut vars {
        if var.is_secret && var.value.is_empty() {
            match load_variable_from_vault_with_path(
                &settings.secrets,
                &var.name,
                var.kdbx_entry_path.as_deref(),
                var.vault_entry_name.as_deref(),
            ) {
                Ok(Some(pwd)) => var.value = pwd,
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

/// Deletes a connection's vault credentials from the configured backend.
///
/// For KeePass backends, deletes the hierarchical entry. For flat backends,
/// deletes by the standard lookup key format.
///
/// This is called during permanent deletion (empty trash) — not during
/// soft-delete to trash, so that restore works without re-entering passwords.
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
                let entry_path =
                    rustconn_core::secret::KeePassHierarchy::build_entry_path(connection, groups);
                let base_path = entry_path.strip_prefix("RustConn/").unwrap_or(&entry_path);
                let entry_name = format!("{base_path} ({protocol_str})");
                let key_file = settings.secrets.kdbx_key_file.clone();
                let kdbx = std::path::Path::new(kdbx_path);
                let key = key_file.as_ref().map(std::path::Path::new);
                // KeePass delete is done by saving empty entry — or we just log
                // that KeePass entries should be cleaned manually, since the KDBX
                // API doesn't expose a delete_entry method directly.
                // For now, attempt to overwrite with empty password as a best-effort.
                rustconn_core::secret::KeePassStatus::save_password_to_kdbx(
                    kdbx,
                    settings.secrets.kdbx_password.as_ref(),
                    key,
                    &entry_name,
                    "",
                    "",
                    None,
                )
                .map_err(|e| format!("{e}"))
            } else {
                Ok(()) // No KDBX configured, nothing to clean
            }
        }
        _ => {
            let backend_type = select_backend_for_load(&settings.secrets);
            let lookup_key = generate_store_key(
                &connection.name,
                &connection.host,
                &protocol_str,
                backend_type,
            );
            dispatch_vault_op(&settings.secrets, &lookup_key, VaultOp::Delete)?;
            Ok(())
        }
    }
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
                rustconn_core::secret::KeePassStatus::save_password_to_kdbx(
                    kdbx,
                    settings.secrets.kdbx_password.as_ref(),
                    key,
                    &group_path,
                    "",
                    "",
                    None,
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
                    )
                    .map_err(|e| format!("{e}"))?;

                if let Some(pwd) = password_opt {
                    use secrecy::ExposeSecret;
                    // Write to new entry
                    let new_entry_path =
                        rustconn_core::secret::KeePassHierarchy::build_entry_path(new_conn, groups);
                    let new_base = new_entry_path
                        .strip_prefix("RustConn/")
                        .unwrap_or(&new_entry_path);
                    let new_entry_name = format!("{new_base} ({protocol_str})");
                    let username = new_conn.username.as_deref().unwrap_or("");
                    let url = format!("{}://{}", protocol_str, &new_conn.host);
                    rustconn_core::secret::KeePassStatus::save_password_to_kdbx(
                        kdbx,
                        settings.secrets.kdbx_password.as_ref(),
                        key,
                        &new_entry_name,
                        username,
                        pwd.expose_secret(),
                        Some(&url),
                    )
                    .map_err(|e| format!("{e}"))?;
                }
                Ok(())
            } else {
                Ok(())
            }
        }
        _ => {
            let backend_type = select_backend_for_load(&settings.secrets);
            let old_key =
                generate_store_key(&old_conn.name, &old_conn.host, &protocol_str, backend_type);
            let new_key =
                generate_store_key(&new_conn.name, &new_conn.host, &protocol_str, backend_type);

            if let Some(creds) = dispatch_vault_op(&settings.secrets, &old_key, VaultOp::Retrieve)?
            {
                dispatch_vault_op(&settings.secrets, &new_key, VaultOp::Store(&creds))?;
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
    use rustconn_core::config::SecretBackendType;
    use rustconn_core::secret::SecretBackend;

    let backend_type = select_backend_for_load(secret_settings);

    crate::async_utils::with_runtime(|rt| {
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
            SecretBackendType::OnePassword => {
                let mut backend = rustconn_core::secret::OnePasswordBackend::new();
                if let Some(ref token) = secret_settings.onepassword_service_account_token {
                    backend.set_service_account_token(token.clone());
                }
                std::sync::Arc::new(backend)
            }
            SecretBackendType::Passbolt => {
                let mut backend = rustconn_core::secret::PassboltBackend::new();
                if let Some(ref url) = secret_settings.passbolt_server_url {
                    backend = backend.with_server_address(url.clone());
                }
                if let Some(ref passphrase) = secret_settings.passbolt_passphrase {
                    backend = backend.with_user_password(passphrase.clone());
                }
                std::sync::Arc::new(backend)
            }
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
        };

        match op {
            VaultOp::Store(creds) => {
                tracing::debug!(
                    %lookup_key,
                    ?backend_type,
                    "dispatch_vault_op: storing credentials"
                );
                rt.block_on(async {
                    tokio::time::timeout(
                        std::time::Duration::from_secs(10),
                        backend.store(lookup_key, creds),
                    )
                    .await
                    .map_err(|_| "Vault store timed out after 10s".to_string())?
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
                    tokio::time::timeout(
                        std::time::Duration::from_secs(10),
                        backend.retrieve(lookup_key),
                    )
                    .await
                    .map_err(|_| "Vault retrieve timed out after 10s".to_string())?
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
                    tokio::time::timeout(
                        std::time::Duration::from_secs(10),
                        backend.delete(lookup_key),
                    )
                    .await
                    .map_err(|_| "Vault delete timed out after 10s".to_string())?
                    .map_err(|e| format!("{e}"))
                })?;
                Ok(None)
            }
        }
    })
    .and_then(|r| r)
}

/// Selects the appropriate storage backend for variable secrets.
///
/// Mirrors `CredentialResolver::select_storage_backend` logic.
/// Also used by connection password load/save and variable vault operations.
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
    }
}

/// Generates the correct store key for a connection based on the backend type.
///
/// LibSecret uses `"{name} ({protocol})"` format (matching
/// [`CredentialResolver::generate_keyring_key`]), while all other backends use
/// `"rustconn/{name}"` (matching [`CredentialResolver::generate_lookup_key`]).
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
    use rustconn_core::config::SecretBackendType;

    if backend_type == SecretBackendType::LibSecret {
        // LibSecret format: "{name} ({protocol})" — matches generate_keyring_key
        let name = conn_name.trim().replace('/', "-");
        format!("{name} ({protocol_str})")
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

#[cfg(test)]
mod tests {
    use super::*;
    use rustconn_core::config::{SecretBackendType, SecretSettings};

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
}
