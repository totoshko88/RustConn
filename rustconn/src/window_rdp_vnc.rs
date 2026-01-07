//! RDP and VNC connection methods for main window
//!
//! This module contains functions for starting RDP and VNC connections
//! with password dialogs and credential handling.

use crate::dialogs::PasswordDialog;
use crate::embedded::{EmbeddedSessionTab, RdpLauncher};
use crate::sidebar::ConnectionSidebar;
use crate::split_view::SplitTerminalView;
use crate::state::SharedAppState;
use crate::terminal::TerminalNotebook;
use gtk4::prelude::*;

use std::rc::Rc;
use uuid::Uuid;

/// Type alias for shared sidebar reference
pub type SharedSidebar = Rc<ConnectionSidebar>;

/// Type alias for shared notebook reference
pub type SharedNotebook = Rc<TerminalNotebook>;

/// Type alias for shared split view reference
pub type SharedSplitView = Rc<SplitTerminalView>;

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

    // Get connection info for dialog
    let (conn_name, username, domain) = {
        let state_ref = state.borrow();
        if let Some(conn) = state_ref.get_connection(connection_id) {
            (
                conn.name.clone(),
                conn.username.clone().unwrap_or_default(),
                conn.domain.clone().unwrap_or_default(),
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
            // Cache credentials if requested
            if creds.save_credentials {
                if let Ok(mut state_mut) = state.try_borrow_mut() {
                    state_mut.cache_credentials(
                        connection_id,
                        &creds.username,
                        &creds.password,
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
                &creds.password,
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
    use rustconn_core::models::RdpClientMode;

    let state_ref = state.borrow();

    let Some(conn) = state_ref.get_connection(connection_id) else {
        return;
    };

    let conn_name = conn.name.clone();
    let host = conn.host.clone();
    let port = conn.port;
    let window_mode = conn.window_mode;

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
        Some(state_mut.record_connection_start(&conn_for_history, Some(username)))
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
            username,
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
        username,
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

    // Create embedded RDP widget
    let embedded_widget = EmbeddedRdpWidget::new();

    // Calculate initial resolution from saved window geometry
    let state_ref = state.borrow();
    let settings = state_ref.settings();
    let content_width = settings
        .ui
        .window_width
        .unwrap_or(1200)
        .saturating_sub(settings.ui.sidebar_width.unwrap_or(250));
    let content_height = settings.ui.window_height.unwrap_or(800).saturating_sub(100);
    drop(state_ref);

    #[allow(clippy::cast_sign_loss)]
    let initial_resolution = if content_width > 100 && content_height > 100 {
        (content_width as u32, content_height as u32)
    } else {
        rdp_config
            .resolution
            .as_ref()
            .map_or((1920, 1080), |r| (r.width, r.height))
    };

    let mut embedded_config = EmbeddedRdpConfig::new(host)
        .with_port(port)
        .with_resolution(initial_resolution.0, initial_resolution.1)
        .with_clipboard(true);

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

    // Wrap in Rc to keep widget alive in notebook
    let embedded_widget = Rc::new(embedded_widget);

    // Connect using embedded widget
    if let Err(e) = embedded_widget.connect(&embedded_config) {
        eprintln!("RDP connection failed for '{}': {}", conn_name, e);
        sidebar.update_connection_status(&connection_id.to_string(), "failed");
    } else {
        sidebar.update_connection_status(&connection_id.to_string(), "connecting");
    }

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
            if let Some(info) = notebook_for_state.get_session_info(session_id) {
                if let Some(entry_id) = info.history_entry_id {
                    if let Ok(mut state_mut) = state_for_callback.try_borrow_mut() {
                        state_mut.record_connection_end(entry_id);
                    }
                }
            }
        }
        crate::embedded_rdp::RdpConnectionState::Connected => {
            notebook_for_state.mark_tab_connected(session_id);
            sidebar_for_state.increment_session_count(&connection_id.to_string());
        }
        crate::embedded_rdp::RdpConnectionState::Error => {
            // Record connection failure in history
            if let Some(info) = notebook_for_state.get_session_info(session_id) {
                if let Some(entry_id) = info.history_entry_id {
                    if let Ok(mut state_mut) = state_for_callback.try_borrow_mut() {
                        state_mut.record_connection_failed(entry_id, "RDP connection error");
                    }
                }
            }
        }
        crate::embedded_rdp::RdpConnectionState::Connecting => {}
    });

    // Connect reconnect callback
    let widget_for_reconnect = embedded_widget.clone();
    embedded_widget.connect_reconnect(move || {
        if let Err(e) = widget_for_reconnect.reconnect() {
            eprintln!("RDP reconnect failed: {}", e);
        }
    });

    notebook.add_embedded_rdp_tab(session_id, connection_id, conn_name, embedded_widget);

    // Store history entry ID in session for later use
    if let Some(entry_id) = history_entry_id {
        notebook.set_history_entry_id(session_id, entry_id);
    }

    // Show notebook for RDP session tab
    split_view.widget().set_visible(false);
    split_view.widget().set_vexpand(false);
    notebook.widget().set_vexpand(true);
    notebook.notebook().set_vexpand(true);

    // If Fullscreen mode, maximize the window
    if matches!(window_mode, rustconn_core::models::WindowMode::Fullscreen) {
        if let Some(window) = notebook
            .widget()
            .ancestor(gtk4::ApplicationWindow::static_type())
        {
            if let Some(app_window) = window.downcast_ref::<gtk4::ApplicationWindow>() {
                app_window.maximize();
            }
        }
    }

    // Update last_connected timestamp
    if let Ok(mut state_mut) = state.try_borrow_mut() {
        let _ = state_mut.update_last_connected(connection_id);
    }
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
        eprintln!("Failed to start RDP session '{}': {}", conn_name, e);
        sidebar.update_connection_status(&connection_id.to_string(), "failed");
        // Record connection failure in history
        if let Some(entry_id) = history_entry_id {
            if let Ok(mut state_mut) = state.try_borrow_mut() {
                state_mut.record_connection_failed(entry_id, &e.to_string());
            }
        }
        true
    } else {
        sidebar.increment_session_count(&connection_id.to_string());
        // Record connection end when external process exits (we can't track this easily)
        // For external sessions, we record end immediately as we don't have state tracking
        if let Some(entry_id) = history_entry_id {
            if let Ok(mut state_mut) = state.try_borrow_mut() {
                state_mut.record_connection_end(entry_id);
            }
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
    if let Ok(mut state_mut) = state.try_borrow_mut() {
        let _ = state_mut.update_last_connected(connection_id);
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
    let (conn_name, conn_host) = {
        let state_ref = state.borrow();
        if let Some(conn) = state_ref.get_connection(connection_id) {
            (conn.name.clone(), conn.host.clone())
        } else {
            return;
        }
    };

    // Create and show password dialog
    let dialog = PasswordDialog::new(Some(window));
    dialog.set_connection_name(&conn_name);

    // Try to load password from KeePass asynchronously
    {
        use gtk4::glib;
        use secrecy::ExposeSecret;
        let state_ref = state.borrow();
        let settings = state_ref.settings();

        if settings.secrets.kdbx_enabled {
            if let Some(kdbx_path) = settings.secrets.kdbx_path.clone() {
                let db_password = settings
                    .secrets
                    .kdbx_password
                    .as_ref()
                    .map(|p| p.expose_secret().to_string());
                let key_file = settings.secrets.kdbx_key_file.clone();

                // Build lookup key with protocol for uniqueness
                // Format: "name (vnc)" or "host (vnc)" if name is empty
                let base_name = if conn_name.trim().is_empty() {
                    conn_host.clone()
                } else {
                    conn_name.clone()
                };
                let lookup_key = format!("{base_name} (vnc)");

                // Get password entry for async callback
                let password_entry = dialog.password_entry().clone();

                // Drop state borrow before spawning
                drop(state_ref);

                // Run KeePass operation in background thread to avoid blocking UI
                let (tx, rx) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let result =
                        rustconn_core::secret::KeePassStatus::get_password_from_kdbx_with_key(
                            &kdbx_path,
                            db_password.as_deref(),
                            key_file.as_deref(),
                            &lookup_key,
                            None, // Protocol already included in lookup_key
                        );
                    let _ = tx.send(result);
                });

                // Poll for result using idle callback
                glib::idle_add_local_once(move || {
                    fn check_result(
                        rx: std::sync::mpsc::Receiver<Result<Option<String>, String>>,
                        password_entry: gtk4::Entry,
                    ) {
                        match rx.try_recv() {
                            Ok(Ok(Some(password))) => {
                                password_entry.set_text(&password);
                            }
                            Ok(Ok(None) | Err(_)) => {
                                // No password found or error - just continue without pre-fill
                            }
                            Err(std::sync::mpsc::TryRecvError::Empty) => {
                                // Not ready yet, schedule another check
                                glib::timeout_add_local_once(
                                    std::time::Duration::from_millis(50),
                                    move || check_result(rx, password_entry),
                                );
                            }
                            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                                // Thread error - just continue without pre-fill
                            }
                        }
                    }
                    check_result(rx, password_entry);
                });
            }
        }
    }

    let sidebar_clone = sidebar.clone();
    dialog.show(move |result| {
        if let Some(creds) = result {
            // Cache credentials if requested
            if creds.save_credentials {
                if let Ok(mut state_mut) = state.try_borrow_mut() {
                    state_mut.cache_credentials(connection_id, "", &creds.password, "");
                }
            }

            // Start VNC with password
            start_vnc_session_with_password(
                &state,
                &notebook,
                &split_view,
                &sidebar_clone,
                connection_id,
                &creds.password,
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
    let state_ref = state.borrow();

    let Some(conn) = state_ref.get_connection(connection_id) else {
        return;
    };

    let conn_name = conn.name.clone();
    let host = conn.host.clone();
    let port = conn.port;

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
                if let Some(info) = notebook_for_state.get_session_info(session_id) {
                    if let Some(entry_id) = info.history_entry_id {
                        if let Ok(mut state_mut) = state_for_callback.try_borrow_mut() {
                            state_mut.record_connection_end(entry_id);
                        }
                    }
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
                eprintln!("VNC reconnect failed: {}", e);
            }
        });

        // Initiate connection with VNC config
        if let Err(e) = vnc_widget.connect_with_config(&host, port, Some(password), &vnc_config) {
            eprintln!("Failed to connect VNC session '{}': {}", conn_name, e);
            sidebar.update_connection_status(&connection_id.to_string(), "failed");
        } else {
            sidebar.update_connection_status(&connection_id.to_string(), "connecting");
        }
    }

    // VNC displays in notebook tab - hide split view and expand notebook
    split_view.widget().set_visible(false);
    split_view.widget().set_vexpand(false);
    notebook.widget().set_vexpand(true);
    notebook.notebook().set_vexpand(true);

    // Update last_connected timestamp
    if let Ok(mut state_mut) = state.try_borrow_mut() {
        let _ = state_mut.update_last_connected(connection_id);
    }
}
