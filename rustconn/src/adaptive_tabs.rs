//! Adaptive tab bar for managing many session tabs
//!
//! This module provides an adaptive tab bar that dynamically adjusts tab display
//! based on available space:
//! - Full mode: Icon + full name + close button
//! - Compact mode: Icon + truncated name + close button
//! - Icon mode: Icon only + close button
//! - Overflow mode: Shows overflow menu button when tabs don't fit
//!
//! All tabs have tooltips showing full connection name and host.

use gtk4::prelude::*;
use gtk4::{gdk, Box as GtkBox, Button, Image, Label, Orientation, Popover, ScrolledWindow};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use uuid::Uuid;

/// Tab display mode based on available space
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabDisplayMode {
    /// Full mode: icon + full name
    Full,
    /// Compact mode: icon + truncated name (max 12 chars)
    Compact,
    /// Icon mode: icon only
    IconOnly,
}

/// Information about a single tab
#[derive(Debug, Clone)]
pub struct TabInfo {
    /// Session ID
    pub session_id: Uuid,
    /// Connection ID
    pub connection_id: Uuid,
    /// Full display name
    pub name: String,
    /// Host/address for tooltip
    pub host: String,
    /// Protocol type (ssh, rdp, vnc, spice)
    pub protocol: String,
    /// Icon name for the protocol
    pub icon_name: String,
}

impl TabInfo {
    /// Creates a new tab info
    #[must_use]
    pub fn new(
        session_id: Uuid,
        connection_id: Uuid,
        name: String,
        host: String,
        protocol: String,
    ) -> Self {
        let icon_name = Self::get_protocol_icon(&protocol);
        Self {
            session_id,
            connection_id,
            name,
            host,
            protocol,
            icon_name,
        }
    }

    /// Gets the icon name for a protocol
    fn get_protocol_icon(protocol: &str) -> String {
        // Check for ZeroTrust with provider info (format: "zerotrust:provider")
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
            }
            .to_string();
        }

        match protocol.to_lowercase().as_str() {
            "ssh" => "network-server-symbolic",
            "rdp" => "computer-symbolic",
            "vnc" => "video-display-symbolic",
            "spice" => "video-x-generic-symbolic",
            "zerotrust" => "folder-remote-symbolic",
            _ => "network-server-symbolic",
        }
        .to_string()
    }

    /// Gets truncated name for compact mode
    #[must_use]
    pub fn truncated_name(&self, max_chars: usize) -> String {
        if self.name.chars().count() <= max_chars {
            self.name.clone()
        } else {
            let truncated: String = self.name.chars().take(max_chars - 1).collect();
            format!("{truncated}…")
        }
    }

    /// Gets tooltip text
    #[must_use]
    pub fn tooltip(&self) -> String {
        if self.host.is_empty() {
            self.name.clone()
        } else {
            format!("{}\n{}", self.name, self.host)
        }
    }
}

/// Single tab widget that adapts to display mode
#[allow(dead_code)] // Fields kept for GTK widget lifecycle
struct AdaptiveTab {
    container: GtkBox,
    icon: Image,
    label: Label,
    close_button: Button,
    info: TabInfo,
}

impl AdaptiveTab {
    /// Creates a new adaptive tab
    fn new(
        info: TabInfo,
        on_click: impl Fn(Uuid) + 'static,
        on_close: impl Fn(Uuid) + 'static,
    ) -> Self {
        let container = GtkBox::new(Orientation::Horizontal, 4);
        container.add_css_class("adaptive-tab");

        // Icon
        let icon = Image::from_icon_name(&info.icon_name);
        icon.set_pixel_size(16);
        container.append(&icon);

        // Label
        let label = Label::new(Some(&info.name));
        label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        label.set_max_width_chars(20);
        container.append(&label);

        // Close button
        let close_button = Button::from_icon_name("window-close-symbolic");
        close_button.add_css_class("flat");
        close_button.add_css_class("circular");
        close_button.set_tooltip_text(Some("Close tab"));
        container.append(&close_button);

        // Set tooltip
        container.set_tooltip_text(Some(&info.tooltip()));

        // Make clickable
        let gesture = gtk4::GestureClick::new();
        gesture.set_button(gdk::BUTTON_PRIMARY);
        let session_id = info.session_id;
        gesture.connect_released(move |_, _, _, _| {
            on_click(session_id);
        });
        container.add_controller(gesture);

        // Close button handler
        let session_id = info.session_id;
        close_button.connect_clicked(move |_| {
            on_close(session_id);
        });

        Self {
            container,
            icon,
            label,
            close_button,
            info,
        }
    }

    /// Updates display mode
    fn set_display_mode(&self, mode: TabDisplayMode) {
        match mode {
            TabDisplayMode::Full => {
                self.label.set_visible(true);
                self.label.set_text(&self.info.name);
                self.label.set_max_width_chars(20);
            }
            TabDisplayMode::Compact => {
                self.label.set_visible(true);
                self.label.set_text(&self.info.truncated_name(10));
                self.label.set_max_width_chars(10);
            }
            TabDisplayMode::IconOnly => {
                self.label.set_visible(false);
            }
        }
    }

    /// Sets whether this tab is active
    fn set_active(&self, active: bool) {
        if active {
            self.container.add_css_class("active-tab");
        } else {
            self.container.remove_css_class("active-tab");
        }
    }

    /// Gets the widget
    fn widget(&self) -> &GtkBox {
        &self.container
    }
}

/// Adaptive tab bar widget
pub struct AdaptiveTabBar {
    /// Main container
    container: GtkBox,
    /// Scrolled window for tabs
    scroll: ScrolledWindow,
    /// Box containing visible tabs
    tabs_box: GtkBox,
    /// Overflow menu button
    overflow_button: Button,
    /// Overflow popover
    overflow_popover: Popover,
    /// Box inside overflow popover
    overflow_box: GtkBox,
    /// All tabs
    tabs: Rc<RefCell<HashMap<Uuid, AdaptiveTab>>>,
    /// Tab order (for maintaining order)
    tab_order: Rc<RefCell<Vec<Uuid>>>,
    /// Currently active tab
    active_tab: Rc<Cell<Option<Uuid>>>,
    /// Current display mode
    display_mode: Rc<Cell<TabDisplayMode>>,
    /// Callback for tab selection
    on_select: Rc<RefCell<Option<Box<dyn Fn(Uuid)>>>>,
    /// Callback for tab close
    on_close: Rc<RefCell<Option<Box<dyn Fn(Uuid)>>>>,
    /// Minimum tab width in full mode
    min_tab_width_full: i32,
    /// Minimum tab width in compact mode
    min_tab_width_compact: i32,
    /// Minimum tab width in icon mode
    min_tab_width_icon: i32,
}

impl AdaptiveTabBar {
    /// Creates a new adaptive tab bar
    #[must_use]
    pub fn new() -> Self {
        let container = GtkBox::new(Orientation::Horizontal, 0);
        container.add_css_class("adaptive-tab-bar");

        // Scrolled window for tabs
        let scroll = ScrolledWindow::new();
        scroll.set_hexpand(true);
        scroll.set_policy(gtk4::PolicyType::External, gtk4::PolicyType::Never);
        scroll.set_propagate_natural_width(true);

        // Box for tabs
        let tabs_box = GtkBox::new(Orientation::Horizontal, 2);
        tabs_box.add_css_class("tabs-container");
        scroll.set_child(Some(&tabs_box));
        container.append(&scroll);

        // Overflow button
        let overflow_button = Button::from_icon_name("view-more-symbolic");
        overflow_button.add_css_class("flat");
        overflow_button.set_tooltip_text(Some("More tabs"));
        overflow_button.set_visible(false);
        container.append(&overflow_button);

        // Overflow popover
        let overflow_popover = Popover::new();
        overflow_popover.set_parent(&overflow_button);

        let overflow_scroll = ScrolledWindow::new();
        overflow_scroll.set_max_content_height(400);
        overflow_scroll.set_propagate_natural_height(true);
        overflow_scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);

        let overflow_box = GtkBox::new(Orientation::Vertical, 2);
        overflow_box.add_css_class("overflow-menu");
        overflow_scroll.set_child(Some(&overflow_box));
        overflow_popover.set_child(Some(&overflow_scroll));

        // Connect overflow button
        let popover = overflow_popover.clone();
        overflow_button.connect_clicked(move |_| {
            popover.popup();
        });

        let tab_bar = Self {
            container,
            scroll,
            tabs_box,
            overflow_button,
            overflow_popover,
            overflow_box,
            tabs: Rc::new(RefCell::new(HashMap::new())),
            tab_order: Rc::new(RefCell::new(Vec::new())),
            active_tab: Rc::new(Cell::new(None)),
            display_mode: Rc::new(Cell::new(TabDisplayMode::Full)),
            on_select: Rc::new(RefCell::new(None)),
            on_close: Rc::new(RefCell::new(None)),
            min_tab_width_full: 150,
            min_tab_width_compact: 80,
            min_tab_width_icon: 40,
        };

        // Connect resize handler
        tab_bar.setup_resize_handler();

        tab_bar
    }

    /// Sets up the resize handler for adaptive layout
    fn setup_resize_handler(&self) {
        let tabs = self.tabs.clone();
        let tab_order = self.tab_order.clone();
        let display_mode = self.display_mode.clone();
        let overflow_button = self.overflow_button.clone();
        let min_full = self.min_tab_width_full;
        let min_compact = self.min_tab_width_compact;
        let min_icon = self.min_tab_width_icon;

        self.scroll
            .connect_notify_local(Some("width-request"), move |scroll, _| {
                let available_width = scroll.width();
                if available_width <= 0 {
                    return;
                }

                let tabs_ref = tabs.borrow();
                let order = tab_order.borrow();
                let tab_count = order.len() as i32;

                if tab_count == 0 {
                    return;
                }

                // Calculate which mode fits
                let new_mode = if available_width >= tab_count * min_full {
                    TabDisplayMode::Full
                } else if available_width >= tab_count * min_compact {
                    TabDisplayMode::Compact
                } else if available_width >= tab_count * min_icon {
                    TabDisplayMode::IconOnly
                } else {
                    // Need overflow menu
                    TabDisplayMode::IconOnly
                };

                // Update tabs if mode changed
                if new_mode != display_mode.get() {
                    display_mode.set(new_mode);
                    for tab in tabs_ref.values() {
                        tab.set_display_mode(new_mode);
                    }
                }

                // Show/hide overflow button
                let need_overflow = available_width < tab_count * min_icon;
                overflow_button.set_visible(need_overflow);
            });
    }

    /// Adds a new tab
    pub fn add_tab(&self, info: TabInfo) {
        let session_id = info.session_id;

        // Create callbacks
        let on_select = self.on_select.clone();
        let active_tab = self.active_tab.clone();
        let tabs = self.tabs.clone();

        let on_click = move |id: Uuid| {
            // Update active state
            if let Some(prev_id) = active_tab.get() {
                if let Some(prev_tab) = tabs.borrow().get(&prev_id) {
                    prev_tab.set_active(false);
                }
            }
            if let Some(tab) = tabs.borrow().get(&id) {
                tab.set_active(true);
            }
            active_tab.set(Some(id));

            // Call user callback
            if let Some(ref callback) = *on_select.borrow() {
                callback(id);
            }
        };

        let on_close_cb = self.on_close.clone();
        let on_close = move |id: Uuid| {
            if let Some(ref callback) = *on_close_cb.borrow() {
                callback(id);
            }
        };

        let tab = AdaptiveTab::new(info.clone(), on_click, on_close);
        tab.set_display_mode(self.display_mode.get());

        // Add to visible tabs
        self.tabs_box.append(tab.widget());

        // Add to overflow menu
        self.add_to_overflow_menu(&info);

        // Store tab
        self.tabs.borrow_mut().insert(session_id, tab);
        self.tab_order.borrow_mut().push(session_id);

        // Trigger layout update
        self.update_layout();
    }

    /// Adds a tab entry to the overflow menu
    fn add_to_overflow_menu(&self, info: &TabInfo) {
        let row = GtkBox::new(Orientation::Horizontal, 8);
        row.add_css_class("overflow-item");
        row.set_margin_start(8);
        row.set_margin_end(8);
        row.set_margin_top(4);
        row.set_margin_bottom(4);

        let icon = Image::from_icon_name(&info.icon_name);
        icon.set_pixel_size(16);
        row.append(&icon);

        let label = Label::new(Some(&info.name));
        label.set_hexpand(true);
        label.set_halign(gtk4::Align::Start);
        label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        row.append(&label);

        // Make clickable
        let gesture = gtk4::GestureClick::new();
        gesture.set_button(gdk::BUTTON_PRIMARY);
        let session_id = info.session_id;
        let on_select = self.on_select.clone();
        let popover = self.overflow_popover.clone();
        gesture.connect_released(move |_, _, _, _| {
            popover.popdown();
            if let Some(ref callback) = *on_select.borrow() {
                callback(session_id);
            }
        });
        row.add_controller(gesture);

        // Store session_id in widget name for removal
        row.set_widget_name(&info.session_id.to_string());

        self.overflow_box.append(&row);
    }

    /// Removes a tab
    pub fn remove_tab(&self, session_id: Uuid) {
        // Remove from tabs
        if let Some(tab) = self.tabs.borrow_mut().remove(&session_id) {
            self.tabs_box.remove(tab.widget());
        }

        // Remove from order
        self.tab_order.borrow_mut().retain(|&id| id != session_id);

        // Remove from overflow menu
        let session_str = session_id.to_string();
        let mut child = self.overflow_box.first_child();
        while let Some(widget) = child {
            let next = widget.next_sibling();
            if widget.widget_name() == session_str {
                self.overflow_box.remove(&widget);
                break;
            }
            child = next;
        }

        // Update active tab if needed
        if self.active_tab.get() == Some(session_id) {
            self.active_tab.set(None);
        }

        self.update_layout();
    }

    /// Sets the active tab
    pub fn set_active(&self, session_id: Uuid) {
        // Deactivate previous
        if let Some(prev_id) = self.active_tab.get() {
            if let Some(prev_tab) = self.tabs.borrow().get(&prev_id) {
                prev_tab.set_active(false);
            }
        }

        // Activate new
        if let Some(tab) = self.tabs.borrow().get(&session_id) {
            tab.set_active(true);
        }
        self.active_tab.set(Some(session_id));
    }

    /// Sets the tab selection callback
    pub fn connect_tab_selected<F: Fn(Uuid) + 'static>(&self, callback: F) {
        *self.on_select.borrow_mut() = Some(Box::new(callback));
    }

    /// Sets the tab close callback
    pub fn connect_tab_closed<F: Fn(Uuid) + 'static>(&self, callback: F) {
        *self.on_close.borrow_mut() = Some(Box::new(callback));
    }

    /// Updates the layout based on available space
    fn update_layout(&self) {
        // Force a resize check
        self.scroll.queue_resize();
    }

    /// Gets the main widget
    #[must_use]
    pub fn widget(&self) -> &GtkBox {
        &self.container
    }

    /// Gets the number of tabs
    #[must_use]
    pub fn tab_count(&self) -> usize {
        self.tabs.borrow().len()
    }

    /// Checks if a tab exists
    #[must_use]
    pub fn has_tab(&self, session_id: Uuid) -> bool {
        self.tabs.borrow().contains_key(&session_id)
    }

    /// Updates tab info (e.g., when connection name changes)
    pub fn update_tab_info(&self, session_id: Uuid, name: &str, host: &str) {
        if let Some(tab) = self.tabs.borrow().get(&session_id) {
            // Update tooltip
            let tooltip = if host.is_empty() {
                name.to_string()
            } else {
                format!("{name}\n{host}")
            };
            tab.container.set_tooltip_text(Some(&tooltip));

            // Update label based on current mode
            match self.display_mode.get() {
                TabDisplayMode::Full => {
                    tab.label.set_text(name);
                }
                TabDisplayMode::Compact => {
                    let truncated = if name.chars().count() <= 10 {
                        name.to_string()
                    } else {
                        let t: String = name.chars().take(9).collect();
                        format!("{t}…")
                    };
                    tab.label.set_text(&truncated);
                }
                TabDisplayMode::IconOnly => {
                    // Label is hidden, nothing to update
                }
            }
        }

        // Update overflow menu
        let session_str = session_id.to_string();
        let mut child = self.overflow_box.first_child();
        while let Some(widget) = child {
            if widget.widget_name() == session_str {
                if let Some(row) = widget.downcast_ref::<GtkBox>() {
                    // Find and update label
                    let mut row_child = row.first_child();
                    while let Some(w) = row_child {
                        if let Some(label) = w.downcast_ref::<Label>() {
                            label.set_text(name);
                            break;
                        }
                        row_child = w.next_sibling();
                    }
                }
                break;
            }
            child = widget.next_sibling();
        }
    }
}

impl Default for AdaptiveTabBar {
    fn default() -> Self {
        Self::new()
    }
}

/// CSS styles for adaptive tabs
pub const ADAPTIVE_TABS_CSS: &str = r"
.adaptive-tab-bar {
    background: @theme_bg_color;
    border-bottom: 1px solid @borders;
}

.tabs-container {
    padding: 2px 4px;
}

.adaptive-tab {
    padding: 4px 8px;
    border-radius: 4px;
    min-height: 24px;
}

.adaptive-tab:hover {
    background: alpha(@theme_fg_color, 0.1);
}

.active-tab {
    background: alpha(@theme_selected_bg_color, 0.3);
}

.active-tab:hover {
    background: alpha(@theme_selected_bg_color, 0.4);
}

.overflow-menu {
    padding: 4px;
}

.overflow-item {
    padding: 6px 8px;
    border-radius: 4px;
}

.overflow-item:hover {
    background: alpha(@theme_fg_color, 0.1);
}
";
