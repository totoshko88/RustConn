//! SSH Agent settings tab

use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, Frame, Label, ListBox, Orientation, ScrolledWindow, Spinner};
use rustconn_core::ssh_agent::SshAgentManager;
use std::cell::RefCell;
use std::rc::Rc;

/// Creates the SSH Agent settings tab
#[allow(clippy::type_complexity)]
pub fn create_ssh_agent_tab() -> (
    ScrolledWindow,
    Label,
    Label,
    Button,
    ListBox,
    Button,
    Spinner,
    Label,
    Button,
) {
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .build();

    let main_vbox = GtkBox::new(Orientation::Vertical, 12);
    main_vbox.set_margin_top(12);
    main_vbox.set_margin_bottom(12);
    main_vbox.set_margin_start(12);
    main_vbox.set_margin_end(12);
    main_vbox.set_valign(gtk4::Align::Start);

    // SSH Agent Status
    let status_frame = Frame::builder()
        .label("SSH Agent Status")
        .margin_bottom(12)
        .build();

    let status_vbox = GtkBox::new(Orientation::Vertical, 6);
    status_vbox.set_margin_top(6);
    status_vbox.set_margin_bottom(6);
    status_vbox.set_margin_start(6);
    status_vbox.set_margin_end(6);

    let ssh_agent_status_label = Label::builder()
        .label("Status: Checking...")
        .halign(gtk4::Align::Start)
        .build();

    let ssh_agent_socket_label = Label::builder()
        .label("Socket: Not available")
        .halign(gtk4::Align::Start)
        .build();

    let control_hbox = GtkBox::new(Orientation::Horizontal, 6);
    let ssh_agent_start_button = Button::with_label("Start Agent");
    let ssh_agent_refresh_button = Button::with_label("Refresh");
    control_hbox.append(&ssh_agent_start_button);
    control_hbox.append(&ssh_agent_refresh_button);

    status_vbox.append(&ssh_agent_status_label);
    status_vbox.append(&ssh_agent_socket_label);
    status_vbox.append(&control_hbox);

    status_frame.set_child(Some(&status_vbox));

    // SSH Keys Management
    let keys_frame = Frame::builder().label("SSH Keys").margin_bottom(12).build();

    let keys_vbox = GtkBox::new(Orientation::Vertical, 6);
    keys_vbox.set_margin_top(6);
    keys_vbox.set_margin_bottom(6);
    keys_vbox.set_margin_start(6);
    keys_vbox.set_margin_end(6);

    let keys_scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .min_content_height(200)
        .build();

    let ssh_agent_keys_list = ListBox::builder()
        .selection_mode(gtk4::SelectionMode::Single)
        .build();

    keys_scrolled.set_child(Some(&ssh_agent_keys_list));

    let keys_control_hbox = GtkBox::new(Orientation::Horizontal, 6);
    let ssh_agent_add_key_button = Button::with_label("Add Key");
    let ssh_agent_loading_spinner = Spinner::new();
    keys_control_hbox.append(&ssh_agent_add_key_button);
    keys_control_hbox.append(&ssh_agent_loading_spinner);

    let ssh_agent_error_label = Label::builder()
        .label("")
        .halign(gtk4::Align::Start)
        .css_classes(["error"])
        .build();

    keys_vbox.append(&keys_scrolled);
    keys_vbox.append(&keys_control_hbox);
    keys_vbox.append(&ssh_agent_error_label);

    keys_frame.set_child(Some(&keys_vbox));

    main_vbox.append(&status_frame);
    main_vbox.append(&keys_frame);

    scrolled.set_child(Some(&main_vbox));

    (
        scrolled,
        ssh_agent_status_label,
        ssh_agent_socket_label,
        ssh_agent_start_button,
        ssh_agent_keys_list,
        ssh_agent_add_key_button,
        ssh_agent_loading_spinner,
        ssh_agent_error_label,
        ssh_agent_refresh_button,
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

    // Update status
    match manager.get_status() {
        Ok(status) => {
            let status_text = if status.running {
                "Status: Running"
            } else {
                "Status: Not running"
            };
            ssh_agent_status_label.set_text(status_text);
        }
        Err(_) => {
            ssh_agent_status_label.set_text("Status: Error checking agent");
        }
    }

    // Update socket path
    if let Some(socket) = manager.socket_path() {
        ssh_agent_socket_label.set_text(&format!("Socket: {socket}"));
    } else {
        ssh_agent_socket_label.set_text("Socket: Not available");
    }

    // Clear existing keys
    while let Some(child) = ssh_agent_keys_list.first_child() {
        ssh_agent_keys_list.remove(&child);
    }

    // Load keys - simplified for now since we need to check what methods are available
    if let Ok(key_files) = SshAgentManager::list_key_files() {
        for key_file in key_files {
            let key_row = GtkBox::new(Orientation::Horizontal, 6);
            key_row.set_margin_top(6);
            key_row.set_margin_bottom(6);
            key_row.set_margin_start(6);
            key_row.set_margin_end(6);

            let key_info = GtkBox::new(Orientation::Vertical, 2);
            let key_path_label = Label::builder()
                .label(&format!("Key: {}", key_file.display()))
                .halign(gtk4::Align::Start)
                .build();

            key_info.append(&key_path_label);

            let remove_button = Button::builder()
                .label("Remove")
                .css_classes(["destructive-action"])
                .build();

            key_row.append(&key_info);
            key_row.append(&remove_button);
            key_row.set_hexpand(true);

            ssh_agent_keys_list.append(&key_row);
        }
    } else {
        let error_row = Label::new(Some("Failed to load SSH key files"));
        ssh_agent_keys_list.append(&error_row);
    }
}
