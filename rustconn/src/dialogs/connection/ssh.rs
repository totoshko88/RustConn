//! SSH protocol options for the connection dialog
//!
//! This module provides the SSH-specific UI components including:
//! - Authentication method selection (Password, Public Key, Keyboard Interactive, SSH Agent)
//! - Key source selection (Default, File, Agent)
//! - Connection options (Jump Host, ProxyJump, IdentitiesOnly, ControlMaster)
//! - Session options (Agent Forwarding, X11 Forwarding, Compression, Startup Command)

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, CheckButton, DropDown, Entry, Label, Orientation, StringList};
use libadwaita as adw;
use rustconn_core::sftp::{SocketPathValidation, validate_socket_path};

use super::protocol_layout::ProtocolLayoutBuilder;
use super::widgets::{CheckboxRowBuilder, EntryRowBuilder};
use crate::i18n::i18n;

/// Named struct for SSH options widgets.
///
/// Contains all the widgets needed for SSH configuration.
/// Adding a new SSH option requires only adding a field here and in
/// `create_ssh_options()` — no more 24-element tuple destructuring.
pub struct SshOptionsWidgets {
    pub container: GtkBox,
    /// Content box for appending additional groups (e.g. Port Forwarding)
    pub content: GtkBox,
    pub auth_dropdown: DropDown,
    pub key_source_dropdown: DropDown,
    /// The ActionRow containing the key source dropdown (for updating subtitle/tooltip)
    pub key_source_row: adw::ActionRow,
    pub key_entry: Entry,
    pub key_button: Button,
    pub agent_key_dropdown: DropDown,
    pub jump_host_dropdown: DropDown,
    pub proxy_entry: Entry,
    pub proxy_command_entry: Entry,
    pub identities_only: CheckButton,
    pub control_master: CheckButton,
    pub agent_forwarding: CheckButton,
    pub waypipe: CheckButton,
    pub x11_forwarding: CheckButton,
    pub compression: CheckButton,
    pub verbose: CheckButton,
    pub startup_entry: Entry,
    pub options_entry: Entry,
    /// MOSH settings group (hidden by default, shown when protocol is MOSH)
    pub mosh_group: adw::PreferencesGroup,
    pub mosh_port_range_entry: Entry,
    pub mosh_predict_dropdown: DropDown,
    pub mosh_server_binary_entry: Entry,
    pub ssh_agent_socket_entry: adw::EntryRow,
    pub ssh_pkcs11_entry: adw::EntryRow,
    pub ssh_remote_path_entry: adw::EntryRow,
    pub keep_alive_interval: adw::SpinRow,
    pub keep_alive_count_max: adw::SpinRow,
}

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
    let (
        auth_group,
        auth_dropdown,
        key_source_dropdown,
        key_source_row,
        key_entry,
        key_button,
        agent_key_dropdown,
    ) = create_authentication_group();
    content.append(&auth_group);

    // === Connection Options Group ===
    let (
        connection_group,
        jump_host_dropdown,
        proxy_entry,
        proxy_command_entry,
        identities_only,
        control_master,
        keep_alive_interval,
        keep_alive_count_max,
    ) = create_connection_group();
    content.append(&connection_group);

    // === Session Group ===
    let (
        session_group,
        agent_forwarding,
        waypipe,
        x11_forwarding,
        compression,
        verbose,
        startup_entry,
        options_entry,
        ssh_agent_socket_entry,
        ssh_pkcs11_entry,
        ssh_remote_path_entry,
    ) = create_session_group();
    content.append(&session_group);

    // === MOSH Settings Group (hidden by default, shown when protocol is MOSH) ===
    let mosh_group = adw::PreferencesGroup::builder()
        .title(i18n("MOSH Settings"))
        .description(i18n("Configure UDP port range and prediction behavior."))
        .visible(false)
        .build();

    let (mosh_port_range_row, mosh_port_range_entry) = EntryRowBuilder::new(i18n("Port Range"))
        .subtitle(i18n("UDP port range for MOSH (start:end)"))
        .placeholder("60000:60010")
        .build();
    mosh_group.add(&mosh_port_range_row);

    // Predict Mode dropdown
    let predict_items = [i18n("Adaptive"), i18n("Always"), i18n("Never")];
    let predict_strs: Vec<&str> = predict_items.iter().map(String::as_str).collect();
    let predict_model = StringList::new(&predict_strs);
    let mosh_predict_dropdown = DropDown::builder()
        .model(&predict_model)
        .selected(0)
        .build();
    let predict_row = adw::ActionRow::builder()
        .title(i18n("Predict Mode"))
        .subtitle(i18n("Controls speculative local echo of keystrokes"))
        .build();
    predict_row.add_suffix(&mosh_predict_dropdown);
    predict_row.set_activatable_widget(Some(&mosh_predict_dropdown));
    mosh_group.add(&predict_row);

    let (mosh_server_binary_row, mosh_server_binary_entry) =
        EntryRowBuilder::new(i18n("Server Binary"))
            .subtitle(i18n("Path to mosh-server on the remote host (optional)"))
            .placeholder("/usr/bin/mosh-server")
            .build();
    mosh_group.add(&mosh_server_binary_row);

    content.append(&mosh_group);

    SshOptionsWidgets {
        container,
        content,
        auth_dropdown,
        key_source_dropdown,
        key_source_row,
        key_entry,
        key_button,
        agent_key_dropdown,
        jump_host_dropdown,
        proxy_entry,
        proxy_command_entry,
        identities_only,
        control_master,
        agent_forwarding,
        waypipe,
        x11_forwarding,
        compression,
        verbose,
        startup_entry,
        options_entry,
        mosh_group,
        mosh_port_range_entry,
        mosh_predict_dropdown,
        mosh_server_binary_entry,
        ssh_agent_socket_entry,
        ssh_pkcs11_entry,
        ssh_remote_path_entry,
        keep_alive_interval,
        keep_alive_count_max,
    }
}

/// Creates the Authentication preferences group
fn create_authentication_group() -> (
    adw::PreferencesGroup,
    DropDown,
    DropDown,
    adw::ActionRow,
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

    // Key source dropdown (0=Default, 1=File, 2=Agent, 3=Inherit)
    let key_source_items: Vec<String> = vec![
        i18n("Default"),
        i18n("File"),
        i18n("Agent"),
        i18n("Inherit from group"),
    ];
    let key_source_refs: Vec<&str> = key_source_items.iter().map(String::as_str).collect();
    let key_source_list = StringList::new(&key_source_refs);
    let key_source_dropdown = DropDown::new(Some(key_source_list), gtk4::Expression::NONE);
    key_source_dropdown.set_selected(0);

    let key_source_row = adw::ActionRow::builder()
        .title(i18n("Key Source"))
        .subtitle(i18n(
            "Default tries ~/.ssh/id_rsa, id_ed25519, id_ecdsa automatically",
        ))
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
    key_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Browse for SSH key file",
    ))]);

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

    // Inline validation: check key file exists when path is entered manually
    {
        let entry_clone = key_entry.clone();
        key_entry.connect_changed(move |_| {
            let path_text = entry_clone.text();
            let path_str = path_text.trim();
            if path_str.is_empty() {
                entry_clone.remove_css_class("error");
                entry_clone.set_tooltip_text(None);
                return;
            }
            // Expand ~ to home directory
            let expanded = if let Some(rest) = path_str.strip_prefix("~/") {
                if let Some(home) = std::env::var_os("HOME") {
                    std::path::PathBuf::from(home).join(rest)
                } else {
                    std::path::PathBuf::from(path_str)
                }
            } else {
                std::path::PathBuf::from(path_str)
            };
            if expanded.exists() {
                entry_clone.remove_css_class("error");
                entry_clone.set_tooltip_text(None);
            } else {
                entry_clone.add_css_class("error");
                entry_clone.set_tooltip_text(Some(&i18n("Key file not found")));
            }
        });
    }

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
        key_source_row,
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
            0 | 3 => {
                // Default or Inherit - hide both rows
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
            0 | 2 => {
                // Password / Keyboard Interactive - hide all key-related rows
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
                // Public Key - show key source row
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
    Entry,
    CheckButton,
    CheckButton,
    adw::SpinRow,
    adw::SpinRow,
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
    jump_host_dropdown.set_enable_search(true);
    // Limit width so long hostnames don't stretch the dialog
    jump_host_dropdown.set_size_request(200, -1);
    jump_host_dropdown.set_hexpand(false);

    let jump_host_row = adw::ActionRow::builder()
        .title(i18n("Jump Host"))
        .subtitle(i18n("Connect via another SSH connection"))
        .build();
    jump_host_row.add_suffix(&jump_host_dropdown);
    connection_group.add(&jump_host_row);

    // ProxyJump entry
    let (proxy_row, proxy_entry) = EntryRowBuilder::new(i18n("ProxyJump"))
        .subtitle(i18n("Jump host for tunneling (-J)"))
        .placeholder("user@jumphost")
        .build();
    connection_group.add(&proxy_row);

    // ProxyCommand entry (e.g., for Tor .onion hosts)
    let (proxy_command_row, proxy_command_entry) = EntryRowBuilder::new(i18n("ProxyCommand"))
        .subtitle(i18n("Custom proxy command (e.g., for .onion hosts)"))
        .placeholder("ncat --proxy 127.0.0.1:9050 --proxy-type socks5 %h %p")
        .build();
    connection_group.add(&proxy_command_row);

    // IdentitiesOnly switch
    let (identities_row, identities_only) =
        CheckboxRowBuilder::new(&i18n("Use Only Specified Key"))
            .subtitle(&i18n("Prevents trying other keys (IdentitiesOnly)"))
            .build();
    connection_group.add(&identities_row);

    // ControlMaster switch
    let (control_master_row, control_master) =
        CheckboxRowBuilder::new(&i18n("Connection Multiplexing"))
            .subtitle(&i18n("Reuse connections (ControlMaster)"))
            .build();
    connection_group.add(&control_master_row);

    // Keep-Alive Interval (ServerAliveInterval)
    // 0 = disabled, range 0..3600, default 60 for new connections
    let keep_alive_adjustment = gtk4::Adjustment::new(60.0, 0.0, 3600.0, 1.0, 10.0, 0.0);
    let keep_alive_interval = adw::SpinRow::new(Some(&keep_alive_adjustment), 1.0, 0);
    keep_alive_interval.set_title(&i18n("Keep-Alive Interval"));
    keep_alive_interval.set_subtitle(&i18n("Seconds between keep-alive packets (0 = disabled)"));
    connection_group.add(&keep_alive_interval);

    // Keep-Alive Count Max (ServerAliveCountMax)
    // 0 = disabled, range 0..100
    let keep_alive_count_adjustment = gtk4::Adjustment::new(3.0, 0.0, 100.0, 1.0, 5.0, 0.0);
    let keep_alive_count_max = adw::SpinRow::new(Some(&keep_alive_count_adjustment), 1.0, 0);
    keep_alive_count_max.set_title(&i18n("Keep-Alive Count"));
    keep_alive_count_max.set_subtitle(&i18n("Disconnect after this many unanswered packets"));
    connection_group.add(&keep_alive_count_max);

    (
        connection_group,
        jump_host_dropdown,
        proxy_entry,
        proxy_command_entry,
        identities_only,
        control_master,
        keep_alive_interval,
        keep_alive_count_max,
    )
}

/// Creates the Session preferences group
fn create_session_group() -> (
    adw::PreferencesGroup,
    CheckButton,
    CheckButton,
    CheckButton,
    CheckButton,
    CheckButton,
    Entry,
    Entry,
    adw::EntryRow,
    adw::EntryRow,
    adw::EntryRow,
) {
    let session_group = adw::PreferencesGroup::builder()
        .title(i18n("Session"))
        .build();

    // Agent Forwarding switch
    let (agent_forwarding_row, agent_forwarding) =
        CheckboxRowBuilder::new(i18n("Agent Forwarding"))
            .subtitle(i18n("Forward SSH agent to remote host (-A)"))
            .build();
    session_group.add(&agent_forwarding_row);

    // Waypipe (Wayland application forwarding)
    let (waypipe_row, waypipe) = CheckboxRowBuilder::new(i18n("Waypipe"))
        .subtitle(i18n("Wayland application forwarding via waypipe"))
        .build();
    session_group.add(&waypipe_row);

    // X11 Forwarding switch
    let (x11_forwarding_row, x11_forwarding) = CheckboxRowBuilder::new(i18n("X11 Forwarding"))
        .subtitle(i18n("Forward X11 display to local host (-X)"))
        .build();
    session_group.add(&x11_forwarding_row);

    // Compression switch
    let (compression_row, compression) = CheckboxRowBuilder::new(i18n("Compression"))
        .subtitle(i18n("Enable compression for slow connections (-C)"))
        .build();
    session_group.add(&compression_row);

    // Verbose/debug switch
    let (verbose_row, verbose) = CheckboxRowBuilder::new(i18n("Verbose"))
        .subtitle(i18n("Show SSH debug output for troubleshooting (-v)"))
        .build();
    session_group.add(&verbose_row);

    // Startup command entry
    let (startup_row, startup_entry) = EntryRowBuilder::new(i18n("Startup Command"))
        .subtitle(i18n("Execute after connection established"))
        .placeholder(i18n("Command to run on connect"))
        .build();
    session_group.add(&startup_row);

    // Custom options entry
    let (options_row, options_entry) = EntryRowBuilder::new(i18n("Custom Options"))
        .subtitle(i18n("Key=Value, comma-separated (-o flags)"))
        .placeholder("StrictHostKeyChecking=no, ServerAliveInterval=60")
        .build();
    session_group.add(&options_row);

    // SSH Agent Socket entry
    let ssh_agent_socket_entry = adw::EntryRow::builder()
        .title(i18n("SSH Agent Socket"))
        .build();
    ssh_agent_socket_entry.set_tooltip_text(Some(&i18n(
        "Overrides global setting and auto-detected socket for this connection",
    )));

    // Real-time validation feedback
    ssh_agent_socket_entry.connect_changed(|entry| {
        let text = entry.text();
        let path = text.as_str();
        entry.remove_css_class("success");
        entry.remove_css_class("warning");
        entry.remove_css_class("error");
        match validate_socket_path(path) {
            SocketPathValidation::Empty => {}
            SocketPathValidation::Valid => {
                entry.add_css_class("success");
            }
            SocketPathValidation::NotFound => {
                entry.add_css_class("warning");
            }
            SocketPathValidation::NotAbsolute => {
                entry.add_css_class("error");
            }
        }
    });
    session_group.add(&ssh_agent_socket_entry);

    // PKCS#11 provider library path (hardware-token auth: YubiKey/PIV/smart cards)
    let ssh_pkcs11_entry = adw::EntryRow::builder()
        .title(i18n("PKCS#11 Provider"))
        .build();
    ssh_pkcs11_entry.set_tooltip_text(Some(&i18n(
        "Path to a PKCS#11 library for hardware-token login (e.g. /usr/lib64/libykcs11.so.2)",
    )));

    // Real-time validation: warn if the path is not an existing absolute file
    ssh_pkcs11_entry.connect_changed(|entry| {
        let text = entry.text();
        let path = text.trim();
        entry.remove_css_class("success");
        entry.remove_css_class("warning");
        entry.remove_css_class("error");
        if path.is_empty() || path.eq_ignore_ascii_case("none") {
            // Empty or the literal `none` (disables an inherited provider) — neutral
        } else if !std::path::Path::new(path).is_absolute() {
            entry.add_css_class("error");
        } else if std::path::Path::new(path).is_file() {
            entry.add_css_class("success");
        } else {
            // Absolute but not found — may exist only inside a sandbox/container
            entry.add_css_class("warning");
        }
    });
    session_group.add(&ssh_pkcs11_entry);

    // SFTP remote directory: the file manager opens here instead of the server
    // root. Empty → the login home directory is detected automatically (#212).
    let ssh_remote_path_entry = adw::EntryRow::builder()
        .title(i18n("SFTP Remote Directory"))
        .build();
    ssh_remote_path_entry.set_tooltip_text(Some(&i18n(
        "Directory the SFTP file browser opens in. Leave empty to detect the home directory automatically.",
    )));
    session_group.add(&ssh_remote_path_entry);

    (
        session_group,
        agent_forwarding,
        waypipe,
        x11_forwarding,
        compression,
        verbose,
        startup_entry,
        options_entry,
        ssh_agent_socket_entry,
        ssh_pkcs11_entry,
        ssh_remote_path_entry,
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

        // Check for duplicate local port
        let existing = data.borrow();
        if existing.iter().any(|pf| pf.local_port == local_port) {
            local_port_clone.add_css_class("error");
            local_port_clone.set_tooltip_text(Some(&i18n("Port already in use")));
            return;
        }
        drop(existing);
        local_port_clone.remove_css_class("error");
        local_port_clone.set_tooltip_text(None);

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
    row_box.set_margin_top(6);
    row_box.set_margin_bottom(6);
    row_box.set_margin_start(12);
    row_box.set_margin_end(12);

    let summary_label = Label::builder()
        .label(pf.display_summary())
        .hexpand(true)
        .halign(gtk4::Align::Start)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .build();

    let remove_button = Button::builder()
        .icon_name("user-trash-symbolic")
        .css_classes(["flat"])
        .tooltip_text(i18n("Remove this port forward"))
        .build();
    remove_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Remove port forward",
    ))]);

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
