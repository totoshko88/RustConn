//! Connection and group creation dialogs for main window
//!
//! This module contains dialog functions for creating new connections and groups,
//! including template picker and parent group selection.

use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use rustconn_core::models::PasswordSource;
use uuid::Uuid;

use super::MainWindow;
use crate::alert;
use crate::dialogs::{ConnectionDialog, ImportDialog};
use crate::i18n::{i18n, i18n_f};
use crate::sidebar::ConnectionSidebar;
use crate::state::SharedAppState;

/// How long to wait for a `Bitwarden` auto-unlock before reporting timeout.
///
/// 30 seconds covers the worst case where the user has to type the master
/// password in an interactive prompt; below that, slow GPG/keyring backends
/// would falsely time out.
const BITWARDEN_UNLOCK_TIMEOUT: Duration = Duration::from_secs(30);

/// How long to wait for a single vault retrieve before reporting timeout.
///
/// 10 seconds is the standard project-wide vault budget — see `secrets-guide.md`.
const VAULT_RETRIEVE_TIMEOUT: Duration = Duration::from_secs(10);

/// Type alias for shared sidebar reference
pub type SharedSidebar = Rc<ConnectionSidebar>;

/// Shows the new connection dialog (always creates blank connection)
pub fn show_new_connection_dialog(
    window: &gtk4::Window,
    state: SharedAppState,
    sidebar: SharedSidebar,
) {
    // Always show regular connection dialog (no template picker)
    show_new_connection_dialog_internal(window, state, sidebar, None, None, None);
}

/// Shows the new connection dialog with a pre-selected group
pub fn show_new_connection_dialog_in_group(
    window: &gtk4::Window,
    state: SharedAppState,
    sidebar: SharedSidebar,
    group_id: Uuid,
) {
    show_new_connection_dialog_internal(window, state, sidebar, None, Some(group_id), None);
}

/// Shows the new connection dialog pre-filled with data from a `Connection`.
///
/// Used by the Connection Wizard "Advanced..." button to transfer partial data
/// into the full editor.
pub fn show_new_connection_dialog_prefilled(
    window: &gtk4::Window,
    state: SharedAppState,
    sidebar: SharedSidebar,
    connection: rustconn_core::models::Connection,
    password: Option<secrecy::SecretString>,
) {
    show_new_connection_dialog_internal_prefilled(window, state, sidebar, connection, password);
}

/// Internal function to show the new connection dialog with optional template
#[expect(
    clippy::too_many_lines,
    reason = "long match/dispatch over many enum variants; splitting per variant only relocates the boilerplate"
)]
pub fn show_new_connection_dialog_internal(
    window: &gtk4::Window,
    state: SharedAppState,
    sidebar: SharedSidebar,
    template: Option<rustconn_core::models::ConnectionTemplate>,
    group_id: Option<Uuid>,
    on_created: Option<Rc<dyn Fn()>>,
) {
    let dialog = ConnectionDialog::new(Some(&window.clone().upcast()), state.clone());
    dialog.setup_key_file_chooser(Some(&window.clone().upcast()));

    // Set available groups
    {
        let state_ref = state.borrow();
        let mut groups: Vec<_> = state_ref.list_groups().into_iter().cloned().collect();
        groups.sort_by_key(|a| a.name.to_lowercase());
        dialog.set_groups(&groups);
        let connections: Vec<_> = state_ref.list_connections().into_iter().cloned().collect();
        dialog.set_connections(&connections);
    }

    // Set preferred backend based on settings (filters password source dropdown)
    {
        let state_ref = state.borrow();
        let preferred_backend = state_ref.settings().secrets.preferred_backend;
        dialog.set_preferred_backend(preferred_backend);
    }

    // Populate variable dropdown with secret global variables
    {
        let state_ref = state.borrow();
        let global_vars = state_ref.settings().global_variables.clone();
        dialog.set_global_variables(&global_vars);
    }

    // Set up password visibility toggle and source visibility
    dialog.connect_password_visibility_toggle();
    dialog.connect_password_source_visibility();
    dialog.update_password_row_visibility();

    // Set up password load button with KeePass settings
    {
        let state_ref = state.borrow();
        let settings = state_ref.settings();
        let groups: Vec<rustconn_core::models::ConnectionGroup> =
            state_ref.list_groups().iter().cloned().cloned().collect();
        dialog.connect_password_load_button_with_groups(
            settings.secrets.kdbx_enabled,
            settings.secrets.kdbx_path.clone(),
            settings.secrets.kdbx_password.as_ref(),
            settings.secrets.kdbx_key_file.clone(),
            groups.clone(),
            settings.secrets.clone(),
        );
        dialog.connect_vault_test_button(
            settings.secrets.kdbx_enabled,
            settings.secrets.kdbx_path.clone(),
            settings.secrets.kdbx_password.as_ref(),
            settings.secrets.kdbx_key_file.clone(),
            groups,
            settings.secrets.clone(),
        );
    }

    // If template provided, pre-populate the dialog
    if let Some(ref tmpl) = template {
        let connection = tmpl.apply(None);
        dialog.set_connection(&connection);
        dialog
            .dialog()
            .set_title(&i18n("New Connection from Template"));
    }

    // Pre-select group if specified (e.g. from "New Connection in Group" context menu)
    if let Some(gid) = group_id {
        dialog.set_selected_group(gid);
    }

    let window_clone = window.clone();
    dialog.run(move |result| {
        if let Some(dialog_result) = result {
            let conn = dialog_result.connection;
            let password = dialog_result.password;

            if let Ok(mut state_mut) = state.try_borrow_mut() {
                // Clone values needed for password saving before creating connection
                let conn_name = conn.name.clone();
                let conn_host = conn.host.clone();
                let conn_username = conn.username.clone();
                let password_source = conn.password_source.clone();
                let protocol = conn.protocol;

                match state_mut.create_connection(conn) {
                    Ok(conn_id) => {
                        // Save password to vault if password source is Vault
                        // and password was provided
                        if password_source == PasswordSource::Vault
                            && let Some(pwd) = password
                        {
                            let settings = state_mut.settings().clone();
                            let groups: Vec<_> =
                                state_mut.list_groups().into_iter().cloned().collect();
                            let conn_for_path = state_mut.get_connection(conn_id).cloned();
                            let username = conn_username.unwrap_or_default();

                            crate::state::save_password_to_vault(
                                &settings,
                                &groups,
                                conn_for_path.as_ref(),
                                &conn_name,
                                &conn_host,
                                protocol,
                                &username,
                                &pwd,
                                conn_id,
                            );
                        }

                        // Release borrow before scheduling reload
                        drop(state_mut);
                        // Defer sidebar reload to next main loop iteration
                        // This prevents UI freeze during save operation
                        let state_clone = state.clone();
                        let sidebar_clone = sidebar.clone();
                        let on_created_cb = on_created.clone();
                        glib::idle_add_local_once(move || {
                            MainWindow::reload_sidebar_preserving_state(
                                &state_clone,
                                &sidebar_clone,
                            );
                            // Notify the caller (e.g. the tunnel builder) so it can
                            // refresh its own connection list now the new one exists.
                            if let Some(cb) = on_created_cb {
                                cb();
                            }
                        });
                    }
                    Err(e) => {
                        // Show error in UI dialog with proper transient parent
                        alert::show_error(&window_clone, &i18n("Error Creating Connection"), &e);
                    }
                }
            }
        }
    });
}

/// Internal: shows the new connection dialog pre-filled with an existing Connection object.
///
/// Similar to `show_new_connection_dialog_internal` but uses `set_connection` directly
/// instead of applying a template.
#[expect(
    clippy::too_many_lines,
    reason = "long match/dispatch over many enum variants; splitting per variant only relocates the boilerplate"
)]
fn show_new_connection_dialog_internal_prefilled(
    window: &gtk4::Window,
    state: SharedAppState,
    sidebar: SharedSidebar,
    connection: rustconn_core::models::Connection,
    password: Option<secrecy::SecretString>,
) {
    let dialog = ConnectionDialog::new(Some(&window.clone().upcast()), state.clone());
    dialog.setup_key_file_chooser(Some(&window.clone().upcast()));

    // Set available groups
    {
        let state_ref = state.borrow();
        let mut groups: Vec<_> = state_ref.list_groups().into_iter().cloned().collect();
        groups.sort_by_key(|a| a.name.to_lowercase());
        dialog.set_groups(&groups);
        let connections: Vec<_> = state_ref.list_connections().into_iter().cloned().collect();
        dialog.set_connections(&connections);
    }

    // Set preferred backend based on settings
    {
        let state_ref = state.borrow();
        let preferred_backend = state_ref.settings().secrets.preferred_backend;
        dialog.set_preferred_backend(preferred_backend);
    }

    // Populate variable dropdown with secret global variables
    {
        let state_ref = state.borrow();
        let global_vars = state_ref.settings().global_variables.clone();
        dialog.set_global_variables(&global_vars);
    }

    // Set up password visibility toggle and source visibility
    dialog.connect_password_visibility_toggle();
    dialog.connect_password_source_visibility();
    dialog.update_password_row_visibility();

    // Set up password load button with KeePass settings
    {
        let state_ref = state.borrow();
        let settings = state_ref.settings();
        let groups: Vec<rustconn_core::models::ConnectionGroup> =
            state_ref.list_groups().iter().cloned().cloned().collect();
        dialog.connect_password_load_button_with_groups(
            settings.secrets.kdbx_enabled,
            settings.secrets.kdbx_path.clone(),
            settings.secrets.kdbx_password.as_ref(),
            settings.secrets.kdbx_key_file.clone(),
            groups.clone(),
            settings.secrets.clone(),
        );
        dialog.connect_vault_test_button(
            settings.secrets.kdbx_enabled,
            settings.secrets.kdbx_path.clone(),
            settings.secrets.kdbx_password.as_ref(),
            settings.secrets.kdbx_key_file.clone(),
            groups,
            settings.secrets.clone(),
        );
    }

    // Pre-fill dialog with the connection data from wizard
    dialog.set_connection(&connection);
    // Restore the password typed in the wizard (set_connection only restores
    // the source, not the secret value).
    if let Some(ref pwd) = password {
        dialog.set_password(pwd);
    }
    // set_connection() titled this "Edit Connection"; this is a new connection
    // handed off from the wizard, so use the Advanced editor's own title rather
    // than the plain "New Connection" of the simplified wizard.
    dialog
        .dialog()
        .set_title(&i18n("New Connection (Advanced)"));

    let window_clone = window.clone();
    dialog.run(move |result| {
        if let Some(dialog_result) = result {
            let conn = dialog_result.connection;
            let password = dialog_result.password;

            if let Ok(mut state_mut) = state.try_borrow_mut() {
                // Clone values needed for password saving before creating connection
                let conn_name = conn.name.clone();
                let conn_host = conn.host.clone();
                let conn_username = conn.username.clone();
                let password_source = conn.password_source.clone();
                let protocol = conn.protocol;

                match state_mut.create_connection(conn) {
                    Ok(conn_id) => {
                        // Save password to vault if password source is Vault
                        if password_source == rustconn_core::models::PasswordSource::Vault
                            && let Some(pwd) = password
                        {
                            let settings = state_mut.settings().clone();
                            let groups: Vec<_> =
                                state_mut.list_groups().into_iter().cloned().collect();
                            let conn_for_path = state_mut.get_connection(conn_id).cloned();
                            let username = conn_username.unwrap_or_default();

                            crate::state::save_password_to_vault(
                                &settings,
                                &groups,
                                conn_for_path.as_ref(),
                                &conn_name,
                                &conn_host,
                                protocol,
                                &username,
                                &pwd,
                                conn_id,
                            );
                        }

                        // Release borrow before scheduling reload
                        drop(state_mut);
                        let state_clone = state.clone();
                        let sidebar_clone = sidebar.clone();
                        glib::idle_add_local_once(move || {
                            MainWindow::reload_sidebar_preserving_state(
                                &state_clone,
                                &sidebar_clone,
                            );
                        });
                    }
                    Err(e) => {
                        alert::show_error(&window_clone, &i18n("Error Creating Connection"), &e);
                    }
                }
            }
        }
    });
}

/// Shows the new group dialog with optional parent selection
pub fn show_new_group_dialog(window: &gtk4::Window, state: SharedAppState, sidebar: SharedSidebar) {
    show_new_group_dialog_with_parent(window, state, sidebar, None);
}

/// Shows the new group dialog with parent group selection
#[expect(
    clippy::needless_pass_by_value,
    clippy::too_many_lines,
    reason = "value is consumed by trait/API contract and the body dispatches over many variants; restructuring would scatter related logic"
)]
pub fn show_new_group_dialog_with_parent(
    window: &gtk4::Window,
    state: SharedAppState,
    sidebar: SharedSidebar,
    preselected_parent: Option<Uuid>,
) {
    let group_dialog = adw::Dialog::builder()
        .title(i18n("New Group"))
        .content_width(450)
        .build();

    // Header bar with Create icon button (GNOME HIG)
    let header = adw::HeaderBar::new();
    let create_btn = gtk4::Button::from_icon_name("list-add-symbolic");
    create_btn.set_tooltip_text(Some(&i18n("Create")));
    create_btn.update_property(&[gtk4::accessible::Property::Label(&i18n("Create"))]);
    create_btn.add_css_class("suggested-action");
    header.pack_start(&create_btn);

    // Scrollable content with clamp
    let clamp = adw::Clamp::builder()
        .maximum_size(600)
        .tightening_threshold(400)
        .build();

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    clamp.set_child(Some(&content));

    // Use ToolbarView for proper adw::Dialog layout
    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&clamp));
    group_dialog.set_child(Some(&toolbar_view));

    // === Group Details ===
    let details_group = adw::PreferencesGroup::builder()
        .title(i18n("Group Details"))
        .build();

    // Group name using EntryRow
    let name_row = adw::EntryRow::builder().title(i18n("Name")).build();
    details_group.add(&name_row);

    // Group icon using EntryRow
    let icon_row = adw::EntryRow::builder()
        .title(i18n("Icon"))
        .text("")
        .build();
    icon_row.set_tooltip_text(Some(&i18n(
        "Enter an emoji (e.g. 🇺🇦) or GTK icon name (e.g. starred-symbolic)",
    )));
    details_group.add(&icon_row);

    // Parent group dropdown
    let state_ref = state.borrow();

    // Sort by full path (displayed string)
    let mut groups: Vec<(Uuid, String)> = state_ref
        .list_groups()
        .iter()
        .map(|g| {
            let path = state_ref
                .get_group_path(g.id)
                .unwrap_or_else(|| g.name.clone());
            (g.id, path)
        })
        .collect();
    groups.sort_by_key(|a| a.1.to_lowercase());
    drop(state_ref);

    let mut group_ids: Vec<Option<Uuid>> = vec![None];
    let mut strings: Vec<String> = vec![i18n("(None - Root Level)")];
    let mut preselected_index = 0u32;

    for (id, path) in groups {
        strings.push(path);
        group_ids.push(Some(id));

        if preselected_parent == Some(id) {
            preselected_index = (group_ids.len() - 1) as u32;
        }
    }

    let string_list = gtk4::StringList::new(
        &strings
            .iter()
            .map(std::string::String::as_str)
            .collect::<Vec<_>>(),
    );
    let parent_dropdown = gtk4::DropDown::builder()
        .model(&string_list)
        .selected(preselected_index)
        .valign(gtk4::Align::Center)
        .build();

    let parent_row = adw::ActionRow::builder()
        .title(i18n("Parent"))
        .subtitle(i18n("Optional - leave empty for root level"))
        .build();
    parent_row.add_suffix(&parent_dropdown);
    details_group.add(&parent_row);

    content.append(&details_group);

    // === Inheritable Credentials ===
    let credentials_group = adw::PreferencesGroup::builder()
        .title(i18n("Default Credentials"))
        .description(i18n("Credentials inherited by connections in this group"))
        .build();

    let username_row = adw::EntryRow::builder().title(i18n("Username")).build();
    credentials_group.add(&username_row);

    // Password Source dropdown
    let password_source_list = gtk4::StringList::new(&[
        &i18n("Prompt"),
        &i18n("Vault"),
        &i18n("Variable"),
        &i18n("Inherit"),
        &i18n("None"),
    ]);
    let password_source_dropdown = gtk4::DropDown::builder()
        .model(&password_source_list)
        .valign(gtk4::Align::Center)
        .build();

    // Set default to Vault (index 1) — uses whichever backend is configured
    password_source_dropdown.set_selected(1);

    let password_source_row = adw::ActionRow::builder().title(i18n("Password")).build();
    password_source_row.add_suffix(&password_source_dropdown);
    credentials_group.add(&password_source_row);

    // Password Value entry with visibility toggle and load button
    let password_entry = gtk4::Entry::builder()
        .placeholder_text(i18n("Password value"))
        .visibility(false)
        .hexpand(true)
        .build();
    let password_visibility_btn = gtk4::Button::builder()
        .icon_name("view-reveal-symbolic")
        .tooltip_text(i18n("Show/hide password"))
        .valign(gtk4::Align::Center)
        .build();
    password_visibility_btn.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Toggle password visibility",
    ))]);
    let password_load_btn = gtk4::Button::builder()
        .icon_name("folder-symbolic")
        .tooltip_text(i18n("Load password from vault"))
        .valign(gtk4::Align::Center)
        .build();
    password_load_btn.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Load password from vault",
    ))]);

    let password_value_row = adw::ActionRow::builder().title(i18n("Value")).build();
    password_value_row.add_suffix(&password_entry);
    password_value_row.add_suffix(&password_visibility_btn);
    password_value_row.add_suffix(&password_load_btn);
    credentials_group.add(&password_value_row);

    // Show value row based on default selection (Vault = index 1)
    let show_value = password_source_dropdown.selected() == 1;
    password_value_row.set_visible(show_value);

    // Connect password source dropdown to show/hide value row
    let value_row_clone = password_value_row.clone();
    password_source_dropdown.connect_selected_notify(move |dropdown| {
        let selected = dropdown.selected();
        // Show for Vault(1) only
        let show = selected == 1;
        value_row_clone.set_visible(show);
    });

    // Connect password visibility toggle
    let password_entry_clone = password_entry.clone();
    let is_visible = std::rc::Rc::new(std::cell::Cell::new(false));
    password_visibility_btn.connect_clicked(move |btn| {
        let currently_visible = is_visible.get();
        let new_visible = !currently_visible;
        is_visible.set(new_visible);
        password_entry_clone.set_visibility(new_visible);
        if new_visible {
            btn.set_icon_name("view-conceal-symbolic");
        } else {
            btn.set_icon_name("view-reveal-symbolic");
        }
    });

    // Connect password load button - loads password from configured vault
    let password_entry_for_load = password_entry.clone();
    let password_source_for_load = password_source_dropdown.clone();
    let name_row_for_load = name_row.clone();
    let state_for_load = state.clone();
    let window_for_load = group_dialog.clone();
    password_load_btn.connect_clicked(move |btn| {
        let group_name = name_row_for_load.text().to_string();
        if group_name.trim().is_empty() {
            alert::show_validation_error(&window_for_load, &i18n("Enter group name first"));
            return;
        }

        let password_source_idx = password_source_for_load.selected();
        let lookup_key = format!("group:{}", group_name.replace('/', "-"));

        // Get settings for vault access
        let settings = state_for_load.borrow().settings().clone();

        let password_entry_clone = password_entry_for_load.clone();
        let window_clone = window_for_load.clone();
        let btn_clone = btn.clone();

        btn.set_sensitive(false);
        btn.set_icon_name("content-loading-symbolic");

        if password_source_idx == 1 {
            // Vault — load from configured backend
            if settings.secrets.kdbx_enabled
                && matches!(
                    settings.secrets.preferred_backend,
                    rustconn_core::config::SecretBackendType::KeePassXc
                        | rustconn_core::config::SecretBackendType::KdbxFile
                )
            {
                // KeePass backend
                let Some(kdbx_path) = settings.secrets.kdbx_path.clone() else {
                    alert::show_validation_error(&window_clone, &i18n("Vault not configured"));
                    btn_clone.set_sensitive(true);
                    btn_clone.set_icon_name("folder-symbolic");
                    return;
                };
                let key_file = settings.secrets.kdbx_key_file.clone();
                let entry_name = format!("RustConn/Groups/{group_name}");

                crate::utils::spawn_blocking_with_callback(
                    move || {
                        let key_file_path = key_file.as_ref().map(std::path::Path::new);
                        rustconn_core::secret::KeePassStatus::get_password_from_kdbx_with_key(
                            std::path::Path::new(&kdbx_path),
                            None,
                            key_file_path,
                            &entry_name,
                            None,
                        )
                    },
                    move |result: rustconn_core::error::SecretResult<
                        Option<secrecy::SecretString>,
                    >| {
                        btn_clone.set_sensitive(true);
                        btn_clone.set_icon_name("folder-symbolic");
                        match result {
                            Ok(Some(pwd)) => {
                                use secrecy::ExposeSecret;
                                password_entry_clone.set_text(pwd.expose_secret());
                            }
                            Ok(None) => {
                                alert::show_validation_error(
                                    &window_clone,
                                    &i18n("No password found for this group"),
                                );
                            }
                            Err(e) => {
                                let msg = e.to_string();
                                tracing::error!("Failed to load password: {msg}");
                                alert::show_error(&window_clone, &i18n("Load Error"), &msg);
                            }
                        }
                    },
                );
            } else {
                // Generic backend — dispatch based on preferred_backend
                let secret_settings = settings.secrets.clone();
                crate::utils::spawn_blocking_with_callback(
                    move || {
                        use rustconn_core::config::SecretBackendType;
                        use rustconn_core::secret::SecretBackend;

                        let backend_type = crate::state::select_backend_for_load(&secret_settings);
                        match backend_type {
                            SecretBackendType::Bitwarden => {
                                crate::async_utils::with_runtime(|rt| {
                                    let backend = rt.block_on(async {
                                        tokio::time::timeout(
                                            BITWARDEN_UNLOCK_TIMEOUT,
                                            rustconn_core::secret::auto_unlock(&secret_settings),
                                        )
                                        .await
                                        .map_err(|_| "Bitwarden auto-unlock timed out".to_string())?
                                        .map_err(|e| format!("{e}"))
                                    })?;
                                    rt.block_on(async {
                                        tokio::time::timeout(
                                            VAULT_RETRIEVE_TIMEOUT,
                                            backend.retrieve(&lookup_key),
                                        )
                                        .await
                                        .map_err(|_| "Vault retrieve timed out".to_string())?
                                        .map_err(|e| format!("{e}"))
                                    })
                                })?
                            }
                            SecretBackendType::OnePassword => {
                                let mut backend = rustconn_core::secret::OnePasswordBackend::new();
                                if let Some(ref token) =
                                    secret_settings.onepassword_service_account_token
                                {
                                    backend.set_service_account_token(token.clone());
                                }
                                crate::async_utils::with_runtime(|rt| {
                                    rt.block_on(async {
                                        tokio::time::timeout(
                                            VAULT_RETRIEVE_TIMEOUT,
                                            backend.retrieve(&lookup_key),
                                        )
                                        .await
                                        .map_err(|_| "Vault retrieve timed out".to_string())?
                                        .map_err(|e| format!("{e}"))
                                    })
                                })?
                            }
                            SecretBackendType::Passbolt => {
                                let mut backend = rustconn_core::secret::PassboltBackend::new();
                                if let Some(ref url) = secret_settings.passbolt_server_url {
                                    backend = backend.with_server_address(url.clone());
                                }
                                if let Some(ref passphrase) = secret_settings.passbolt_passphrase {
                                    backend = backend.with_user_password(passphrase.clone());
                                }
                                crate::async_utils::with_runtime(|rt| {
                                    rt.block_on(async {
                                        tokio::time::timeout(
                                            VAULT_RETRIEVE_TIMEOUT,
                                            backend.retrieve(&lookup_key),
                                        )
                                        .await
                                        .map_err(|_| "Vault retrieve timed out".to_string())?
                                        .map_err(|e| format!("{e}"))
                                    })
                                })?
                            }
                            SecretBackendType::Pass => {
                                let backend =
                                    rustconn_core::secret::PassBackend::from_secret_settings(
                                        &secret_settings,
                                    );
                                crate::async_utils::with_runtime(|rt| {
                                    rt.block_on(async {
                                        tokio::time::timeout(
                                            VAULT_RETRIEVE_TIMEOUT,
                                            backend.retrieve(&lookup_key),
                                        )
                                        .await
                                        .map_err(|_| "Vault retrieve timed out".to_string())?
                                        .map_err(|e| format!("{e}"))
                                    })
                                })?
                            }
                            SecretBackendType::LibSecret
                            | SecretBackendType::MacOsKeychain
                            | SecretBackendType::KeePassXc
                            | SecretBackendType::KdbxFile => {
                                // macOS uses the Keychain; LibSecretBackend (oo7)
                                // is not compiled there (R10.1, R10.2).
                                #[cfg(target_os = "macos")]
                                let backend = rustconn_core::secret::MacOsKeychainBackend::new();
                                #[cfg(not(target_os = "macos"))]
                                let backend =
                                    rustconn_core::secret::LibSecretBackend::new("rustconn");
                                crate::async_utils::with_runtime(|rt| {
                                    rt.block_on(async {
                                        tokio::time::timeout(
                                            VAULT_RETRIEVE_TIMEOUT,
                                            backend.retrieve(&lookup_key),
                                        )
                                        .await
                                        .map_err(|_| "Vault retrieve timed out".to_string())?
                                        .map_err(|e| format!("{e}"))
                                    })
                                })?
                            }
                            SecretBackendType::EncryptedFile => {
                                // Application-managed encrypted file: retrieve by
                                // the flat lookup key, same key scheme as the
                                // other app-managed backends.
                                let backend = rustconn_core::secret::EncryptedFileBackend::new();
                                crate::async_utils::with_runtime(|rt| {
                                    rt.block_on(async {
                                        tokio::time::timeout(
                                            VAULT_RETRIEVE_TIMEOUT,
                                            backend.retrieve(&lookup_key),
                                        )
                                        .await
                                        .map_err(|_| "Vault retrieve timed out".to_string())?
                                        .map_err(|e| format!("{e}"))
                                    })
                                })?
                            }
                        }
                    },
                    move |result: Result<Option<rustconn_core::models::Credentials>, String>| {
                        btn_clone.set_sensitive(true);
                        btn_clone.set_icon_name("folder-symbolic");
                        match result {
                            Ok(Some(creds)) => {
                                if let Some(pwd) = creds.expose_password() {
                                    password_entry_clone.set_text(pwd);
                                } else {
                                    alert::show_validation_error(
                                        &window_clone,
                                        &i18n("No password found for this group"),
                                    );
                                }
                            }
                            Ok(None) => {
                                alert::show_validation_error(
                                    &window_clone,
                                    &i18n("No password found for this group"),
                                );
                            }
                            Err(e) => {
                                tracing::error!("Failed to load password: {e}");
                                alert::show_error(&window_clone, &i18n("Load Error"), &e);
                            }
                        }
                    },
                );
            }
        } else {
            btn.set_sensitive(true);
            btn.set_icon_name("folder-symbolic");
            alert::show_validation_error(&window_clone, &i18n("Select Vault to load password"));
        }
    });

    let domain_row = adw::EntryRow::builder().title(i18n("Domain")).build();
    credentials_group.add(&domain_row);

    content.append(&credentials_group);

    // === Description Section ===
    let description_group = adw::PreferencesGroup::builder()
        .title(i18n("Description"))
        .description(i18n("Notes, contacts, project info"))
        .build();

    let description_view = gtk4::TextView::builder()
        .wrap_mode(gtk4::WrapMode::Word)
        .accepts_tab(false)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(8)
        .right_margin(8)
        .build();

    let description_scroll = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .min_content_height(144)
        .hexpand(true)
        .child(&description_view)
        .build();
    description_scroll.add_css_class("card");

    description_group.add(&description_scroll);
    content.append(&description_group);

    // Connect create button
    let state_clone = state.clone();
    let sidebar_clone = sidebar;
    let window_clone = group_dialog.clone();
    let name_row_clone = name_row;
    let icon_row_clone = icon_row;
    let dropdown_clone = parent_dropdown;
    let username_row_clone = username_row;
    let password_entry_clone2 = password_entry.clone();
    let password_source_clone = password_source_dropdown.clone();
    let domain_row_clone = domain_row;
    let description_buffer = description_view.buffer();

    create_btn.connect_clicked(move |_| {
        let name = name_row_clone.text().to_string();
        if name.trim().is_empty() {
            alert::show_validation_error(&window_clone, &i18n("Group name cannot be empty"));
            return;
        }

        let selected_idx = dropdown_clone.selected() as usize;
        let parent_id = if selected_idx < group_ids.len() {
            group_ids[selected_idx]
        } else {
            None
        };

        // Capture credential values
        let username = username_row_clone.text().to_string();
        // Capture password directly into SecretString — never let it live as
        // a plain String in this closure (M-PUBLIC-DEBUG / SecretString rules).
        let password = secrecy::SecretString::from(password_entry_clone2.text().to_string());
        let domain = domain_row_clone.text().to_string();

        // Capture description
        let description = {
            let start = description_buffer.start_iter();
            let end = description_buffer.end_iter();
            description_buffer.text(&start, &end, false).to_string()
        };
        let has_description = !description.trim().is_empty();

        // Get selected password source
        let password_source_idx = password_source_clone.selected();
        let new_password_source = match password_source_idx {
            0 => PasswordSource::Prompt,
            1 => PasswordSource::Vault,
            2 => PasswordSource::Variable(String::new()),
            3 => PasswordSource::Inherit,
            _ => PasswordSource::None,
        };

        let has_username = !username.trim().is_empty();
        // Password is relevant for Vault only
        let has_password =
            !secrecy::ExposeSecret::expose_secret(&password).is_empty() && password_source_idx == 1;
        let has_domain = !domain.trim().is_empty();

        // Capture icon
        let icon_text = icon_row_clone.text().trim().to_string();
        let has_icon = !icon_text.is_empty();

        // Validate icon
        if has_icon && let Err(e) = rustconn_core::dialog_utils::validate_icon(&icon_text) {
            alert::show_validation_error(&window_clone, &i18n(&e.to_string()));
            return;
        }

        if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
            let result = if let Some(pid) = parent_id {
                state_mut.create_group_with_parent(name, pid)
            } else {
                state_mut.create_group(name)
            };

            match result {
                Ok(group_id) => {
                    // Update group with credentials/description/icon if provided
                    if (has_username
                        || has_domain
                        || has_password
                        || has_description
                        || has_icon
                        || !matches!(new_password_source, PasswordSource::None))
                        && let Some(existing) = state_mut.get_group(group_id).cloned()
                    {
                        let mut updated = existing;
                        if has_username {
                            updated.username = Some(username.clone());
                        }
                        if has_domain {
                            updated.domain = Some(domain.clone());
                        }
                        if has_description {
                            updated.description = Some(description.clone());
                        }
                        if has_icon {
                            updated.icon = Some(icon_text.clone());
                        }
                        // Set the selected password source
                        updated.password_source = Some(new_password_source.clone());

                        if let Err(e) = state_mut.update_group(group_id, updated) {
                            alert::show_error(
                                &window_clone,
                                &i18n("Error Updating Group"),
                                &e.clone(),
                            );
                            // Don't return, allow closing window since group was created
                        }
                    }

                    // Save password if provided - use appropriate backend
                    if has_password {
                        // Get group path for hierarchical storage
                        let groups: Vec<_> = state_mut.list_groups().into_iter().cloned().collect();
                        let group = state_mut.get_group(group_id).cloned();
                        let settings = state_mut.settings().clone();

                        if let Some(grp) = group {
                            let group_path =
                                rustconn_core::secret::KeePassHierarchy::build_group_entry_path(
                                    &grp, &groups,
                                );
                            let lookup_key = grp.id.to_string();

                            if new_password_source == PasswordSource::Vault {
                                // Save to vault using configured backend
                                crate::state::save_group_password_to_vault(
                                    &settings,
                                    &group_path,
                                    &lookup_key,
                                    &username,
                                    &password,
                                );
                            }
                        }
                    }

                    drop(state_mut);
                    // Defer sidebar reload to prevent UI freeze
                    let state = state_clone.clone();
                    let sidebar = sidebar_clone.clone();
                    let window = window_clone.clone();
                    glib::idle_add_local_once(move || {
                        MainWindow::reload_sidebar_preserving_state(&state, &sidebar);
                        window.close();
                    });
                }
                Err(e) => {
                    alert::show_error(&window_clone, &i18n("Error"), &e);
                }
            }
        }
    });

    group_dialog.present(Some(window));
}

/// Shows the import dialog
pub fn show_import_dialog(window: &gtk4::Window, state: SharedAppState, sidebar: SharedSidebar) {
    let dialog = ImportDialog::new(Some(&window.clone().upcast()));

    let window_clone = window.clone();
    dialog.run_with_source(move |result, source_name| {
        let Some(import_result) = result else {
            return;
        };

        // Detect duplicates by name before applying (same case-insensitive
        // rule the importer uses for auto-renaming) and let the user decide
        // instead of silently creating renamed copies.
        let duplicate_names: std::collections::HashSet<String> = state
            .try_borrow()
            .map(|s| {
                import_result
                    .connections
                    .iter()
                    .filter(|c| s.connection_exists_by_name(&c.name))
                    .map(|c| c.name.to_lowercase())
                    .collect()
            })
            .unwrap_or_default();
        let duplicate_count = import_result
            .connections
            .iter()
            .filter(|c| duplicate_names.contains(&c.name.to_lowercase()))
            .count();

        if duplicate_count == 0 {
            apply_import_result(&window_clone, &state, &sidebar, import_result, &source_name);
            return;
        }

        let confirm = adw::AlertDialog::new(
            Some(&i18n("Import Duplicates?")),
            Some(&i18n_f(
                "{} of {} connections have the same name as existing ones. Imported copies are kept alongside the existing connections under a numbered name.",
                &[
                    &duplicate_count.to_string(),
                    &import_result.connections.len().to_string(),
                ],
            )),
        );
        confirm.add_response("cancel", &i18n("Cancel"));
        confirm.add_response("skip", &i18n("Skip Duplicates"));
        confirm.add_response("all", &i18n("Import All"));
        confirm.set_response_appearance("all", adw::ResponseAppearance::Suggested);
        confirm.set_default_response(Some("all"));
        confirm.set_close_response("cancel");

        // The result is consumed by exactly one response; Fn closures
        // cannot move it out directly, hence the Cell.
        let pending = std::rc::Rc::new(std::cell::RefCell::new(Some(import_result)));
        let window2 = window_clone.clone();
        let state2 = state.clone();
        let sidebar2 = sidebar.clone();
        let source2 = source_name.clone();
        confirm.connect_response(None, move |_, response| {
            let Some(mut import_result) = pending.borrow_mut().take() else {
                return;
            };
            match response {
                "skip" => {
                    import_result
                        .connections
                        .retain(|c| !duplicate_names.contains(&c.name.to_lowercase()));
                    apply_import_result(&window2, &state2, &sidebar2, import_result, &source2);
                }
                "all" => {
                    apply_import_result(&window2, &state2, &sidebar2, import_result, &source2);
                }
                _ => {}
            }
        });
        confirm.present(Some(&window_clone));
    });
}

/// Applies an import result to the app state and reports the outcome.
fn apply_import_result(
    window: &gtk4::Window,
    state: &SharedAppState,
    sidebar: &SharedSidebar,
    import_result: rustconn_core::import::ImportResult,
    source_name: &str,
) {
    let Ok(mut state_mut) = state.try_borrow_mut() else {
        return;
    };
    let connection_count = import_result.connections.len();
    tracing::info!(count = connection_count, "Importing connections");

    match state_mut.import_connections_with_source(&import_result, source_name) {
        Ok(count) => {
            // Merge snippets if present (native format)
            let snippet_count = import_result.snippets.len();
            for snippet in import_result.snippets {
                if let Err(e) = state_mut.create_snippet(snippet) {
                    tracing::warn!("Failed to import snippet: {e}");
                }
            }
            drop(state_mut);
            // Defer sidebar reload to prevent UI freeze
            let state_clone = state.clone();
            let sidebar_clone = sidebar.clone();
            let window = window.clone();
            let source = source_name.to_string();
            glib::idle_add_local_once(move || {
                MainWindow::reload_sidebar_preserving_state(&state_clone, &sidebar_clone);
                let msg = if snippet_count > 0 {
                    i18n_f(
                        "Imported {} connections and {} snippets to '{}' group",
                        &[&count.to_string(), &snippet_count.to_string(), &source],
                    )
                } else {
                    i18n_f(
                        "Imported {} connections to '{}' group",
                        &[&count.to_string(), &source],
                    )
                };
                // Non-blocking success feedback (GNOME HIG: toast, not dialog).
                crate::toast::show_toast_on_window(&window, &msg, crate::toast::ToastType::Success);
            });
        }
        Err(e) => {
            alert::show_error(window, &i18n("Import Failed"), &e);
        }
    }
}
