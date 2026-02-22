//! View logic for the sidebar (list items)
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, DragSource, GestureClick, Image, Label, ListItem, ListView, MultiSelection,
    Orientation, SignalListItemFactory, SingleSelection, TreeExpander, TreeListRow, gdk, glib,
};
use std::cell::RefCell;
use std::rc::Rc;

use crate::sidebar::ConnectionItem;
use crate::sidebar_ui;

/// Sets up a list item widget
///
/// # Accessibility
/// Each list item is set up with proper accessible properties:
/// - Status icons have live region for dynamic updates
/// - Labels are associated with their icons
#[allow(clippy::too_many_lines)]
pub fn setup_list_item(
    _factory: &SignalListItemFactory,
    list_item: &ListItem,
    _group_ops_mode: bool,
) {
    let expander = TreeExpander::new();

    let content_box = GtkBox::new(Orientation::Horizontal, 8);
    content_box.set_margin_start(4);
    content_box.set_margin_end(4);
    content_box.set_margin_top(4);
    content_box.set_margin_bottom(4);

    let icon = Image::from_icon_name("network-server-symbolic");
    content_box.append(&icon);

    let status_icon = Image::from_icon_name("emblem-default-symbolic");
    status_icon.set_pixel_size(10);
    status_icon.set_visible(false);
    status_icon.add_css_class("status-icon");
    content_box.append(&status_icon);

    let label = Label::new(None);
    label.set_halign(gtk4::Align::Start);
    label.set_hexpand(true);
    content_box.append(&label);

    expander.set_child(Some(&content_box));
    list_item.set_child(Some(&expander));

    // Set up drag source for reorganization
    let drag_source = DragSource::new();
    drag_source.set_actions(gdk::DragAction::MOVE);

    // Store list_item reference for drag prepare
    let list_item_weak_drag = list_item.downgrade();
    drag_source.connect_prepare(move |_source, _x, _y| {
        // Get the item from the list item
        let list_item = list_item_weak_drag.upgrade()?;
        let row = list_item.item()?.downcast::<TreeListRow>().ok()?;
        let item = row.item()?.downcast::<ConnectionItem>().ok()?;

        // Delegate to drag_drop helper
        crate::sidebar::drag_drop::prepare_drag_data(&item)
    });

    // Visual feedback during drag
    // Requirement 7.4: Visual feedback during drag
    let list_item_weak_begin = list_item.downgrade();
    drag_source.connect_drag_begin(move |_source, _drag| {
        if let Some(list_item) = list_item_weak_begin.upgrade()
            && let Some(expander) = list_item.child()
        {
            expander.add_css_class("dragging");
        }
    });

    // Clean up drop indicator when drag ends
    let list_item_weak_end = list_item.downgrade();
    drag_source.connect_drag_end(move |source, _drag, _delete_data| {
        // Remove dragging CSS class
        if let Some(list_item) = list_item_weak_end.upgrade()
            && let Some(expander) = list_item.child()
        {
            expander.remove_css_class("dragging");
        }

        // Find the sidebar and hide the drop indicator
        if let Some(widget) = source.widget()
            && let Some(list_view) = widget.ancestor(ListView::static_type())
        {
            // Remove all drop-related CSS classes
            list_view.remove_css_class("drop-active");
            list_view.remove_css_class("drop-into-group");
        }
    });

    expander.add_controller(drag_source);

    // Set up right-click context menu
    // Note: is_group will be determined at bind time via list_item data
    let gesture = GestureClick::new();
    gesture.set_button(gdk::BUTTON_SECONDARY);
    let list_item_weak = list_item.downgrade();
    gesture.connect_pressed(move |gesture, _n_press, x, y| {
        if let Some(widget) = gesture.widget() {
            // First, select this item so context menu actions work on it
            if let Some(list_item) = list_item_weak.upgrade() {
                // Get the position of this item and select it
                let position = list_item.position();
                if let Some(list_view) = widget.ancestor(ListView::static_type())
                    && let Some(list_view) = list_view.downcast_ref::<ListView>()
                    && let Some(model) = list_view.model()
                {
                    if let Some(selection) = model.downcast_ref::<SingleSelection>() {
                        selection.set_selected(position);
                    } else if let Some(selection) = model.downcast_ref::<MultiSelection>() {
                        // In multi-selection mode, select only this item for context menu
                        selection.unselect_all();
                        selection.select_item(position, false);
                    }
                }
            }

            // Check if this is a group by looking at the icon
            let is_group = widget
                .first_child()
                .and_then(|c| c.first_child())
                .and_then(|c| c.downcast::<gtk4::Image>().ok())
                .is_some_and(|img| {
                    img.icon_name()
                        .is_some_and(|n| n.as_str() == "folder-symbolic")
                });

            // Detect SSH protocol from the ConnectionItem data
            let is_ssh = list_item_weak
                .upgrade()
                .and_then(|li| li.item())
                .and_then(|obj| obj.downcast::<gtk4::TreeListRow>().ok())
                .and_then(|row| row.item())
                .and_then(|obj| obj.downcast::<ConnectionItem>().ok())
                .is_some_and(|item| {
                    let p = item.protocol();
                    p == "ssh" || p == "sftp"
                });

            sidebar_ui::show_context_menu_for_item(&widget, x, y, is_group, is_ssh);
        }
    });
    expander.add_controller(gesture);
}

/// Binds data to a list item
pub fn bind_list_item(
    _factory: &SignalListItemFactory,
    list_item: &ListItem,
    handlers: &Rc<RefCell<std::collections::HashMap<ListItem, glib::SignalHandlerId>>>,
    query: &str,
) {
    let Some(expander) = list_item.child().and_downcast::<TreeExpander>() else {
        return;
    };

    let Some(row) = list_item.item().and_downcast::<TreeListRow>() else {
        return;
    };

    expander.set_list_row(Some(&row));

    let Some(item) = row.item().and_downcast::<ConnectionItem>() else {
        return;
    };

    let Some(content_box) = expander.child().and_downcast::<GtkBox>() else {
        return;
    };

    let Some(icon) = content_box.first_child().and_downcast::<Image>() else {
        return;
    };

    let Some(status_icon) = icon.next_sibling().and_downcast::<Image>() else {
        return;
    };

    let Some(label) = status_icon.next_sibling().and_downcast::<Label>() else {
        return;
    };

    // Helper to set text with highlighting
    let set_label_text = |label: &Label, text: &str| {
        if query.is_empty() {
            label.set_text(text);
        } else {
            let markup = crate::sidebar::search::highlight_match(text, query);
            label.set_markup(&markup);
        }
    };

    if item.is_group() {
        icon.set_icon_name(Some("folder-symbolic"));
        set_label_text(&label, &item.name());
        // Groups don't have connection status
        status_icon.set_visible(false);

        // Add drop controller for dropping into groups
    } else {
        // Set icon based on protocol
        let protocol = item.protocol();

        // Use sidebar_ui directly
        let icon_name = sidebar_ui::get_protocol_icon(&protocol);
        icon.set_icon_name(Some(icon_name));
        set_label_text(&label, &item.name());

        // Setup status monitoring logic
        // Update status icon
        if let Some(status_icon) = content_box
            .first_child()
            .and_then(|c| c.next_sibling())
            .and_downcast::<gtk4::Image>()
        {
            // Helper to update icon state with accessibility announcements
            let update_icon = |icon: &gtk4::Image, status: &str| {
                icon.remove_css_class("status-connected");
                icon.remove_css_class("status-connecting");
                icon.remove_css_class("status-failed");

                if status == "connected" {
                    icon.set_icon_name(Some("emblem-default-symbolic"));
                    icon.set_visible(true);
                    icon.add_css_class("status-connected");
                    // Accessibility: announce status change
                    icon.update_property(&[gtk4::accessible::Property::Label("Connected")]);
                } else if status == "connecting" {
                    icon.set_icon_name(Some("network-transmit-receive-symbolic"));
                    icon.set_visible(true);
                    icon.add_css_class("status-connecting");
                    // Accessibility: announce status change
                    icon.update_property(&[gtk4::accessible::Property::Label("Connecting")]);
                } else if status == "failed" {
                    icon.set_icon_name(Some("dialog-error-symbolic"));
                    icon.set_visible(true);
                    icon.add_css_class("status-failed");
                    // Accessibility: announce status change
                    icon.update_property(&[gtk4::accessible::Property::Label("Connection failed")]);
                } else {
                    icon.set_visible(false);
                    icon.update_property(&[gtk4::accessible::Property::Label("")]);
                }
            };

            // Initial update
            update_icon(&status_icon, &item.status());

            // Connect to notify::status
            let status_icon_clone = status_icon.clone();
            let handler_id =
                item.connect_notify_local(Some("status"), move |item: &ConnectionItem, _| {
                    update_icon(&status_icon_clone, &item.status());
                });

            // Store handler ID on list_item for cleanup
            handlers.borrow_mut().insert(list_item.clone(), handler_id);
        }

        // Update label with dirty indicator for documents
        if let Some(label) = content_box.last_child().and_downcast::<Label>() {
            let name = item.name();
            if item.is_document() && item.is_dirty() {
                let text = format!("• {name}");
                set_label_text(&label, &text);
            } else {
                set_label_text(&label, &name);
            }
        }
    }
}

/// Unbinds data from a list item
pub fn unbind_list_item(
    _factory: &SignalListItemFactory,
    list_item: &ListItem,
    handlers: &Rc<RefCell<std::collections::HashMap<ListItem, glib::SignalHandlerId>>>,
) {
    // Remove signal handler if exists
    if let Some(handler_id) = handlers.borrow_mut().remove(list_item)
        && let Some(row) = list_item.item().and_downcast::<TreeListRow>()
        && let Some(item) = row.item().and_downcast::<ConnectionItem>()
    {
        item.disconnect(handler_id);
    }
}
