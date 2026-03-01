//! RDP and VNC connection methods for main window
//!
//! This module contains functions for starting RDP and VNC connections
//! with password dialogs and credential handling.

use crate::dialogs::PasswordDialog;
use crate::embedded::{EmbeddedSessionTab, RdpLauncher};
use crate::sidebar::ConnectionSidebar;
use crate::split_view::SplitViewBridge;
use crate::state::SharedAppState;
use crate::terminal::TerminalNotebook;
use gtk4::prelude::*;
use rustconn_core::models::PasswordSource;
use secrecy::ExposeSecret;

use std::rc::Rc;
use uuid::Uuid;

/// Type alias for shared sidebar reference
pub type SharedSidebar = Rc<ConnectionSidebar>;

/// Type alias for shared notebook reference
pub type SharedNotebook = Rc<TerminalNotebook>;

/// Type alias for shared split view reference
pub type SharedSplitView = Rc<SplitViewBridge>;

/// Starts an RDP connection with password dialog
#[allow(clippy::too_many_arguments)]
pub fn start_rdp_with_password_dialog(
    state: SharedAppState,
    notebook: SharedNotebook,
    split_view: SharedSplitView,
    sidebar: SharedSidebar,
    connection_id: Uuid,
    window: &gtk4::Window,
) {
    use rustconn_core::variables::{VariableManager, VariableScope};

    // Helper function to substitute variables
    let substitute_vars = |input: &str, global_variables: &[rustconn_core::Variable]| -> String {
        if !input.contains("${") {
            return input.to_string();
        }
        let mut manager = VariableManager::new();
        for var in global_variables {
            manager.set_global(var.clone());
        }
        manager
            .substitute_for_command(input, VariableScope::Global)
            .unwrap_or_else(|_| input.to_string())
    };

    // Check if we have cached credentials (fast, non-blocking)
    let cached = {
        let state_ref = state.borrow();
        state_ref.get_cached_credentials(connection_id).map(|c| {
            use secrecy::ExposeSecret;
            (
                c.username.clone(),
                c.password.expose_secret().to_string(),
                c.domain.clone(),
            )
        })
    };

    if let Some((username, password, domain)) = cached {
        // Use cached credentials directly
        start_rdp_session_with_credentials(
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

    // Get connection info for dialog with variable substitution
    let (conn_name, username, domain) = {
        let state_ref = state.borrow();
        if let Some(conn) = state_ref.get_connection(connection_id) {
            let global_variables = state_ref.settings().global_variables.clone();
            let raw_username = conn.username.clone().unwrap_or_default();
            let raw_domain = conn.domain.clone().unwrap_or_default();
            (
                conn.name.clone(),
                substitute_vars(&raw_username, &global_variables),
                substitute_vars(&raw_domain, &global_variables),
            )
        } else {
            return;
        }
    };

    // Create and show password dialog
    let dialog = PasswordDialog::new(Some(window));
    dialog.set_connection_name(&conn_name);
    dialog.set_username(&username);
    dialog.set_domain(&domain);

    let sidebar_clone = sidebar.clone();
    dialog.show(move |result| {
        if let Some(creds) = result {
            // Determine if we should save: explicit request OR password_source == Vault
            let should_save = creds.save_credentials || {
                let state_ref = state.borrow();
                state_ref
                    .get_connection(connection_id)
                    .map(|c| c.password_source == PasswordSource::Vault)
                    .unwrap_or(false)
            };

            if should_save {
                // Get connection details for vault save
                let conn_host = {
                    let state_ref = state.borrow();
                    state_ref
                        .get_connection(connection_id)
                        .map(|c| c.host.clone())
                        .unwrap_or_default()
                };

                if let Ok(state_ref) = state.try_borrow() {
                    let settings = state_ref.settings().clone();
                    let groups: Vec<_> = state_ref.list_groups().into_iter().cloned().collect();
                    let conn = state_ref.get_connection(connection_id);
                    let protocol = rustconn_core::models::ProtocolType::Rdp;

                    crate::state::save_password_to_vault(
                        &settings,
                        &groups,
                        conn,
                        &conn_name,
                        &conn_host,
                        protocol,
                        &creds.username,
                        creds.password.expose_secret(),
                        connection_id,
                    );
                }

                // Also cache for immediate use
                if let Ok(mut state_mut) = state.try_borrow_mut() {
                    state_mut.cache_credentials(
                        connection_id,
                        &creds.username,
                        creds.password.expose_secret(),
                        &creds.domain,
                    );
                }
            }

            // Start RDP with credentials
            start_rdp_session_with_credentials(
                &state,
                &notebook,
                &split_view,
                &sidebar_clone,
                connection_id,
                &creds.username,
                creds.password.expose_secret(),
                &creds.domain,
            );
        }
    });
}

/// Starts RDP session with provided credentials
#[allow(clippy::too_many_arguments)]
pub fn start_rdp_session_with_credentials(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    split_view: &SharedSplitView,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    username: &str,
    password: &str,
    domain: &str,
) {
    // Port check is now done earlier in handle_rdp_credentials
    start_rdp_session_internal(
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

/// Internal function to start RDP session (after port check)
#[allow(clippy::too_many_arguments)]
fn start_rdp_session_internal(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    split_view: &SharedSplitView,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    username: &str,
    password: &str,
    domain: &str,
) {
    use rustconn_core::models::RdpClientMode;
    use rustconn_core::variables::{VariableManager, VariableScope};

    let state_ref = state.borrow();

    let Some(conn) = state_ref.get_connection(connection_id) else {
        return;
    };

    let conn_name = conn.name.clone();
    let port = conn.port;
    let window_mode = conn.window_mode;

    // Get global variables for substitution
    let global_variables = state_ref.settings().global_variables.clone();

    // Helper function to substitute variables
    let substitute = |input: &str| -> String {
        if !input.contains("${") {
            return input.to_string();
        }
        let mut manager = VariableManager::new();
        for var in &global_variables {
            manager.set_global(var.clone());
        }
        manager
            .substitute_for_command(input, VariableScope::Global)
            .unwrap_or_else(|_| input.to_string())
    };

    // Apply variable substitution to host and username
    let host = substitute(&conn.host);
    let username = substitute(username);

    // Get RDP-specific options
    let rdp_config = if let rustconn_core::ProtocolConfig::Rdp(config) = &conn.protocol_config {
        config.clone()
    } else {
        rustconn_core::models::RdpConfig::default()
    };

    // Clone connection for history recording
    let conn_for_history = conn.clone();

    drop(state_ref);

    // Record connection start in history
    let history_entry_id = if let Ok(mut state_mut) = state.try_borrow_mut() {
        Some(state_mut.record_connection_start(&conn_for_history, Some(&username)))
    } else {
        None
    };

    // Check client mode - if Embedded, use EmbeddedRdpWidget with fallback to external
    if rdp_config.client_mode == RdpClientMode::Embedded {
        start_embedded_rdp_session(
            state,
            notebook,
            split_view,
            sidebar,
            connection_id,
            &conn_name,
            &host,
            port,
            &username,
            password,
            domain,
            window_mode,
            &rdp_config,
            history_entry_id,
        );
        return;
    }

    // External mode - use xfreerdp in external window
    start_external_rdp_session(
        state,
        notebook,
        split_view,
        sidebar,
        connection_id,
        &conn_name,
        &host,
        port,
        &username,
        password,
        domain,
        &rdp_config,
        history_entry_id,
    );
}

/// Starts embedded RDP session
#[allow(clippy::too_many_arguments)]
fn start_embedded_rdp_session(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    split_view: &SharedSplitView,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    conn_name: &str,
    host: &str,
    port: u16,
    username: &str,
    password: &str,
    domain: &str,
    window_mode: rustconn_core::models::WindowMode,
    rdp_config: &rustconn_core::models::RdpConfig,
    history_entry_id: Option<Uuid>,
) {
    use crate::embedded_rdp::{EmbeddedRdpWidget, RdpConfig as EmbeddedRdpConfig};
    use gtk4::glib;

    // Create embedded RDP widget
    let embedded_widget = EmbeddedRdpWidget::new();

    // We'll connect after the widget is realized to get actual size
    // For now, create config with placeholder resolution
    let mut embedded_config = EmbeddedRdpConfig::new(host)
        .with_port(port)
        .with_resolution(1280, 720) // Placeholder, will be updated
        .with_clipboard(true)
        .with_performance_mode(rdp_config.performance_mode);

    if !username.is_empty() {
        embedded_config = embedded_config.with_username(username);
    }
    if !password.is_empty() {
        embedded_config = embedded_config.with_password(password);
    }
    if !domain.is_empty() {
        embedded_config = embedded_config.with_domain(domain);
    }

    // Add extra args
    if !rdp_config.custom_args.is_empty() {
        embedded_config = embedded_config.with_extra_args(rdp_config.custom_args.clone());
    }

    // Add shared folders for drive redirection
    if !rdp_config.shared_folders.is_empty() {
        use crate::embedded_rdp::EmbeddedSharedFolder;
        let folders: Vec<EmbeddedSharedFolder> = rdp_config
            .shared_folders
            .iter()
            .map(|f| EmbeddedSharedFolder {
                local_path: f.local_path.clone(),
                share_name: f.share_name.clone(),
            })
            .collect();
        embedded_config = embedded_config.with_shared_folders(folders);
    }

    // Pass keyboard layout override if configured
    embedded_config.keyboard_layout = rdp_config.keyboard_layout;

    // Pass scale override for HiDPI support
    embedded_config.scale_override = rdp_config.scale_override;

    // Wrap in Rc to keep widget alive in notebook
    let embedded_widget = Rc::new(embedded_widget);

    let session_id = Uuid::new_v4();

    // Connect state change callback
    let notebook_for_state = notebook.clone();
    let sidebar_for_state = sidebar.clone();
    let state_for_callback = state.clone();
    embedded_widget.connect_state_changed(move |rdp_state| match rdp_state {
        crate::embedded_rdp::RdpConnectionState::Disconnected => {
            notebook_for_state.mark_tab_disconnected(session_id);
            sidebar_for_state.decrement_session_count(
                &connection_id.to_string(),
                notebook_for_state.get_session_info(session_id).is_some(),
            );
            // Record connection end in history
            if let Some(info) = notebook_for_state.get_session_info(session_id)
                && let Some(entry_id) = info.history_entry_id
                && let Ok(mut state_mut) = state_for_callback.try_borrow_mut()
            {
                state_mut.record_connection_end(entry_id);
            }
        }
        crate::embedded_rdp::RdpConnectionState::Connected => {
            notebook_for_state.mark_tab_connected(session_id);
            sidebar_for_state.increment_session_count(&connection_id.to_string());
        }
        crate::embedded_rdp::RdpConnectionState::Error => {
            // Record connection failure in history
            if let Some(info) = notebook_for_state.get_session_info(session_id)
                && let Some(entry_id) = info.history_entry_id
                && let Ok(mut state_mut) = state_for_callback.try_borrow_mut()
            {
                state_mut.record_connection_failed(entry_id, "RDP connection error");
            }
        }
        crate::embedded_rdp::RdpConnectionState::Connecting => {}
    });

    // Connect reconnect callback
    let widget_for_reconnect = embedded_widget.clone();
    embedded_widget.connect_reconnect(move || {
        if let Err(e) = widget_for_reconnect.reconnect() {
            tracing::error!(%e, "RDP reconnect failed");
        }
    });

    // Connect fallback callback — shows toast when IronRDP falls back to FreeRDP
    // (e.g. xrdp protocol incompatibility — IronRDP issue #139)
    let notebook_for_fallback = notebook.clone();
    embedded_widget.connect_fallback(move |reason| {
        tracing::warn!(protocol = "rdp", reason = %reason, "RDP fallback triggered");
        if let Some(window) = notebook_for_fallback
            .widget()
            .ancestor(gtk4::Window::static_type())
            .and_then(|w| w.downcast::<gtk4::Window>().ok())
        {
            crate::toast::show_toast_on_window(&window, reason, crate::toast::ToastType::Warning);
        }
    });

    // Add tab first, then connect after widget is realized
    notebook.add_embedded_rdp_tab(
        session_id,
        connection_id,
        conn_name,
        embedded_widget.clone(),
    );

    // Store history entry ID in session for later use
    if let Some(entry_id) = history_entry_id {
        notebook.set_history_entry_id(session_id, entry_id);
    }

    // Show notebook for RDP session tab
    split_view.widget().set_visible(false);
    split_view.widget().set_vexpand(false);
    notebook.widget().set_vexpand(true);
    notebook.show_tab_view_content();

    // If Fullscreen mode, maximize the window
    if matches!(window_mode, rustconn_core::models::WindowMode::Fullscreen)
        && let Some(window) = notebook
            .widget()
            .ancestor(gtk4::ApplicationWindow::static_type())
        && let Some(app_window) = window.downcast_ref::<gtk4::ApplicationWindow>()
    {
        app_window.maximize();
    }

    // Update last_connected timestamp
    if let Ok(mut state_mut) = state.try_borrow_mut()
        && let Err(e) = state_mut.update_last_connected(connection_id)
    {
        tracing::warn!(?e, "Failed to update last_connected");
    }

    // Connect after a short delay to let GTK layout the widget
    // This ensures we get the actual widget size for RDP resolution
    let widget_for_connect = embedded_widget.clone();
    let sidebar_for_connect = sidebar.clone();
    let conn_name_owned = conn_name.to_string();
    glib::timeout_add_local_once(std::time::Duration::from_millis(100), move || {
        // Get actual widget size from drawing area
        let drawing_area = widget_for_connect.drawing_area();
        let raw_width = drawing_area.width().unsigned_abs();
        let raw_height = drawing_area.height().unsigned_abs();

        // Round down to multiple of 4 for RDP compatibility
        // Many RDP servers and codecs require dimensions divisible by 4
        let actual_width = ((raw_width / 4) * 4).max(640);
        let actual_height = ((raw_height / 4) * 4).max(480);

        tracing::info!(
            "[RDP Init] Actual widget size after layout: {}x{} (raw: {}x{})",
            actual_width,
            actual_height,
            raw_width,
            raw_height
        );

        // Update config with actual resolution
        let final_config = embedded_config.with_resolution(actual_width, actual_height);

        // Now connect with correct resolution
        if let Err(e) = widget_for_connect.connect(&final_config) {
            tracing::error!(%e, connection = %conn_name_owned, "RDP connection failed");
            sidebar_for_connect.update_connection_status(&connection_id.to_string(), "failed");
        } else {
            sidebar_for_connect.update_connection_status(&connection_id.to_string(), "connecting");
        }
    });
}

/// Starts external RDP session using xfreerdp
#[allow(clippy::too_many_arguments)]
fn start_external_rdp_session(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    split_view: &SharedSplitView,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    conn_name: &str,
    host: &str,
    port: u16,
    username: &str,
    password: &str,
    domain: &str,
    rdp_config: &rustconn_core::models::RdpConfig,
    history_entry_id: Option<Uuid>,
) {
    let (tab, _is_embedded) = EmbeddedSessionTab::new(connection_id, conn_name, "rdp");
    let session_id = tab.id();

    // Get resolution from RDP config
    let resolution = rdp_config.resolution.as_ref().map(|r| (r.width, r.height));

    // Get extra args from RDP config
    let extra_args = rdp_config.custom_args.clone();

    // Prepare domain (use None if empty)
    let domain_opt = if domain.is_empty() {
        None
    } else {
        Some(domain)
    };

    // Convert shared folders
    let shared_folders: Vec<(String, std::path::PathBuf)> = rdp_config
        .shared_folders
        .iter()
        .map(|f| (f.share_name.clone(), f.local_path.clone()))
        .collect();

    // Start RDP connection using xfreerdp
    let connection_failed = if let Err(e) = RdpLauncher::start_with_geometry(
        &tab,
        host,
        port,
        Some(username),
        Some(password),
        domain_opt,
        resolution,
        &extra_args,
        None,
        false,
        &shared_folders,
    ) {
        tracing::error!(%e, connection = %conn_name, "Failed to start RDP session");
        sidebar.update_connection_status(&connection_id.to_string(), "failed");
        // Record connection failure in history
        if let Some(entry_id) = history_entry_id
            && let Ok(mut state_mut) = state.try_borrow_mut()
        {
            state_mut.record_connection_failed(entry_id, &e.to_string());
        }
        true
    } else {
        sidebar.increment_session_count(&connection_id.to_string());
        // Record connection end when external process exits (we can't track this easily)
        // For external sessions, we record end immediately as we don't have state tracking
        if let Some(entry_id) = history_entry_id
            && let Ok(mut state_mut) = state.try_borrow_mut()
        {
            state_mut.record_connection_end(entry_id);
        }
        false
    };

    if connection_failed {
        return;
    }

    // Add tab widget to notebook
    notebook.add_embedded_session_tab(session_id, conn_name, "rdp", tab.widget());

    // Add to split_view
    if let Some(info) = notebook.get_session_info(session_id) {
        split_view.add_session(info, None);
    }

    // Update last_connected
    if let Ok(mut state_mut) = state.try_borrow_mut()
        && let Err(e) = state_mut.update_last_connected(connection_id)
    {
        tracing::warn!(?e, "Failed to update last_connected");
    }
}

/// Starts a VNC connection with password dialog
#[allow(clippy::too_many_arguments)]
pub fn start_vnc_with_password_dialog(
    state: SharedAppState,
    notebook: SharedNotebook,
    split_view: SharedSplitView,
    sidebar: SharedSidebar,
    connection_id: Uuid,
    window: &gtk4::Window,
) {
    // Check if we have cached credentials (fast, non-blocking)
    let cached_password = {
        let state_ref = state.borrow();
        state_ref.get_cached_credentials(connection_id).map(|c| {
            use secrecy::ExposeSecret;
            c.password.expose_secret().to_string()
        })
    };

    if let Some(password) = cached_password {
        // Use cached credentials directly
        start_vnc_session_with_password(
            &state,
            &notebook,
            &split_view,
            &sidebar,
            connection_id,
            &password,
        );
        return;
    }

    // Get connection info for dialog
    let (conn_name, lookup_key) = {
        let state_ref = state.borrow();
        if let Some(conn) = state_ref.get_connection(connection_id) {
            // Build hierarchical entry path using KeePassHierarchy
            let groups: Vec<rustconn_core::models::ConnectionGroup> =
                state_ref.list_groups().iter().cloned().cloned().collect();
            let entry_path =
                rustconn_core::secret::KeePassHierarchy::build_entry_path(conn, &groups);

            // Strip RustConn/ prefix since get_password_from_kdbx_with_key adds it back
            let entry_name = entry_path.strip_prefix("RustConn/").unwrap_or(&entry_path);
            let key = format!("{entry_name} (vnc)");

            (conn.name.clone(), key)
        } else {
            return;
        }
    };

    // Create and show password dialog
    let dialog = PasswordDialog::new(Some(window));
    dialog.set_connection_name(&conn_name);

    // Try to load password from KeePass asynchronously
    {
        use crate::utils::spawn_blocking_with_callback;
        let state_ref = state.borrow();
        let settings = state_ref.settings();

        if settings.secrets.kdbx_enabled
            && matches!(
                settings.secrets.preferred_backend,
                rustconn_core::config::SecretBackendType::KeePassXc
                    | rustconn_core::config::SecretBackendType::KdbxFile
            )
            && let Some(kdbx_path) = settings.secrets.kdbx_path.clone()
        {
            let db_password = settings.secrets.kdbx_password.clone();
            let key_file = settings.secrets.kdbx_key_file.clone();

            // Use pre-built lookup key with hierarchical path
            let lookup_key_clone = lookup_key.clone();

            // Get password entry for async callback
            let password_entry = dialog.password_entry().clone();

            // Drop state borrow before spawning
            drop(state_ref);

            // Run KeePass operation in background thread using utility function
            spawn_blocking_with_callback(
                move || {
                    rustconn_core::secret::KeePassStatus::get_password_from_kdbx_with_key(
                        &kdbx_path,
                        db_password.as_ref(),
                        key_file.as_deref(),
                        &lookup_key_clone,
                        None, // Protocol already included in lookup_key
                    )
                },
                move |result: rustconn_core::error::SecretResult<Option<secrecy::SecretString>>| {
                    if let Ok(Some(password)) = result {
                        use secrecy::ExposeSecret;
                        password_entry.set_text(password.expose_secret());
                    }
                    // Silently ignore errors - just continue without pre-fill
                },
            );
        }
    }

    let sidebar_clone = sidebar.clone();
    dialog.show(move |result| {
        if let Some(creds) = result {
            // Determine if we should save: explicit request OR password_source == Vault
            let should_save = creds.save_credentials || {
                let state_ref = state.borrow();
                state_ref
                    .get_connection(connection_id)
                    .map(|c| c.password_source == PasswordSource::Vault)
                    .unwrap_or(false)
            };

            if should_save {
                // Get connection details for vault save
                let conn_host = {
                    let state_ref = state.borrow();
                    state_ref
                        .get_connection(connection_id)
                        .map(|c| c.host.clone())
                        .unwrap_or_default()
                };

                if let Ok(state_ref) = state.try_borrow() {
                    let settings = state_ref.settings().clone();
                    let groups: Vec<_> = state_ref.list_groups().into_iter().cloned().collect();
                    let conn = state_ref.get_connection(connection_id);
                    let protocol = rustconn_core::models::ProtocolType::Vnc;

                    crate::state::save_password_to_vault(
                        &settings,
                        &groups,
                        conn,
                        &conn_name,
                        &conn_host,
                        protocol,
                        "", // VNC doesn't use username
                        creds.password.expose_secret(),
                        connection_id,
                    );
                }

                // Also cache for immediate use
                if let Ok(mut state_mut) = state.try_borrow_mut() {
                    state_mut.cache_credentials(
                        connection_id,
                        "",
                        creds.password.expose_secret(),
                        "",
                    );
                }
            }

            // Start VNC with password
            start_vnc_session_with_password(
                &state,
                &notebook,
                &split_view,
                &sidebar_clone,
                connection_id,
                creds.password.expose_secret(),
            );
        }
    });
}

/// Starts VNC session with provided password
pub fn start_vnc_session_with_password(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    split_view: &SharedSplitView,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    password: &str,
) {
    // Port check is now done earlier in handle_vnc_credentials
    start_vnc_session_internal(
        state,
        notebook,
        split_view,
        sidebar,
        connection_id,
        password,
    );
}

/// Internal function to start VNC session (after port check)
fn start_vnc_session_internal(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    split_view: &SharedSplitView,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    password: &str,
) {
    use rustconn_core::variables::{VariableManager, VariableScope};

    let state_ref = state.borrow();

    let Some(conn) = state_ref.get_connection(connection_id) else {
        return;
    };

    let conn_name = conn.name.clone();
    let port = conn.port;

    // Get global variables for substitution
    let global_variables = state_ref.settings().global_variables.clone();

    // Apply variable substitution to host
    let host = if conn.host.contains("${") {
        let mut manager = VariableManager::new();
        for var in &global_variables {
            manager.set_global(var.clone());
        }
        manager
            .substitute_for_command(&conn.host, VariableScope::Global)
            .unwrap_or_else(|_| conn.host.clone())
    } else {
        conn.host.clone()
    };

    // Get VNC-specific configuration
    let vnc_config = if let rustconn_core::ProtocolConfig::Vnc(config) = &conn.protocol_config {
        config.clone()
    } else {
        rustconn_core::models::VncConfig::default()
    };

    // Clone connection for history recording
    let conn_for_history = conn.clone();

    drop(state_ref);

    // Record connection start in history
    let history_entry_id = if let Ok(mut state_mut) = state.try_borrow_mut() {
        Some(
            state_mut
                .record_connection_start(&conn_for_history, conn_for_history.username.as_deref()),
        )
    } else {
        None
    };

    // Create VNC session tab with native widget
    let session_id = notebook.create_vnc_session_tab(connection_id, &conn_name);

    // Store history entry ID in session for later use
    if let Some(entry_id) = history_entry_id {
        notebook.set_history_entry_id(session_id, entry_id);
    }

    // Get the VNC widget and initiate connection with config
    if let Some(vnc_widget) = notebook.get_vnc_widget(session_id) {
        // Connect state change callback
        let notebook_for_state = notebook.clone();
        let sidebar_for_state = sidebar.clone();
        let state_for_callback = state.clone();
        vnc_widget.connect_state_changed(move |vnc_state| {
            if vnc_state == crate::session::SessionState::Disconnected {
                notebook_for_state.mark_tab_disconnected(session_id);
                sidebar_for_state.decrement_session_count(&connection_id.to_string(), false);
                // Record connection end in history
                if let Some(info) = notebook_for_state.get_session_info(session_id)
                    && let Some(entry_id) = info.history_entry_id
                    && let Ok(mut state_mut) = state_for_callback.try_borrow_mut()
                {
                    state_mut.record_connection_end(entry_id);
                }
            } else if vnc_state == crate::session::SessionState::Connected {
                notebook_for_state.mark_tab_connected(session_id);
                sidebar_for_state.increment_session_count(&connection_id.to_string());
            }
        });

        // Connect reconnect callback
        let widget_for_reconnect = vnc_widget.clone();
        vnc_widget.connect_reconnect(move || {
            if let Err(e) = widget_for_reconnect.reconnect() {
                tracing::error!(%e, "VNC reconnect failed");
            }
        });

        // Initiate connection with VNC config
        if let Err(e) = vnc_widget.connect_with_config(&host, port, Some(password), &vnc_config) {
            tracing::error!(%e, connection = %conn_name, "Failed to connect VNC session");
            sidebar.update_connection_status(&connection_id.to_string(), "failed");
        } else {
            sidebar.update_connection_status(&connection_id.to_string(), "connecting");
        }
    }

    // VNC displays in notebook tab - hide split view and expand notebook
    split_view.widget().set_visible(false);
    split_view.widget().set_vexpand(false);
    notebook.widget().set_vexpand(true);
    notebook.show_tab_view_content();

    // Update last_connected timestamp
    if let Ok(mut state_mut) = state.try_borrow_mut()
        && let Err(e) = state_mut.update_last_connected(connection_id)
    {
        tracing::warn!(?e, "Failed to update last_connected");
    }
}
