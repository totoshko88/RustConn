//! UI helper functions for connection sidebar
//!
//! This module contains UI-related helper functions for creating popovers,
//! context menus, and other visual elements used by the sidebar widget.

use gtk4::gdk;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, Orientation};

/// Shows the context menu for a connection item with group awareness
pub fn show_context_menu_for_item(
    widget: &impl IsA<gtk4::Widget>,
    x: f64,
    y: f64,
    is_group: bool,
    is_ssh: bool,
) {
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

    // Rename option (for both connections and groups)
    let rename_btn = create_menu_button("Rename");
    let win = window_clone.clone();
    let popover_c = popover_ref.clone();
    rename_btn.connect_clicked(move |_| {
        if let Some(p) = popover_c.upgrade() {
            p.popdown();
        }
        if let Some(action) = win.lookup_action("rename-item") {
            action.activate(None);
        }
    });
    menu_box.append(&rename_btn);

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

        // Run Snippet option - opens snippet picker for the selected connection
        let snippet_btn = create_menu_button("Run Snippet...");
        let win = window_clone.clone();
        let popover_c = popover_ref.clone();
        snippet_btn.connect_clicked(move |_| {
            if let Some(p) = popover_c.upgrade() {
                p.popdown();
            }
            if let Some(action) = win.lookup_action("run-snippet-for-connection") {
                action.activate(None);
            }
        });
        menu_box.append(&snippet_btn);

        // Wake On LAN option
        let wol_btn = create_menu_button("Wake On LAN");
        let win = window_clone.clone();
        let popover_c = popover_ref.clone();
        wol_btn.connect_clicked(move |_| {
            if let Some(p) = popover_c.upgrade() {
                p.popdown();
            }
            if let Some(action) = win.lookup_action("wake-on-lan") {
                action.activate(None);
            }
        });
        menu_box.append(&wol_btn);

        // Open SFTP option (SSH connections only)
        if is_ssh {
            let sftp_btn = create_menu_button("Open SFTP");
            let win = window_clone.clone();
            let popover_c = popover_ref.clone();
            sftp_btn.connect_clicked(move |_| {
                if let Some(p) = popover_c.upgrade() {
                    p.popdown();
                }
                if let Some(action) = win.lookup_action("open-sftp") {
                    action.activate(None);
                }
            });
            menu_box.append(&sftp_btn);
        }
    }

    let delete_btn = create_menu_button("Delete");
    delete_btn.add_css_class("destructive-action");
    delete_btn.add_css_class("context-menu-destructive");
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

/// Returns the appropriate icon name for a protocol string
///
/// For ZeroTrust connections, the protocol string may include provider info
/// in the format "zerotrust:provider" (e.g., "zerotrust:aws", "zerotrust:gcloud").
/// All ZeroTrust connections use the same icon regardless of provider.
///
/// Icons are aligned with `rustconn_core::protocol::icons::get_protocol_icon()`.
#[must_use]
pub fn get_protocol_icon(protocol: &str) -> &'static str {
    // All ZeroTrust variants use the same icon (matches filter button)
    if protocol.starts_with("zerotrust") {
        return "security-high-symbolic";
    }

    // Standard protocol icons — aligned with rustconn-core icons.rs
    match protocol {
        "ssh" => "utilities-terminal-symbolic",
        "rdp" => "computer-symbolic",
        "vnc" => "video-display-symbolic",
        "spice" => "preferences-desktop-remote-desktop-symbolic",
        "telnet" => "call-start-symbolic",
        "serial" => "modem-symbolic",
        "sftp" => "folder-remote-symbolic",
        "kubernetes" => "application-x-executable-symbolic",
        "info" => "dialog-information-symbolic",
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

    // New Group button (highlighted as create action)
    let new_group_button = Button::with_label("New Group");
    new_group_button.set_tooltip_text(Some("Create a new group"));
    new_group_button.set_action_name(Some("win.new-group"));
    new_group_button.add_css_class("suggested-action");
    new_group_button.update_property(&[gtk4::accessible::Property::Label("Create new group")]);
    bar.append(&new_group_button);

    // Move to Group button
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

    // Delete button (rightmost, destructive)
    let delete_button = Button::with_label("Delete");
    delete_button.set_tooltip_text(Some("Delete all selected items"));
    delete_button.set_action_name(Some("win.delete-selected"));
    delete_button.add_css_class("destructive-action");
    delete_button.update_property(&[gtk4::accessible::Property::Label(
        "Delete selected connections",
    )]);
    bar.append(&delete_button);

    bar
}

/// Creates the sidebar bottom toolbar with secondary actions
///
/// Layout: [Group Ops] [History] [A-Z Sort] [Recent] [Import] [Export] [KeePass]
#[must_use]
pub fn create_sidebar_bottom_toolbar() -> (GtkBox, Button) {
    let toolbar = GtkBox::new(Orientation::Horizontal, 4);
    toolbar.set_margin_start(8);
    toolbar.set_margin_end(8);
    toolbar.set_margin_top(6);
    toolbar.set_margin_bottom(6);
    toolbar.set_halign(gtk4::Align::Center);

    // Group operations button
    let group_ops_button = Button::from_icon_name("view-list-symbolic");
    group_ops_button.set_tooltip_text(Some("Group Operations Mode"));
    group_ops_button.set_action_name(Some("win.group-operations"));
    group_ops_button.update_property(&[gtk4::accessible::Property::Label(
        "Enable group operations mode for multi-select",
    )]);
    toolbar.append(&group_ops_button);

    // Connection History button
    let history_button = Button::from_icon_name("document-open-recent-symbolic");
    history_button.set_tooltip_text(Some("Connection History"));
    history_button.set_action_name(Some("win.show-history"));
    history_button.update_property(&[gtk4::accessible::Property::Label("View connection history")]);
    toolbar.append(&history_button);

    // Sort alphabetically button
    let sort_button = Button::from_icon_name("view-sort-ascending-symbolic");
    sort_button.set_tooltip_text(Some("Sort Alphabetically"));
    sort_button.set_action_name(Some("win.sort-connections"));
    sort_button.update_property(&[gtk4::accessible::Property::Label(
        "Sort connections alphabetically",
    )]);
    toolbar.append(&sort_button);

    // Sort by recent usage button
    let sort_recent_button = Button::from_icon_name("appointment-soon-symbolic");
    sort_recent_button.set_tooltip_text(Some("Sort by Recent Usage"));
    sort_recent_button.set_action_name(Some("win.sort-recent"));
    sort_recent_button.update_property(&[gtk4::accessible::Property::Label(
        "Sort connections by recent usage",
    )]);
    toolbar.append(&sort_recent_button);

    // Import button
    let import_button = Button::from_icon_name("document-open-symbolic");
    import_button.set_tooltip_text(Some("Import Connections (Ctrl+I)"));
    import_button.set_action_name(Some("win.import"));
    import_button.update_property(&[gtk4::accessible::Property::Label(
        "Import connections from external sources",
    )]);
    toolbar.append(&import_button);

    // Export button
    let export_button = Button::from_icon_name("document-save-symbolic");
    export_button.set_tooltip_text(Some("Export Connections"));
    export_button.set_action_name(Some("win.export"));
    export_button.update_property(&[gtk4::accessible::Property::Label(
        "Export connections to file",
    )]);
    toolbar.append(&export_button);

    // Password vault button - shows integration status
    let keepass_button = Button::from_icon_name("dialog-password-symbolic");
    keepass_button.set_tooltip_text(Some("Open Password Vault"));
    keepass_button.set_action_name(Some("win.open-keepass"));
    keepass_button.add_css_class("keepass-button");
    keepass_button.update_property(&[gtk4::accessible::Property::Label(
        "Open password vault for credential management",
    )]);
    toolbar.append(&keepass_button);

    (toolbar, keepass_button)
}

/// Shows the context menu for empty space in the sidebar
pub fn show_empty_space_context_menu(widget: &impl IsA<gtk4::Widget>, x: f64, y: f64) {
    let Some(root) = widget.root() else { return };
    let Some(window) = root.downcast_ref::<gtk4::ApplicationWindow>() else {
        return;
    };

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
    let window_clone = window.clone();

    // Quick Connect
    let quick_connect_btn = create_menu_button("Quick Connect");
    let win = window_clone.clone();
    let popover_c = popover_ref.clone();
    quick_connect_btn.connect_clicked(move |_| {
        if let Some(p) = popover_c.upgrade() {
            p.popdown();
        }
        if let Some(action) = win.lookup_action("quick-connect") {
            action.activate(None);
        }
    });
    menu_box.append(&quick_connect_btn);

    // New Connection
    let new_conn_btn = create_menu_button("New Connection");
    let win = window_clone.clone();
    let popover_c = popover_ref.clone();
    new_conn_btn.connect_clicked(move |_| {
        if let Some(p) = popover_c.upgrade() {
            p.popdown();
        }
        if let Some(action) = win.lookup_action("new-connection") {
            action.activate(None);
        }
    });
    menu_box.append(&new_conn_btn);

    // New Group
    let new_group_btn = create_menu_button("New Group");
    let win = window_clone.clone();
    let popover_c = popover_ref.clone();
    new_group_btn.connect_clicked(move |_| {
        if let Some(p) = popover_c.upgrade() {
            p.popdown();
        }
        if let Some(action) = win.lookup_action("new-group") {
            action.activate(None);
        }
    });
    menu_box.append(&new_group_btn);

    // Separator
    let sep = gtk4::Separator::new(Orientation::Horizontal);
    sep.set_margin_top(6);
    sep.set_margin_bottom(6);
    menu_box.append(&sep);

    // Import
    let import_btn = create_menu_button("Import...");
    let win = window_clone.clone();
    let popover_c = popover_ref.clone();
    import_btn.connect_clicked(move |_| {
        if let Some(p) = popover_c.upgrade() {
            p.popdown();
        }
        if let Some(action) = win.lookup_action("import") {
            action.activate(None);
        }
    });
    menu_box.append(&import_btn);

    // Export
    let export_btn = create_menu_button("Export...");
    let win = window_clone;
    let popover_c = popover_ref;
    export_btn.connect_clicked(move |_| {
        if let Some(p) = popover_c.upgrade() {
            p.popdown();
        }
        if let Some(action) = win.lookup_action("export") {
            action.activate(None);
        }
    });
    menu_box.append(&export_btn);

    popover.set_child(Some(&menu_box));
    popover.set_parent(window);

    // Calculate position
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

    popover.connect_closed(|p| {
        p.unparent();
    });

    popover.popup();
}
