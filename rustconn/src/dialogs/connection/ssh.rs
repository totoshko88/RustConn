//! SSH protocol options for the connection dialog
//!
//! This module provides the SSH-specific UI components including:
//! - Authentication method selection (Password, Public Key, Keyboard Interactive, SSH Agent)
//! - Key source selection (Default, File, Agent)
//! - Connection options (Jump Host, ProxyJump, IdentitiesOnly, ControlMaster)
//! - Session options (Agent Forwarding, X11 Forwarding, Compression, Startup Command)

// These functions are prepared for future refactoring when dialog.rs is further modularized
#![allow(dead_code)]

use super::protocol_layout::ProtocolLayoutBuilder;
use super::widgets::{CheckboxRowBuilder, EntryRowBuilder};
use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, CheckButton, DropDown, Entry, Label, Orientation, StringList};
use libadwaita as adw;
use std::cell::RefCell;
use std::rc::Rc;

use crate::i18n::i18n;

/// Return type for SSH options creation
///
/// Contains all the widgets needed for SSH configuration:
/// - Container box
/// - Auth method dropdown
/// - Key source dropdown
/// - Key file entry and browse button
/// - Agent key dropdown
/// - Jump host dropdown
/// - Proxy entry
/// - Various checkbuttons for options
/// - Startup command and custom options entries
#[allow(clippy::type_complexity)]
pub type SshOptionsWidgets = (
    GtkBox,
    DropDown,    // auth_dropdown
    DropDown,    // key_source_dropdown
    Entry,       // key_entry
    Button,      // key_button
    DropDown,    // agent_key_dropdown
    DropDown,    // jump_host_dropdown
    Entry,       // proxy_entry
    CheckButton, // identities_only
    CheckButton, // control_master
    CheckButton, // agent_forwarding
    CheckButton, // waypipe
    CheckButton, // x11_forwarding
    CheckButton, // compression
    Entry,       // startup_entry
    Entry,       // options_entry
);

/// Creates the SSH options panel using libadwaita components following GNOME HIG.
///
/// The panel is organized into three groups:
/// - Authentication: Method selection, key source, key file/agent selection
/// - Connection: Jump host, ProxyJump, IdentitiesOnly, ControlMaster
/// - Session: Agent Forwarding, X11 Forwarding, Compression, Startup Command, Custom Options
#[must_use]
pub fn create_ssh_options() -> SshOptionsWidgets {
    let (container, content) = ProtocolLayoutBuilder::new().build();

    // === Authentication Group ===
    let (auth_group, auth_dropdown, key_source_dropdown, key_entry, key_button, agent_key_dropdown) =
        create_authentication_group();
    content.append(&auth_group);

    // === Connection Options Group ===
    let (connection_group, jump_host_dropdown, proxy_entry, identities_only, control_master) =
        create_connection_group();
    content.append(&connection_group);

    // === Session Group ===
    let (
        session_group,
        agent_forwarding,
        waypipe,
        x11_forwarding,
        compression,
        startup_entry,
        options_entry,
    ) = create_session_group();
    content.append(&session_group);

    (
        container,
        auth_dropdown,
        key_source_dropdown,
        key_entry,
        key_button,
        agent_key_dropdown,
        jump_host_dropdown,
        proxy_entry,
        identities_only,
        control_master,
        agent_forwarding,
        waypipe,
        x11_forwarding,
        compression,
        startup_entry,
        options_entry,
    )
}

/// Creates the Authentication preferences group
#[allow(clippy::type_complexity)]
fn create_authentication_group() -> (
    adw::PreferencesGroup,
    DropDown,
    DropDown,
    Entry,
    Button,
    DropDown,
) {
    let auth_group = adw::PreferencesGroup::builder()
        .title(i18n("Authentication"))
        .build();

    // Auth method dropdown
    let auth_items: Vec<String> = vec![
        i18n("Password"),
        i18n("Public Key"),
        i18n("Keyboard Interactive"),
        i18n("SSH Agent"),
        i18n("Security Key (FIDO2)"),
    ];
    let auth_refs: Vec<&str> = auth_items.iter().map(String::as_str).collect();
    let auth_list = StringList::new(&auth_refs);
    let auth_dropdown = DropDown::new(Some(auth_list), gtk4::Expression::NONE);
    auth_dropdown.set_selected(0);

    let auth_row = adw::ActionRow::builder()
        .title(i18n("Method"))
        .subtitle(i18n("How to authenticate with the server"))
        .build();
    auth_row.add_suffix(&auth_dropdown);
    auth_group.add(&auth_row);

    // Key source dropdown
    let key_source_items: Vec<String> = vec![i18n("Default"), i18n("File"), i18n("Agent")];
    let key_source_refs: Vec<&str> = key_source_items.iter().map(String::as_str).collect();
    let key_source_list = StringList::new(&key_source_refs);
    let key_source_dropdown = DropDown::new(Some(key_source_list), gtk4::Expression::NONE);
    key_source_dropdown.set_selected(0);

    let key_source_row = adw::ActionRow::builder()
        .title(i18n("Key Source"))
        .subtitle(i18n("Where to get the SSH key from"))
        .build();
    key_source_row.add_suffix(&key_source_dropdown);
    auth_group.add(&key_source_row);

    // Key file entry with browse button
    let key_entry = Entry::builder()
        .hexpand(true)
        .placeholder_text(i18n("Path to SSH key"))
        .valign(gtk4::Align::Center)
        .build();
    let key_button = Button::builder()
        .icon_name("folder-open-symbolic")
        .tooltip_text(i18n("Browse for key file"))
        .valign(gtk4::Align::Center)
        .build();

    let key_file_row = adw::ActionRow::builder()
        .title(i18n("Key File"))
        .subtitle(i18n("Path to private key file"))
        .build();
    key_file_row.add_suffix(&key_entry);
    key_file_row.add_suffix(&key_button);
    auth_group.add(&key_file_row);

    // Agent key dropdown
    let no_keys_items: Vec<String> = vec![i18n("(No keys loaded)")];
    let no_keys_refs: Vec<&str> = no_keys_items.iter().map(String::as_str).collect();
    let agent_key_list = StringList::new(&no_keys_refs);
    let agent_key_dropdown = DropDown::new(Some(agent_key_list), gtk4::Expression::NONE);
    agent_key_dropdown.set_selected(0);
    agent_key_dropdown.set_sensitive(false);
    agent_key_dropdown.set_hexpand(false);

    let agent_key_row = adw::ActionRow::builder()
        .title(i18n("Key"))
        .subtitle(i18n("Select from SSH agent"))
        .build();
    agent_key_row.add_suffix(&agent_key_dropdown);
    auth_group.add(&agent_key_row);

    // Connect key source dropdown to show/hide appropriate fields
    connect_key_source_visibility(
        &key_source_dropdown,
        &key_file_row,
        &agent_key_row,
        &key_entry,
        &key_button,
        &agent_key_dropdown,
    );

    // Connect auth method dropdown to show/hide key-related rows
    connect_auth_method_visibility(
        &auth_dropdown,
        &key_source_row,
        &key_file_row,
        &agent_key_row,
        &agent_key_dropdown,
    );

    // Set initial state (Password selected - hide key source)
    key_source_row.set_visible(false);
    key_file_row.set_visible(false);
    agent_key_row.set_visible(false);
    key_entry.set_sensitive(false);
    key_button.set_sensitive(false);
    agent_key_dropdown.set_sensitive(false);

    (
        auth_group,
        auth_dropdown,
        key_source_dropdown,
        key_entry,
        key_button,
        agent_key_dropdown,
    )
}

/// Connects key source dropdown to show/hide appropriate fields
fn connect_key_source_visibility(
    key_source_dropdown: &DropDown,
    key_file_row: &adw::ActionRow,
    agent_key_row: &adw::ActionRow,
    key_entry: &Entry,
    key_button: &Button,
    agent_key_dropdown: &DropDown,
) {
    let key_file_row_clone = key_file_row.clone();
    let agent_key_row_clone = agent_key_row.clone();
    let key_entry_clone = key_entry.clone();
    let key_button_clone = key_button.clone();
    let agent_key_dropdown_clone = agent_key_dropdown.clone();

    key_source_dropdown.connect_selected_notify(move |dropdown| {
        let selected = dropdown.selected();
        match selected {
            0 => {
                // Default - hide both rows
                key_file_row_clone.set_visible(false);
                agent_key_row_clone.set_visible(false);
                key_entry_clone.set_sensitive(false);
                key_button_clone.set_sensitive(false);
                agent_key_dropdown_clone.set_sensitive(false);
            }
            1 => {
                // File - show file row, hide agent row
                key_file_row_clone.set_visible(true);
                agent_key_row_clone.set_visible(false);
                key_entry_clone.set_sensitive(true);
                key_button_clone.set_sensitive(true);
                agent_key_dropdown_clone.set_sensitive(false);
            }
            2 => {
                // Agent - hide file row, show agent row
                key_file_row_clone.set_visible(false);
                agent_key_row_clone.set_visible(true);
                key_entry_clone.set_sensitive(false);
                key_button_clone.set_sensitive(false);
                agent_key_dropdown_clone.set_sensitive(true);
            }
            _ => {}
        }
    });
}

/// Connects auth method dropdown to show/hide key-related rows
fn connect_auth_method_visibility(
    auth_dropdown: &DropDown,
    key_source_row: &adw::ActionRow,
    key_file_row: &adw::ActionRow,
    agent_key_row: &adw::ActionRow,
    agent_key_dropdown: &DropDown,
) {
    let key_source_row_clone = key_source_row.clone();
    let key_file_row_clone = key_file_row.clone();
    let agent_key_row_clone = agent_key_row.clone();
    let agent_key_dropdown_clone = agent_key_dropdown.clone();

    auth_dropdown.connect_selected_notify(move |dropdown| {
        let selected = dropdown.selected();
        match selected {
            0 => {
                // Password - hide all key-related rows
                key_source_row_clone.set_visible(false);
                key_file_row_clone.set_visible(false);
                agent_key_row_clone.set_visible(false);
            }
            3 => {
                // SSH Agent - hide key source, show agent key directly
                key_source_row_clone.set_visible(false);
                key_file_row_clone.set_visible(false);
                agent_key_row_clone.set_visible(true);
                agent_key_dropdown_clone.set_sensitive(true);
            }
            4 => {
                // Security Key (FIDO2) - show key file row for sk key path
                key_source_row_clone.set_visible(false);
                key_file_row_clone.set_visible(true);
                agent_key_row_clone.set_visible(false);
            }
            _ => {
                // Public Key, Keyboard Interactive - show key source row
                key_source_row_clone.set_visible(true);
                // Key file/agent rows visibility is controlled by key_source_dropdown
            }
        }
    });
}

/// Creates the Connection preferences group
fn create_connection_group() -> (
    adw::PreferencesGroup,
    DropDown,
    Entry,
    CheckButton,
    CheckButton,
) {
    let connection_group = adw::PreferencesGroup::builder()
        .title(i18n("Connection"))
        .build();

    // Jump Host dropdown
    let none_items: Vec<String> = vec![i18n("(None)")];
    let none_refs: Vec<&str> = none_items.iter().map(String::as_str).collect();
    let jump_host_list = StringList::new(&none_refs);
    let jump_host_dropdown = DropDown::new(Some(jump_host_list), gtk4::Expression::NONE);
    jump_host_dropdown.set_selected(0);

    let jump_host_row = adw::ActionRow::builder()
        .title(i18n("Jump Host"))
        .subtitle(i18n("Connect via another SSH connection"))
        .build();
    jump_host_row.add_suffix(&jump_host_dropdown);
    connection_group.add(&jump_host_row);

    // ProxyJump entry
    let (proxy_row, proxy_entry) = EntryRowBuilder::new("ProxyJump")
        .subtitle("Jump host for tunneling (-J)")
        .placeholder("user@jumphost")
        .build();
    connection_group.add(&proxy_row);

    // IdentitiesOnly switch
    let (identities_row, identities_only) = CheckboxRowBuilder::new("Use Only Specified Key")
        .subtitle("Prevents trying other keys (IdentitiesOnly)")
        .build();
    connection_group.add(&identities_row);

    // ControlMaster switch
    let (control_master_row, control_master) = CheckboxRowBuilder::new("Connection Multiplexing")
        .subtitle("Reuse connections (ControlMaster)")
        .build();
    connection_group.add(&control_master_row);

    (
        connection_group,
        jump_host_dropdown,
        proxy_entry,
        identities_only,
        control_master,
    )
}

/// Creates the Session preferences group
#[allow(clippy::type_complexity)]
fn create_session_group() -> (
    adw::PreferencesGroup,
    CheckButton,
    CheckButton,
    CheckButton,
    CheckButton,
    Entry,
    Entry,
) {
    let session_group = adw::PreferencesGroup::builder()
        .title(i18n("Session"))
        .build();

    // Agent Forwarding switch
    let (agent_forwarding_row, agent_forwarding) = CheckboxRowBuilder::new("Agent Forwarding")
        .subtitle("Forward SSH agent to remote host (-A)")
        .build();
    session_group.add(&agent_forwarding_row);

    // Waypipe (Wayland application forwarding)
    let (waypipe_row, waypipe) = CheckboxRowBuilder::new("Waypipe")
        .subtitle("Wayland application forwarding via waypipe")
        .build();
    session_group.add(&waypipe_row);

    // X11 Forwarding switch
    let (x11_forwarding_row, x11_forwarding) = CheckboxRowBuilder::new("X11 Forwarding")
        .subtitle("Forward X11 display to local host (-X)")
        .build();
    session_group.add(&x11_forwarding_row);

    // Compression switch
    let (compression_row, compression) = CheckboxRowBuilder::new("Compression")
        .subtitle("Enable compression for slow connections (-C)")
        .build();
    session_group.add(&compression_row);

    // Startup command entry
    let (startup_row, startup_entry) = EntryRowBuilder::new("Startup Command")
        .subtitle("Execute after connection established")
        .placeholder("Command to run on connect")
        .build();
    session_group.add(&startup_row);

    // Custom options entry
    let (options_row, options_entry) = EntryRowBuilder::new("Custom Options")
        .subtitle("Additional SSH command-line options")
        .placeholder("-o Key=Value")
        .build();
    session_group.add(&options_row);

    (
        session_group,
        agent_forwarding,
        waypipe,
        x11_forwarding,
        compression,
        startup_entry,
        options_entry,
    )
}

/// Creates the Port Forwarding preferences group
///
/// Provides UI for managing SSH port forwarding rules (local, remote, dynamic).
/// The list box and data vector are owned by `ConnectionDialog` and passed in.
#[must_use]
pub fn create_port_forwarding_group(
    forwards_list: &gtk4::ListBox,
    forwards_data: &Rc<RefCell<Vec<rustconn_core::models::PortForward>>>,
) -> adw::PreferencesGroup {
    let pf_group = adw::PreferencesGroup::builder()
        .title(i18n("Port Forwarding"))
        .description(i18n("Forward ports through the SSH tunnel"))
        .build();

    // Direction dropdown
    let direction_items: Vec<String> = vec![
        i18n("Local (-L)"),
        i18n("Remote (-R)"),
        i18n("Dynamic (-D)"),
    ];
    let direction_refs: Vec<&str> = direction_items.iter().map(String::as_str).collect();
    let direction_list = StringList::new(&direction_refs);
    let direction_dropdown = DropDown::new(Some(direction_list), gtk4::Expression::NONE);
    direction_dropdown.set_selected(0);

    let direction_row = adw::ActionRow::builder()
        .title(i18n("Direction"))
        .subtitle(i18n("Type of port forwarding"))
        .build();
    direction_row.add_suffix(&direction_dropdown);
    pf_group.add(&direction_row);

    // Local port entry
    let local_port_entry = Entry::builder()
        .placeholder_text("8080")
        .input_purpose(gtk4::InputPurpose::Digits)
        .max_width_chars(6)
        .valign(gtk4::Align::Center)
        .build();
    let local_port_row = adw::ActionRow::builder()
        .title(i18n("Local Port"))
        .subtitle(i18n("Port to bind locally"))
        .build();
    local_port_row.add_suffix(&local_port_entry);
    pf_group.add(&local_port_row);

    // Remote host entry
    let remote_host_entry = Entry::builder()
        .placeholder_text("localhost")
        .valign(gtk4::Align::Center)
        .build();
    let remote_host_row = adw::ActionRow::builder()
        .title(i18n("Remote Host"))
        .subtitle(i18n("Destination host on the remote side"))
        .build();
    remote_host_row.add_suffix(&remote_host_entry);
    pf_group.add(&remote_host_row);

    // Remote port entry
    let remote_port_entry = Entry::builder()
        .placeholder_text("80")
        .input_purpose(gtk4::InputPurpose::Digits)
        .max_width_chars(6)
        .valign(gtk4::Align::Center)
        .build();
    let remote_port_row = adw::ActionRow::builder()
        .title(i18n("Remote Port"))
        .subtitle(i18n("Destination port on the remote side"))
        .build();
    remote_port_row.add_suffix(&remote_port_entry);
    pf_group.add(&remote_port_row);

    // Show/hide remote host/port based on direction
    let remote_host_row_clone = remote_host_row.clone();
    let remote_port_row_clone = remote_port_row.clone();
    direction_dropdown.connect_selected_notify(move |dd| {
        let is_dynamic = dd.selected() == 2;
        remote_host_row_clone.set_visible(!is_dynamic);
        remote_port_row_clone.set_visible(!is_dynamic);
    });

    // Add button
    let add_button = Button::builder()
        .label(i18n("Add Forward"))
        .css_classes(["suggested-action"])
        .build();

    let add_row = adw::ActionRow::builder().build();
    add_row.add_suffix(&add_button);
    pf_group.add(&add_row);

    // Existing forwards list
    pf_group.add(forwards_list);

    // Wire up add button
    let data = forwards_data.clone();
    let list = forwards_list.clone();
    let dir_dd = direction_dropdown.clone();
    let local_port_clone = local_port_entry.clone();
    let remote_host_clone = remote_host_entry.clone();
    let remote_port_clone = remote_port_entry.clone();

    add_button.connect_clicked(move |_| {
        let local_port_text = local_port_clone.text();
        let local_port: u16 = match local_port_text.trim().parse() {
            Ok(p) if p > 0 => p,
            _ => return, // silently ignore invalid input
        };

        let direction = match dir_dd.selected() {
            1 => rustconn_core::models::PortForwardDirection::Remote,
            2 => rustconn_core::models::PortForwardDirection::Dynamic,
            _ => rustconn_core::models::PortForwardDirection::Local,
        };

        let (remote_host, remote_port) = if matches!(
            direction,
            rustconn_core::models::PortForwardDirection::Dynamic
        ) {
            (String::new(), 0)
        } else {
            let rh = remote_host_clone.text().trim().to_string();
            let rh = if rh.is_empty() {
                "localhost".to_string()
            } else {
                rh
            };
            let rp: u16 = remote_port_clone.text().trim().parse().unwrap_or(0);
            if rp == 0 {
                return; // need a valid remote port for L/R
            }
            (rh, rp)
        };

        let pf = rustconn_core::models::PortForward {
            direction,
            local_port,
            remote_host,
            remote_port,
        };

        // Add to data
        data.borrow_mut().push(pf.clone());

        // Add row to list
        add_port_forward_row_to_list(&list, &data, data.borrow().len() - 1, &pf);

        // Clear inputs
        local_port_clone.set_text("");
        remote_host_clone.set_text("");
        remote_port_clone.set_text("");
    });

    pf_group
}

/// Adds a single port forward row to the list box
pub fn add_port_forward_row_to_list(
    list: &gtk4::ListBox,
    data: &Rc<RefCell<Vec<rustconn_core::models::PortForward>>>,
    idx: usize,
    pf: &rustconn_core::models::PortForward,
) {
    let row_box = GtkBox::new(Orientation::Horizontal, 8);
    row_box.set_margin_top(4);
    row_box.set_margin_bottom(4);
    row_box.set_margin_start(8);
    row_box.set_margin_end(8);

    let summary_label = Label::builder()
        .label(&pf.display_summary())
        .hexpand(true)
        .halign(gtk4::Align::Start)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .build();

    let remove_button = Button::builder()
        .icon_name("user-trash-symbolic")
        .css_classes(["flat"])
        .tooltip_text(i18n("Remove this port forward"))
        .build();

    let data_clone = data.clone();
    let list_clone = list.clone();
    remove_button.connect_clicked(move |_| {
        let mut forwards = data_clone.borrow_mut();
        if idx < forwards.len() {
            forwards.remove(idx);
        }
        drop(forwards);
        // Rebuild list
        while let Some(child) = list_clone.first_child() {
            list_clone.remove(&child);
        }
        let forwards = data_clone.borrow();
        for (i, f) in forwards.iter().enumerate() {
            add_port_forward_row_to_list(&list_clone, &data_clone, i, f);
        }
    });

    row_box.append(&summary_label);
    row_box.append(&remove_button);
    list.append(&row_box);
}
