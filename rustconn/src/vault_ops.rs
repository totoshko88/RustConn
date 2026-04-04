//! Vault credential operations
//!
//! Functions for saving, loading, renaming, deleting, and copying credentials
//! in the configured secret backend (KeePass, libsecret, Bitwarden, 1Password,
//! Passbolt, Pass). Extracted from `state.rs` to reduce module complexity.

/// Shows an error toast when saving to vault fails.
///
/// Uses `glib::idle_add_local_once` to ensure the toast is shown on the GTK
/// main thread. Falls back to stderr if no active window is found.
fn show_vault_save_error_toast() {
    use gtk4::prelude::*;
    gtk4::glib::idle_add_local_once(|| {
        if let Some(app) = gtk4::gio::Application::default()
            && let Some(gtk_app) = app.downcast_ref::<gtk4::Application>()
            && let Some(window) = gtk_app.active_window()
        {
            crate::toast::show_toast_on_window(
                &window,
                &crate::i18n::i18n("Failed to save password to vault"),
                crate::toast::ToastType::Error,
            );
            return;
        }
        tracing::warn!("Could not show vault save error toast: no active window");
    });
}

/// Saves a connection password to the configured vault backend.
///
/// Dispatches to KeePass (hierarchical) or generic backend (flat key)
/// based on the current settings.
#[allow(clippy::too_many_arguments)]
pub fn save_password_to_vault(
    settings: &rustconn_core::config::AppSettings,
    groups: &[rustconn_core::models::ConnectionGroup],
    conn: Option<&rustconn_core::models::Connection>,
    conn_name: &str,
    conn_host: &str,
    protocol: rustconn_core::models::ProtocolType,
    username: &str,
    password: &str,
    conn_id: uuid::Uuid,
) {
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
            let pwd = password.to_string();

            crate::utils::spawn_blocking_with_callback(
                move || {
                    let kdbx = std::path::Path::new(&kdbx_path);
                    let key = key_file.as_ref().map(|p| std::path::Path::new(p));
                    rustconn_core::secret::KeePassStatus::save_password_to_kdbx(
                        kdbx,
                        db_password.as_ref(),
                        key,
                        &entry_name,
                        &username,
                        &pwd,
                        Some(&url),
                    )
                },
                move |result| {
                    if let Err(e) = result {
                        tracing::error!("Failed to save password to vault: {e}");
                        show_vault_save_error_toast();
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
        let pwd = password.to_string();
        let secret_settings = settings.secrets.clone();

        crate::utils::spawn_blocking_with_callback(
            move || {
                let creds = rustconn_core::models::Credentials {
                    username: Some(username),
                    password: Some(secrecy::SecretString::from(pwd)),
                    key_passphrase: None,
                    domain: None,
                };
                dispatch_vault_op(&secret_settings, &lookup_key, VaultOp::Store(&creds))?;
                Ok(())
            },
            move |result: Result<(), String>| {
                if let Err(e) = result {
                    tracing::error!("Failed to save password to vault: {e}");
                    show_vault_save_error_toast();
                } else {
                    tracing::info!("Password saved to vault for connection {conn_id}");
                }
            },
        );
    }
}

/// Saves a group password to the configured vault backend.
pub fn save_group_password_to_vault(
    settings: &rustconn_core::config::AppSettings,
    group_path: &str,
    lookup_key: &str,
    username: &str,
    password: &str,
) {
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
            let password_val = password.to_string();

            crate::utils::spawn_blocking_with_callback(
                move || {
                    let kdbx = std::path::Path::new(&kdbx_path);
                    let key = key_file.as_ref().map(|p| std::path::Path::new(p));
                    rustconn_core::secret::KeePassStatus::save_password_to_kdbx(
                        kdbx,
                        db_password.as_ref(),
                        key,
                        &entry_name,
                        &username_val,
                        &password_val,
                        None,
                    )
                },
                move |result| {
                    if let Err(e) = result {
                        tracing::error!("Failed to save group password to vault: {e}");
                        show_vault_save_error_toast();
                    } else {
                        tracing::info!("Group password saved to vault");
                    }
                },
            );
        }
    } else {
        let lookup_key = lookup_key.to_string();
        let username_val = username.to_string();
        let password_val = password.to_string();
        let secret_settings = settings.secrets.clone();

        crate::utils::spawn_blocking_with_callback(
            move || {
                let creds = rustconn_core::models::Credentials {
                    username: Some(username_val),
                    password: Some(secrecy::SecretString::from(password_val)),
                    key_passphrase: None,
                    domain: None,
                };
                dispatch_vault_op(&secret_settings, &lookup_key, VaultOp::Store(&creds))?;
                Ok(())
            },
            move |result: Result<(), String>| {
                if let Err(e) = result {
                    tracing::error!("Failed to save group password to vault: {e}");
                    show_vault_save_error_toast();
                } else {
                    tracing::info!("Group password saved to vault");
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
                key_file.as_ref().map(|p| std::path::Path::new(p)),
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
            SecretBackendType::LibSecret => {
                // LibSecret uses "{name} ({protocol})" format
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
                key_file.as_ref().map(|p| std::path::Path::new(p)),
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
    let mut affected_group_ids = vec![changed_group_id];
    collect_descendant_groups(changed_group_id, new_groups, &mut affected_group_ids);

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
            let key = key_file.as_ref().map(|p| std::path::Path::new(p));
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

/// Collects all descendant group IDs recursively.
fn collect_descendant_groups(
    parent_id: uuid::Uuid,
    groups: &[rustconn_core::models::ConnectionGroup],
    result: &mut Vec<uuid::Uuid>,
) {
    for group in groups {
        if group.parent_id == Some(parent_id) && !result.contains(&group.id) {
            result.push(group.id);
            collect_descendant_groups(group.id, groups, result);
        }
    }
}

/// Saves a secret variable value to the configured vault backend.
///
/// Respects `preferred_backend` from secret settings, using the same
/// backend selection logic as connection passwords.
pub fn save_variable_to_vault(
    settings: &rustconn_core::config::SecretSettings,
    var_name: &str,
    password: &str,
) -> Result<(), String> {
    use rustconn_core::config::SecretBackendType;

    let lookup_key = rustconn_core::variable_secret_key(var_name);
    let backend_type = select_backend_for_load(settings);

    tracing::debug!(?backend_type, var_name, "Saving secret variable to vault");

    let creds = rustconn_core::models::Credentials {
        username: None,
        password: Some(secrecy::SecretString::from(password.to_string())),
        key_passphrase: None,
        domain: None,
    };

    match backend_type {
        SecretBackendType::KdbxFile | SecretBackendType::KeePassXc => {
            if let Some(kdbx_path) = settings.kdbx_path.as_ref() {
                let key_file = settings.kdbx_key_file.clone();
                let kdbx = std::path::Path::new(kdbx_path);
                let key = key_file.as_ref().map(|p| std::path::Path::new(p));
                rustconn_core::secret::KeePassStatus::save_password_to_kdbx(
                    kdbx,
                    settings.kdbx_password.as_ref(),
                    key,
                    &lookup_key,
                    "",
                    password,
                    None,
                )
                .map_err(|e| format!("{e}"))
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

/// Loads a secret variable value from the configured vault backend.
///
/// Respects `preferred_backend` from secret settings, using the same
/// backend selection logic as connection passwords.
pub fn load_variable_from_vault(
    settings: &rustconn_core::config::SecretSettings,
    var_name: &str,
) -> Result<Option<String>, String> {
    use rustconn_core::config::SecretBackendType;
    use secrecy::ExposeSecret;

    let lookup_key = rustconn_core::variable_secret_key(var_name);
    let backend_type = select_backend_for_load(settings);

    tracing::debug!(
        ?backend_type,
        var_name,
        "Loading secret variable from vault"
    );

    match backend_type {
        SecretBackendType::KdbxFile | SecretBackendType::KeePassXc => {
            if let Some(kdbx_path) = settings.kdbx_path.as_ref() {
                let key_file = settings.kdbx_key_file.clone();
                let kdbx = std::path::Path::new(kdbx_path);
                let key = key_file.as_ref().map(|p| std::path::Path::new(p));
                rustconn_core::secret::KeePassStatus::get_password_from_kdbx_with_key(
                    kdbx,
                    settings.kdbx_password.as_ref(),
                    key,
                    &lookup_key,
                    None,
                )
                .map(|opt| opt.map(|s| s.expose_secret().to_string()))
                .map_err(|e| format!("{e}"))
            } else {
                Err("KeePass enabled but no database file configured".to_string())
            }
        }
        _ => {
            let creds = dispatch_vault_op(settings, &lookup_key, VaultOp::Retrieve)?;
            Ok(creds.and_then(|c| c.expose_password().map(String::from)))
        }
    }
}

/// Returns global variables with secret values restored from vault.
///
/// Non-secret variables are returned as-is. Secret variables with empty
/// values have their values loaded from the configured vault backend.
/// Vault load failures are logged but do not prevent other variables
/// from being returned.
pub fn resolve_global_variables(
    settings: &rustconn_core::config::AppSettings,
) -> Vec<rustconn_core::Variable> {
    let mut vars = settings.global_variables.clone();
    for var in &mut vars {
        if var.is_secret && var.value.is_empty() {
            match load_variable_from_vault(&settings.secrets, &var.name) {
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
                let key = key_file.as_ref().map(|p| std::path::Path::new(p));
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
                let key = key_file.as_ref().map(|p| std::path::Path::new(p));
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
                let key = key_file.as_ref().map(|p| std::path::Path::new(p));

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
            SecretBackendType::Bitwarden => std::sync::Arc::new(
                rt.block_on(rustconn_core::secret::auto_unlock(secret_settings))
                    .map_err(|e| format!("{e}"))?,
            ),
            SecretBackendType::OnePassword => {
                std::sync::Arc::new(rustconn_core::secret::OnePasswordBackend::new())
            }
            SecretBackendType::Passbolt => {
                std::sync::Arc::new(rustconn_core::secret::PassboltBackend::new())
            }
            SecretBackendType::Pass => std::sync::Arc::new(
                rustconn_core::secret::PassBackend::from_secret_settings(secret_settings),
            ),
            SecretBackendType::LibSecret
            | SecretBackendType::KeePassXc
            | SecretBackendType::KdbxFile => {
                std::sync::Arc::new(rustconn_core::secret::LibSecretBackend::new("rustconn"))
            }
        };

        match op {
            VaultOp::Store(creds) => {
                tracing::debug!(
                    %lookup_key,
                    ?backend_type,
                    "dispatch_vault_op: storing credentials"
                );
                rt.block_on(backend.store(lookup_key, creds))
                    .map_err(|e| format!("{e}"))?;
                tracing::debug!(%lookup_key, "dispatch_vault_op: store succeeded");
                Ok(None)
            }
            VaultOp::Retrieve => {
                tracing::debug!(
                    %lookup_key,
                    ?backend_type,
                    "dispatch_vault_op: retrieving credentials"
                );
                let result = rt
                    .block_on(backend.retrieve(lookup_key))
                    .map_err(|e| format!("{e}"))?;
                tracing::debug!(
                    %lookup_key,
                    found = result.is_some(),
                    "dispatch_vault_op: retrieve completed"
                );
                Ok(result)
            }
            VaultOp::Delete => {
                rt.block_on(backend.delete(lookup_key))
                    .map_err(|e| format!("{e}"))?;
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
