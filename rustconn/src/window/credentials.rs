//! Credential resolution for connection startup
//!
//! Extracted from `window/mod.rs` — handles vault lookups, caching,
//! and password prompts before handing off to protocol-specific starters.

use super::*;

impl MainWindow {
    /// Starts a connection with credential resolution
    ///
    /// This method implements the credential resolution flow:
    /// 1. Check the connection's `password_source` setting
    /// 2. Try to resolve credentials from configured backends (`KeePass`, Keyring)
    /// 3. Fall back to cached credentials if available
    /// 4. Prompt user if no credentials found and required
    ///
    /// Uses async credential resolution to avoid blocking the GTK main thread.
    pub(crate) fn start_connection_with_credential_resolution(
        state: SharedAppState,
        notebook: SharedNotebook,
        split_view: SharedSplitView,
        sidebar: SharedSidebar,
        monitoring: types::SharedMonitoring,
        connection_id: Uuid,
        activity: Option<types::SharedActivityCoordinator>,
    ) {
        // Acquire busy guard — spinner shows while connection is in progress.
        // The guard is moved into closures so it stays alive until the
        // connection completes (or the credential dialog is dismissed).
        let busy_guard = acquire_busy_guard();

        // Get connection info and cached credentials (fast, non-blocking)
        let (protocol_type, cached_credentials) = {
            let Ok(state_ref) = state.try_borrow() else {
                tracing::warn!("Could not borrow state for credential resolution");
                drop(busy_guard);
                return;
            };

            let conn = match state_ref.get_connection(connection_id) {
                Some(c) => c,
                None => return,
            };

            let protocol_type = conn.protocol_config.protocol_type();

            let cached = state_ref.get_cached_credentials(connection_id).map(|c| {
                use secrecy::ExposeSecret;
                use zeroize::Zeroizing;
                (
                    c.username.clone(),
                    Zeroizing::new(c.password.expose_secret().to_string()),
                    c.domain.clone(),
                )
            });

            (protocol_type, cached)
        };

        // SFTP via mc uses SSH key in agent — skip vault credential
        // resolution entirely to avoid ~12s of sequential Bitwarden
        // CLI calls that mc never uses.
        if protocol_type == rustconn_core::models::ProtocolType::Sftp {
            drop(busy_guard);
            Self::handle_sftp_connect(
                &state,
                &notebook,
                Some(&sidebar),
                Some(&split_view),
                connection_id,
            );
            return;
        }

        // Web bookmarks open in the default browser — we don't pass credentials
        // to the browser, so vault resolution is unnecessary overhead (~3s for
        // KeePassXC). Credentials are resolved on-demand via Copy Password.
        if protocol_type == rustconn_core::models::ProtocolType::Web {
            drop(busy_guard);
            Self::handle_web_connect(&state, &sidebar, connection_id);
            return;
        }

        // Skip async credential resolution for connections that don't use
        // vault passwords (None = SSH key / no password, Prompt = ask user).
        // This avoids ~12s of sequential Bitwarden CLI calls for connections
        // that never need vault credentials.
        let needs_vault_resolution = {
            let Ok(state_ref) = state.try_borrow() else {
                drop(busy_guard);
                return;
            };
            state_ref
                .get_connection(connection_id)
                .map(|c| {
                    // Skip vault lookup for connections that don't need it
                    if matches!(
                        c.password_source,
                        rustconn_core::models::PasswordSource::None
                            | rustconn_core::models::PasswordSource::Prompt
                    ) {
                        return false;
                    }
                    // Skip vault lookup for Zero Trust Generic — the custom command
                    // handles its own authentication interactively in the terminal
                    if let rustconn_core::ProtocolConfig::ZeroTrust(ref zt) = c.protocol_config
                        && matches!(
                            zt.provider,
                            rustconn_core::models::ZeroTrustProvider::Generic
                        )
                    {
                        return false;
                    }
                    true
                })
                .unwrap_or(false)
        };

        if !needs_vault_resolution {
            drop(busy_guard);
            Self::handle_resolved_credentials(
                state,
                notebook,
                split_view,
                sidebar,
                monitoring,
                connection_id,
                protocol_type,
                None,
                None,
                activity,
            );
            return;
        }

        // If we have cached credentials, use them immediately (no async needed)
        if let Some((username, password, domain)) = cached_credentials {
            Self::handle_resolved_credentials(
                state,
                notebook,
                split_view,
                sidebar,
                monitoring,
                connection_id,
                protocol_type,
                Some(rustconn_core::Credentials::with_password(
                    &username,
                    password.as_str(),
                )),
                Some((username, password, domain)),
                activity,
            );
            return;
        }

        // Resolve credentials asynchronously to avoid blocking GTK main thread
        let state_clone = state.clone();
        let notebook_clone = notebook.clone();
        let split_view_clone = split_view.clone();
        let sidebar_clone = sidebar.clone();
        let monitoring_clone = monitoring.clone();
        let activity_clone = activity;

        {
            let Ok(state_ref) = state.try_borrow() else {
                tracing::warn!("Could not borrow state for async credential resolution");
                return;
            };

            // Handle CredentialResolutionResult variants with appropriate dialogs
            state_ref.resolve_credentials_gtk(connection_id, move |result| {
                // Keep busy guard alive until the callback completes
                use rustconn_core::sync::CredentialResolutionResult;
                let _busy_guard = busy_guard;

                let resolution = match result {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!("Failed to resolve credentials: {e}");
                        // Fall through with no credentials — protocol handler will prompt if needed
                        CredentialResolutionResult::NotNeeded
                    }
                };

                match resolution {
                    CredentialResolutionResult::Resolved(creds) => {
                        Self::handle_resolved_credentials(
                            state_clone,
                            notebook_clone,
                            split_view_clone,
                            sidebar_clone,
                            monitoring_clone,
                            connection_id,
                            protocol_type,
                            Some(creds),
                            None,
                            activity_clone,
                        );
                    }
                    CredentialResolutionResult::NotNeeded => {
                        Self::handle_resolved_credentials(
                            state_clone,
                            notebook_clone,
                            split_view_clone,
                            sidebar_clone,
                            monitoring_clone,
                            connection_id,
                            protocol_type,
                            None,
                            None,
                            activity_clone,
                        );
                    }
                    CredentialResolutionResult::VariableMissing {
                        variable_name,
                        description,
                        ..
                    } => {
                        // Show variable setup dialog so the user can enter the value
                        let conn_name = state_clone
                            .try_borrow()
                            .ok()
                            .and_then(|s| s.get_connection(connection_id).map(|c| c.name.clone()))
                            .unwrap_or_default();
                        // Build dynamic backend list: configured preferred backend + LibSecret fallback.
                        let (backend_names_owned, backend_types) = {
                            let preferred = state_clone
                                .try_borrow()
                                .ok()
                                .map(|s| s.settings().secrets.preferred_backend)
                                .unwrap_or_default();

                            let mut names: Vec<String> = Vec::new();
                            let mut types: Vec<rustconn_core::config::SecretBackendType> = Vec::new();

                            // Add preferred backend first (if it's not LibSecret)
                            match preferred {
                                rustconn_core::config::SecretBackendType::KeePassXc
                                | rustconn_core::config::SecretBackendType::KdbxFile => {
                                    names.push("KeePassXC".to_string());
                                    types.push(preferred);
                                }
                                rustconn_core::config::SecretBackendType::Bitwarden => {
                                    names.push("Bitwarden".to_string());
                                    types.push(preferred);
                                }
                                rustconn_core::config::SecretBackendType::OnePassword => {
                                    names.push("1Password".to_string());
                                    types.push(preferred);
                                }
                                rustconn_core::config::SecretBackendType::Passbolt => {
                                    names.push("Passbolt".to_string());
                                    types.push(preferred);
                                }
                                rustconn_core::config::SecretBackendType::Pass => {
                                    names.push("Pass".to_string());
                                    types.push(preferred);
                                }
                                rustconn_core::config::SecretBackendType::MacOsKeychain => {
                                    names.push("macOS Keychain".to_string());
                                    types.push(preferred);
                                }
                                rustconn_core::config::SecretBackendType::LibSecret => {}
                                rustconn_core::config::SecretBackendType::EncryptedFile => {
                                    // App-managed encrypted file: a real,
                                    // selectable storage target (unlike LibSecret,
                                    // which is only offered as the fallback below).
                                    // Descriptive label → i18n (not a brand name).
                                    names.push(crate::i18n::i18n("Encrypted file"));
                                    types.push(preferred);
                                }
                            }

                            // Always add LibSecret as fallback option
                            names.push("LibSecret".to_string());
                            types.push(rustconn_core::config::SecretBackendType::LibSecret);

                            (names, types)
                        };

                        let backend_refs: Vec<&str> =
                            backend_names_owned.iter().map(|s| s.as_str()).collect();

                        {
                            let state_var = state_clone.clone();
                            let notebook_var = notebook_clone.clone();
                            let split_var = split_view_clone.clone();
                            let sidebar_var = sidebar_clone.clone();
                            let monitoring_var = monitoring_clone.clone();
                            let activity_var = activity_clone.clone();
                            let variable_name_owned = variable_name.clone();

                            crate::dialogs::show_variable_setup_dialog(
                                notebook_clone.widget(),
                                &conn_name,
                                &variable_name,
                                description.as_deref(),
                                &backend_refs,
                                move |response| {
                                    if let crate::dialogs::VariableSetupResponse::Save { value, backend_index } = response {
                                        // Map dialog backend index to SecretBackendType using the dynamic list
                                        let selected_backend = backend_types
                                            .get(backend_index as usize)
                                            .copied()
                                            .unwrap_or(rustconn_core::config::SecretBackendType::LibSecret);

                                        // Save the variable value using the user-selected backend
                                        let save_ok = if let Ok(state_ref) = state_var.try_borrow() {
                                            let mut settings = state_ref.settings().secrets.clone();
                                            // Override preferred_backend to match user's choice
                                            settings.preferred_backend = selected_backend;
                                            match crate::state::save_variable_to_vault(&settings, &variable_name_owned, &value) {
                                                Ok(()) => true,
                                                Err(e) => {
                                                    tracing::error!(
                                                        var_name = %variable_name_owned,
                                                        error = %e,
                                                        "Failed to save variable to vault"
                                                    );
                                                    if let Some(root) = notebook_var.widget().root()
                                                        && let Some(window) = root.downcast_ref::<gtk4::Window>()
                                                    {
                                                        crate::toast::show_toast_on_window(
                                                            window,
                                                            &crate::i18n::i18n_f(
                                                                "Failed to save variable to vault: {}",
                                                                &[&e],
                                                            ),
                                                            crate::toast::ToastType::Error,
                                                        );
                                                    }
                                                    false
                                                }
                                            }
                                        } else {
                                            false
                                        };
                                        if save_ok {
                                            // Retry with the newly saved variable
                                            Self::handle_resolved_credentials(
                                                state_var.clone(),
                                                notebook_var.clone(),
                                                split_var.clone(),
                                                sidebar_var.clone(),
                                                monitoring_var.clone(),
                                                connection_id,
                                                protocol_type,
                                                None,
                                                None,
                                                activity_var.clone(),
                                            );
                                        }
                                    }
                                    // Cancel — do nothing, connection is not started
                                },
                            );
                        }
                    }
                    CredentialResolutionResult::BackendNotConfigured { .. } => {
                        // Show backend missing dialog
                        {
                            let state_be = state_clone.clone();
                            let notebook_be = notebook_clone.clone();
                            let split_be = split_view_clone.clone();
                            let sidebar_be = sidebar_clone.clone();
                            let monitoring_be = monitoring_clone.clone();
                            let activity_be = activity_clone.clone();

                            crate::dialogs::show_backend_missing_dialog(
                                notebook_clone.widget(),
                                move |response| {
                                    match response {
                                        crate::dialogs::BackendMissingResponse::EnterManually => {
                                            // Proceed without credentials — protocol handler will prompt
                                            Self::handle_resolved_credentials(
                                                state_be.clone(),
                                                notebook_be.clone(),
                                                split_be.clone(),
                                                sidebar_be.clone(),
                                                monitoring_be.clone(),
                                                connection_id,
                                                protocol_type,
                                                None,
                                                None,
                                                activity_be.clone(),
                                            );
                                        }
                                        crate::dialogs::BackendMissingResponse::OpenSettings => {
                                            // Connection not started — open settings so user can configure backend
                                            tracing::info!("User chose to open settings to configure secret backend");
                                            if let Some(root) = notebook_be.widget().root()
                                                && let Some(win) = root.downcast_ref::<gtk4::Window>()
                                            {
                                                gtk4::prelude::WidgetExt::activate_action(
                                                    win,
                                                    "win.settings",
                                                    None,
                                                )
                                                .ok();
                                            }
                                        }
                                    }
                                },
                            );
                        }
                    }
                    CredentialResolutionResult::VaultEntryMissing { connection_name, lookup_key } => {
                        // Vault entry not found — show informational toast and proceed
                        // without credentials; protocol handler will prompt for password
                        tracing::info!(
                            %connection_name,
                            %lookup_key,
                            "Vault entry not found — user will be prompted for password"
                        );
                        if let Some(root) = notebook_clone.widget().root()
                            && let Some(window) = root.downcast_ref::<gtk4::Window>()
                        {
                            crate::toast::show_toast_on_window(
                                window,
                                &crate::i18n::i18n_f(
                                    "Vault entry not found for '{}'. You will be prompted for a password.",
                                    &[&connection_name],
                                ),
                                crate::toast::ToastType::Warning,
                            );
                        }
                        Self::handle_resolved_credentials(
                            state_clone,
                            notebook_clone,
                            split_view_clone,
                            sidebar_clone,
                            monitoring_clone,
                            connection_id,
                            protocol_type,
                            None,
                            None,
                            activity_clone,
                        );
                    }
                }
            });
        }
    }

    /// Handles resolved credentials and starts the appropriate connection
    ///
    /// This is called either immediately (if cached credentials exist) or
    /// from the async callback (after credential resolution completes).
    #[expect(
        clippy::too_many_arguments,
        reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
    )]
    fn handle_resolved_credentials(
        state: SharedAppState,
        notebook: SharedNotebook,
        split_view: SharedSplitView,
        sidebar: SharedSidebar,
        monitoring: types::SharedMonitoring,
        connection_id: Uuid,
        protocol_type: rustconn_core::ProtocolType,
        resolved_credentials: Option<rustconn_core::Credentials>,
        cached_credentials: Option<(String, zeroize::Zeroizing<String>, String)>,
        activity: Option<types::SharedActivityCoordinator>,
    ) {
        use rustconn_core::models::ProtocolType;

        match protocol_type {
            ProtocolType::Rdp => {
                Self::handle_rdp_credentials(
                    state,
                    notebook,
                    split_view,
                    sidebar,
                    connection_id,
                    resolved_credentials,
                    cached_credentials,
                );
            }
            ProtocolType::Vnc => {
                Self::handle_vnc_credentials(
                    state,
                    notebook,
                    split_view,
                    sidebar,
                    monitoring,
                    connection_id,
                    resolved_credentials,
                    cached_credentials,
                );
            }
            ProtocolType::Ssh
            | ProtocolType::Spice
            | ProtocolType::ZeroTrust
            | ProtocolType::Telnet
            | ProtocolType::Serial
            | ProtocolType::Kubernetes
            | ProtocolType::Mosh => {
                // For SSH/SPICE, cache credentials if available and start connection
                if let Some(ref creds) = resolved_credentials
                    && let (Some(username), Some(password)) =
                        (&creds.username, creds.expose_password())
                    && let Ok(mut state_mut) = state.try_borrow_mut()
                {
                    state_mut.cache_credentials(connection_id, username, password, "");
                }
                Self::start_connection_with_split(
                    &state,
                    &notebook,
                    &split_view,
                    &sidebar,
                    &monitoring,
                    connection_id,
                    activity.as_ref(),
                );
            }
            ProtocolType::Sftp => {
                // SFTP connections open the file manager directly
                Self::handle_sftp_connect(
                    &state,
                    &notebook,
                    Some(&sidebar),
                    Some(&split_view),
                    connection_id,
                );
            }
            ProtocolType::Web => {
                // Web connections open URL in default browser — no tab created
                // Cache credentials if available (for Copy Password context menu)
                if let Some(ref creds) = resolved_credentials
                    && let (Some(username), Some(password)) =
                        (&creds.username, creds.expose_password())
                    && let Ok(mut state_mut) = state.try_borrow_mut()
                {
                    state_mut.cache_credentials(connection_id, username, password, "");
                }
                Self::handle_web_connect(&state, &sidebar, connection_id);
            }
        }
    }

    /// Handles RDP credential resolution and connection start
    fn handle_rdp_credentials(
        state: SharedAppState,
        notebook: SharedNotebook,
        split_view: SharedSplitView,
        sidebar: SharedSidebar,
        connection_id: Uuid,
        resolved_credentials: Option<rustconn_core::Credentials>,
        cached_credentials: Option<(String, zeroize::Zeroizing<String>, String)>,
    ) {
        // Check if port check is needed BEFORE prompting for credentials
        let (should_check, host, port, timeout) = {
            let state_ref = state.borrow();
            let settings = state_ref.settings();
            let conn = state_ref.get_connection(connection_id);
            if let Some(conn) = conn {
                // Centralized probe-bypass logic handles jump host, RDP Gateway, etc.
                let should = conn.should_pre_connect_check(&settings.connection);
                if conn.bypasses_direct_probe() {
                    tracing::debug!(
                        protocol = "rdp",
                        host = %conn.host,
                        "Skipping port check — connection bypasses direct probe"
                    );
                }
                (
                    should,
                    conn.host.clone(),
                    conn.port,
                    settings.connection.port_check_timeout_secs,
                )
            } else {
                return;
            }
        };

        if should_check {
            // Run port check in background thread BEFORE showing password dialog
            let state_clone = state.clone();
            let notebook_clone = notebook.clone();
            let split_view_clone = split_view.clone();
            let sidebar_clone = sidebar.clone();

            crate::utils::spawn_blocking_with_callback(
                move || rustconn_core::check_port(&host, port, timeout),
                move |result| {
                    match result {
                        Ok(_) => {
                            // Port is open, proceed with credential handling
                            Self::handle_rdp_credentials_internal(
                                state_clone,
                                notebook_clone,
                                split_view_clone,
                                sidebar_clone,
                                connection_id,
                                resolved_credentials,
                                cached_credentials,
                            );
                        }
                        Err(e) => {
                            // Port check failed, show error with retry and update sidebar
                            tracing::warn!("Port check failed for RDP connection: {e}");
                            sidebar_clone
                                .update_connection_status(&connection_id.to_string(), "failed");
                            // Record the failed attempt in history (no session is
                            // created on a port-check failure).
                            if let Ok(mut state_mut) = state_clone.try_borrow_mut()
                                && let Some(conn) = state_mut.get_connection(connection_id).cloned()
                            {
                                let username = conn.username.clone();
                                state_mut.record_connection_attempt_failed(
                                    &conn,
                                    username.as_deref(),
                                    &e.to_string(),
                                );
                            }
                            if let Some(root) = notebook_clone.widget().root()
                                && let Some(window) = root.downcast_ref::<gtk4::Window>()
                            {
                                crate::toast::show_retry_toast_on_window(
                                    window,
                                    &crate::i18n::i18n("Connection failed. Host unreachable."),
                                    &connection_id.to_string(),
                                );
                            }
                        }
                    }
                },
            );
        } else {
            // Port check disabled, proceed directly
            Self::handle_rdp_credentials_internal(
                state,
                notebook,
                split_view,
                sidebar,
                connection_id,
                resolved_credentials,
                cached_credentials,
            );
        }
    }

    /// Internal RDP credential handling (after port check)
    fn handle_rdp_credentials_internal(
        state: SharedAppState,
        notebook: SharedNotebook,
        split_view: SharedSplitView,
        sidebar: SharedSidebar,
        connection_id: Uuid,
        resolved_credentials: Option<rustconn_core::Credentials>,
        cached_credentials: Option<(String, zeroize::Zeroizing<String>, String)>,
    ) {
        // Use resolved credentials if available
        if let Some(ref creds) = resolved_credentials
            && let (Some(username), Some(password)) = (&creds.username, creds.expose_password())
        {
            // The vault stores only username + password (domain is saved as
            // None — see save_password_to_vault), so the domain must come from
            // the connection config. Without it, NLA/CredSSP logs in as the
            // bare user instead of DOMAIN\user and the server rejects it with
            // STATUS_LOGON_FAILURE (0xC000006D). The manual/prompt path already
            // forwards conn.domain, so this keeps the vault path consistent.
            let domain = creds
                .domain
                .clone()
                .or_else(|| {
                    if let Ok(s) = state.try_borrow() {
                        s.get_connection(connection_id)
                            .and_then(|c| c.domain.clone())
                    } else {
                        tracing::warn!(
                            "Cannot borrow state for domain lookup; \
                             NLA may fail if the connection requires DOMAIN\\user"
                        );
                        None
                    }
                })
                .unwrap_or_default();
            Self::start_rdp_session_with_credentials(
                &state,
                &notebook,
                &split_view,
                &sidebar,
                connection_id,
                username,
                password,
                &domain,
            );
            return;
        }

        // Use cached credentials if available
        if let Some((username, password, domain)) = cached_credentials {
            Self::start_rdp_session_with_credentials(
                &state,
                &notebook,
                &split_view,
                &sidebar,
                connection_id,
                &username,
                &password,
                &domain,
            );
            return;
        }

        // Need to prompt for credentials
        // When password_source is None, try with empty password first —
        // the password dialog will be shown on retry if authentication fails.
        {
            let try_empty = state
                .try_borrow()
                .ok()
                .and_then(|s| s.get_connection(connection_id).cloned())
                .is_some_and(|c| c.password_source == rustconn_core::models::PasswordSource::None);

            if try_empty {
                let username = state
                    .try_borrow()
                    .ok()
                    .and_then(|s| {
                        s.get_connection(connection_id)
                            .and_then(|c| c.username.clone())
                    })
                    .unwrap_or_default();
                Self::start_rdp_session_with_credentials(
                    &state,
                    &notebook,
                    &split_view,
                    &sidebar,
                    connection_id,
                    &username,
                    "",
                    "",
                );
                return;
            }
        }

        if let Some(window) = notebook
            .widget()
            .ancestor(adw::ApplicationWindow::static_type())
            && let Some(app_window) = window.downcast_ref::<adw::ApplicationWindow>()
        {
            Self::start_rdp_with_password_dialog(
                state,
                notebook,
                split_view,
                sidebar,
                connection_id,
                app_window,
            );
        }
    }

    /// Handles VNC credential resolution and connection start
    #[expect(
        clippy::too_many_arguments,
        reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
    )]
    fn handle_vnc_credentials(
        state: SharedAppState,
        notebook: SharedNotebook,
        split_view: SharedSplitView,
        sidebar: SharedSidebar,
        monitoring: types::SharedMonitoring,
        connection_id: Uuid,
        resolved_credentials: Option<rustconn_core::Credentials>,
        cached_credentials: Option<(String, zeroize::Zeroizing<String>, String)>,
    ) {
        // Check if port check is needed BEFORE prompting for credentials
        let (should_check, host, port, timeout) = {
            let state_ref = state.borrow();
            let settings = state_ref.settings();
            let conn = state_ref.get_connection(connection_id);
            if let Some(conn) = conn {
                let should = conn.should_pre_connect_check(&settings.connection);
                (
                    should,
                    conn.host.clone(),
                    conn.port,
                    settings.connection.port_check_timeout_secs,
                )
            } else {
                return;
            }
        };

        if should_check {
            // Run port check in background thread BEFORE showing password dialog
            let state_clone = state.clone();
            let notebook_clone = notebook.clone();
            let split_view_clone = split_view.clone();
            let sidebar_clone = sidebar.clone();
            let monitoring_clone = monitoring.clone();

            crate::utils::spawn_blocking_with_callback(
                move || rustconn_core::check_port(&host, port, timeout),
                move |result| {
                    match result {
                        Ok(_) => {
                            // Port is open, proceed with credential handling
                            Self::handle_vnc_credentials_internal(
                                state_clone,
                                notebook_clone,
                                split_view_clone,
                                sidebar_clone,
                                monitoring_clone,
                                connection_id,
                                resolved_credentials,
                                cached_credentials,
                            );
                        }
                        Err(e) => {
                            // Port check failed, show error with retry and update sidebar
                            tracing::warn!("Port check failed for VNC connection: {e}");
                            sidebar_clone
                                .update_connection_status(&connection_id.to_string(), "failed");
                            // Record the failed attempt in history (no session is
                            // created on a port-check failure).
                            if let Ok(mut state_mut) = state_clone.try_borrow_mut()
                                && let Some(conn) = state_mut.get_connection(connection_id).cloned()
                            {
                                let username = conn.username.clone();
                                state_mut.record_connection_attempt_failed(
                                    &conn,
                                    username.as_deref(),
                                    &e.to_string(),
                                );
                            }
                            if let Some(root) = notebook_clone.widget().root()
                                && let Some(window) = root.downcast_ref::<gtk4::Window>()
                            {
                                crate::toast::show_retry_toast_on_window(
                                    window,
                                    &crate::i18n::i18n("Connection failed. Host unreachable."),
                                    &connection_id.to_string(),
                                );
                            }
                        }
                    }
                },
            );
        } else {
            // Port check disabled, proceed directly
            Self::handle_vnc_credentials_internal(
                state,
                notebook,
                split_view,
                sidebar,
                monitoring,
                connection_id,
                resolved_credentials,
                cached_credentials,
            );
        }
    }

    /// Internal VNC credential handling (after port check)
    #[expect(
        clippy::too_many_arguments,
        reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
    )]
    fn handle_vnc_credentials_internal(
        state: SharedAppState,
        notebook: SharedNotebook,
        split_view: SharedSplitView,
        sidebar: SharedSidebar,
        monitoring: types::SharedMonitoring,
        connection_id: Uuid,
        resolved_credentials: Option<rustconn_core::Credentials>,
        cached_credentials: Option<(String, zeroize::Zeroizing<String>, String)>,
    ) {
        // Use resolved credentials if available (VNC only needs password)
        if let Some(ref creds) = resolved_credentials
            && let Some(password) = creds.expose_password()
        {
            if let Ok(mut state_mut) = state.try_borrow_mut() {
                state_mut.cache_credentials(connection_id, "", password, "");
            }
            Self::start_connection_with_split(
                &state,
                &notebook,
                &split_view,
                &sidebar,
                &monitoring,
                connection_id,
                None,
            );
            return;
        }

        // Use cached credentials if available
        if cached_credentials.is_some() {
            Self::start_connection_with_split(
                &state,
                &notebook,
                &split_view,
                &sidebar,
                &monitoring,
                connection_id,
                None,
            );
            return;
        }

        // Need to prompt for VNC password
        // When password_source is None, try with empty password first —
        // many VNC servers don't require authentication.  The password
        // dialog will be shown on retry if the empty password fails.
        {
            let try_empty = state
                .try_borrow()
                .ok()
                .and_then(|s| s.get_connection(connection_id).cloned())
                .is_some_and(|c| c.password_source == rustconn_core::models::PasswordSource::None);

            if try_empty {
                // Use start_vnc_session_with_password (not start_connection_with_split)
                // because it handles SSH tunnel creation for jump host connections.
                rdp_vnc::start_vnc_session_with_password(
                    &state,
                    &notebook,
                    &split_view,
                    &sidebar,
                    connection_id,
                    "",
                );
                return;
            }
        }

        if let Some(window) = notebook
            .widget()
            .ancestor(adw::ApplicationWindow::static_type())
            && let Some(app_window) = window.downcast_ref::<adw::ApplicationWindow>()
        {
            Self::start_vnc_with_password_dialog(
                state,
                notebook,
                split_view,
                sidebar,
                connection_id,
                app_window,
            );
        }
    }

    /// Starts an RDP connection with password dialog
    fn start_rdp_with_password_dialog(
        state: SharedAppState,
        notebook: SharedNotebook,
        split_view: SharedSplitView,
        sidebar: SharedSidebar,
        connection_id: Uuid,
        window: &adw::ApplicationWindow,
    ) {
        rdp_vnc::start_rdp_with_password_dialog(
            state,
            notebook,
            split_view,
            sidebar,
            connection_id,
            window.upcast_ref(),
        );
    }

    /// Starts RDP session with provided credentials
    #[expect(
        clippy::too_many_arguments,
        reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
    )]
    fn start_rdp_session_with_credentials(
        state: &SharedAppState,
        notebook: &SharedNotebook,
        split_view: &SharedSplitView,
        sidebar: &SharedSidebar,
        connection_id: Uuid,
        username: &str,
        password: &str,
        domain: &str,
    ) {
        rdp_vnc::start_rdp_session_with_credentials(
            state,
            notebook,
            split_view,
            sidebar,
            connection_id,
            username,
            password,
            domain,
        );
    }

    /// Starts a VNC connection with password dialog
    fn start_vnc_with_password_dialog(
        state: SharedAppState,
        notebook: SharedNotebook,
        split_view: SharedSplitView,
        sidebar: SharedSidebar,
        connection_id: Uuid,
        window: &adw::ApplicationWindow,
    ) {
        rdp_vnc::start_vnc_with_password_dialog(
            state,
            notebook,
            split_view,
            sidebar,
            connection_id,
            window.upcast_ref(),
        );
    }
}
