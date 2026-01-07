//! SSH Agent settings tab using libadwaita components

use adw::prelude::*;
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

/// Loads SSH agent settings into UI controls
pub fn load_ssh_agent_settings(
    ssh_agent_status_label: &Label,
    ssh_agent_socket_label: &Label,
    ssh_agent_keys_list: &ListBox,
    ssh_agent_manager: &Rc<RefCell<SshAgentManager>>,
) {
    let manager = ssh_agent_manager.borrow();

    if let Ok(status) = manager.get_status() {
        let status_text = if status.running {
            "Running"
        } else {
            "Not running"
        };
        ssh_agent_status_label.set_text(status_text);

        ssh_agent_status_label.remove_css_class("error");
        ssh_agent_status_label.remove_css_class("success");
        ssh_agent_status_label.remove_css_class("dim-label");

        if status.running {
            ssh_agent_status_label.add_css_class("success");
        } else {
            ssh_agent_status_label.add_css_class("dim-label");
        }
    } else {
        ssh_agent_status_label.set_text("Error");
        ssh_agent_status_label.add_css_class("error");
    }

    if let Some(socket) = manager.socket_path() {
        ssh_agent_socket_label.set_text(socket);
    } else {
        ssh_agent_socket_label.set_text("Not available");
    }

    // Clear existing keys
    while let Some(child) = ssh_agent_keys_list.first_child() {
        ssh_agent_keys_list.remove(&child);
    }

    if let Ok(status) = manager.get_status() {
        if status.running {
            if status.keys.is_empty() {
                let empty_row = adw::ActionRow::builder()
                    .title("No keys loaded")
                    .subtitle("Add keys using ssh-add or the button above")
                    .build();
                ssh_agent_keys_list.append(&empty_row);
            } else {
                for key in &status.keys {
                    let key_row = create_loaded_key_row(key);
                    ssh_agent_keys_list.append(&key_row);
                }
            }
        } else {
            let empty_row = adw::ActionRow::builder()
                .title("Agent not running")
                .subtitle("Start the agent to manage keys")
                .build();
            ssh_agent_keys_list.append(&empty_row);
        }
    } else {
        let empty_row = adw::ActionRow::builder()
            .title("Agent not running")
            .subtitle("Start the agent to manage keys")
            .build();
        ssh_agent_keys_list.append(&empty_row);
    }
}

/// Populates the available keys list with load buttons
pub fn populate_available_keys_list(
    available_keys_list: &ListBox,
    ssh_agent_manager: &Rc<RefCell<SshAgentManager>>,
    ssh_agent_keys_list: &ListBox,
    ssh_agent_status_label: &Label,
    ssh_agent_socket_label: &Label,
) {
    // Clear existing items
    while let Some(child) = available_keys_list.first_child() {
        available_keys_list.remove(&child);
    }

    match SshAgentManager::list_key_files() {
        Ok(key_files) if key_files.is_empty() => {
            let empty_row = adw::ActionRow::builder()
                .title("No SSH key files found")
                .subtitle("Generate keys with ssh-keygen")
                .build();
            available_keys_list.append(&empty_row);
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
                let manager_clone = ssh_agent_manager.clone();
                let keys_list_clone = ssh_agent_keys_list.clone();
                let status_label_clone = ssh_agent_status_label.clone();
                let socket_label_clone = ssh_agent_socket_label.clone();
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
                available_keys_list.append(&key_row);
            }
        }
        Err(_) => {
            let error_row = adw::ActionRow::builder()
                .title("Failed to scan ~/.ssh/ directory")
                .build();
            error_row.add_css_class("error");
            available_keys_list.append(&error_row);
        }
    }
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

    let main_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();

    let header = adw::HeaderBar::builder().show_end_title_buttons(false).build();

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

    main_box.append(&header);
    main_box.append(&content);
    dialog.set_content(Some(&main_box));

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
    let parent_window_clone = parent_window.clone();

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
                // Show error toast on parent window if it's a PreferencesWindow
                if let Some(pref_window) = parent_window_clone.downcast_ref::<adw::PreferencesWindow>()
                {
                    pref_window.add_toast(adw::Toast::new(&format!("Failed to add key: {e}")));
                }
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

fn create_loaded_key_row(key: &rustconn_core::ssh_agent::AgentKey) -> adw::ActionRow {
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

    let fingerprint = key.fingerprint.clone();
    remove_button.connect_clicked(move |_| {
        tracing::info!("Remove key requested: {}", fingerprint);
    });

    row.add_suffix(&remove_button);
    row
}
