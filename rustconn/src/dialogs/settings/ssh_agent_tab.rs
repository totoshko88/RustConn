//! SSH Agent settings tab using libadwaita components
//!
//! SSH agent status and key loading is performed asynchronously to avoid blocking the UI.

use adw::prelude::*;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, Label, ListBox, Orientation, Spinner};
use libadwaita as adw;
use rustconn_core::ssh_agent::SshAgentManager;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

/// Creates the SSH Agent settings page using AdwPreferencesPage
#[allow(clippy::type_complexity)]
pub fn create_ssh_agent_page() -> (
    adw::PreferencesPage,
    Label,
    Label,
    Button,
    ListBox,
    Button,
    Spinner,
    Label,
    Button,
    ListBox, // available_keys_list
) {
    let page = adw::PreferencesPage::builder()
        .title("SSH Agent")
        .icon_name("network-server-symbolic")
        .build();

    // === Agent Status Group ===
    let status_group = adw::PreferencesGroup::builder()
        .title("Agent Status")
        .build();

    let ssh_agent_status_label = Label::builder()
        .label("Checking...")
        .halign(gtk4::Align::End)
        .valign(gtk4::Align::Center)
        .build();
    let status_row = adw::ActionRow::builder().title("Status").build();
    status_row.add_suffix(&ssh_agent_status_label);
    status_group.add(&status_row);

    let ssh_agent_socket_label = Label::builder()
        .label("")
        .halign(gtk4::Align::End)
        .valign(gtk4::Align::Center)
        .css_classes(["dim-label"])
        .selectable(true)
        .ellipsize(gtk4::pango::EllipsizeMode::Middle)
        .max_width_chars(40)
        .build();
    let socket_row = adw::ActionRow::builder().title("Socket").build();
    socket_row.add_suffix(&ssh_agent_socket_label);
    status_group.add(&socket_row);

    // Control buttons row
    let ssh_agent_start_button = Button::builder()
        .label("Start Agent")
        .valign(gtk4::Align::Center)
        .build();
    let ssh_agent_refresh_button = Button::builder()
        .icon_name("view-refresh-symbolic")
        .valign(gtk4::Align::Center)
        .tooltip_text("Refresh status")
        .build();

    let buttons_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .valign(gtk4::Align::Center)
        .build();
    buttons_box.append(&ssh_agent_start_button);
    buttons_box.append(&ssh_agent_refresh_button);

    let control_row = adw::ActionRow::builder().title("Controls").build();
    control_row.add_suffix(&buttons_box);
    status_group.add(&control_row);

    page.add(&status_group);

    // === Loaded Keys Group ===
    let keys_group = adw::PreferencesGroup::builder()
        .title("Loaded Keys")
        .description("Keys currently loaded in the SSH agent")
        .build();

    let ssh_agent_keys_list = ListBox::builder()
        .selection_mode(gtk4::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    keys_group.add(&ssh_agent_keys_list);

    let ssh_agent_loading_spinner = Spinner::new();
    let ssh_agent_error_label = Label::builder()
        .label("")
        .halign(gtk4::Align::Start)
        .css_classes(["error"])
        .build();

    // Add Key button
    let ssh_agent_add_key_button = Button::builder()
        .label("Add Key")
        .valign(gtk4::Align::Center)
        .css_classes(["suggested-action"])
        .build();
    let add_key_row = adw::ActionRow::builder()
        .title("Add SSH Key")
        .subtitle("Load a key from file")
        .activatable(true)
        .build();
    add_key_row.add_suffix(&ssh_agent_add_key_button);
    keys_group.add(&add_key_row);

    page.add(&keys_group);

    // === Available Key Files Group ===
    let available_group = adw::PreferencesGroup::builder()
        .title("Available Key Files")
        .description("Key files found in ~/.ssh/")
        .build();

    let available_keys_list = ListBox::builder()
        .selection_mode(gtk4::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();

    available_group.add(&available_keys_list);
    page.add(&available_group);

    (
        page,
        ssh_agent_status_label,
        ssh_agent_socket_label,
        ssh_agent_start_button,
        ssh_agent_keys_list,
        ssh_agent_add_key_button,
        ssh_agent_loading_spinner,
        ssh_agent_error_label,
        ssh_agent_refresh_button,
        available_keys_list,
    )
}

/// Loads SSH agent settings into UI controls asynchronously
pub fn load_ssh_agent_settings(
    ssh_agent_status_label: &Label,
    ssh_agent_socket_label: &Label,
    ssh_agent_keys_list: &ListBox,
    ssh_agent_manager: &Rc<RefCell<SshAgentManager>>,
) {
    // Show loading state immediately
    ssh_agent_status_label.set_text("Checking...");
    ssh_agent_status_label.remove_css_class("error");
    ssh_agent_status_label.remove_css_class("success");
    ssh_agent_status_label.add_css_class("dim-label");
    ssh_agent_socket_label.set_text("...");

    // Clear existing keys and show loading
    while let Some(child) = ssh_agent_keys_list.first_child() {
        ssh_agent_keys_list.remove(&child);
    }
    let loading_row = adw::ActionRow::builder().title("Loading keys...").build();
    let spinner = Spinner::builder().spinning(true).build();
    loading_row.add_prefix(&spinner);
    ssh_agent_keys_list.append(&loading_row);

    // Clone for async closure
    let status_label = ssh_agent_status_label.clone();
    let socket_label = ssh_agent_socket_label.clone();
    let keys_list = ssh_agent_keys_list.clone();
    let manager = ssh_agent_manager.clone();

    // Run status check asynchronously
    glib::spawn_future_local(async move {
        // Get status in background
        let status_result = {
            let mgr = manager.borrow();
            let socket = mgr.socket_path().map(String::from);
            let status = mgr.get_status();
            (socket, status)
        };

        // Update UI on main thread
        let (socket, status) = status_result;

        if let Some(socket_path) = socket {
            socket_label.set_text(&socket_path);
        } else {
            socket_label.set_text("Not available");
        }

        // Clear loading row
        while let Some(child) = keys_list.first_child() {
            keys_list.remove(&child);
        }

        if let Ok(agent_status) = status {
            let status_text = if agent_status.running {
                "Running"
            } else {
                "Not running"
            };
            status_label.set_text(status_text);
            status_label.remove_css_class("error");
            status_label.remove_css_class("dim-label");

            if agent_status.running {
                status_label.add_css_class("success");

                if agent_status.keys.is_empty() {
                    let empty_row = adw::ActionRow::builder()
                        .title("No keys loaded")
                        .subtitle("Add keys using ssh-add or the button above")
                        .build();
                    keys_list.append(&empty_row);
                } else {
                    for key in &agent_status.keys {
                        let key_row = create_loaded_key_row(
                            key,
                            &manager,
                            &keys_list,
                            &status_label,
                            &socket_label,
                        );
                        keys_list.append(&key_row);
                    }
                }
            } else {
                status_label.add_css_class("dim-label");
                let empty_row = adw::ActionRow::builder()
                    .title("Agent not running")
                    .subtitle("Start the agent to manage keys")
                    .build();
                keys_list.append(&empty_row);
            }
        } else {
            status_label.set_text("Error");
            status_label.remove_css_class("dim-label");
            status_label.add_css_class("error");

            let empty_row = adw::ActionRow::builder()
                .title("Agent not running")
                .subtitle("Start the agent to manage keys")
                .build();
            keys_list.append(&empty_row);
        }
    });
}

/// Populates the available keys list with load buttons asynchronously
pub fn populate_available_keys_list(
    available_keys_list: &ListBox,
    ssh_agent_manager: &Rc<RefCell<SshAgentManager>>,
    ssh_agent_keys_list: &ListBox,
    ssh_agent_status_label: &Label,
    ssh_agent_socket_label: &Label,
) {
    // Clear existing items and show loading
    while let Some(child) = available_keys_list.first_child() {
        available_keys_list.remove(&child);
    }
    let loading_row = adw::ActionRow::builder()
        .title("Scanning ~/.ssh/...")
        .build();
    let spinner = Spinner::builder().spinning(true).build();
    loading_row.add_prefix(&spinner);
    available_keys_list.append(&loading_row);

    // Clone for async closure
    let keys_list = available_keys_list.clone();
    let manager = ssh_agent_manager.clone();
    let agent_keys_list = ssh_agent_keys_list.clone();
    let status_label = ssh_agent_status_label.clone();
    let socket_label = ssh_agent_socket_label.clone();

    glib::spawn_future_local(async move {
        // List key files (this is a quick filesystem operation but still async for consistency)
        let key_files_result = SshAgentManager::list_key_files();

        // Clear loading row
        while let Some(child) = keys_list.first_child() {
            keys_list.remove(&child);
        }

        match key_files_result {
            Ok(key_files) if key_files.is_empty() => {
                let empty_row = adw::ActionRow::builder()
                    .title("No SSH key files found")
                    .subtitle("Generate keys with ssh-keygen")
                    .build();
                keys_list.append(&empty_row);
            }
            Ok(key_files) => {
                for key_file in key_files {
                    let key_name = key_file
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("Unknown")
                        .to_string();
                    let key_path_str = key_file.display().to_string();

                    let key_row = adw::ActionRow::builder()
                        .title(&key_name)
                        .subtitle(&key_path_str)
                        .build();

                    let load_button = Button::builder()
                        .icon_name("list-add-symbolic")
                        .valign(gtk4::Align::Center)
                        .tooltip_text("Load this key")
                        .build();

                    // Connect load button handler
                    let manager_clone = manager.clone();
                    let keys_list_clone = agent_keys_list.clone();
                    let status_label_clone = status_label.clone();
                    let socket_label_clone = socket_label.clone();
                    let key_path = key_file.clone();

                    load_button.connect_clicked(move |button| {
                        add_key_with_passphrase_dialog(
                            button,
                            &key_path,
                            &manager_clone,
                            &keys_list_clone,
                            &status_label_clone,
                            &socket_label_clone,
                        );
                    });

                    key_row.add_suffix(&load_button);
                    keys_list.append(&key_row);
                }
            }
            Err(_) => {
                let error_row = adw::ActionRow::builder()
                    .title("Failed to scan ~/.ssh/ directory")
                    .build();
                error_row.add_css_class("error");
                keys_list.append(&error_row);
            }
        }
    });
}

/// Shows a passphrase dialog and adds the key to the agent
fn add_key_with_passphrase_dialog(
    button: &Button,
    key_path: &Path,
    ssh_agent_manager: &Rc<RefCell<SshAgentManager>>,
    ssh_agent_keys_list: &ListBox,
    ssh_agent_status_label: &Label,
    ssh_agent_socket_label: &Label,
) {
    // Get the window for the dialog
    let Some(root) = button.root() else {
        tracing::error!("Cannot get root window for passphrase dialog");
        return;
    };
    let Some(parent_window) = root.downcast_ref::<gtk4::Window>() else {
        tracing::error!("Root is not a Window");
        return;
    };

    let key_name = key_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("key")
        .to_string();

    // Create passphrase dialog using adw::Window
    let dialog = adw::Window::builder()
        .title(&format!("Add Key: {key_name}"))
        .transient_for(parent_window)
        .modal(true)
        .default_width(400)
        .default_height(180)
        .build();

    let toolbar_view = adw::ToolbarView::new();

    let header = adw::HeaderBar::builder()
        .show_end_title_buttons(false)
        .build();

    let content = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    let body_label = Label::builder()
        .label("Enter passphrase (leave empty if key has no passphrase)")
        .wrap(true)
        .halign(gtk4::Align::Start)
        .build();

    let passphrase_entry = gtk4::PasswordEntry::builder()
        .placeholder_text("Passphrase (optional)")
        .show_peek_icon(true)
        .hexpand(true)
        .build();

    let button_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .halign(gtk4::Align::End)
        .build();

    let cancel_button = Button::builder().label("Cancel").build();
    let add_button = Button::builder()
        .label("Add Key")
        .css_classes(["suggested-action"])
        .build();

    button_box.append(&cancel_button);
    button_box.append(&add_button);

    content.append(&body_label);
    content.append(&passphrase_entry);
    content.append(&button_box);

    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&content));
    dialog.set_content(Some(&toolbar_view));

    // Connect cancel button
    let dialog_clone = dialog.clone();
    cancel_button.connect_clicked(move |_| {
        dialog_clone.close();
    });

    // Connect add button
    let manager_clone = ssh_agent_manager.clone();
    let keys_list_clone = ssh_agent_keys_list.clone();
    let status_label_clone = ssh_agent_status_label.clone();
    let socket_label_clone = ssh_agent_socket_label.clone();
    let key_path_clone = key_path.to_path_buf();
    let dialog_clone2 = dialog.clone();

    add_button.connect_clicked(move |_| {
        let passphrase_text = passphrase_entry.text();
        let passphrase = if passphrase_text.is_empty() {
            None
        } else {
            Some(passphrase_text.as_str())
        };

        let manager = manager_clone.borrow();
        match manager.add_key(&key_path_clone, passphrase) {
            Ok(()) => {
                tracing::info!("Key added successfully: {}", key_path_clone.display());
                dialog_clone2.close();
                // Refresh the keys list
                drop(manager);
                load_ssh_agent_settings(
                    &status_label_clone,
                    &socket_label_clone,
                    &keys_list_clone,
                    &manager_clone,
                );
            }
            Err(e) => {
                tracing::error!("Failed to add key: {e}");
                // Log the error - toast notifications are handled by the parent dialog
                // The user will see the key wasn't added when the list doesn't update
                dialog_clone2.close();
            }
        }
    });

    dialog.present();
}

/// Shows a file chooser dialog to add a key from any location
pub fn show_add_key_file_chooser(
    button: &Button,
    ssh_agent_manager: &Rc<RefCell<SshAgentManager>>,
    ssh_agent_keys_list: &ListBox,
    ssh_agent_status_label: &Label,
    ssh_agent_socket_label: &Label,
) {
    let Some(root) = button.root() else {
        tracing::error!("Cannot get root window for file chooser");
        return;
    };
    let Some(window) = root.downcast_ref::<gtk4::Window>() else {
        tracing::error!("Root is not a Window");
        return;
    };

    let file_dialog = gtk4::FileDialog::builder()
        .title("Select SSH Key File")
        .modal(true)
        .build();

    // Set initial folder to ~/.ssh if it exists
    if let Some(home) = dirs::home_dir() {
        let ssh_dir = home.join(".ssh");
        if ssh_dir.exists() {
            let file = gtk4::gio::File::for_path(&ssh_dir);
            file_dialog.set_initial_folder(Some(&file));
        }
    }

    let manager_clone = ssh_agent_manager.clone();
    let keys_list_clone = ssh_agent_keys_list.clone();
    let status_label_clone = ssh_agent_status_label.clone();
    let socket_label_clone = ssh_agent_socket_label.clone();
    let button_clone = button.clone();

    file_dialog.open(Some(window), gtk4::gio::Cancellable::NONE, move |result| {
        if let Ok(file) = result {
            if let Some(path) = file.path() {
                add_key_with_passphrase_dialog(
                    &button_clone,
                    &path,
                    &manager_clone,
                    &keys_list_clone,
                    &status_label_clone,
                    &socket_label_clone,
                );
            }
        }
    });
}

fn create_loaded_key_row(
    key: &rustconn_core::ssh_agent::AgentKey,
    ssh_agent_manager: &Rc<RefCell<SshAgentManager>>,
    ssh_agent_keys_list: &ListBox,
    ssh_agent_status_label: &Label,
    ssh_agent_socket_label: &Label,
) -> adw::ActionRow {
    let title = format!("{} ({} bits)", key.key_type, key.bits);
    let subtitle = if key.comment.is_empty() {
        format!("SHA256:{}", key.fingerprint)
    } else {
        format!("{} • SHA256:{}", key.comment, key.fingerprint)
    };

    let row = adw::ActionRow::builder()
        .title(&title)
        .subtitle(&subtitle)
        .build();

    let remove_button = Button::builder()
        .icon_name("user-trash-symbolic")
        .valign(gtk4::Align::Center)
        .tooltip_text("Remove from agent")
        .css_classes(["destructive-action", "flat"])
        .build();

    // Connect remove button handler
    let manager_clone = ssh_agent_manager.clone();
    let keys_list_clone = ssh_agent_keys_list.clone();
    let status_label_clone = ssh_agent_status_label.clone();
    let socket_label_clone = ssh_agent_socket_label.clone();
    let comment = key.comment.clone();

    remove_button.connect_clicked(move |button| {
        // Try to find the key file path from comment (usually contains the path)
        // If comment is empty or doesn't look like a path, we need to use fingerprint
        let key_path = if !comment.is_empty() && comment.contains('/') {
            std::path::PathBuf::from(&comment)
        } else {
            // Try common SSH key locations
            if let Some(home) = dirs::home_dir() {
                let ssh_dir = home.join(".ssh");
                // Try to find key by fingerprint in available keys
                if let Ok(key_files) = SshAgentManager::list_key_files() {
                    // For now, we'll use ssh-add -d with the fingerprint directly
                    // This requires a different approach - use ssh-add -D to remove all
                    // or find the key file that matches
                    for key_file in key_files {
                        // We can't easily match fingerprint to file without loading each key
                        // So we'll try the comment as a hint
                        if key_file.to_string_lossy().contains(&comment) {
                            return remove_key_and_refresh(
                                button,
                                &key_file,
                                &manager_clone,
                                &keys_list_clone,
                                &status_label_clone,
                                &socket_label_clone,
                            );
                        }
                    }
                }
                ssh_dir.join("id_rsa") // fallback
            } else {
                std::path::PathBuf::from(&comment)
            }
        };

        remove_key_and_refresh(
            button,
            &key_path,
            &manager_clone,
            &keys_list_clone,
            &status_label_clone,
            &socket_label_clone,
        );
    });

    row.add_suffix(&remove_button);
    row
}

/// Helper function to remove a key and refresh the UI
fn remove_key_and_refresh(
    _button: &Button,
    key_path: &std::path::Path,
    ssh_agent_manager: &Rc<RefCell<SshAgentManager>>,
    ssh_agent_keys_list: &ListBox,
    ssh_agent_status_label: &Label,
    ssh_agent_socket_label: &Label,
) {
    let manager = ssh_agent_manager.borrow();
    match manager.remove_key(key_path) {
        Ok(()) => {
            tracing::info!("Key removed successfully: {}", key_path.display());
            drop(manager);
            // Refresh the keys list
            load_ssh_agent_settings(
                ssh_agent_status_label,
                ssh_agent_socket_label,
                ssh_agent_keys_list,
                ssh_agent_manager,
            );
        }
        Err(e) => {
            tracing::error!("Failed to remove key: {e}");
            // Show error toast on parent dialog if available
            // Note: PreferencesDialog doesn't inherit from Root, so we just log the error
            tracing::error!("Failed to remove key: {e}");
        }
    }
}
