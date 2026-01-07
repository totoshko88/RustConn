//! Tab management for terminal notebook
//!
//! This module handles tab creation, display modes, and overflow menu.

use gtk4::prelude::*;
use gtk4::{gdk, Box as GtkBox, Button, Image, Label, MenuButton, Notebook, Orientation, Popover};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use uuid::Uuid;

use super::types::{TabDisplayMode, TabLabelWidgets};

/// Gets the icon name for a protocol
#[must_use]
pub fn get_protocol_icon(protocol: &str) -> &'static str {
    // Handle zerotrust:provider format
    if let Some(provider) = protocol.strip_prefix("zerotrust:") {
        return match provider {
            "aws" | "aws_ssm" => "network-workgroup-symbolic",
            "gcloud" | "gcp_iap" => "weather-overcast-symbolic",
            "azure" | "azure_bastion" => "weather-few-clouds-symbolic",
            "azure_ssh" => "weather-showers-symbolic",
            "oci" | "oci_bastion" => "drive-harddisk-symbolic",
            "cloudflare" | "cloudflare_access" => "security-high-symbolic",
            "teleport" => "emblem-system-symbolic",
            "tailscale" | "tailscale_ssh" => "network-vpn-symbolic",
            "boundary" => "dialog-password-symbolic",
            "generic" => "system-run-symbolic",
            _ => "folder-remote-symbolic",
        };
    }

    match protocol.to_lowercase().as_str() {
        "ssh" => "network-server-symbolic",
        "rdp" => "computer-symbolic",
        "vnc" => "video-display-symbolic",
        "spice" => "video-x-generic-symbolic",
        "zerotrust" => "folder-remote-symbolic",
        _ => "network-server-symbolic",
    }
}

/// Truncates a name to max_chars with ellipsis
#[must_use]
pub fn truncate_name(name: &str, max_chars: usize) -> String {
    if name.chars().count() <= max_chars {
        name.to_string()
    } else {
        let truncated: String = name.chars().take(max_chars - 1).collect();
        format!("{truncated}…")
    }
}

/// Applies display mode to a single tab
pub fn apply_display_mode_to_tab(widgets: &TabLabelWidgets, mode: TabDisplayMode) {
    match mode {
        TabDisplayMode::Full => {
            widgets.label.set_visible(true);
            widgets.label.set_text(&widgets.full_name);
            widgets.label.set_max_width_chars(20);
        }
        TabDisplayMode::Compact => {
            widgets.label.set_visible(true);
            let truncated = truncate_name(&widgets.full_name, 10);
            widgets.label.set_text(&truncated);
            widgets.label.set_max_width_chars(10);
        }
        TabDisplayMode::IconOnly => {
            widgets.label.set_visible(false);
        }
    }
}

/// Updates tab display mode based on available space
pub fn update_tab_display_mode(
    notebook: &Notebook,
    display_mode: &Rc<Cell<TabDisplayMode>>,
    tab_labels: &Rc<RefCell<HashMap<Uuid, TabLabelWidgets>>>,
    overflow_button: &MenuButton,
) {
    let available_width = notebook.width();
    if available_width <= 0 {
        return;
    }

    let tab_count = tab_labels.borrow().len();
    if tab_count == 0 {
        overflow_button.set_visible(false);
        return;
    }

    // Estimate tab widths for each mode
    let min_full = 150;
    let min_compact = 80;
    let min_icon = 40;

    let tab_count_i32 = tab_count as i32;

    let new_mode = if available_width >= tab_count_i32 * min_full {
        TabDisplayMode::Full
    } else if available_width >= tab_count_i32 * min_compact {
        TabDisplayMode::Compact
    } else {
        TabDisplayMode::IconOnly
    };

    // Show overflow button when even icon mode doesn't fit well
    let need_overflow = available_width < tab_count_i32 * min_icon;
    overflow_button.set_visible(need_overflow);

    // Update tabs if mode changed
    if new_mode != display_mode.get() {
        display_mode.set(new_mode);

        for widgets in tab_labels.borrow().values() {
            apply_display_mode_to_tab(widgets, new_mode);
        }
    }
}

/// Creates a tab label with protocol icon, title, close button, and drag source
#[allow(clippy::too_many_arguments)]
pub fn create_tab_label_with_protocol(
    title: &str,
    session_id: Uuid,
    notebook: &Notebook,
    sessions: &Rc<RefCell<HashMap<Uuid, u32>>>,
    protocol: &str,
    host: &str,
    tab_labels: &Rc<RefCell<HashMap<Uuid, TabLabelWidgets>>>,
    overflow_box: &GtkBox,
) -> GtkBox {
    let tab_box = GtkBox::new(Orientation::Horizontal, 4);
    tab_box.add_css_class("session-tab");

    // Set accessible properties for screen readers
    let accessible_label = format!("{title} - {protocol} session");
    let accessible_desc = if host.is_empty() {
        format!("{protocol} session tab. Drag to split pane or click close to end session.")
    } else {
        format!("{protocol} session to {host}. Drag to split pane or click close to end session.")
    };
    crate::utils::set_accessible_properties(&tab_box, &accessible_label, Some(&accessible_desc));

    // Protocol icon
    let icon_name = get_protocol_icon(protocol);
    let icon = Image::from_icon_name(icon_name);
    icon.set_pixel_size(16);
    icon.add_css_class("tab-icon");
    tab_box.append(&icon);

    // Label with ellipsis
    let label = Label::new(Some(title));
    label.set_hexpand(true);
    label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    label.set_max_width_chars(20);
    label.add_css_class("tab-label");
    tab_box.append(&label);

    // Close button
    let close_button = Button::from_icon_name("window-close-symbolic");
    close_button.add_css_class("flat");
    close_button.add_css_class("circular");
    close_button.add_css_class("tab-close-button");
    close_button.set_tooltip_text(Some("Close tab"));
    // Set accessible label for close button
    crate::utils::set_accessible_label(&close_button, &format!("Close {title} session"));

    // Connect close button - directly close this specific tab without switching first
    let notebook_weak = notebook.downgrade();
    let sessions_clone = sessions.clone();
    close_button.connect_clicked(move |button| {
        if notebook_weak.upgrade().is_some() {
            let sessions = sessions_clone.borrow();
            if sessions.contains_key(&session_id) {
                drop(sessions);
                // Activate close-tab-by-id action with session_id as parameter
                if let Some(root) = button.root() {
                    if let Some(window) = root.downcast_ref::<gtk4::ApplicationWindow>() {
                        let session_id_str = session_id.to_string();
                        gtk4::prelude::ActionGroupExt::activate_action(
                            window,
                            "close-tab-by-id",
                            Some(&session_id_str.to_variant()),
                        );
                    }
                }
            }
        }
    });

    tab_box.append(&close_button);

    // Add drag source for dragging sessions to split panes
    let drag_source = gtk4::DragSource::new();
    drag_source.set_actions(gdk::DragAction::MOVE);

    let session_id_str = session_id.to_string();
    drag_source.connect_prepare(move |_source, _x, _y| {
        let value = gtk4::glib::Value::from(&session_id_str);
        let content = gdk::ContentProvider::for_value(&value);
        Some(content)
    });

    tab_box.add_controller(drag_source);

    // Set tooltip with full name and host
    let tooltip = if host.is_empty() {
        format!("{title}\nDrag to split pane")
    } else {
        format!("{title}\n{host}\nDrag to split pane")
    };
    tab_box.set_tooltip_text(Some(&tooltip));

    // Store tab label widgets for adaptive display
    tab_labels.borrow_mut().insert(
        session_id,
        TabLabelWidgets {
            container: tab_box.clone(),
            icon: icon.clone(),
            label: label.clone(),
            full_name: title.to_string(),
        },
    );

    // Add to overflow menu
    add_to_overflow_menu(
        overflow_box,
        session_id,
        title,
        host,
        icon_name,
        notebook,
        sessions,
    );

    tab_box
}

/// Adds a session entry to the overflow menu
pub fn add_to_overflow_menu(
    overflow_box: &GtkBox,
    session_id: Uuid,
    title: &str,
    host: &str,
    icon_name: &str,
    notebook: &Notebook,
    sessions: &Rc<RefCell<HashMap<Uuid, u32>>>,
) {
    let row = GtkBox::new(Orientation::Horizontal, 8);
    row.add_css_class("overflow-item");
    row.set_margin_start(4);
    row.set_margin_end(4);
    row.set_margin_top(2);
    row.set_margin_bottom(2);

    let icon = Image::from_icon_name(icon_name);
    icon.set_pixel_size(16);
    row.append(&icon);

    let label = Label::new(Some(title));
    label.set_hexpand(true);
    label.set_halign(gtk4::Align::Start);
    label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    row.append(&label);

    // Set tooltip
    let tooltip = if host.is_empty() {
        title.to_string()
    } else {
        format!("{title}\n{host}")
    };
    row.set_tooltip_text(Some(&tooltip));

    // Make clickable - switch to this tab
    let gesture = gtk4::GestureClick::new();
    gesture.set_button(gdk::BUTTON_PRIMARY);
    let notebook_weak = notebook.downgrade();
    let sessions_clone = sessions.clone();
    gesture.connect_released(move |gesture, _, _, _| {
        if let Some(notebook) = notebook_weak.upgrade() {
            let sessions = sessions_clone.borrow();
            if let Some(&page_num) = sessions.get(&session_id) {
                notebook.set_current_page(Some(page_num));
            }
        }
        // Close popover
        if let Some(widget) = gesture.widget() {
            if let Some(popover) = widget.ancestor(Popover::static_type()) {
                if let Some(popover) = popover.downcast_ref::<Popover>() {
                    popover.popdown();
                }
            }
        }
    });
    row.add_controller(gesture);

    // Store session_id in widget name for removal
    row.set_widget_name(&session_id.to_string());

    overflow_box.append(&row);
}

/// Removes a session from the overflow menu
pub fn remove_from_overflow_menu(overflow_box: &GtkBox, session_id: Uuid) {
    let session_str = session_id.to_string();
    let mut child = overflow_box.first_child();
    while let Some(widget) = child {
        let next = widget.next_sibling();
        if widget.widget_name() == session_str {
            overflow_box.remove(&widget);
            break;
        }
        child = next;
    }
}
