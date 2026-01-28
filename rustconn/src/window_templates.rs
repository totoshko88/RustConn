//! Template-related methods for the main window
//!
//! This module contains methods for managing connection templates.

use crate::alert;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{Label, Orientation};
use std::rc::Rc;

use crate::dialogs::{ConnectionDialog, TemplateDialog, TemplateManagerDialog};
use crate::sidebar::ConnectionSidebar;
use crate::state::SharedAppState;
use crate::window::MainWindow;
use rustconn_core::models::{Credentials, PasswordSource};

/// Type alias for shared sidebar
pub type SharedSidebar = Rc<ConnectionSidebar>;

/// Shows the templates manager window
#[allow(clippy::too_many_lines)]
pub fn show_templates_manager(
    window: &gtk4::Window,
    state: SharedAppState,
    sidebar: SharedSidebar,
) {
    let manager_dialog = TemplateManagerDialog::new(Some(&window.clone().upcast()));

    // Load templates from config file and active document
    let templates = {
        let state_ref = state.borrow();
        state_ref.get_all_templates()
    };
    manager_dialog.set_templates(templates);

    // Get references for closures
    let templates_list = manager_dialog.templates_list().clone();
    let state_templates = manager_dialog.state_templates().clone();
    let manager_window = manager_dialog.window().clone();

    // Connect filter dropdown
    if let Some(content) = manager_window.child() {
        if let Some(vbox) = content.downcast_ref::<gtk4::Box>() {
            if let Some(filter_box) = vbox.first_child() {
                if let Some(hbox) = filter_box.downcast_ref::<gtk4::Box>() {
                    if let Some(dropdown_widget) = hbox.last_child() {
                        if let Some(filter_dropdown) =
                            dropdown_widget.downcast_ref::<gtk4::DropDown>()
                        {
                            let list_clone = templates_list.clone();
                            let templates_clone = state_templates.clone();
                            filter_dropdown.connect_selected_notify(move |dropdown| {
                                let selected = dropdown.selected();
                                let filter = match selected {
                                    1 => Some(rustconn_core::models::ProtocolType::Ssh),
                                    2 => Some(rustconn_core::models::ProtocolType::Rdp),
                                    3 => Some(rustconn_core::models::ProtocolType::Vnc),
                                    4 => Some(rustconn_core::models::ProtocolType::Spice),
                                    _ => None,
                                };
                                refresh_templates_list(&list_clone, &templates_clone, filter);
                            });
                        }
                    }
                }
            }
        }
    }

    // Set up "New Template" callback
    {
        let state_clone = state.clone();
        let templates_clone = state_templates.clone();
        let list_clone = templates_list.clone();
        let manager_clone = manager_window.clone();
        manager_dialog.set_on_new(move || {
            let dialog = TemplateDialog::new(Some(&manager_clone.clone().upcast()));
            let state_inner = state_clone.clone();
            let templates_inner = templates_clone.clone();
            let list_inner = list_clone.clone();
            let manager_inner = manager_clone.clone();
            dialog.run(move |result| {
                if let Some(template) = result {
                    // Add to state templates (local cache)
                    templates_inner.borrow_mut().push(template.clone());
                    // Save to config file and active document
                    if let Ok(mut state_mut) = state_inner.try_borrow_mut() {
                        if let Err(e) = state_mut.add_template(template) {
                            alert::show_error(&manager_inner, "Error Saving Template", &e);
                        }
                    }
                    // Refresh list
                    refresh_templates_list(&list_inner, &templates_inner, None);
                }
            });
        });
    }

    // Set up "Edit" callback
    {
        let state_clone = state.clone();
        let templates_clone = state_templates.clone();
        let list_clone = templates_list.clone();
        let manager_clone = manager_window.clone();
        manager_dialog.set_on_edit(move |template| {
            let id = template.id;
            let dialog = TemplateDialog::new(Some(&manager_clone.clone().upcast()));
            dialog.set_template(&template);
            let state_inner = state_clone.clone();
            let templates_inner = templates_clone.clone();
            let list_inner = list_clone.clone();
            let manager_inner = manager_clone.clone();
            dialog.run(move |result| {
                if let Some(updated) = result {
                    // Update in state templates (local cache)
                    let mut templates = templates_inner.borrow_mut();
                    if let Some(pos) = templates.iter().position(|t| t.id == id) {
                        templates[pos] = updated.clone();
                    }
                    drop(templates);
                    // Update in config file and active document
                    if let Ok(mut state_mut) = state_inner.try_borrow_mut() {
                        if let Err(e) = state_mut.update_template(updated) {
                            alert::show_error(&manager_inner, "Error Saving Template", &e);
                        }
                    }
                    // Refresh list
                    refresh_templates_list(&list_inner, &templates_inner, None);
                }
            });
        });
    }

    // Set up "Delete" callback
    {
        let state_clone = state.clone();
        let templates_clone = state_templates.clone();
        let list_clone = templates_list.clone();
        let manager_clone = manager_window.clone();
        let state_inner = state_clone.clone();
        let templates_inner = templates_clone.clone();
        let list_inner = list_clone.clone();
        let manager_clone_for_confirm = manager_clone.clone();
        manager_dialog.set_on_delete(move |id| {
            let state_inner = state_inner.clone();
            let templates_inner = templates_inner.clone();
            let list_inner = list_inner.clone();
            alert::show_confirm(
                &manager_clone_for_confirm,
                "Delete Template?",
                "Are you sure you want to delete this template?",
                "Delete",
                true,
                move |confirmed| {
                    if confirmed {
                        // Remove from state templates (local cache)
                        templates_inner.borrow_mut().retain(|t| t.id != id);
                        // Remove from config file and active document
                        if let Ok(mut state_mut) = state_inner.try_borrow_mut() {
                            if let Err(e) = state_mut.delete_template(id) {
                                eprintln!("Failed to delete template: {e}");
                            }
                        }
                        // Refresh list
                        refresh_templates_list(&list_inner, &templates_inner, None);
                    }
                },
            );
        });
    }

    // Set up "Use Template" callback
    {
        let state_clone = state.clone();
        let manager_clone = manager_window.clone();
        let sidebar_clone = sidebar.clone();
        manager_dialog.set_on_template_selected(move |template_opt| {
            if let Some(template) = template_opt {
                // Create connection from template
                show_new_connection_from_template(
                    manager_clone.upcast_ref(),
                    state_clone.clone(),
                    sidebar_clone.clone(),
                    &template,
                );
            }
        });
    }

    manager_dialog.present();
}

/// Refreshes the templates list with optional protocol filter
#[allow(clippy::too_many_lines)]
pub fn refresh_templates_list(
    list: &gtk4::ListBox,
    templates: &std::rc::Rc<std::cell::RefCell<Vec<rustconn_core::models::ConnectionTemplate>>>,
    protocol_filter: Option<rustconn_core::models::ProtocolType>,
) {
    use rustconn_core::models::ProtocolType;

    // Clear existing rows
    while let Some(row) = list.row_at_index(0) {
        list.remove(&row);
    }

    let templates_ref = templates.borrow();

    // Group templates by protocol
    let mut ssh_templates: Vec<&rustconn_core::models::ConnectionTemplate> = Vec::new();
    let mut rdp_templates: Vec<&rustconn_core::models::ConnectionTemplate> = Vec::new();
    let mut vnc_templates: Vec<&rustconn_core::models::ConnectionTemplate> = Vec::new();
    let mut spice_templates: Vec<&rustconn_core::models::ConnectionTemplate> = Vec::new();

    for template in templates_ref.iter() {
        if let Some(filter) = protocol_filter {
            if template.protocol != filter {
                continue;
            }
        }
        match template.protocol {
            ProtocolType::Ssh | ProtocolType::ZeroTrust => ssh_templates.push(template),
            ProtocolType::Rdp => rdp_templates.push(template),
            ProtocolType::Vnc => vnc_templates.push(template),
            ProtocolType::Spice => spice_templates.push(template),
        }
    }

    // Helper to add section header
    let add_section_header = |list: &gtk4::ListBox, title: &str| {
        let label = Label::builder()
            .label(title)
            .halign(gtk4::Align::Start)
            .css_classes(["heading"])
            .margin_top(8)
            .margin_bottom(4)
            .margin_start(8)
            .build();
        let row = gtk4::ListBoxRow::builder()
            .child(&label)
            .selectable(false)
            .activatable(false)
            .build();
        list.append(&row);
    };

    // Helper to add template row
    let add_template_row =
        |list: &gtk4::ListBox, template: &rustconn_core::models::ConnectionTemplate| {
            let hbox = gtk4::Box::new(Orientation::Horizontal, 8);
            hbox.set_margin_top(8);
            hbox.set_margin_bottom(8);
            hbox.set_margin_start(8);
            hbox.set_margin_end(8);

            // Protocol icon
            let icon_name = match template.protocol {
                ProtocolType::Ssh => "utilities-terminal-symbolic",
                ProtocolType::Rdp => "computer-symbolic",
                ProtocolType::Vnc => "video-display-symbolic",
                ProtocolType::Spice => "video-display-symbolic",
                ProtocolType::ZeroTrust => "cloud-symbolic",
            };
            let icon = gtk4::Image::from_icon_name(icon_name);
            hbox.append(&icon);

            // Template info
            let info_box = gtk4::Box::new(Orientation::Vertical, 2);
            info_box.set_hexpand(true);

            let name_label = Label::builder()
                .label(&template.name)
                .halign(gtk4::Align::Start)
                .css_classes(["heading"])
                .build();
            info_box.append(&name_label);

            let details = if let Some(ref desc) = template.description {
                desc.clone()
            } else {
                let mut parts = Vec::new();
                if !template.host.is_empty() {
                    parts.push(format!("Host: {}", template.host));
                }
                parts.push(format!("Port: {}", template.port));
                if let Some(ref user) = template.username {
                    parts.push(format!("User: {user}"));
                }
                parts.join(" | ")
            };

            let details_label = Label::builder()
                .label(&details)
                .halign(gtk4::Align::Start)
                .css_classes(["dim-label"])
                .build();
            info_box.append(&details_label);

            hbox.append(&info_box);

            let row = gtk4::ListBoxRow::builder().child(&hbox).build();
            row.set_widget_name(&format!("template-{}", template.id));
            list.append(&row);
        };

    // Add SSH templates
    if !ssh_templates.is_empty() && protocol_filter.is_none() {
        add_section_header(list, "SSH Templates");
    }
    for template in ssh_templates {
        add_template_row(list, template);
    }

    // Add RDP templates
    if !rdp_templates.is_empty() && protocol_filter.is_none() {
        add_section_header(list, "RDP Templates");
    }
    for template in rdp_templates {
        add_template_row(list, template);
    }

    // Add VNC templates
    if !vnc_templates.is_empty() && protocol_filter.is_none() {
        add_section_header(list, "VNC Templates");
    }
    for template in vnc_templates {
        add_template_row(list, template);
    }

    // Add SPICE templates
    if !spice_templates.is_empty() && protocol_filter.is_none() {
        add_section_header(list, "SPICE Templates");
    }
    for template in spice_templates {
        add_template_row(list, template);
    }
}

/// Shows the new connection dialog pre-populated from a template
pub fn show_new_connection_from_template(
    window: &gtk4::Window,
    state: SharedAppState,
    sidebar: SharedSidebar,
    template: &rustconn_core::models::ConnectionTemplate,
) {
    // Create connection from template
    let connection = template.apply(None);

    let dialog = ConnectionDialog::new(Some(window));
    dialog.setup_key_file_chooser(Some(window));

    // Set available groups
    {
        let state_ref = state.borrow();
        let groups: Vec<_> = state_ref.list_groups().into_iter().cloned().collect();
        dialog.set_groups(&groups);
    }

    // Set preferred backend based on settings (filters password source dropdown)
    {
        let state_ref = state.borrow();
        let preferred_backend = state_ref.settings().secrets.preferred_backend;
        dialog.set_preferred_backend(preferred_backend);
    }

    // Set up password visibility toggle and source visibility
    dialog.connect_password_visibility_toggle();
    dialog.connect_password_source_visibility();
    dialog.update_password_row_visibility();

    // Set up password load button with KeePass settings
    {
        use secrecy::ExposeSecret;
        let state_ref = state.borrow();
        let settings = state_ref.settings();
        dialog.connect_password_load_button(
            settings.secrets.kdbx_enabled,
            settings.secrets.kdbx_path.clone(),
            settings
                .secrets
                .kdbx_password
                .as_ref()
                .map(|p| p.expose_secret().to_string()),
            settings.secrets.kdbx_key_file.clone(),
        );
    }

    // Pre-populate with template values
    dialog.set_connection(&connection);
    // Reset the title since we're creating a new connection
    dialog
        .window()
        .set_title(Some("New Connection from Template"));

    let window_clone = window.clone();
    dialog.run(move |result| {
        if let Some(dialog_result) = result {
            let conn = dialog_result.connection;
            let password = dialog_result.password;

            if let Ok(mut state_mut) = state.try_borrow_mut() {
                // Clone values needed for password saving
                let conn_name = conn.name.clone();
                let conn_host = conn.host.clone();
                let conn_username = conn.username.clone();
                let password_source = conn.password_source;
                let protocol = conn.protocol;

                match state_mut.create_connection(conn) {
                    Ok(conn_id) => {
                        // Save password to KeePass if needed
                        if password_source == PasswordSource::KeePass {
                            if let Some(pwd) = password.clone() {
                                let settings = state_mut.settings().clone();
                                if settings.secrets.kdbx_enabled {
                                    if let Some(kdbx_path) = settings.secrets.kdbx_path.clone() {
                                        let key_file = settings.secrets.kdbx_key_file.clone();
                                        let entry_name = format!(
                                            "{} ({})",
                                            conn_name,
                                            protocol.as_str().to_lowercase()
                                        );
                                        let username = conn_username.clone().unwrap_or_default();
                                        let url = format!(
                                            "{}://{}",
                                            protocol.as_str().to_lowercase(),
                                            conn_host
                                        );

                                        crate::utils::spawn_blocking_with_callback(
                                            move || {
                                                let kdbx = std::path::Path::new(&kdbx_path);
                                                let key = key_file
                                                    .as_ref()
                                                    .map(|p| std::path::Path::new(p));
                                                rustconn_core::secret::KeePassStatus
                                                    ::save_password_to_kdbx(
                                                        kdbx,
                                                        None,
                                                        key,
                                                        &entry_name,
                                                        &username,
                                                        &pwd,
                                                        Some(&url),
                                                    )
                                            },
                                            move |result| {
                                                if let Err(e) = result {
                                                    tracing::error!(
                                                        "Failed to save password to KeePass: {}",
                                                        e
                                                    );
                                                } else {
                                                    tracing::info!(
                                                        "Password saved to KeePass for \
                                                         connection {}",
                                                        conn_id
                                                    );
                                                }
                                            },
                                        );
                                    }
                                }
                            }
                        }

                        // Save password to Keyring if needed
                        if password_source == PasswordSource::Keyring {
                            if let Some(pwd) = password.clone() {
                                let lookup_key = format!(
                                    "{} ({})",
                                    conn_name.replace('/', "-"),
                                    protocol.as_str().to_lowercase()
                                );
                                let username = conn_username.clone().unwrap_or_default();

                                crate::utils::spawn_blocking_with_callback(
                                    move || {
                                        use rustconn_core::secret::SecretBackend;
                                        let backend = rustconn_core::secret::LibSecretBackend::new(
                                            "rustconn",
                                        );
                                        let creds = Credentials {
                                            username: Some(username),
                                            password: Some(secrecy::SecretString::from(pwd)),
                                            key_passphrase: None,
                                            domain: None,
                                        };
                                        let rt = tokio::runtime::Runtime::new()
                                            .map_err(|e| format!("Runtime error: {e}"))?;
                                        rt.block_on(backend.store(&lookup_key, &creds))
                                            .map_err(|e| format!("{e}"))
                                    },
                                    move |result: Result<(), String>| {
                                        if let Err(e) = result {
                                            tracing::error!(
                                                "Failed to save password to Keyring: {}",
                                                e
                                            );
                                        } else {
                                            tracing::info!(
                                                "Password saved to Keyring for connection {}",
                                                conn_id
                                            );
                                        }
                                    },
                                );
                            }
                        }

                        // Save password to Bitwarden if needed
                        if password_source == PasswordSource::Bitwarden {
                            if let Some(pwd) = password {
                                let lookup_key = format!(
                                    "{} ({})",
                                    conn_name.replace('/', "-"),
                                    protocol.as_str().to_lowercase()
                                );
                                let username = conn_username.unwrap_or_default();

                                crate::utils::spawn_blocking_with_callback(
                                    move || {
                                        use rustconn_core::secret::SecretBackend;
                                        let backend =
                                            rustconn_core::secret::BitwardenBackend::new();
                                        let creds = Credentials {
                                            username: Some(username),
                                            password: Some(secrecy::SecretString::from(pwd)),
                                            key_passphrase: None,
                                            domain: None,
                                        };
                                        let rt = tokio::runtime::Runtime::new()
                                            .map_err(|e| format!("Runtime error: {e}"))?;
                                        rt.block_on(backend.store(&lookup_key, &creds))
                                            .map_err(|e| format!("{e}"))
                                    },
                                    move |result: Result<(), String>| {
                                        if let Err(e) = result {
                                            tracing::error!(
                                                "Failed to save password to Bitwarden: {}",
                                                e
                                            );
                                        } else {
                                            tracing::info!(
                                                "Password saved to Bitwarden for connection {}",
                                                conn_id
                                            );
                                        }
                                    },
                                );
                            }
                        }

                        drop(state_mut);
                        // Defer sidebar reload to prevent UI freeze
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
                        alert::show_error(&window_clone, "Error Creating Connection", &e);
                    }
                }
            }
        }
    });
}
