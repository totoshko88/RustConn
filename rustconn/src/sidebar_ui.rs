//! UI helper functions for connection sidebar
//!
//! This module contains UI-related helper functions for creating popovers,
//! context menus, and other visual elements used by the sidebar widget.

use gtk4::gdk;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, EventControllerKey, Label, Orientation, SearchEntry};
use std::cell::RefCell;
use std::rc::Rc;

/// Creates the search help popover with syntax documentation
#[must_use]
pub fn create_search_help_popover() -> gtk4::Popover {
    let popover = gtk4::Popover::new();
    popover.set_autohide(true);

    let content = GtkBox::new(Orientation::Vertical, 8);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    // Title
    let title = Label::builder()
        .label("<b>Search Syntax</b>")
        .use_markup(true)
        .halign(gtk4::Align::Start)
        .build();
    content.append(&title);

    // Description
    let desc = Label::builder()
        .label("Use operators to filter connections:")
        .halign(gtk4::Align::Start)
        .css_classes(["dim-label"])
        .build();
    content.append(&desc);

    // Operators list
    let operators = [
        ("protocol:ssh", "Filter by protocol (ssh, rdp, vnc, spice)"),
        ("tag:production", "Filter by tag"),
        ("group:servers", "Filter by group name"),
        ("prop:environment", "Filter by custom property"),
    ];

    let grid = gtk4::Grid::builder()
        .row_spacing(4)
        .column_spacing(12)
        .margin_top(8)
        .build();

    for (i, (operator, description)) in operators.iter().enumerate() {
        let op_label = Label::builder()
            .label(&format!("<tt>{operator}</tt>"))
            .use_markup(true)
            .halign(gtk4::Align::Start)
            .build();
        let desc_label = Label::builder()
            .label(*description)
            .halign(gtk4::Align::Start)
            .css_classes(["dim-label"])
            .build();
        #[allow(clippy::cast_possible_wrap)]
        {
            grid.attach(&op_label, 0, i as i32, 1, 1);
            grid.attach(&desc_label, 1, i as i32, 1, 1);
        }
    }
    content.append(&grid);

    // Examples section
    let examples_title = Label::builder()
        .label("<b>Examples</b>")
        .use_markup(true)
        .halign(gtk4::Align::Start)
        .margin_top(8)
        .build();
    content.append(&examples_title);

    let examples = [
        "protocol:ssh web",
        "tag:prod server",
        "group:aws protocol:rdp",
    ];

    for example in examples {
        let example_label = Label::builder()
            .label(&format!("<tt>{example}</tt>"))
            .use_markup(true)
            .halign(gtk4::Align::Start)
            .margin_start(8)
            .build();
        content.append(&example_label);
    }

    popover.set_child(Some(&content));
    popover
}

/// Sets up search entry hints for operator autocomplete and history navigation
#[allow(clippy::needless_pass_by_value)]
pub fn setup_search_entry_hints(
    search_entry: &SearchEntry,
    _search_entry_clone: &SearchEntry,
    history_popover: &gtk4::Popover,
    _search_history: &Rc<RefCell<Vec<String>>>,
) {
    // Show history on down arrow when empty
    let history_popover_clone = history_popover.clone();
    let key_controller = EventControllerKey::new();
    let search_entry_clone = search_entry.clone();
    key_controller.connect_key_pressed(move |_controller, key, _code, _state| {
        if key == gdk::Key::Down && search_entry_clone.text().is_empty() {
            history_popover_clone.popup();
            return gtk4::glib::Propagation::Stop;
        }
        gtk4::glib::Propagation::Proceed
    });
    search_entry.add_controller(key_controller);
}

/// Creates the search history popover
#[must_use]
pub fn create_history_popover(
    search_entry: &SearchEntry,
    search_history: Rc<RefCell<Vec<String>>>,
) -> gtk4::Popover {
    let popover = gtk4::Popover::new();
    popover.set_autohide(true);

    let content = GtkBox::new(Orientation::Vertical, 4);
    content.set_margin_top(8);
    content.set_margin_bottom(8);
    content.set_margin_start(8);
    content.set_margin_end(8);

    // Title
    let title = Label::builder()
        .label("<b>Recent Searches</b>")
        .use_markup(true)
        .halign(gtk4::Align::Start)
        .build();
    content.append(&title);

    // History list container
    let history_list = GtkBox::new(Orientation::Vertical, 2);
    history_list.set_margin_top(4);
    content.append(&history_list);

    // Update history list when popover is shown
    let search_entry_clone = search_entry.clone();
    let history_list_clone = history_list.clone();
    let search_history_clone = search_history.clone();
    let popover_clone = popover.clone();
    popover.connect_show(move |_| {
        // Clear existing items
        while let Some(child) = history_list_clone.first_child() {
            history_list_clone.remove(&child);
        }

        // Add history items
        let history = search_history_clone.borrow();
        if history.is_empty() {
            let empty_label = Label::builder()
                .label("No recent searches")
                .css_classes(["dim-label"])
                .build();
            history_list_clone.append(&empty_label);
        } else {
            for query in history.iter() {
                let button = Button::builder()
                    .label(query)
                    .css_classes(["flat"])
                    .halign(gtk4::Align::Start)
                    .build();

                let search_entry_for_btn = search_entry_clone.clone();
                let query_clone = query.clone();
                let popover_for_btn = popover_clone.clone();
                button.connect_clicked(move |_| {
                    search_entry_for_btn.set_text(&query_clone);
                    popover_for_btn.popdown();
                });

                history_list_clone.append(&button);
            }
        }
    });

    popover.set_child(Some(&content));
    popover
}

/// Shows the context menu for a connection item with group awareness
pub fn show_context_menu_for_item(widget: &impl IsA<gtk4::Widget>, x: f64, y: f64, is_group: bool) {
    // Get the root window to access actions
    let Some(root) = widget.root() else { return };
    let Some(window) = root.downcast_ref::<gtk4::ApplicationWindow>() else {
        return;
    };

    // Create a custom popover with buttons instead of PopoverMenu
    // This ensures actions are properly activated
    let popover = gtk4::Popover::new();

    let menu_box = GtkBox::new(Orientation::Vertical, 0);
    menu_box.set_margin_top(6);
    menu_box.set_margin_bottom(6);
    menu_box.set_margin_start(6);
    menu_box.set_margin_end(6);

    // Helper to create menu button
    let create_menu_button = |label: &str| -> Button {
        let btn = Button::with_label(label);
        btn.set_has_frame(false);
        btn.add_css_class("flat");
        btn.set_halign(gtk4::Align::Start);
        btn
    };

    let popover_ref = popover.downgrade();

    // Use lookup_action and activate on the window (which implements ActionMap)
    let window_clone = window.clone();

    if !is_group {
        let connect_btn = create_menu_button("Connect");
        let win = window_clone.clone();
        let popover_c = popover_ref.clone();
        connect_btn.connect_clicked(move |_| {
            if let Some(p) = popover_c.upgrade() {
                p.popdown();
            }
            if let Some(action) = win.lookup_action("connect") {
                action.activate(None);
            }
        });
        menu_box.append(&connect_btn);
    }

    let edit_btn = create_menu_button("Edit");
    let win = window_clone.clone();
    let popover_c = popover_ref.clone();
    edit_btn.connect_clicked(move |_| {
        if let Some(p) = popover_c.upgrade() {
            p.popdown();
        }
        if let Some(action) = win.lookup_action("edit-connection") {
            action.activate(None);
        }
    });
    menu_box.append(&edit_btn);

    // View Details option (only for connections, not groups)
    if !is_group {
        let details_btn = create_menu_button("View Details");
        let win = window_clone.clone();
        let popover_c = popover_ref.clone();
        details_btn.connect_clicked(move |_| {
            if let Some(p) = popover_c.upgrade() {
                p.popdown();
            }
            if let Some(action) = win.lookup_action("view-details") {
                action.activate(None);
            }
        });
        menu_box.append(&details_btn);
    }

    if !is_group {
        let duplicate_btn = create_menu_button("Duplicate");
        let win = window_clone.clone();
        let popover_c = popover_ref.clone();
        duplicate_btn.connect_clicked(move |_| {
            if let Some(p) = popover_c.upgrade() {
                p.popdown();
            }
            if let Some(action) = win.lookup_action("duplicate-connection") {
                action.activate(None);
            }
        });
        menu_box.append(&duplicate_btn);

        let move_btn = create_menu_button("Move to Group...");
        let win = window_clone.clone();
        let popover_c = popover_ref.clone();
        move_btn.connect_clicked(move |_| {
            if let Some(p) = popover_c.upgrade() {
                p.popdown();
            }
            if let Some(action) = win.lookup_action("move-to-group") {
                action.activate(None);
            }
        });
        menu_box.append(&move_btn);
    }

    let delete_btn = create_menu_button("Delete");
    delete_btn.add_css_class("destructive-action");
    let win = window_clone;
    let popover_c = popover_ref;
    delete_btn.connect_clicked(move |_| {
        if let Some(p) = popover_c.upgrade() {
            p.popdown();
        }
        if let Some(action) = win.lookup_action("delete-connection") {
            action.activate(None);
        }
    });
    menu_box.append(&delete_btn);

    popover.set_child(Some(&menu_box));

    // Attach popover to the window
    popover.set_parent(window);

    // Calculate absolute position for the popover
    let widget_bounds = widget.compute_bounds(window);
    #[allow(clippy::cast_possible_truncation)]
    let (popup_x, popup_y) = if let Some(bounds) = widget_bounds {
        (bounds.x() as i32 + x as i32, bounds.y() as i32 + y as i32)
    } else {
        (x as i32, y as i32)
    };

    popover.set_pointing_to(Some(&gdk::Rectangle::new(popup_x, popup_y, 1, 1)));
    popover.set_autohide(true);
    popover.set_has_arrow(true);

    // Connect to closed signal to unparent the popover
    popover.connect_closed(|p| {
        p.unparent();
    });

    popover.popup();
}

/// Shows the context menu for a connection item (non-group)
#[allow(dead_code)]
pub fn show_context_menu(widget: &impl IsA<gtk4::Widget>, x: f64, y: f64) {
    show_context_menu_for_item(widget, x, y, false);
}

/// Returns the appropriate icon name for a protocol string
///
/// For ZeroTrust connections, the protocol string may include provider info
/// in the format "zerotrust:provider" (e.g., "zerotrust:aws", "zerotrust:gcloud").
/// This allows showing provider-specific icons for cloud CLI connections.
///
/// Note: We use standard GTK symbolic icons that are guaranteed to exist
/// in all icon themes. Provider-specific icons (aws-symbolic, etc.) are not
/// available in standard themes, so we use semantic alternatives.
#[must_use]
pub fn get_protocol_icon(protocol: &str) -> &'static str {
    // Check for ZeroTrust with provider info (format: "zerotrust:provider")
    if let Some(provider) = protocol.strip_prefix("zerotrust:") {
        // Use standard GTK/Adwaita icons that are guaranteed to exist
        // Each provider has a unique icon - no duplicates with SSH or other protocols
        return match provider {
            "aws" | "aws_ssm" => "network-workgroup-symbolic", // AWS - workgroup
            "gcloud" | "gcp_iap" => "weather-overcast-symbolic", // GCP - cloud
            "azure" | "azure_bastion" => "weather-few-clouds-symbolic", // Azure - clouds
            "azure_ssh" => "weather-showers-symbolic",         // Azure SSH - showers
            "oci" | "oci_bastion" => "drive-harddisk-symbolic", // OCI - harddisk
            "cloudflare" | "cloudflare_access" => "security-high-symbolic", // Cloudflare
            "teleport" => "emblem-system-symbolic",            // Teleport - system/gear
            "tailscale" | "tailscale_ssh" => "network-vpn-symbolic", // Tailscale - VPN
            "boundary" => "dialog-password-symbolic",          // Boundary - password/lock
            "generic" => "system-run-symbolic",                // Generic - run command
            _ => "folder-remote-symbolic",                     // Unknown - remote folder
        };
    }

    // Standard protocol icons - each protocol has a distinct icon
    match protocol {
        "ssh" => "network-server-symbolic",
        "rdp" => "computer-symbolic",
        "vnc" => "video-display-symbolic",
        "spice" => "video-x-generic-symbolic",
        "zerotrust" => "folder-remote-symbolic",
        _ => "network-server-symbolic",
    }
}

/// Creates the bulk actions toolbar for group operations mode
#[must_use]
pub fn create_bulk_actions_bar() -> GtkBox {
    let bar = GtkBox::new(Orientation::Horizontal, 4);
    bar.set_margin_start(8);
    bar.set_margin_end(8);
    bar.set_margin_top(4);
    bar.set_margin_bottom(4);
    bar.add_css_class("bulk-actions-bar");

    // Delete Selected button
    let delete_button = Button::with_label("Delete Selected");
    delete_button.set_tooltip_text(Some("Delete all selected items"));
    delete_button.set_action_name(Some("win.delete-selected"));
    delete_button.add_css_class("destructive-action");
    delete_button.update_property(&[gtk4::accessible::Property::Label(
        "Delete selected connections",
    )]);
    bar.append(&delete_button);

    // Move to Group dropdown button
    let move_button = Button::with_label("Move to Group...");
    move_button.set_tooltip_text(Some("Move selected items to a group"));
    move_button.set_action_name(Some("win.move-selected-to-group"));
    move_button.update_property(&[gtk4::accessible::Property::Label(
        "Move selected connections to group",
    )]);
    bar.append(&move_button);

    // Select All button
    let select_all_button = Button::with_label("Select All");
    select_all_button.set_tooltip_text(Some("Select all items (Ctrl+A)"));
    select_all_button.set_action_name(Some("win.select-all"));
    select_all_button
        .update_property(&[gtk4::accessible::Property::Label("Select all connections")]);
    bar.append(&select_all_button);

    // Clear Selection button
    let clear_button = Button::with_label("Clear");
    clear_button.set_tooltip_text(Some("Clear selection (Escape)"));
    clear_button.set_action_name(Some("win.clear-selection"));
    clear_button.update_property(&[gtk4::accessible::Property::Label("Clear selection")]);
    bar.append(&clear_button);

    bar
}

/// Creates the button box at the bottom of the sidebar
#[must_use]
pub fn create_button_box() -> GtkBox {
    let button_box = GtkBox::new(Orientation::Horizontal, 4);
    button_box.set_margin_start(8);
    button_box.set_margin_end(8);
    button_box.set_margin_top(8);
    button_box.set_margin_bottom(8);
    button_box.set_halign(gtk4::Align::Center);

    // Add connection button
    let add_button = Button::from_icon_name("list-add-symbolic");
    add_button.set_tooltip_text(Some("Add Connection (Ctrl+N)"));
    add_button.set_action_name(Some("win.new-connection"));
    add_button.update_property(&[gtk4::accessible::Property::Label("Add new connection")]);
    button_box.append(&add_button);

    // Delete button
    let delete_button = Button::from_icon_name("list-remove-symbolic");
    delete_button.set_tooltip_text(Some("Delete Selected (Delete)"));
    delete_button.set_action_name(Some("win.delete-connection"));
    delete_button.update_property(&[gtk4::accessible::Property::Label(
        "Delete selected connection or group",
    )]);
    button_box.append(&delete_button);

    // Add group button
    let add_group_button = Button::from_icon_name("folder-new-symbolic");
    add_group_button.set_tooltip_text(Some("Add Group (Ctrl+Shift+N)"));
    add_group_button.set_action_name(Some("win.new-group"));
    add_group_button.update_property(&[gtk4::accessible::Property::Label("Add new group")]);
    button_box.append(&add_group_button);

    // Quick connect button
    let quick_connect_button = Button::from_icon_name("network-transmit-symbolic");
    quick_connect_button.set_tooltip_text(Some("Quick Connect (without saving)"));
    quick_connect_button.set_action_name(Some("win.quick-connect"));
    quick_connect_button.update_property(&[gtk4::accessible::Property::Label(
        "Quick connect without saving",
    )]);
    button_box.append(&quick_connect_button);

    // Group operations button
    let group_ops_button = Button::from_icon_name("view-list-symbolic");
    group_ops_button.set_tooltip_text(Some("Group Operations Mode"));
    group_ops_button.set_action_name(Some("win.group-operations"));
    group_ops_button.update_property(&[gtk4::accessible::Property::Label(
        "Enable group operations mode for multi-select",
    )]);
    button_box.append(&group_ops_button);

    // Sort button
    let sort_button = Button::from_icon_name("view-sort-ascending-symbolic");
    sort_button.set_tooltip_text(Some("Sort Alphabetically"));
    sort_button.set_action_name(Some("win.sort-connections"));
    sort_button.update_property(&[gtk4::accessible::Property::Label(
        "Sort connections alphabetically",
    )]);
    button_box.append(&sort_button);

    // Sort Recent button
    let sort_recent_button = Button::from_icon_name("document-open-recent-symbolic");
    sort_recent_button.set_tooltip_text(Some("Sort by Recent Usage"));
    sort_recent_button.set_action_name(Some("win.sort-recent"));
    sort_recent_button.update_property(&[gtk4::accessible::Property::Label(
        "Sort connections by recent usage",
    )]);
    button_box.append(&sort_recent_button);

    // Import button
    let import_button = Button::from_icon_name("document-open-symbolic");
    import_button.set_tooltip_text(Some("Import Connections (Ctrl+I)"));
    import_button.set_action_name(Some("win.import"));
    import_button.update_property(&[gtk4::accessible::Property::Label(
        "Import connections from external sources",
    )]);
    button_box.append(&import_button);

    // KeePass button - opens KeePassXC with configured database
    let keepass_button = Button::from_icon_name("dialog-password-symbolic");
    keepass_button.set_tooltip_text(Some("Open KeePass Database"));
    keepass_button.set_action_name(Some("win.open-keepass"));
    keepass_button.set_sensitive(false); // Disabled by default, enabled when integration is active
    keepass_button.update_property(&[gtk4::accessible::Property::Label(
        "Open KeePassXC password database",
    )]);
    button_box.append(&keepass_button);

    // Export button
    let export_button = Button::from_icon_name("document-save-symbolic");
    export_button.set_tooltip_text(Some("Export Configuration"));
    export_button.set_action_name(Some("win.export"));
    export_button.update_property(&[gtk4::accessible::Property::Label(
        "Export configuration to file",
    )]);
    button_box.append(&export_button);

    button_box
}
