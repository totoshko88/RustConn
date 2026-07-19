//! SPICE protocol options for the connection dialog
//!
//! This module provides the SPICE-specific UI components including:
//! - TLS encryption settings
//! - CA certificate configuration
//! - USB redirection
//! - Clipboard sharing
//! - Image compression settings
//! - Shared folders management
//! - Jump host selection

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, DropDown, Entry, FileDialog, Label, Orientation, ScrolledWindow,
    StringList,
};
use libadwaita as adw;
use rustconn_core::models::SharedFolder;

use crate::i18n::i18n;

/// Creates the SPICE options panel using libadwaita components following GNOME HIG.
#[expect(
    clippy::too_many_lines,
    clippy::type_complexity,
    reason = "long match dispatch returning a documented tuple; aliasing would obscure the data flow"
)]
pub(super) fn create_spice_options() -> (
    GtkBox,
    adw::SwitchRow,
    Entry,
    adw::SwitchRow,
    adw::SwitchRow,
    adw::SwitchRow,
    DropDown,
    Entry,
    adw::SwitchRow,
    Rc<RefCell<Vec<SharedFolder>>>,
    gtk4::ListBox,
    DropDown,
    adw::SwitchRow,
    Entry,
) {
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .vexpand(true)
        .build();

    let clamp = adw::Clamp::builder()
        .maximum_size(600)
        .tightening_threshold(400)
        .build();

    let content = GtkBox::new(Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    // === Security Group ===
    let security_group = adw::PreferencesGroup::builder()
        .title(i18n("Security"))
        .build();

    // TLS enabled
    let tls_check = adw::SwitchRow::builder()
        .title(i18n("TLS Encryption"))
        .subtitle(i18n("Encrypt connection with TLS"))
        .active(false)
        .build();
    security_group.add(&tls_check);

    // CA certificate path
    let ca_cert_box = GtkBox::new(Orientation::Horizontal, 4);
    ca_cert_box.set_valign(gtk4::Align::Center);
    let ca_cert_entry = Entry::builder()
        .hexpand(true)
        .placeholder_text(i18n("Path to CA certificate"))
        .build();
    let ca_cert_button = Button::builder()
        .icon_name("folder-open-symbolic")
        .tooltip_text(i18n("Browse for certificate"))
        .build();
    ca_cert_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Browse for CA certificate file",
    ))]);
    ca_cert_box.append(&ca_cert_entry);
    ca_cert_box.append(&ca_cert_button);

    let ca_cert_row = adw::ActionRow::builder()
        .title(i18n("CA Certificate"))
        .subtitle(i18n("Certificate authority for TLS verification"))
        .build();
    ca_cert_row.add_suffix(&ca_cert_box);
    security_group.add(&ca_cert_row);

    // Wire browse button — open a file chooser and write the picked path back.
    // Parent window is resolved from the widget tree at click time, mirroring
    // the unix-socket browse button.
    let ca_cert_entry_for_browse = ca_cert_entry.clone();
    ca_cert_button.connect_clicked(move |btn| {
        let file_dialog = FileDialog::builder()
            .title(i18n("Select CA Certificate"))
            .modal(true)
            .build();

        let parent = btn.root().and_downcast::<gtk4::Window>();
        let entry = ca_cert_entry_for_browse.clone();
        file_dialog.open(
            parent.as_ref(),
            gtk4::gio::Cancellable::NONE,
            move |result| {
                if let Ok(file) = result
                    && let Some(path) = file.path()
                {
                    entry.set_text(&path.to_string_lossy());
                }
            },
        );
    });

    // SPICE-2: Inline file validation for CA certificate path
    ca_cert_entry.connect_changed(move |entry| {
        let path_text = entry.text();
        let path_str = path_text.trim();
        if path_str.is_empty() {
            entry.remove_css_class("error");
            entry.set_tooltip_text(None);
        } else {
            let path = std::path::Path::new(path_str);
            if path.exists() {
                entry.remove_css_class("error");
                entry.set_tooltip_text(None);
            } else {
                entry.add_css_class("error");
                entry.set_tooltip_text(Some(&i18n("File not found")));
            }
        }
    });

    // Skip certificate verification
    let skip_verify_check = adw::SwitchRow::builder()
        .title(i18n("Skip Verification"))
        .subtitle(i18n("Disable certificate verification (insecure)"))
        .active(false)
        .build();
    security_group.add(&skip_verify_check);

    content.append(&security_group);

    // === Features Group ===
    let features_group = adw::PreferencesGroup::builder()
        .title(i18n("Features"))
        .build();

    // USB redirection
    let usb_check = adw::SwitchRow::builder()
        .title(i18n("USB Redirection"))
        .subtitle(i18n("Forward USB devices to remote"))
        .active(false)
        .build();
    features_group.add(&usb_check);

    // Clipboard sharing
    let clipboard_check = adw::SwitchRow::builder()
        .title(i18n("Clipboard Sharing"))
        .subtitle(i18n("Synchronize clipboard with remote"))
        .active(true)
        .build();
    features_group.add(&clipboard_check);

    // Image compression
    let comp_items: Vec<String> = vec![
        i18n("Auto"),
        i18n("Off"),
        "GLZ".to_string(),
        "LZ".to_string(),
        "QUIC".to_string(),
    ];
    let comp_strs: Vec<&str> = comp_items.iter().map(String::as_str).collect();
    let compression_list = StringList::new(&comp_strs);
    let compression_dropdown = DropDown::new(Some(compression_list), gtk4::Expression::NONE);
    compression_dropdown.set_selected(0);
    compression_dropdown.set_valign(gtk4::Align::Center);

    let compression_row = adw::ActionRow::builder()
        .title(i18n("Image Compression"))
        .subtitle(i18n("Algorithm for image data"))
        .build();
    compression_row.add_suffix(&compression_dropdown);
    features_group.add(&compression_row);

    // Proxy
    let proxy_entry = Entry::builder()
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .placeholder_text(i18n("http://proxy:3128"))
        .build();
    let proxy_row = adw::ActionRow::builder()
        .title(i18n("SPICE Proxy"))
        .subtitle(i18n(
            "Proxy URL for tunnelled connections (e.g. Proxmox VE)",
        ))
        .build();
    proxy_row.add_suffix(&proxy_entry);
    features_group.add(&proxy_row);

    // Show local cursor
    let show_local_cursor_check = adw::SwitchRow::builder()
        .title(i18n("Show Local Cursor"))
        .subtitle(i18n("Hide to avoid double cursor in embedded mode"))
        .active(true)
        .build();
    features_group.add(&show_local_cursor_check);

    content.append(&features_group);

    // Wire TLS toggle to CA cert and skip verify sensitivity
    let ca_cert_row_clone = ca_cert_row.clone();
    let skip_verify_check_clone = skip_verify_check.clone();
    tls_check.connect_active_notify(move |check| {
        let on = check.is_active();
        ca_cert_row_clone.set_sensitive(on);
        skip_verify_check_clone.set_sensitive(on);
        if !on {
            skip_verify_check_clone.set_active(false);
        }
    });
    ca_cert_row.set_sensitive(false);
    skip_verify_check.set_sensitive(false);

    // === Shared Folders Group ===
    let folders_group = adw::PreferencesGroup::builder()
        .title(i18n("Shared Folders"))
        .description(i18n("Local folders accessible from remote session"))
        .build();

    let folders_list = gtk4::ListBox::builder()
        .selection_mode(gtk4::SelectionMode::Single)
        .css_classes(["boxed-list"])
        .build();
    folders_list.set_placeholder(Some(&Label::new(Some(&i18n("No shared folders")))));

    let folders_scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .min_content_height(80)
        .max_content_height(120)
        .child(&folders_list)
        .build();
    folders_group.add(&folders_scrolled);

    let folders_buttons = GtkBox::new(Orientation::Horizontal, 8);
    folders_buttons.set_halign(gtk4::Align::End);
    folders_buttons.set_margin_top(12);
    let add_folder_btn = Button::builder()
        .label(i18n("Add"))
        .css_classes(["suggested-action"])
        .build();
    let remove_folder_btn = Button::builder()
        .label(i18n("Remove"))
        .sensitive(false)
        .build();
    folders_buttons.append(&add_folder_btn);
    folders_buttons.append(&remove_folder_btn);
    folders_group.add(&folders_buttons);

    content.append(&folders_group);

    let shared_folders: Rc<RefCell<Vec<SharedFolder>>> = Rc::new(RefCell::new(Vec::new()));

    // Connect add folder button
    super::shared_folders::connect_add_folder_button(
        &add_folder_btn,
        &folders_list,
        &shared_folders,
    );

    // Connect remove folder button
    super::shared_folders::connect_remove_folder_button(
        &remove_folder_btn,
        &folders_list,
        &shared_folders,
    );

    // Enable/disable remove button based on selection
    let remove_btn_for_selection = remove_folder_btn;
    folders_list.connect_row_selected(move |_, row| {
        remove_btn_for_selection.set_sensitive(row.is_some());
    });

    // === Connection Group (Jump Host) ===
    let spice_connection_group = adw::PreferencesGroup::builder()
        .title(i18n("Connection"))
        .build();

    // Unix socket switch
    let unix_socket_check = adw::SwitchRow::builder()
        .title(i18n("Unix Socket"))
        .subtitle(i18n("Connect via local unix socket instead of network"))
        .active(false)
        .build();
    spice_connection_group.add(&unix_socket_check);

    // Unix socket path
    let socket_path_box = GtkBox::new(Orientation::Horizontal, 4);
    socket_path_box.set_valign(gtk4::Align::Center);
    let socket_path_entry = Entry::builder()
        .hexpand(true)
        // Literal example path — not translatable
        .placeholder_text("/run/libvirt/qemu/vm-spice.sock")
        .build();
    let socket_path_button = Button::builder()
        .icon_name("folder-open-symbolic")
        .tooltip_text(i18n("Browse for unix socket file"))
        .build();
    socket_path_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Browse for unix socket file",
    ))]);
    socket_path_box.append(&socket_path_entry);
    socket_path_box.append(&socket_path_button);

    let socket_path_row = adw::ActionRow::builder()
        .title(i18n("Socket Path"))
        .subtitle(i18n("Path to SPICE unix socket file"))
        .build();
    socket_path_row.add_suffix(&socket_path_box);
    socket_path_row.set_sensitive(false);
    spice_connection_group.add(&socket_path_row);

    let none_items: Vec<String> = vec![i18n("(None)")];
    let none_refs: Vec<&str> = none_items.iter().map(String::as_str).collect();
    let spice_jump_host_list = StringList::new(&none_refs);
    let spice_jump_host_dropdown =
        DropDown::new(Some(spice_jump_host_list), gtk4::Expression::NONE);
    spice_jump_host_dropdown.set_selected(0);
    spice_jump_host_dropdown.set_enable_search(true);
    spice_jump_host_dropdown.set_size_request(200, -1);
    spice_jump_host_dropdown.set_hexpand(false);

    let spice_jump_host_row = adw::ActionRow::builder()
        .title(i18n("Jump Host"))
        .subtitle(i18n("Tunnel SPICE through an SSH connection"))
        .build();
    spice_jump_host_row.add_suffix(&spice_jump_host_dropdown);
    spice_connection_group.add(&spice_jump_host_row);

    // Wire unix socket toggle — when enabled: show socket path, hide jump host
    let socket_path_row_clone = socket_path_row.clone();
    let spice_jump_host_row_clone = spice_jump_host_row.clone();
    unix_socket_check.connect_active_notify(move |check| {
        let on = check.is_active();
        socket_path_row_clone.set_sensitive(on);
        spice_jump_host_row_clone.set_sensitive(!on);
    });

    // Wire browse button — open a file chooser and write the picked path back
    let socket_path_entry_for_browse = socket_path_entry.clone();
    socket_path_button.connect_clicked(move |btn| {
        let file_dialog = FileDialog::builder()
            .title(i18n("Select SPICE Unix Socket"))
            .modal(true)
            .build();

        // Default to the common libvirt socket directory when it exists
        let default_dir = std::path::Path::new("/run/libvirt/qemu");
        if default_dir.is_dir() {
            file_dialog.set_initial_folder(Some(&gtk4::gio::File::for_path(default_dir)));
        }

        // Parent window is resolved from the widget tree at click time
        let parent = btn.root().and_downcast::<gtk4::Window>();
        let entry = socket_path_entry_for_browse.clone();
        file_dialog.open(
            parent.as_ref(),
            gtk4::gio::Cancellable::NONE,
            move |result| {
                if let Ok(file) = result
                    && let Some(path) = file.path()
                {
                    entry.set_text(&path.to_string_lossy());
                }
            },
        );
    });

    content.append(&spice_connection_group);

    clamp.set_child(Some(&content));
    scrolled.set_child(Some(&clamp));

    let vbox = GtkBox::new(Orientation::Vertical, 0);
    vbox.append(&scrolled);

    (
        vbox,
        tls_check,
        ca_cert_entry,
        skip_verify_check,
        usb_check,
        clipboard_check,
        compression_dropdown,
        proxy_entry,
        show_local_cursor_check,
        shared_folders,
        folders_list,
        spice_jump_host_dropdown,
        unix_socket_check,
        socket_path_entry,
    )
}
