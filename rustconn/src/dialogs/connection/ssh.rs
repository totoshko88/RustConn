//! SSH protocol options for the connection dialog
//!
//! This module provides the SSH-specific UI components including:
//! - Authentication method selection (Password, Public Key, Keyboard Interactive, SSH Agent)
//! - Key source selection (Default, File, Agent)
//! - Connection options (Jump Host, ProxyJump, IdentitiesOnly, ControlMaster)
//! - Session options (Agent Forwarding, X11 Forwarding, Compression, Startup Command)

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, CheckButton, DropDown, Entry, Orientation, ScrolledWindow, StringList,
};
use libadwaita as adw;

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
        x11_forwarding,
        compression,
        startup_entry,
        options_entry,
    ) = create_session_group();
    content.append(&session_group);

    clamp.set_child(Some(&content));
    scrolled.set_child(Some(&clamp));

    let vbox = GtkBox::new(Orientation::Vertical, 0);
    vbox.append(&scrolled);

    (
        vbox,
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
        .title("Authentication")
        .build();

    // Auth method dropdown
    let auth_list = StringList::new(&[
        "Password",
        "Public Key",
        "Keyboard Interactive",
        "SSH Agent",
    ]);
    let auth_dropdown = DropDown::new(Some(auth_list), gtk4::Expression::NONE);
    auth_dropdown.set_selected(0);

    let auth_row = adw::ActionRow::builder()
        .title("Method")
        .subtitle("How to authenticate with the server")
        .build();
    auth_row.add_suffix(&auth_dropdown);
    auth_group.add(&auth_row);

    // Key source dropdown
    let key_source_list = StringList::new(&["Default", "File", "Agent"]);
    let key_source_dropdown = DropDown::new(Some(key_source_list), gtk4::Expression::NONE);
    key_source_dropdown.set_selected(0);

    let key_source_row = adw::ActionRow::builder()
        .title("Key Source")
        .subtitle("Where to get the SSH key from")
        .build();
    key_source_row.add_suffix(&key_source_dropdown);
    auth_group.add(&key_source_row);

    // Key file entry with browse button
    let key_entry = Entry::builder()
        .hexpand(true)
        .placeholder_text("Path to SSH key")
        .valign(gtk4::Align::Center)
        .build();
    let key_button = Button::builder()
        .icon_name("folder-open-symbolic")
        .tooltip_text("Browse for key file")
        .valign(gtk4::Align::Center)
        .build();

    let key_file_row = adw::ActionRow::builder()
        .title("Key File")
        .subtitle("Path to private key file")
        .build();
    key_file_row.add_suffix(&key_entry);
    key_file_row.add_suffix(&key_button);
    auth_group.add(&key_file_row);

    // Agent key dropdown
    let agent_key_list = StringList::new(&["(No keys loaded)"]);
    let agent_key_dropdown = DropDown::new(Some(agent_key_list), gtk4::Expression::NONE);
    agent_key_dropdown.set_selected(0);
    agent_key_dropdown.set_sensitive(false);
    agent_key_dropdown.set_hexpand(false);

    let agent_key_row = adw::ActionRow::builder()
        .title("Key")
        .subtitle("Select from SSH agent")
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
    let connection_group = adw::PreferencesGroup::builder().title("Connection").build();

    // Jump Host dropdown
    let jump_host_list = StringList::new(&["(None)"]);
    let jump_host_dropdown = DropDown::new(Some(jump_host_list), gtk4::Expression::NONE);
    jump_host_dropdown.set_selected(0);

    let jump_host_row = adw::ActionRow::builder()
        .title("Jump Host")
        .subtitle("Connect via another SSH connection")
        .build();
    jump_host_row.add_suffix(&jump_host_dropdown);
    connection_group.add(&jump_host_row);

    // ProxyJump entry
    let proxy_entry = Entry::builder()
        .hexpand(true)
        .placeholder_text("user@jumphost")
        .valign(gtk4::Align::Center)
        .build();

    let proxy_row = adw::ActionRow::builder()
        .title("ProxyJump")
        .subtitle("Jump host for tunneling (-J)")
        .build();
    proxy_row.add_suffix(&proxy_entry);
    connection_group.add(&proxy_row);

    // IdentitiesOnly switch
    let identities_only = CheckButton::new();
    let identities_row = adw::ActionRow::builder()
        .title("Use Only Specified Key")
        .subtitle("Prevents trying other keys (IdentitiesOnly)")
        .activatable_widget(&identities_only)
        .build();
    identities_row.add_suffix(&identities_only);
    connection_group.add(&identities_row);

    // ControlMaster switch
    let control_master = CheckButton::new();
    let control_master_row = adw::ActionRow::builder()
        .title("Connection Multiplexing")
        .subtitle("Reuse connections (ControlMaster)")
        .activatable_widget(&control_master)
        .build();
    control_master_row.add_suffix(&control_master);
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
    Entry,
    Entry,
) {
    let session_group = adw::PreferencesGroup::builder().title("Session").build();

    // Agent Forwarding switch
    let agent_forwarding = CheckButton::new();
    let agent_forwarding_row = adw::ActionRow::builder()
        .title("Agent Forwarding")
        .subtitle("Forward SSH agent to remote host (-A)")
        .activatable_widget(&agent_forwarding)
        .build();
    agent_forwarding_row.add_suffix(&agent_forwarding);
    session_group.add(&agent_forwarding_row);

    // X11 Forwarding switch
    let x11_forwarding = CheckButton::new();
    let x11_forwarding_row = adw::ActionRow::builder()
        .title("X11 Forwarding")
        .subtitle("Forward X11 display to local host (-X)")
        .activatable_widget(&x11_forwarding)
        .build();
    x11_forwarding_row.add_suffix(&x11_forwarding);
    session_group.add(&x11_forwarding_row);

    // Compression switch
    let compression = CheckButton::new();
    let compression_row = adw::ActionRow::builder()
        .title("Compression")
        .subtitle("Enable compression for slow connections (-C)")
        .activatable_widget(&compression)
        .build();
    compression_row.add_suffix(&compression);
    session_group.add(&compression_row);

    // Startup command entry
    let startup_entry = Entry::builder()
        .hexpand(true)
        .placeholder_text("Command to run on connect")
        .valign(gtk4::Align::Center)
        .build();

    let startup_row = adw::ActionRow::builder()
        .title("Startup Command")
        .subtitle("Execute after connection established")
        .build();
    startup_row.add_suffix(&startup_entry);
    session_group.add(&startup_row);

    // Custom options entry
    let options_entry = Entry::builder()
        .hexpand(true)
        .placeholder_text("-o Key=Value")
        .valign(gtk4::Align::Center)
        .build();

    let options_row = adw::ActionRow::builder()
        .title("Custom Options")
        .subtitle("Additional SSH command-line options")
        .build();
    options_row.add_suffix(&options_entry);
    session_group.add(&options_row);

    (
        session_group,
        agent_forwarding,
        x11_forwarding,
        compression,
        startup_entry,
        options_entry,
    )
}
