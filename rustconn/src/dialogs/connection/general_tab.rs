//! General tab for the connection dialog
//!
//! Contains the basic connection fields: name, icon, protocol, host, port,
//! username, domain, password source, tags, group, and description.
//!
//! Uses `adw::PreferencesGroup` sections following GNOME HIG, consistent
//! with the Advanced and SSH tabs.

use crate::i18n::i18n;
use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, DropDown, Entry, Label, Orientation, ScrolledWindow, SpinButton,
    StringList, TextView, WrapMode,
};
use libadwaita as adw;

/// Widgets created by the General tab, replacing the previous 30-element tuple.
#[allow(dead_code)]
pub(super) struct BasicTabWidgets {
    pub container: GtkBox,
    pub name_entry: Entry,
    pub icon_entry: Entry,
    pub description_view: TextView,
    pub host_entry: Entry,
    pub host_label: Label,
    pub port_spin: SpinButton,
    pub port_label: Label,
    pub username_entry: Entry,
    pub username_label: Label,
    pub domain_entry: Entry,
    pub domain_label: Label,
    pub tags_entry: Entry,
    pub tags_label: Label,
    pub protocol_dropdown: DropDown,
    pub password_source_dropdown: DropDown,
    pub password_source_label: Label,
    pub password_entry: Entry,
    pub password_entry_label: Label,
    pub password_visibility_button: Button,
    pub password_load_button: Button,
    pub vault_test_button: Button,
    pub password_row: GtkBox,
    pub variable_dropdown: DropDown,
    pub variable_row: GtkBox,
    pub group_dropdown: DropDown,
    pub username_load_button: Button,
    pub domain_load_button: Button,
    pub script_command_entry: Entry,
    pub script_test_button: Button,
    pub script_row: GtkBox,
}

/// Creates the basic/general tab with all core connection fields.
///
/// Uses `adw::PreferencesGroup` sections (Identity, Connection, Authentication,
/// Organization) following GNOME HIG, consistent with the Advanced tab.
/// Content is wrapped in `adw::Clamp` to limit max width on wide windows.
#[must_use]
pub(super) fn create_basic_tab() -> BasicTabWidgets {
    let vbox = GtkBox::new(Orientation::Vertical, 0);
    vbox.set_margin_top(12);
    vbox.set_margin_bottom(12);
    vbox.set_margin_start(12);
    vbox.set_margin_end(12);

    // === Identity Section ===
    let identity_group = adw::PreferencesGroup::builder()
        .title(i18n("Identity"))
        .build();

    let name_entry = Entry::builder()
        .placeholder_text(i18n("Connection name"))
        .hexpand(true)
        .width_chars(20)
        .max_width_chars(40)
        .build();
    let name_row = adw::ActionRow::builder().title(i18n("Name")).build();
    name_row.add_suffix(&name_entry);
    name_entry.set_valign(gtk4::Align::Center);
    identity_group.add(&name_row);

    let icon_entry = Entry::builder()
        .placeholder_text(i18n("Emoji or icon name"))
        .hexpand(true)
        .width_chars(16)
        .max_width_chars(30)
        .build();
    icon_entry.set_tooltip_text(Some(&i18n(
        "Enter an emoji (e.g. 🇺🇦) or GTK icon name (e.g. starred-symbolic)",
    )));
    let icon_row = adw::ActionRow::builder()
        .title(i18n("Icon"))
        .subtitle(i18n("Optional"))
        .build();
    icon_row.add_suffix(&icon_entry);
    icon_entry.set_valign(gtk4::Align::Center);
    identity_group.add(&icon_row);

    let protocol_items: Vec<String> = vec![
        "SSH".to_string(),
        "RDP".to_string(),
        "VNC".to_string(),
        "SPICE".to_string(),
        i18n("Zero Trust"),
        "Telnet".to_string(),
        "Serial".to_string(),
        "SFTP".to_string(),
        "Kubernetes".to_string(),
        "MOSH".to_string(),
        "Web".to_string(),
    ];
    let protocol_strs: Vec<&str> = protocol_items.iter().map(String::as_str).collect();
    let protocol_list = StringList::new(&protocol_strs);
    let protocol_dropdown = DropDown::builder().model(&protocol_list).build();
    protocol_dropdown.set_selected(0);
    protocol_dropdown.set_valign(gtk4::Align::Center);
    let protocol_row = adw::ActionRow::builder().title(i18n("Protocol")).build();
    protocol_row.add_suffix(&protocol_dropdown);
    identity_group.add(&protocol_row);

    vbox.append(&identity_group);

    // === Connection Section ===
    let connection_group = adw::PreferencesGroup::builder()
        .title(i18n("Connection"))
        .build();

    let host_label = Label::new(Some(&i18n("Host")));
    let host_entry = Entry::builder()
        .placeholder_text(i18n("hostname or IP"))
        .hexpand(true)
        .width_chars(20)
        .max_width_chars(40)
        .build();
    host_entry.set_valign(gtk4::Align::Center);
    let host_row = adw::ActionRow::builder().title(i18n("Host")).build();
    host_row.add_suffix(&host_entry);
    connection_group.add(&host_row);

    let port_label = Label::new(Some(&i18n("Port")));
    let port_adj = gtk4::Adjustment::new(22.0, 1.0, 65535.0, 1.0, 10.0, 0.0);
    let port_spin = SpinButton::builder()
        .adjustment(&port_adj)
        .climb_rate(1.0)
        .digits(0)
        .valign(gtk4::Align::Center)
        .build();
    let port_desc = Label::builder()
        .label(i18n("SSH, Well-Known"))
        .css_classes(["dim-label"])
        .valign(gtk4::Align::Center)
        .build();
    let port_suffix = GtkBox::new(Orientation::Horizontal, 8);
    port_suffix.append(&port_spin);
    port_suffix.append(&port_desc);
    let port_row = adw::ActionRow::builder().title(i18n("Port")).build();
    port_row.add_suffix(&port_suffix);
    connection_group.add(&port_row);

    // Update port description when port changes
    let port_desc_clone = port_desc.clone();
    port_spin.connect_value_changed(move |spin| {
        #[allow(clippy::cast_sign_loss)]
        let port = spin.value() as u16;
        let desc = get_port_description(port);
        port_desc_clone.set_label(&desc);
    });

    vbox.append(&connection_group);

    // === Authentication Section ===
    let auth_group = adw::PreferencesGroup::builder()
        .title(i18n("Authentication"))
        .build();

    let current_user = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_default();
    let placeholder = if current_user.is_empty() {
        i18n("Username")
    } else {
        format!("(default: {current_user})")
    };
    let username_label = Label::new(Some(&i18n("Username")));
    let username_entry = Entry::builder()
        .placeholder_text(&placeholder)
        .hexpand(true)
        .width_chars(16)
        .max_width_chars(40)
        .valign(gtk4::Align::Center)
        .build();
    let username_load_button = Button::builder()
        .icon_name("folder-download-symbolic")
        .tooltip_text(i18n("Load from selected group"))
        .sensitive(false)
        .valign(gtk4::Align::Center)
        .build();
    username_load_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Load username from selected group",
    ))]);
    let username_row = adw::ActionRow::builder().title(i18n("Username")).build();
    username_row.add_suffix(&username_entry);
    username_row.add_suffix(&username_load_button);
    auth_group.add(&username_row);

    // Domain (RDP only — visibility controlled by protocol dropdown)
    let domain_label = Label::new(Some(&i18n("Domain")));
    let domain_entry = Entry::builder()
        .placeholder_text(i18n("Optional (e.g., WORKGROUP)"))
        .hexpand(true)
        .width_chars(16)
        .max_width_chars(40)
        .valign(gtk4::Align::Center)
        .build();
    let domain_load_button = Button::builder()
        .icon_name("folder-download-symbolic")
        .tooltip_text(i18n("Load from selected group"))
        .sensitive(false)
        .valign(gtk4::Align::Center)
        .build();
    domain_load_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Load domain from selected group",
    ))]);
    let domain_row = adw::ActionRow::builder()
        .title(i18n("Domain"))
        .subtitle(i18n("Windows authentication"))
        .build();
    domain_row.add_suffix(&domain_entry);
    domain_row.add_suffix(&domain_load_button);
    auth_group.add(&domain_row);

    // Password Source
    let password_source_label = Label::new(Some(&i18n("Password")));
    let pw_src_items: Vec<String> = vec![
        i18n("Prompt"),
        i18n("Vault"),
        i18n("Variable"),
        i18n("Inherit"),
        i18n("None"),
        i18n("Script"),
    ];
    let pw_src_strs: Vec<&str> = pw_src_items.iter().map(String::as_str).collect();
    let password_source_list = StringList::new(&pw_src_strs);
    let password_source_dropdown = DropDown::builder()
        .model(&password_source_list)
        .valign(gtk4::Align::Center)
        .build();
    password_source_dropdown.set_selected(0);
    let pw_source_row = adw::ActionRow::builder()
        .title(i18n("Password Source"))
        .build();
    pw_source_row.add_suffix(&password_source_dropdown);
    auth_group.add(&pw_source_row);

    // Password value row (visible for Vault source)
    let password_entry_label = Label::new(Some(&i18n("Value")));
    let password_entry = Entry::builder()
        .placeholder_text(i18n("Password value"))
        .hexpand(true)
        .visibility(false)
        .valign(gtk4::Align::Center)
        .build();
    let password_visibility_button = Button::builder()
        .icon_name("view-reveal-symbolic")
        .tooltip_text(i18n("Show/hide password"))
        .valign(gtk4::Align::Center)
        .build();
    password_visibility_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Toggle password visibility",
    ))]);
    let password_load_button = Button::builder()
        .icon_name("document-open-symbolic")
        .tooltip_text(i18n("Load password from vault"))
        .valign(gtk4::Align::Center)
        .build();
    password_load_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Load password from vault",
    ))]);
    let vault_test_button = Button::builder()
        .icon_name("emblem-ok-symbolic")
        .tooltip_text(i18n("Test credential resolution"))
        .valign(gtk4::Align::Center)
        .build();
    vault_test_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Test credential resolution",
    ))]);
    let password_value_row = adw::ActionRow::builder().title(i18n("Value")).build();
    password_value_row.add_suffix(&password_entry);
    password_value_row.add_suffix(&password_visibility_button);
    password_value_row.add_suffix(&password_load_button);
    password_value_row.add_suffix(&vault_test_button);
    auth_group.add(&password_value_row);

    // Password row visibility controller (hidden GtkBox for bind_property)
    let password_row = GtkBox::new(Orientation::Horizontal, 0);
    password_row.set_visible(false);
    password_row
        .bind_property("visible", &password_value_row, "visible")
        .sync_create()
        .build();

    // Variable dropdown row (visible for Variable source)
    let variable_name_list = StringList::new(&[]);
    let variable_dropdown = DropDown::builder()
        .model(&variable_name_list)
        .valign(gtk4::Align::Center)
        .build();
    let variable_action_row = adw::ActionRow::builder().title(i18n("Variable")).build();
    variable_action_row.add_suffix(&variable_dropdown);
    auth_group.add(&variable_action_row);

    let variable_row = GtkBox::new(Orientation::Horizontal, 0);
    variable_row.set_visible(false);
    variable_row
        .bind_property("visible", &variable_action_row, "visible")
        .sync_create()
        .build();

    // Script command row (visible for Script source)
    let script_command_entry = Entry::builder()
        .placeholder_text(i18n("e.g. vault kv get -field=password secret/myapp"))
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .build();
    let script_test_button = Button::builder()
        .label(i18n("Test"))
        .tooltip_text(i18n("Test the script command"))
        .valign(gtk4::Align::Center)
        .build();
    let script_action_row = adw::ActionRow::builder().title(i18n("Command")).build();
    script_action_row.add_suffix(&script_command_entry);
    script_action_row.add_suffix(&script_test_button);
    auth_group.add(&script_action_row);

    let script_row = GtkBox::new(Orientation::Horizontal, 0);
    script_row.set_visible(false);
    script_row
        .bind_property("visible", &script_action_row, "visible")
        .sync_create()
        .build();

    vbox.append(&auth_group);

    // === Organization Section ===
    let org_group = adw::PreferencesGroup::builder()
        .title(i18n("Organization"))
        .build();

    let tags_label = Label::new(Some(&i18n("Tags")));
    let tags_entry = Entry::builder()
        .placeholder_text(i18n("tag1, tag2, ..."))
        .hexpand(true)
        .width_chars(16)
        .max_width_chars(40)
        .valign(gtk4::Align::Center)
        .build();
    let tags_row = adw::ActionRow::builder().title(i18n("Tags")).build();
    tags_row.add_suffix(&tags_entry);
    org_group.add(&tags_row);

    let group_items: Vec<String> = vec![i18n("(Root)")];
    let group_strs: Vec<&str> = group_items.iter().map(String::as_str).collect();
    let group_list = StringList::new(&group_strs);
    let group_dropdown = DropDown::builder()
        .model(&group_list)
        .valign(gtk4::Align::Center)
        .build();
    let group_action_row = adw::ActionRow::builder().title(i18n("Group")).build();
    group_action_row.add_suffix(&group_dropdown);
    org_group.add(&group_action_row);

    let description_view = TextView::builder()
        .hexpand(true)
        .vexpand(false)
        .wrap_mode(WrapMode::Word)
        .accepts_tab(false)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(8)
        .right_margin(8)
        .build();
    let desc_scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .min_content_height(100)
        .hexpand(true)
        .child(&description_view)
        .build();
    let desc_row = adw::ActionRow::builder().title(i18n("Description")).build();
    desc_row.add_suffix(&desc_scrolled);
    org_group.add(&desc_row);

    vbox.append(&org_group);

    // Wrap content in Clamp for consistent max-width (GNOME HIG)
    let clamp = adw::Clamp::builder()
        .maximum_size(600)
        .tightening_threshold(400)
        .child(&vbox)
        .build();
    let outer = GtkBox::new(Orientation::Vertical, 0);
    outer.append(&clamp);

    // Accessible label relations for screen readers
    name_entry.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
        name_row.upcast_ref()
    ])]);
    icon_entry.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
        icon_row.upcast_ref()
    ])]);
    protocol_dropdown.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
        protocol_row.upcast_ref()
    ])]);
    host_entry.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
        host_row.upcast_ref()
    ])]);
    port_spin.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
        port_row.upcast_ref()
    ])]);
    username_entry.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
        username_row.upcast_ref()
    ])]);
    domain_entry.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
        domain_row.upcast_ref()
    ])]);
    password_source_dropdown.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
        pw_source_row.upcast_ref(),
    ])]);
    password_entry.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
        password_value_row.upcast_ref(),
    ])]);
    variable_dropdown.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
        variable_action_row.upcast_ref(),
    ])]);
    script_command_entry.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
        script_action_row.upcast_ref(),
    ])]);
    tags_entry.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
        tags_row.upcast_ref()
    ])]);
    group_dropdown.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
        group_action_row.upcast_ref()
    ])]);
    description_view.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
        desc_row.upcast_ref()
    ])]);

    BasicTabWidgets {
        container: outer,
        name_entry,
        icon_entry,
        description_view,
        host_entry,
        host_label,
        port_spin,
        port_label,
        username_entry,
        username_label,
        domain_entry,
        domain_label,
        tags_entry,
        tags_label,
        protocol_dropdown,
        password_source_dropdown,
        password_source_label,
        password_entry,
        password_entry_label,
        password_visibility_button,
        password_load_button,
        vault_test_button,
        password_row,
        variable_dropdown,
        variable_row,
        group_dropdown,
        username_load_button,
        domain_load_button,
        script_command_entry,
        script_test_button,
        script_row,
    }
}

/// Returns a description for the given port number.
pub(super) fn get_port_description(port: u16) -> String {
    let service = match port {
        22 => "SSH",
        23 => "Telnet",
        25 => "SMTP",
        53 => "DNS",
        80 => "HTTP",
        110 => "POP3",
        143 => "IMAP",
        443 => "HTTPS",
        445 => "SMB",
        993 => "IMAPS",
        995 => "POP3S",
        3306 => "MySQL",
        3389 => "RDP",
        5432 => "PostgreSQL",
        5900 => "VNC",
        5901..=5909 => "VNC",
        5985 => "WinRM HTTP",
        5986 => "WinRM HTTPS",
        6379 => "Redis",
        8080 => "HTTP Alt",
        8443 => "HTTPS Alt",
        27017 => "MongoDB",
        _ => "",
    };

    let range = if port <= 1023 {
        i18n("Well-Known")
    } else if port <= 49151 {
        i18n("Registered")
    } else {
        i18n("Dynamic")
    };

    if service.is_empty() {
        range
    } else {
        format!("{service}, {range}")
    }
}
