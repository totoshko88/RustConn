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
use crate::sidebar_ui::{ContextMenuItem, MenuActivation, show_popover};

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
    conn_ctx_gesture.connect_pressed(move |gesture, _n, x, y| {
        if let Some(widget) = gesture.widget()
            && let Some(list_box) = widget.downcast_ref::<ListBox>()
            && let Some(row) = list_box.row_at_y(y as i32)
        {
            let conn_id = row.widget_name();
            if conn_id.is_empty() {
                return;
            }
            list_box.select_row(Some(&row));
            // Anchored on the list box with the click coordinates, so the shared
            // builder can resolve a stable anchor the way the sidebar does.
            show_connection_context_menu_in_smart_folder(list_box, x, y, &conn_id);
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
    } else if rustconn_core::dialog_utils::is_glyph_icon(custom_icon) {
        // Emoji/unicode — show as a label
        let emoji_lbl = Label::new(Some(custom_icon));
        emoji_lbl.add_css_class("emoji-icon");
        emoji_lbl.set_width_chars(2);
        hbox.append(&emoji_lbl);
    } else {
        // GTK icon name, falling back to the protocol icon when the active
        // theme does not carry it.
        let icon = Image::from_icon_name(crate::icon_render::theme_icon_or(
            custom_icon,
            get_protocol_icon(conn.protocol),
        ));
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
///
/// Built through [`crate::sidebar_ui::show_popover`], the same builder the
/// sidebar's own context menus use. This function used to assemble its own
/// popover, which is how it ended up without the accessible menu roles, the
/// arrow-key navigation, the Escape handler, the height cap and the
/// active-popover coordination that builder provides.
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

    let items = vec![
        ContextMenuItem::action(&i18n("Edit"), "edit-smart-folder"),
        ContextMenuItem::Separator,
        ContextMenuItem::action(&i18n("Delete"), "delete-smart-folder").destructive(),
    ];

    show_popover(
        widget,
        window,
        &items,
        x,
        y,
        MenuActivation::PointerFallback,
    );
}

/// Shows a context menu for a connection row inside a smart folder.
///
/// Provides the most common actions: Connect, Edit, Copy Username/Password,
/// Wake On LAN, Check if Online, and Delete. Actions that require sidebar
/// selection first select the connection in the main sidebar via
/// `select-item-by-id` action, then activate the standard window action.
fn show_connection_context_menu_in_smart_folder(
    widget: &impl IsA<gtk4::Widget>,
    x: f64,
    y: f64,
    conn_id: &str,
) {
    let Some(root) = widget.root() else { return };
    let Some(window) = root.downcast_ref::<gtk4::ApplicationWindow>() else {
        return;
    };

    let id = conn_id.to_variant();
    let items = vec![
        ContextMenuItem::action_with_target(&i18n("Connect"), "connect-to", &id),
        ContextMenuItem::Separator,
        ContextMenuItem::action_on_selected(&i18n("Edit"), &id, "edit-connection"),
        ContextMenuItem::Separator,
        ContextMenuItem::action_on_selected(&i18n("Copy Username"), &id, "copy-username"),
        ContextMenuItem::action_on_selected(&i18n("Copy Password"), &id, "copy-password"),
        ContextMenuItem::Separator,
        ContextMenuItem::action_on_selected(&i18n("Wake On LAN"), &id, "wake-on-lan"),
        ContextMenuItem::action_on_selected(&i18n("Check if Online"), &id, "check-host-online"),
        ContextMenuItem::Separator,
        ContextMenuItem::action_on_selected(&i18n("Delete"), &id, "delete-connection")
            .destructive(),
    ];

    show_popover(
        widget,
        window,
        &items,
        x,
        y,
        MenuActivation::PointerFallback,
    );
}
