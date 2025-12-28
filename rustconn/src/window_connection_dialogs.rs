//! Connection and group creation dialogs for main window
//!
//! This module contains dialog functions for creating new connections and groups,
//! including template picker and parent group selection.

use crate::dialogs::{ConnectionDialog, ImportDialog};
use crate::sidebar::ConnectionSidebar;
use crate::state::SharedAppState;
use crate::window::MainWindow;
use gtk4::prelude::*;
use gtk4::{ApplicationWindow, Button, HeaderBar, Label, Orientation, ScrolledWindow};
use std::rc::Rc;
use uuid::Uuid;

/// Type alias for shared sidebar reference
pub type SharedSidebar = Rc<ConnectionSidebar>;

/// Shows the new connection dialog with optional template selection
pub fn show_new_connection_dialog(
    window: &ApplicationWindow,
    state: SharedAppState,
    sidebar: SharedSidebar,
) {
    // Check if there are templates available
    let templates = {
        let state_ref = state.borrow();
        state_ref.get_all_templates()
    };

    if templates.is_empty() {
        // No templates, show regular connection dialog
        show_new_connection_dialog_internal(window, state, sidebar, None);
    } else {
        // Show template picker first
        show_template_picker_for_new_connection(window, state, sidebar, templates);
    }
}

/// Shows a template picker dialog before creating a new connection
#[allow(clippy::too_many_lines)]
pub fn show_template_picker_for_new_connection(
    window: &ApplicationWindow,
    state: SharedAppState,
    sidebar: SharedSidebar,
    templates: Vec<rustconn_core::models::ConnectionTemplate>,
) {
    let picker_window = gtk4::Window::builder()
        .title("Create Connection")
        .transient_for(window)
        .modal(true)
        .default_width(400)
        .default_height(350)
        .build();

    // Create header bar
    let header = HeaderBar::new();
    let cancel_btn = Button::builder().label("Cancel").build();
    header.pack_start(&cancel_btn);
    picker_window.set_titlebar(Some(&header));

    // Create content
    let content = gtk4::Box::new(Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    let title_label = Label::builder()
        .label("Choose how to create your connection:")
        .halign(gtk4::Align::Start)
        .css_classes(["heading"])
        .build();
    content.append(&title_label);

    // Blank connection option
    let blank_btn = Button::builder().label("Start from scratch").build();
    let blank_box = gtk4::Box::new(Orientation::Vertical, 4);
    blank_box.append(&blank_btn);
    let blank_desc = Label::builder()
        .label("Create a new connection with default settings")
        .halign(gtk4::Align::Start)
        .css_classes(["dim-label"])
        .build();
    blank_box.append(&blank_desc);
    content.append(&blank_box);

    // Separator
    let separator = gtk4::Separator::new(Orientation::Horizontal);
    separator.set_margin_top(8);
    separator.set_margin_bottom(8);
    content.append(&separator);

    // Template section
    let template_label = Label::builder()
        .label("Or use a template:")
        .halign(gtk4::Align::Start)
        .build();
    content.append(&template_label);

    // Templates list
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .vexpand(true)
        .build();

    let templates_list = gtk4::ListBox::builder()
        .selection_mode(gtk4::SelectionMode::Single)
        .css_classes(["boxed-list"])
        .build();

    for template in &templates {
        let hbox = gtk4::Box::new(Orientation::Horizontal, 8);
        hbox.set_margin_top(8);
        hbox.set_margin_bottom(8);
        hbox.set_margin_start(8);
        hbox.set_margin_end(8);

        // Protocol icon
        let icon_name = match template.protocol {
            rustconn_core::models::ProtocolType::Ssh => "utilities-terminal-symbolic",
            rustconn_core::models::ProtocolType::Rdp => "computer-symbolic",
            rustconn_core::models::ProtocolType::Vnc => "video-display-symbolic",
            rustconn_core::models::ProtocolType::Spice => "video-display-symbolic",
            rustconn_core::models::ProtocolType::ZeroTrust => "cloud-symbolic",
        };
        let icon = gtk4::Image::from_icon_name(icon_name);
        hbox.append(&icon);

        // Template info
        let info_box = gtk4::Box::new(Orientation::Vertical, 2);
        info_box.set_hexpand(true);

        let name_label = Label::builder()
            .label(&template.name)
            .halign(gtk4::Align::Start)
            .build();
        info_box.append(&name_label);

        if let Some(ref desc) = template.description {
            let desc_label = Label::builder()
                .label(desc)
                .halign(gtk4::Align::Start)
                .css_classes(["dim-label"])
                .build();
            info_box.append(&desc_label);
        }

        hbox.append(&info_box);

        let row = gtk4::ListBoxRow::builder().child(&hbox).build();
        row.set_widget_name(&format!("template-{}", template.id));
        templates_list.append(&row);
    }

    scrolled.set_child(Some(&templates_list));
    content.append(&scrolled);

    // Use template button
    let use_template_btn = Button::builder()
        .label("Use Selected Template")
        .sensitive(false)
        .css_classes(["suggested-action"])
        .build();
    content.append(&use_template_btn);

    picker_window.set_child(Some(&content));

    // Connect selection changed
    let use_btn_clone = use_template_btn.clone();
    templates_list.connect_row_selected(move |_, row| {
        use_btn_clone.set_sensitive(row.is_some());
    });

    // Connect cancel button
    let picker_clone = picker_window.clone();
    cancel_btn.connect_clicked(move |_| {
        picker_clone.close();
    });

    // Connect blank button
    let picker_clone = picker_window.clone();
    let window_clone = window.clone();
    let state_clone = state.clone();
    let sidebar_clone = sidebar.clone();
    blank_btn.connect_clicked(move |_| {
        picker_clone.close();
        show_new_connection_dialog_internal(
            &window_clone,
            state_clone.clone(),
            sidebar_clone.clone(),
            None,
        );
    });

    // Connect use template button
    let picker_clone = picker_window.clone();
    let window_clone = window.clone();
    let state_clone = state.clone();
    let sidebar_clone = sidebar.clone();
    let templates_clone = templates.clone();
    let list_clone = templates_list.clone();
    use_template_btn.connect_clicked(move |_| {
        if let Some(row) = list_clone.selected_row() {
            if let Some(id_str) = row.widget_name().as_str().strip_prefix("template-") {
                if let Ok(id) = Uuid::parse_str(id_str) {
                    if let Some(template) = templates_clone.iter().find(|t| t.id == id) {
                        picker_clone.close();
                        show_new_connection_dialog_internal(
                            &window_clone,
                            state_clone.clone(),
                            sidebar_clone.clone(),
                            Some(template.clone()),
                        );
                    }
                }
            }
        }
    });

    // Double-click on template row
    let picker_clone = picker_window.clone();
    let window_clone = window.clone();
    let state_clone = state;
    let sidebar_clone = sidebar;
    let templates_clone = templates;
    templates_list.connect_row_activated(move |_, row| {
        if let Some(id_str) = row.widget_name().as_str().strip_prefix("template-") {
            if let Ok(id) = Uuid::parse_str(id_str) {
                if let Some(template) = templates_clone.iter().find(|t| t.id == id) {
                    picker_clone.close();
                    show_new_connection_dialog_internal(
                        &window_clone,
                        state_clone.clone(),
                        sidebar_clone.clone(),
                        Some(template.clone()),
                    );
                }
            }
        }
    });

    picker_window.present();
}

/// Internal function to show the new connection dialog with optional template
#[allow(clippy::too_many_lines)]
pub fn show_new_connection_dialog_internal(
    window: &ApplicationWindow,
    state: SharedAppState,
    sidebar: SharedSidebar,
    template: Option<rustconn_core::models::ConnectionTemplate>,
) {
    let dialog = ConnectionDialog::new(Some(&window.clone().upcast()));
    dialog.setup_key_file_chooser(Some(&window.clone().upcast()));

    // Set KeePass enabled state from settings
    {
        let state_ref = state.borrow();
        let keepass_enabled = state_ref.settings().secrets.kdbx_enabled;
        dialog.set_keepass_enabled(keepass_enabled);
    }

    // If template provided, pre-populate the dialog
    if let Some(ref tmpl) = template {
        let connection = tmpl.apply(None);
        dialog.set_connection(&connection);
        dialog
            .window()
            .set_title(Some("New Connection from Template"));
    }

    // Connect save to KeePass callback
    let window_for_keepass = window.clone();
    let state_for_save = state.clone();
    dialog.connect_save_to_keepass(move |name, host, username, password, protocol| {
        use secrecy::ExposeSecret;

        let state_ref = state_for_save.borrow();
        let settings = state_ref.settings();

        if !settings.secrets.kdbx_enabled {
            let alert = gtk4::AlertDialog::builder()
                .message("KeePass Not Enabled")
                .detail("Please enable KeePass integration in Settings first.")
                .modal(true)
                .build();
            alert.show(Some(&window_for_keepass));
            return;
        }

        let Some(kdbx_path) = settings.secrets.kdbx_path.as_ref() else {
            let alert = gtk4::AlertDialog::builder()
                .message("KeePass Database Not Configured")
                .detail("Please select a KeePass database file in Settings.")
                .modal(true)
                .build();
            alert.show(Some(&window_for_keepass));
            return;
        };

        let lookup_key = if name.trim().is_empty() {
            host.to_string()
        } else {
            name.to_string()
        };

        // Get credentials - password and key file can be used together
        let db_password = settings
            .secrets
            .kdbx_password
            .as_ref()
            .map(|p| p.expose_secret());

        // Key file is optional additional authentication
        let key_file = settings.secrets.kdbx_key_file.as_deref();

        // Check if we have at least one credential
        if db_password.is_none() && key_file.is_none() {
            let alert = gtk4::AlertDialog::builder()
                .message("KeePass Credentials Required")
                .detail("Please enter the database password or select a key file in Settings.")
                .modal(true)
                .build();
            alert.show(Some(&window_for_keepass));
            return;
        }

        // Build URL for the entry with correct protocol
        let url = format!("{}://{}", protocol, host);

        // Save to KeePass
        match rustconn_core::secret::KeePassStatus::save_password_to_kdbx(
            kdbx_path,
            db_password,
            key_file,
            &lookup_key,
            username,
            password,
            Some(&url),
        ) {
            Ok(()) => {
                let alert = gtk4::AlertDialog::builder()
                    .message("Password Saved")
                    .detail(format!("Password for '{lookup_key}' saved to KeePass."))
                    .modal(true)
                    .build();
                alert.show(Some(&window_for_keepass));
            }
            Err(e) => {
                let alert = gtk4::AlertDialog::builder()
                    .message("Failed to Save Password")
                    .detail(format!("Error: {e}"))
                    .modal(true)
                    .build();
                alert.show(Some(&window_for_keepass));
            }
        }
    });

    // Connect load from KeePass callback
    let state_for_load = state.clone();
    dialog.connect_load_from_keepass(move |name, host, _protocol| {
        use secrecy::ExposeSecret;

        let state_ref = state_for_load.borrow();
        let settings = state_ref.settings();

        if !settings.secrets.kdbx_enabled {
            return None;
        }

        let kdbx_path = settings.secrets.kdbx_path.as_ref()?;

        let lookup_key = if name.trim().is_empty() {
            host.to_string()
        } else {
            name.to_string()
        };

        // Get credentials - password and key file can be used together
        let db_password = settings
            .secrets
            .kdbx_password
            .as_ref()
            .map(|p| p.expose_secret());

        // Key file is optional additional authentication
        let key_file = settings.secrets.kdbx_key_file.as_deref();

        match rustconn_core::secret::KeePassStatus::get_password_from_kdbx_with_key(
            kdbx_path,
            db_password,
            key_file,
            &lookup_key,
        ) {
            Ok(password) => password,
            Err(e) => {
                eprintln!("Failed to load password from KeePass: {e}");
                None
            }
        }
    });

    let window_clone = window.clone();
    dialog.run(move |result| {
        if let Some(conn) = result {
            if let Ok(mut state_mut) = state.try_borrow_mut() {
                match state_mut.create_connection(conn) {
                    Ok(_) => {
                        // Reload sidebar preserving tree state
                        drop(state_mut);
                        MainWindow::reload_sidebar_preserving_state(&state, &sidebar);
                    }
                    Err(e) => {
                        // Show error in UI dialog with proper transient parent
                        let alert = gtk4::AlertDialog::builder()
                            .message("Error Creating Connection")
                            .detail(&e)
                            .modal(true)
                            .build();
                        alert.show(Some(&window_clone));
                    }
                }
            }
        }
    });
}

/// Shows the new group dialog with optional parent selection
pub fn show_new_group_dialog(
    window: &ApplicationWindow,
    state: SharedAppState,
    sidebar: SharedSidebar,
) {
    show_new_group_dialog_with_parent(window, state, sidebar, None);
}

/// Shows the new group dialog with parent group selection
#[allow(clippy::too_many_lines, clippy::needless_pass_by_value)]
pub fn show_new_group_dialog_with_parent(
    window: &ApplicationWindow,
    state: SharedAppState,
    sidebar: SharedSidebar,
    preselected_parent: Option<Uuid>,
) {
    let entry = gtk4::Entry::new();
    entry.set_placeholder_text(Some("Group name"));

    let group_window = gtk4::Window::builder()
        .title("New Group")
        .transient_for(window)
        .modal(true)
        .default_width(350)
        .build();

    // Create header bar with Cancel/Create buttons
    let header = gtk4::HeaderBar::new();
    let cancel_btn = gtk4::Button::builder().label("Cancel").build();
    let create_btn = gtk4::Button::builder()
        .label("Create")
        .css_classes(["suggested-action"])
        .build();
    header.pack_start(&cancel_btn);
    header.pack_end(&create_btn);
    group_window.set_titlebar(Some(&header));

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    // Group name
    let name_label = Label::new(Some("Group name:"));
    name_label.set_halign(gtk4::Align::Start);
    content.append(&name_label);
    content.append(&entry);

    // Parent group dropdown
    let parent_label = Label::new(Some("Parent group (optional):"));
    parent_label.set_halign(gtk4::Align::Start);
    parent_label.set_margin_top(8);
    content.append(&parent_label);

    let parent_dropdown = gtk4::DropDown::from_strings(&["(None - Root Level)"]);

    // Populate parent dropdown with existing groups
    let state_ref = state.borrow();
    let groups: Vec<_> = state_ref
        .list_groups()
        .iter()
        .map(|g| (*g).clone())
        .collect();
    drop(state_ref);

    let mut group_ids: Vec<Option<Uuid>> = vec![None];
    let mut strings: Vec<String> = vec!["(None - Root Level)".to_string()];
    let mut preselected_index = 0u32;

    for group in &groups {
        let state_ref = state.borrow();
        let path = state_ref
            .get_group_path(group.id)
            .unwrap_or_else(|| group.name.clone());
        drop(state_ref);

        strings.push(path);
        group_ids.push(Some(group.id));

        if preselected_parent == Some(group.id) {
            #[allow(clippy::cast_possible_truncation)]
            {
                preselected_index = (group_ids.len() - 1) as u32;
            }
        }
    }

    let string_list = gtk4::StringList::new(
        &strings
            .iter()
            .map(std::string::String::as_str)
            .collect::<Vec<_>>(),
    );
    parent_dropdown.set_model(Some(&string_list));
    parent_dropdown.set_selected(preselected_index);

    content.append(&parent_dropdown);
    group_window.set_child(Some(&content));

    // Connect cancel button
    let window_clone = group_window.clone();
    cancel_btn.connect_clicked(move |_| {
        window_clone.close();
    });

    // Connect create button
    let state_clone = state.clone();
    let sidebar_clone = sidebar;
    let window_clone = group_window.clone();
    let entry_clone = entry;
    let dropdown_clone = parent_dropdown;
    create_btn.connect_clicked(move |_| {
        let name = entry_clone.text().to_string();
        if name.trim().is_empty() {
            let alert = gtk4::AlertDialog::builder()
                .message("Validation Error")
                .detail("Group name cannot be empty")
                .modal(true)
                .build();
            alert.show(Some(&window_clone));
            return;
        }

        let selected_idx = dropdown_clone.selected() as usize;
        let parent_id = if selected_idx < group_ids.len() {
            group_ids[selected_idx]
        } else {
            None
        };

        if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
            let result = if let Some(pid) = parent_id {
                state_mut.create_group_with_parent(name, pid)
            } else {
                state_mut.create_group(name)
            };

            match result {
                Ok(_) => {
                    drop(state_mut);
                    MainWindow::reload_sidebar_preserving_state(&state_clone, &sidebar_clone);
                    window_clone.close();
                }
                Err(e) => {
                    let alert = gtk4::AlertDialog::builder()
                        .message("Error")
                        .detail(&e)
                        .modal(true)
                        .build();
                    alert.show(Some(&window_clone));
                }
            }
        }
    });

    group_window.present();
}

/// Shows the import dialog
pub fn show_import_dialog(
    window: &ApplicationWindow,
    state: SharedAppState,
    sidebar: SharedSidebar,
) {
    let dialog = ImportDialog::new(Some(&window.clone().upcast()));

    let window_clone = window.clone();
    dialog.run_with_source(move |result, source_name| {
        if let Some(import_result) = result {
            if let Ok(mut state_mut) = state.try_borrow_mut() {
                match state_mut.import_connections_with_source(&import_result, &source_name) {
                    Ok(count) => {
                        drop(state_mut);
                        MainWindow::reload_sidebar_preserving_state(&state, &sidebar);
                        // Show success message with proper transient parent
                        let alert = gtk4::AlertDialog::builder()
                            .message("Import Successful")
                            .detail(format!(
                                "Imported {count} connections to '{source_name}' group"
                            ))
                            .modal(true)
                            .build();
                        alert.show(Some(&window_clone));
                    }
                    Err(e) => {
                        let alert = gtk4::AlertDialog::builder()
                            .message("Import Failed")
                            .detail(&e)
                            .modal(true)
                            .build();
                        alert.show(Some(&window_clone));
                    }
                }
            }
        }
    });
}
