//! Smart Folders sidebar section widget.
//!
//! Provides a collapsible "Smart Folders" section for the sidebar with:
//! - A header with 🔍 icon and "Add" button
//! - Expandable rows: click to reveal matching connections inline
//! - Context menu with Edit / Delete actions
//! - Double-click on a connection row activates `win.connect-to` action
//! - Read-only view (no drag-drop)

use std::cell::Cell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, Image, Label, ListBox, ListBoxRow, Orientation, Revealer,
    RevealerTransitionType, SelectionMode, Widget, gdk,
};
use rustconn_core::get_protocol_icon;
use rustconn_core::models::{Connection, SmartFolder};
use rustconn_core::smart_folder::SmartFolderManager;

use crate::i18n::i18n;

/// Sidebar section that displays smart folders with dynamic connection counts.
pub struct SmartFoldersSidebar {
    /// Root container widget.
    container: GtkBox,
    /// The list box holding smart folder rows.
    list_box: ListBox,
    /// Header label (kept alive for updates).
    #[expect(
        dead_code,
        reason = "kept alive for GTK widget lifecycle / future API exposure"
    )]
    header_label: Label,
    /// Add button (kept alive for signal handler).
    add_button: Button,
}

impl SmartFoldersSidebar {
    /// Creates a new Smart Folders sidebar section.
    #[must_use]
    pub fn new() -> Self {
        let container = GtkBox::new(Orientation::Vertical, 4);
        container.set_margin_top(12);
        container.set_margin_bottom(6);
        container.set_margin_start(12);
        container.set_margin_end(12);

        // --- Header row: icon + label + spacer + add button ---
        let header_row = GtkBox::new(Orientation::Horizontal, 6);
        header_row.set_margin_bottom(6);

        let icon_label = Label::new(Some("🔍"));
        icon_label.add_css_class("heading");
        header_row.append(&icon_label);

        let header_label = Label::new(Some(&i18n("Smart Folders")));
        header_label.add_css_class("heading");
        header_label.set_hexpand(true);
        header_label.set_halign(gtk4::Align::Start);
        header_row.append(&header_label);

        let add_button = Button::from_icon_name("list-add-symbolic");
        add_button.set_tooltip_text(Some(&i18n("New Smart Folder")));
        add_button.add_css_class("flat");
        add_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
            "Create new smart folder",
        ))]);
        header_row.append(&add_button);

        container.append(&header_row);

        // --- Separator ---
        let sep = gtk4::Separator::new(Orientation::Horizontal);
        container.append(&sep);

        // --- List box (read-only, no drag-drop) ---
        let list_box = ListBox::new();
        list_box.set_selection_mode(SelectionMode::Single);
        list_box.add_css_class("navigation-sidebar");
        list_box.set_activate_on_single_click(false);
        container.append(&list_box);

        Self {
            container,
            list_box,
            header_label,
            add_button,
        }
    }

    /// Returns the root GTK widget for embedding in the sidebar.
    #[must_use]
    pub fn widget(&self) -> &Widget {
        self.container.upcast_ref()
    }

    /// Returns a reference to the "Add" button for connecting signals.
    #[must_use]
    pub fn add_button(&self) -> &Button {
        &self.add_button
    }

    /// Returns a reference to the list box for connecting signals.
    #[must_use]
    pub fn list_box(&self) -> &ListBox {
        &self.list_box
    }

    /// Refreshes the list with current smart folders and connections.
    ///
    /// For each folder the manager evaluates matching connections and
    /// displays the folder name with a count badge. Clicking a folder
    /// row expands/collapses the list of matching connections inline.
    pub fn update(&self, folders: &[SmartFolder], connections: &[Connection]) {
        // Remove all existing rows
        while let Some(child) = self.list_box.first_child() {
            self.list_box.remove(&child);
        }

        if folders.is_empty() {
            let placeholder = Label::new(Some(&i18n("No smart folders")));
            placeholder.add_css_class("dim-label");
            placeholder.set_margin_top(12);
            placeholder.set_margin_bottom(12);
            self.list_box.append(&placeholder);
            return;
        }

        let manager = SmartFolderManager::new();

        for folder in folders {
            let matched = manager.evaluate(folder, connections);
            let row_widget = build_expandable_folder_row(folder, &matched);
            self.list_box.append(&row_widget);
        }
    }
}

impl Default for SmartFoldersSidebar {
    fn default() -> Self {
        Self::new()
    }
}

/// Builds an expandable smart folder row.
///
/// The row contains:
/// - A header with expander arrow, folder icon, name, and count badge
/// - A `Revealer` with a nested list of matching connections
///
/// Clicking the header toggles the revealer. Right-click shows context menu.
fn build_expandable_folder_row(folder: &SmartFolder, connections: &[&Connection]) -> GtkBox {
    let outer = GtkBox::new(Orientation::Vertical, 0);

    // --- Header row (clickable) ---
    let header = GtkBox::new(Orientation::Horizontal, 6);
    header.set_margin_top(6);
    header.set_margin_bottom(6);
    header.set_margin_start(6);
    header.set_margin_end(6);

    // Expander arrow
    let arrow = Image::from_icon_name("pan-end-symbolic");
    arrow.add_css_class("dim-label");
    header.append(&arrow);

    // Folder icon (custom emoji or default 📁)
    let icon_str = folder.icon.as_deref().unwrap_or("📁");
    let icon = Label::new(Some(icon_str));
    header.append(&icon);

    // Folder name
    let name_label = Label::new(Some(&folder.name));
    name_label.set_hexpand(true);
    name_label.set_halign(gtk4::Align::Start);
    name_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    header.append(&name_label);

    // Connection count badge
    let count_label = Label::new(Some(&connections.len().to_string()));
    count_label.add_css_class("dim-label");
    header.append(&count_label);

    outer.append(&header);

    // --- Revealer with scrollable connection list ---
    let revealer = Revealer::builder()
        .transition_type(RevealerTransitionType::SlideDown)
        .transition_duration(150)
        .reveal_child(false)
        .build();

    let conn_list = ListBox::new();
    conn_list.set_selection_mode(SelectionMode::Single);
    conn_list.add_css_class("navigation-sidebar");
    conn_list.set_margin_start(24);
    conn_list.set_activate_on_single_click(false);

    for conn in connections {
        let conn_row = build_connection_row(conn);
        conn_list.append(&conn_row);
    }

    // Connection list directly in revealer — the outer ScrolledWindow
    // in the sidebar handles scrolling for the entire smart folders section
    revealer.set_child(Some(&conn_list));
    outer.append(&revealer);

    // --- Toggle expand/collapse on header click ---
    let expanded = Rc::new(Cell::new(false));
    let gesture = gtk4::GestureClick::new();
    gesture.set_button(gdk::BUTTON_PRIMARY);
    let revealer_clone = revealer.clone();
    let arrow_clone = arrow.clone();
    let expanded_clone = expanded.clone();
    gesture.connect_released(move |_gesture, _n, _x, _y| {
        let is_expanded = !expanded_clone.get();
        expanded_clone.set(is_expanded);
        revealer_clone.set_reveal_child(is_expanded);
        if is_expanded {
            arrow_clone.set_icon_name(Some("pan-down-symbolic"));
        } else {
            arrow_clone.set_icon_name(Some("pan-end-symbolic"));
        }
    });
    header.add_controller(gesture);

    // --- Context menu via right-click on header ---
    let ctx_gesture = gtk4::GestureClick::new();
    ctx_gesture.set_button(gdk::BUTTON_SECONDARY);
    let folder_id = folder.id;
    ctx_gesture.connect_pressed(move |gesture, _n, x, y| {
        if let Some(widget) = gesture.widget() {
            // Select the parent ListBoxRow so edit/delete actions can find it
            let mut current: Option<gtk4::Widget> = Some(widget.clone().upcast());
            while let Some(w) = current {
                if let Some(row) = w.downcast_ref::<ListBoxRow>() {
                    if let Some(list_box) = row.parent().and_then(|p| p.downcast::<ListBox>().ok())
                    {
                        list_box.select_row(Some(row));
                    }
                    break;
                }
                current = w.parent();
            }
            show_smart_folder_context_menu(&widget, x, y, folder_id);
        }
    });
    header.add_controller(ctx_gesture);

    // --- Double-click on connection row → connect ---
    conn_list.connect_row_activated(move |_list_box, row| {
        // row-activated fires on Enter or double-click (activate_on_single_click is false)
        let conn_id = row.widget_name();
        if conn_id.is_empty() {
            return;
        }
        if let Some(root) = row.root()
            && let Some(win) = root.downcast_ref::<gtk4::ApplicationWindow>()
            && let Some(action) = win.lookup_action("connect-to")
        {
            action.activate(Some(&conn_id.to_variant()));
        }
    });

    // --- Right-click on connection row → context menu ---
    let conn_ctx_gesture = gtk4::GestureClick::new();
    conn_ctx_gesture.set_button(gdk::BUTTON_SECONDARY);
    conn_ctx_gesture.connect_pressed(move |gesture, _n, _x, y| {
        if let Some(widget) = gesture.widget()
            && let Some(list_box) = widget.downcast_ref::<ListBox>()
            && let Some(row) = list_box.row_at_y(y as i32)
        {
            let conn_id = row.widget_name();
            if conn_id.is_empty() {
                return;
            }
            list_box.select_row(Some(&row));
            show_connection_context_menu_in_smart_folder(&row, conn_id.to_string());
        }
    });
    conn_list.add_controller(conn_ctx_gesture);

    outer
}

/// Builds a single connection row for the expanded smart folder view.
fn build_connection_row(conn: &Connection) -> ListBoxRow {
    let row = ListBoxRow::new();
    // Store connection ID in widget name for retrieval on activation
    row.set_widget_name(&conn.id.to_string());

    let hbox = GtkBox::new(Orientation::Horizontal, 6);
    hbox.set_margin_top(6);
    hbox.set_margin_bottom(6);
    hbox.set_margin_start(6);
    hbox.set_margin_end(6);

    // Connection icon: custom (emoji or GTK icon name) or protocol-based
    let custom_icon = conn.icon.as_deref().unwrap_or("");
    if custom_icon.is_empty() {
        let icon_name = get_protocol_icon(conn.protocol);
        let icon = Image::from_icon_name(icon_name);
        icon.set_pixel_size(16);
        hbox.append(&icon);
    } else if custom_icon.chars().count() <= 2
        && custom_icon.chars().next().is_some_and(|c| !c.is_ascii())
    {
        // Emoji/unicode — show as a label
        let emoji_lbl = Label::new(Some(custom_icon));
        emoji_lbl.add_css_class("emoji-icon");
        emoji_lbl.set_width_chars(2);
        hbox.append(&emoji_lbl);
    } else {
        // GTK icon name
        let icon = Image::from_icon_name(custom_icon);
        icon.set_pixel_size(16);
        hbox.append(&icon);
    }

    // Connection name
    let name_label = Label::new(Some(&conn.name));
    name_label.set_hexpand(true);
    name_label.set_halign(gtk4::Align::Start);
    name_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    hbox.append(&name_label);

    // Host (dim)
    if !conn.host.is_empty() {
        let host_label = Label::new(Some(&conn.host));
        host_label.add_css_class("dim-label");
        host_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        host_label.set_max_width_chars(16);
        hbox.append(&host_label);
    }

    // Tooltip with full info
    let tooltip = if conn.host.is_empty() {
        conn.name.clone()
    } else {
        format!("{}\n{}", conn.name, conn.host)
    };
    row.set_tooltip_text(Some(&tooltip));

    row.set_child(Some(&hbox));
    row
}

/// Shows a context menu with Edit / Delete for a smart folder row.
fn show_smart_folder_context_menu(
    widget: &impl IsA<gtk4::Widget>,
    x: f64,
    y: f64,
    _folder_id: uuid::Uuid,
) {
    let Some(root) = widget.root() else { return };
    let Some(window) = root.downcast_ref::<gtk4::ApplicationWindow>() else {
        return;
    };

    let popover = gtk4::Popover::new();
    // Keep the menu legible under third-party GTK themes (e.g. Breeze on KDE),
    // which otherwise paint the flat-button text to clash with the popover
    // background (#181). Styling lives in assets/style.css.
    popover.add_css_class("context-menu-popover");

    let menu_box = GtkBox::new(Orientation::Vertical, 0);
    menu_box.set_margin_top(6);
    menu_box.set_margin_bottom(6);
    menu_box.set_margin_start(6);
    menu_box.set_margin_end(6);

    let create_menu_button = |label: &str| -> Button {
        let btn = Button::with_label(label);
        btn.set_has_frame(false);
        btn.add_css_class("flat");
        btn.add_css_class("context-menu-item");
        btn.set_halign(gtk4::Align::Start);
        btn
    };

    let popover_ref = popover.downgrade();
    let window_clone = window.clone();

    // Edit
    let edit_btn = create_menu_button(&i18n("Edit"));
    let win = window_clone.clone();
    let popover_c = popover_ref.clone();
    edit_btn.connect_clicked(move |_| {
        if let Some(p) = popover_c.upgrade() {
            p.popdown();
        }
        if let Some(action) = win.lookup_action("edit-smart-folder") {
            action.activate(None);
        }
    });
    menu_box.append(&edit_btn);

    // Delete
    let delete_btn = create_menu_button(&i18n("Delete"));
    delete_btn.add_css_class("context-menu-destructive");
    let win = window_clone;
    let popover_c = popover_ref;
    delete_btn.connect_clicked(move |_| {
        if let Some(p) = popover_c.upgrade() {
            p.popdown();
        }
        if let Some(action) = win.lookup_action("delete-smart-folder") {
            action.activate(None);
        }
    });
    menu_box.append(&delete_btn);

    popover.set_child(Some(&menu_box));
    popover.set_parent(window);

    let widget_bounds = widget.compute_bounds(window);
    #[expect(
        clippy::cast_possible_truncation,
        reason = "value range fits the target type by construction in this code path"
    )]
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

/// Shows a context menu for a connection row inside a smart folder.
///
/// Provides the most common actions: Connect, Edit, Copy Username/Password,
/// Wake On LAN, Check if Online, and Delete. Actions that require sidebar
/// selection first select the connection in the main sidebar via
/// `select-item-by-id` action, then activate the standard window action.
fn show_connection_context_menu_in_smart_folder(row: &ListBoxRow, conn_id: String) {
    let Some(root) = row.root() else { return };
    let Some(window) = root.downcast_ref::<gtk4::ApplicationWindow>() else {
        return;
    };

    let popover = gtk4::Popover::new();
    // Keep the menu legible under third-party GTK themes (e.g. Breeze on KDE),
    // which otherwise paint the flat-button text to clash with the popover
    // background (#181). Styling lives in assets/style.css.
    popover.add_css_class("context-menu-popover");

    let menu_box = GtkBox::new(Orientation::Vertical, 0);
    menu_box.set_margin_top(6);
    menu_box.set_margin_bottom(6);
    menu_box.set_margin_start(6);
    menu_box.set_margin_end(6);

    let create_menu_button = |label: &str| -> Button {
        let btn = Button::with_label(label);
        btn.set_has_frame(false);
        btn.add_css_class("flat");
        btn.add_css_class("context-menu-item");
        btn.set_halign(gtk4::Align::Start);
        btn
    };

    let popover_ref = popover.downgrade();

    // Connect
    let connect_btn = create_menu_button(&i18n("Connect"));
    let win = window.clone();
    let id = conn_id.clone();
    let popover_c = popover_ref.clone();
    connect_btn.connect_clicked(move |_| {
        if let Some(p) = popover_c.upgrade() {
            p.popdown();
        }
        if let Some(action) = win.lookup_action("connect-to") {
            action.activate(Some(&id.to_variant()));
        }
    });
    menu_box.append(&connect_btn);

    // Separator
    menu_box.append(&gtk4::Separator::new(Orientation::Horizontal));

    // Edit
    let edit_btn = create_menu_button(&i18n("Edit"));
    let win = window.clone();
    let id = conn_id.clone();
    let popover_c = popover_ref.clone();
    edit_btn.connect_clicked(move |_| {
        if let Some(p) = popover_c.upgrade() {
            p.popdown();
        }
        if let Some(action) = win.lookup_action("select-item-by-id") {
            action.activate(Some(&id.to_variant()));
        }
        if let Some(action) = win.lookup_action("edit-connection") {
            action.activate(None);
        }
    });
    menu_box.append(&edit_btn);

    // Separator
    menu_box.append(&gtk4::Separator::new(Orientation::Horizontal));

    // Copy Username
    let copy_user_btn = create_menu_button(&i18n("Copy Username"));
    let win = window.clone();
    let id = conn_id.clone();
    let popover_c = popover_ref.clone();
    copy_user_btn.connect_clicked(move |_| {
        if let Some(p) = popover_c.upgrade() {
            p.popdown();
        }
        if let Some(action) = win.lookup_action("select-item-by-id") {
            action.activate(Some(&id.to_variant()));
        }
        if let Some(action) = win.lookup_action("copy-username") {
            action.activate(None);
        }
    });
    menu_box.append(&copy_user_btn);

    // Copy Password
    let copy_pass_btn = create_menu_button(&i18n("Copy Password"));
    let win = window.clone();
    let id = conn_id.clone();
    let popover_c = popover_ref.clone();
    copy_pass_btn.connect_clicked(move |_| {
        if let Some(p) = popover_c.upgrade() {
            p.popdown();
        }
        if let Some(action) = win.lookup_action("select-item-by-id") {
            action.activate(Some(&id.to_variant()));
        }
        if let Some(action) = win.lookup_action("copy-password") {
            action.activate(None);
        }
    });
    menu_box.append(&copy_pass_btn);

    // Separator
    menu_box.append(&gtk4::Separator::new(Orientation::Horizontal));

    // Wake On LAN
    let wol_btn = create_menu_button(&i18n("Wake On LAN"));
    let win = window.clone();
    let id = conn_id.clone();
    let popover_c = popover_ref.clone();
    wol_btn.connect_clicked(move |_| {
        if let Some(p) = popover_c.upgrade() {
            p.popdown();
        }
        if let Some(action) = win.lookup_action("select-item-by-id") {
            action.activate(Some(&id.to_variant()));
        }
        if let Some(action) = win.lookup_action("wake-on-lan") {
            action.activate(None);
        }
    });
    menu_box.append(&wol_btn);

    // Check if Online
    let check_btn = create_menu_button(&i18n("Check if Online"));
    let win = window.clone();
    let id = conn_id.clone();
    let popover_c = popover_ref.clone();
    check_btn.connect_clicked(move |_| {
        if let Some(p) = popover_c.upgrade() {
            p.popdown();
        }
        if let Some(action) = win.lookup_action("select-item-by-id") {
            action.activate(Some(&id.to_variant()));
        }
        if let Some(action) = win.lookup_action("check-host-online") {
            action.activate(None);
        }
    });
    menu_box.append(&check_btn);

    // Separator
    menu_box.append(&gtk4::Separator::new(Orientation::Horizontal));

    // Delete
    let delete_btn = create_menu_button(&i18n("Delete"));
    delete_btn.add_css_class("context-menu-destructive");
    let win = window.clone();
    let id = conn_id;
    let popover_c = popover_ref;
    delete_btn.connect_clicked(move |_| {
        if let Some(p) = popover_c.upgrade() {
            p.popdown();
        }
        if let Some(action) = win.lookup_action("select-item-by-id") {
            action.activate(Some(&id.to_variant()));
        }
        if let Some(action) = win.lookup_action("delete-connection") {
            action.activate(None);
        }
    });
    menu_box.append(&delete_btn);

    popover.set_child(Some(&menu_box));
    popover.set_parent(row);
    popover.set_autohide(true);
    popover.set_has_arrow(true);

    popover.connect_closed(|p| {
        p.unparent();
    });

    popover.popup();
}
