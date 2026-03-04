//! General tab for the connection dialog
//!
//! Contains the basic connection fields: name, icon, protocol, host, port,
//! username, domain, password source, tags, group, and description.

use crate::i18n::i18n;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, DropDown, Entry, Grid, Label, Orientation, ScrolledWindow, SpinButton,
    StringList, TextView, WrapMode,
};

/// Creates the basic/general tab with all core connection fields.
#[allow(clippy::type_complexity)]
pub(super) fn create_basic_tab() -> (
    GtkBox,
    Entry,
    Entry,
    TextView,
    Entry,
    Label,
    SpinButton,
    Label,
    Entry,
    Label,
    Entry,
    Label,
    Entry,
    Label,
    DropDown,
    DropDown,
    Label,
    Entry,
    Label,
    Button,
    Button,
    GtkBox,
    DropDown,
    GtkBox,
    DropDown,
    Button,
    Button,
) {
    let vbox = GtkBox::new(Orientation::Vertical, 8);
    vbox.set_margin_top(12);
    vbox.set_margin_bottom(12);
    vbox.set_margin_start(12);
    vbox.set_margin_end(12);

    let grid = Grid::builder().row_spacing(8).column_spacing(12).build();
    vbox.append(&grid);

    let mut row = 0;

    // Name
    let name_label = Label::builder()
        .label(i18n("Name:"))
        .halign(gtk4::Align::End)
        .build();
    let name_entry = Entry::builder()
        .placeholder_text(i18n("Connection name"))
        .hexpand(true)
        .build();
    grid.attach(&name_label, 0, row, 1, 1);
    grid.attach(&name_entry, 1, row, 2, 1);
    row += 1;

    // Icon (emoji or GTK icon name)
    let icon_label = Label::builder()
        .label(i18n("Icon:"))
        .halign(gtk4::Align::End)
        .build();
    let icon_entry = Entry::builder()
        .placeholder_text(i18n("Emoji or icon name (optional)"))
        .hexpand(true)
        .max_width_chars(30)
        .build();
    icon_entry.set_tooltip_text(Some(&i18n(
        "Enter an emoji (e.g. 🇺🇦) or GTK icon name (e.g. starred-symbolic)",
    )));
    grid.attach(&icon_label, 0, row, 1, 1);
    grid.attach(&icon_entry, 1, row, 2, 1);
    row += 1;

    // Protocol
    let protocol_label_grid = Label::builder()
        .label(i18n("Protocol:"))
        .halign(gtk4::Align::End)
        .build();
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
    ];
    let protocol_strs: Vec<&str> = protocol_items.iter().map(String::as_str).collect();
    let protocol_list = StringList::new(&protocol_strs);
    let protocol_dropdown = DropDown::builder().model(&protocol_list).build();
    protocol_dropdown.set_selected(0);
    grid.attach(&protocol_label_grid, 0, row, 1, 1);
    grid.attach(&protocol_dropdown, 1, row, 2, 1);
    row += 1;

    // Host
    let host_label = Label::builder()
        .label(i18n("Host:"))
        .halign(gtk4::Align::End)
        .build();
    let host_entry = Entry::builder()
        .placeholder_text(i18n("hostname or IP"))
        .hexpand(true)
        .build();
    grid.attach(&host_label, 0, row, 1, 1);
    grid.attach(&host_entry, 1, row, 2, 1);
    row += 1;

    // Port with description
    let port_label = Label::builder()
        .label(i18n("Port:"))
        .halign(gtk4::Align::End)
        .build();
    let port_adj = gtk4::Adjustment::new(22.0, 1.0, 65535.0, 1.0, 10.0, 0.0);
    let port_spin = SpinButton::builder()
        .adjustment(&port_adj)
        .climb_rate(1.0)
        .digits(0)
        .build();
    let port_desc = Label::builder()
        .label(i18n("SSH, Well-Known"))
        .css_classes(["dim-label"])
        .build();
    let port_box = GtkBox::new(Orientation::Horizontal, 8);
    port_box.append(&port_spin);
    port_box.append(&port_desc);
    grid.attach(&port_label, 0, row, 1, 1);
    grid.attach(&port_box, 1, row, 2, 1);
    row += 1;

    // Update port description when port changes
    let port_desc_clone = port_desc.clone();
    port_spin.connect_value_changed(move |spin| {
        #[allow(clippy::cast_sign_loss)]
        let port = spin.value() as u16;
        let desc = get_port_description(port);
        port_desc_clone.set_label(&desc);
    });

    // Username
    let username_label = Label::builder()
        .label(i18n("Username:"))
        .halign(gtk4::Align::End)
        .build();
    let current_user = std::env::var("USER").unwrap_or_default();
    let username_entry = Entry::builder()
        .placeholder_text(&format!("(default: {current_user})"))
        .hexpand(true)
        .build();
    grid.attach(&username_label, 0, row, 1, 1);

    let username_load_button = Button::builder()
        .icon_name("folder-download-symbolic")
        .tooltip_text(i18n("Load from selected group"))
        .sensitive(false)
        .build();
    let username_box = GtkBox::new(Orientation::Horizontal, 4);
    username_box.append(&username_entry);
    username_box.append(&username_load_button);

    grid.attach(&username_box, 1, row, 2, 1);
    row += 1;

    // Domain (for RDP/Windows authentication)
    let domain_label = Label::builder()
        .label(i18n("Domain:"))
        .halign(gtk4::Align::End)
        .build();
    let domain_entry = Entry::builder()
        .placeholder_text(i18n("Optional (e.g., WORKGROUP)"))
        .hexpand(true)
        .build();
    grid.attach(&domain_label, 0, row, 1, 1);

    let domain_load_button = Button::builder()
        .icon_name("folder-download-symbolic")
        .tooltip_text(i18n("Load from selected group"))
        .sensitive(false)
        .build();
    let domain_box = GtkBox::new(Orientation::Horizontal, 4);
    domain_box.append(&domain_entry);
    domain_box.append(&domain_load_button);

    grid.attach(&domain_box, 1, row, 2, 1);
    row += 1;

    // Password Source
    let password_source_label = Label::builder()
        .label(i18n("Password:"))
        .halign(gtk4::Align::End)
        .build();
    let pw_src_items: Vec<String> = vec![
        i18n("Prompt"),
        i18n("Vault"),
        i18n("Variable"),
        i18n("Inherit"),
        i18n("None"),
    ];
    let pw_src_strs: Vec<&str> = pw_src_items.iter().map(String::as_str).collect();
    let password_source_list = StringList::new(&pw_src_strs);
    let password_source_dropdown = DropDown::builder().model(&password_source_list).build();
    password_source_dropdown.set_selected(0);
    grid.attach(&password_source_label, 0, row, 1, 1);
    grid.attach(&password_source_dropdown, 1, row, 2, 1);
    row += 1;

    // Password with visibility toggle - use grid row for proper alignment
    let password_entry_label = Label::builder()
        .label(i18n("Value:"))
        .halign(gtk4::Align::End)
        .build();
    let password_entry = Entry::builder()
        .placeholder_text(i18n("Password value"))
        .hexpand(true)
        .visibility(false)
        .build();
    let password_visibility_button = Button::builder()
        .icon_name("view-reveal-symbolic")
        .tooltip_text(i18n("Show/hide password"))
        .build();
    let password_load_button = Button::builder()
        .icon_name("document-open-symbolic")
        .tooltip_text(i18n("Load password from vault"))
        .build();
    let password_box = GtkBox::new(Orientation::Horizontal, 4);
    password_box.append(&password_entry);
    password_box.append(&password_visibility_button);
    password_box.append(&password_load_button);
    password_box.set_hexpand(true);

    // Password row container - wraps label and entry box for show/hide
    let password_row = GtkBox::new(Orientation::Horizontal, 0);
    password_row.set_visible(false);
    // Attach label and password box to grid for proper alignment
    grid.attach(&password_entry_label, 0, row, 1, 1);
    grid.attach(&password_box, 1, row, 2, 1);
    // Bind visibility of label and box to password_row visibility
    password_row
        .bind_property("visible", &password_entry_label, "visible")
        .sync_create()
        .build();
    password_row
        .bind_property("visible", &password_box, "visible")
        .sync_create()
        .build();
    row += 1;

    // Variable name dropdown — shown when password source is Variable
    let variable_label = Label::builder()
        .label(i18n("Variable:"))
        .halign(gtk4::Align::End)
        .build();
    let variable_name_list = StringList::new(&[]);
    let variable_dropdown = DropDown::builder().model(&variable_name_list).build();
    let variable_row = GtkBox::new(Orientation::Horizontal, 0);
    variable_row.set_visible(false);
    grid.attach(&variable_label, 0, row, 1, 1);
    grid.attach(&variable_dropdown, 1, row, 2, 1);
    variable_row
        .bind_property("visible", &variable_label, "visible")
        .sync_create()
        .build();
    variable_row
        .bind_property("visible", &variable_dropdown, "visible")
        .sync_create()
        .build();
    row += 1;

    // Tags
    let tags_label = Label::builder()
        .label(i18n("Tags:"))
        .halign(gtk4::Align::End)
        .build();
    let tags_entry = Entry::builder()
        .placeholder_text(i18n("tag1, tag2, ..."))
        .hexpand(true)
        .build();
    grid.attach(&tags_label, 0, row, 1, 1);
    grid.attach(&tags_entry, 1, row, 2, 1);
    row += 1;

    // Group
    let group_label = Label::builder()
        .label(i18n("Group:"))
        .halign(gtk4::Align::End)
        .build();
    let group_items: Vec<String> = vec![i18n("(Root)")];
    let group_strs: Vec<&str> = group_items.iter().map(String::as_str).collect();
    let group_list = StringList::new(&group_strs);
    let group_dropdown = DropDown::builder().model(&group_list).build();
    grid.attach(&group_label, 0, row, 1, 1);
    grid.attach(&group_dropdown, 1, row, 2, 1);
    row += 1;

    // Description
    let desc_label = Label::builder()
        .label(i18n("Description:"))
        .halign(gtk4::Align::End)
        .valign(gtk4::Align::Start)
        .build();
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
        .min_content_height(144)
        .hexpand(true)
        .child(&description_view)
        .build();
    grid.attach(&desc_label, 0, row, 1, 1);
    grid.attach(&desc_scrolled, 1, row, 2, 1);

    // Accessible label relations for screen readers (A11Y-01)
    icon_entry.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
        icon_label.upcast_ref()
    ])]);
    name_entry.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
        name_label.upcast_ref()
    ])]);
    protocol_dropdown.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
        protocol_label_grid.upcast_ref(),
    ])]);
    host_entry.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
        host_label.upcast_ref()
    ])]);
    port_spin.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
        port_label.upcast_ref()
    ])]);
    username_entry.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
        username_label.upcast_ref()
    ])]);
    domain_entry.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
        domain_label.upcast_ref()
    ])]);
    password_source_dropdown.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
        password_source_label.upcast_ref(),
    ])]);
    password_entry.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
        password_entry_label.upcast_ref(),
    ])]);
    variable_dropdown.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
        variable_label.upcast_ref()
    ])]);
    tags_entry.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
        tags_label.upcast_ref()
    ])]);
    group_dropdown.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
        group_label.upcast_ref()
    ])]);
    description_view.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
        desc_label.upcast_ref()
    ])]);

    (
        vbox,
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
        password_row,
        variable_dropdown,
        variable_row,
        group_dropdown,
        username_load_button,
        domain_load_button,
    )
}

/// Returns a description for the given port number.
pub(super) fn get_port_description(port: u16) -> String {
    // Well-known service ports
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

    // Port range category
    let range = if port <= 1023 {
        "Well-Known"
    } else if port <= 49151 {
        "Registered"
    } else {
        "Dynamic"
    };

    if service.is_empty() {
        range.to_string()
    } else {
        format!("{service}, {range}")
    }
}
