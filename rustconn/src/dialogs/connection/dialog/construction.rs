//! Window/stack scaffolding and protocol dropdown wiring
//!
//! Mechanically split out of `dialog.rs` (pure code motion).

#![allow(
    clippy::similar_names,
    reason = "module-wide override for legacy code; refactored case by case"
)]

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, DropDown, Entry, Label, Orientation, ScrolledWindow, SpinButton, Stack,
};
use libadwaita as adw;

use super::ConnectionDialog;
use crate::i18n::i18n;

impl ConnectionDialog {
    /// Sets up inline validation for required fields
    pub(super) fn setup_inline_validation_for(dialog: &Self) {
        // Name entry validation
        dialog.name_entry.connect_changed(move |entry| {
            let text = entry.text();
            if text.trim().is_empty() {
                entry.add_css_class(crate::validation::ERROR_CSS_CLASS);
            } else {
                entry.remove_css_class(crate::validation::ERROR_CSS_CLASS);
            }
        });

        // Host entry validation (only when not Zero Trust)
        let protocol_dropdown = dialog.protocol_dropdown.clone();
        dialog.host_entry.connect_changed(move |entry| {
            // Skip validation for Zero Trust (index 4)
            if protocol_dropdown.selected() == 4 {
                entry.remove_css_class(crate::validation::ERROR_CSS_CLASS);
                return;
            }

            let text = entry.text();
            let is_invalid = text.trim().is_empty() || text.contains(' ');
            if is_invalid {
                entry.add_css_class(crate::validation::ERROR_CSS_CLASS);
            } else {
                entry.remove_css_class(crate::validation::ERROR_CSS_CLASS);
            }
        });

        // Clear host validation when switching to Zero Trust
        let host_entry = dialog.host_entry.clone();
        dialog
            .protocol_dropdown
            .connect_notify_local(Some("selected"), move |dropdown, _| {
                if dropdown.selected() == 4 {
                    host_entry.remove_css_class(crate::validation::ERROR_CSS_CLASS);
                }
            });
    }

    /// Creates the main dialog with header bar containing Save button
    pub(super) fn create_window_with_header(
        _parent: Option<&gtk4::Window>,
    ) -> (adw::Dialog, adw::HeaderBar, Button, Button) {
        // Distinct title from the simplified wizard (also "New Connection"),
        // so the full multi-tab editor is recognizable. Edit mode overrides
        // this later via set_connection().
        let dialog = adw::Dialog::builder()
            .title(i18n("New Connection (Advanced)"))
            .content_width(600)
            .content_height(730)
            .build();
        // Set minimum size on the dialog widget to suppress AdwDialog warnings
        dialog.set_width_request(360);
        dialog.set_height_request(400);

        // Header bar with Test icon and Create icon button (GNOME HIG)
        let header = adw::HeaderBar::new();
        let test_btn = Button::from_icon_name("network-transmit-receive-symbolic");
        test_btn.set_tooltip_text(Some(&i18n("Test Connection")));
        test_btn.update_property(&[gtk4::accessible::Property::Label(&i18n("Test connection"))]);
        let save_btn = Button::from_icon_name("list-add-symbolic");
        save_btn.set_tooltip_text(Some(&i18n("Create")));
        save_btn.update_property(&[gtk4::accessible::Property::Label(&i18n("Create"))]);
        save_btn.add_css_class("suggested-action");
        header.pack_start(&test_btn);
        header.pack_start(&save_btn);

        (dialog, header, save_btn, test_btn)
    }

    /// Creates the view stack widget and adds it to the dialog with a bottom
    /// tab bar, following the GNOME HIG pattern for multi-page dialogs
    /// (similar to GNOME Settings / Preferences).
    pub(super) fn create_view_stack(
        dialog: &adw::Dialog,
        header: &adw::HeaderBar,
    ) -> adw::ViewStack {
        let view_stack = adw::ViewStack::new();

        // Bottom tab bar — always visible (GNOME HIG for dialogs with many pages)
        let view_switcher_bar = adw::ViewSwitcherBar::builder()
            .stack(&view_stack)
            .reveal(true)
            .build();

        // Header bar shows the dialog title, no switcher
        header.set_title_widget(None::<&gtk4::Widget>);

        // Each tab provides its own ScrolledWindow, so the ViewStack sits
        // directly in the layout — no outer ScrolledWindow that would steal
        // height allocation from the per-tab scrollers.
        let main_box = GtkBox::new(Orientation::Vertical, 0);
        main_box.set_width_request(360);
        main_box.set_height_request(400);
        main_box.append(header);
        view_stack.set_vexpand(true);
        main_box.append(&view_stack);
        main_box.append(&view_switcher_bar);
        dialog.set_child(Some(&main_box));

        view_stack
    }

    /// Creates the protocol stack and adds it to the view stack
    pub(super) fn create_protocol_stack(view_stack: &adw::ViewStack) -> Stack {
        let protocol_stack = Stack::new();
        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .child(&protocol_stack)
            .build();
        view_stack
            .add_titled(&scrolled, Some("protocol"), &i18n("Protocol"))
            .set_icon_name(Some("network-server-symbolic"));
        protocol_stack
    }

    /// Connects the protocol dropdown to update the stack and port
    #[expect(
        clippy::too_many_arguments,
        reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
    )]
    pub(super) fn connect_protocol_dropdown(
        dropdown: &DropDown,
        stack: &Stack,
        port_spin: &SpinButton,
        host_entry: &Entry,
        host_label: &Label,
        port_label: &Label,
        username_entry: &Entry,
        username_label: &Label,
        tags_entry: &Entry,
        tags_label: &Label,
        password_source_dropdown: &DropDown,
        password_source_label: &Label,
        password_row: &GtkBox,
        domain_entry: &Entry,
        domain_label: &Label,
        mosh_settings_group: &adw::PreferencesGroup,
    ) {
        let stack_clone = stack.clone();
        let port_clone = port_spin.clone();
        let host_entry = host_entry.clone();
        let host_label = host_label.clone();
        let port_label = port_label.clone();
        let username_entry = username_entry.clone();
        let username_label = username_label.clone();
        let tags_entry = tags_entry.clone();
        let tags_label = tags_label.clone();
        let password_source_dropdown = password_source_dropdown.clone();
        let password_source_label = password_source_label.clone();
        let password_row = password_row.clone();
        let domain_entry = domain_entry.clone();
        let domain_label = domain_label.clone();
        let mosh_group = mosh_settings_group.clone();

        dropdown.connect_selected_notify(move |dropdown| {
            let protocols = [
                "ssh",
                "rdp",
                "vnc",
                "spice",
                "zerotrust",
                "telnet",
                "serial",
                "sftp",
                "kubernetes",
                "mosh",
                "web",
            ];
            let selected = dropdown.selected() as usize;
            if selected < protocols.len() {
                let protocol_id = protocols[selected];
                // SFTP and MOSH reuse SSH config tab
                let stack_name = if protocol_id == "sftp" || protocol_id == "mosh" {
                    "ssh"
                } else {
                    protocol_id
                };
                stack_clone.set_visible_child_name(stack_name);
                let default_port = Self::get_default_port(protocol_id);
                if Self::is_default_port(port_clone.value()) {
                    port_clone.set_value(default_port);
                }

                let is_zerotrust = protocol_id == "zerotrust";
                let is_serial = protocol_id == "serial";
                let is_kubernetes = protocol_id == "kubernetes";
                let is_web = protocol_id == "web";
                let hide_network = is_zerotrust || is_serial || is_kubernetes;
                let visible = !hide_network;

                host_entry.set_visible(visible || is_web);
                host_label.set_visible(visible || is_web);
                port_clone.set_visible(visible && !is_web);
                port_label.set_visible(visible && !is_web);
                username_entry.set_visible(visible);
                username_label.set_visible(visible);

                // Update host field label and placeholder for Web protocol
                if is_web {
                    host_label.set_text(&crate::i18n::i18n("URL"));
                    host_entry
                        .set_placeholder_text(Some(&crate::i18n::i18n("https://example.com")));
                } else {
                    host_label.set_text(&crate::i18n::i18n("Host"));
                    host_entry.set_placeholder_text(Some(&crate::i18n::i18n("hostname or IP")));
                }
                tags_entry.set_visible(!is_zerotrust);
                tags_label.set_visible(!is_zerotrust);

                // Password source only relevant for protocols that use credentials:
                // SSH, SFTP, RDP, VNC, SPICE, Web, Telnet. Telnet is an interactive
                // login protocol whose password source (typically None or Prompt) must
                // stay selectable — hiding it left older connections stuck on Vault,
                // which triggered a spurious "Vault entry not found" error (issue #210).
                // Hidden for Serial, MOSH, Kubernetes, Zero Trust — no stored passwords.
                let uses_password = matches!(
                    protocol_id,
                    "ssh" | "sftp" | "rdp" | "vnc" | "spice" | "web" | "telnet"
                );
                password_source_dropdown.set_visible(uses_password);
                password_source_label.set_visible(uses_password);
                // Password row visibility controlled by password_source_dropdown
                if !uses_password {
                    password_row.set_visible(false);
                }

                // Domain only relevant for RDP (GEN-2)
                let is_rdp = protocol_id == "rdp";
                domain_entry.set_visible(is_rdp);
                domain_label.set_visible(is_rdp);

                // MOSH settings group visible only when MOSH is selected
                mosh_group.set_visible(protocol_id == "mosh");
            }
        });
    }

    /// Returns the default port for a protocol
    pub(super) fn get_default_port(protocol_id: &str) -> f64 {
        match protocol_id {
            "rdp" => 3389.0,
            "vnc" | "spice" => 5900.0,
            "zerotrust" | "serial" | "kubernetes" => 0.0,
            "telnet" => 23.0,
            _ => 22.0, // ssh, sftp, mosh
        }
    }

    /// Checks if the port value is one of the default ports
    pub(super) fn is_default_port(port: f64) -> bool {
        const EPSILON: f64 = 0.5;
        (port - 22.0).abs() < EPSILON
            || (port - 23.0).abs() < EPSILON
            || (port - 3389.0).abs() < EPSILON
            || (port - 5900.0).abs() < EPSILON
            || port.abs() < EPSILON
    }
}
