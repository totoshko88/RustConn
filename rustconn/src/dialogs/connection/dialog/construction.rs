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
    Box as GtkBox, Button, DropDown, Entry, Orientation, ScrolledWindow, SpinButton, Stack,
};
use libadwaita as adw;

use super::ConnectionDialog;
use crate::i18n::i18n;

/// Index of the Custom Command entry in the Zero Trust provider dropdown.
///
/// Mirrors `ZeroTrustProvider::Generic` — the order is fixed by
/// `zerotrust::create_zerotrust_options` and by `populate::set_zerotrust_config`.
const ZT_GENERIC_PROVIDER_INDEX: u32 = 10;

/// Index of `PasswordSource::Vault` in the password source dropdown — the one
/// source that shows the inline password entry (see
/// `passwords::update_password_row_visibility`).
const PASSWORD_SOURCE_VAULT_INDEX: u32 = 1;

/// Returns `true` when the Zero Trust provider dropdown has Custom Command selected.
fn is_custom_command_provider(zt_provider_dropdown: &DropDown) -> bool {
    zt_provider_dropdown.selected() == ZT_GENERIC_PROVIDER_INDEX
}

/// The General-tab widgets whose visibility depends on the selected protocol.
///
/// Grouped so the visibility rules live in one function that both the protocol
/// dropdown and the Zero Trust provider dropdown can call, instead of a
/// sixteen-parameter signature repeated per call site.
#[derive(Clone)]
pub(super) struct GeneralFields {
    /// Host (or URL for Web) entry
    pub host_entry: Entry,
    /// Row carrying the Host (or URL) title.
    ///
    /// Hiding and retitling act on the row, because the visible text of a
    /// boxed-list row is its own `title` property. Acting on a separate `Label`
    /// instead left the row titled "Host" under the Web protocol and kept an
    /// empty titled row on screen for protocols that have no host.
    pub host_row: adw::ActionRow,
    /// Port spin button
    pub port_spin: SpinButton,
    /// Row carrying the Port title — see [`Self::host_row`].
    pub port_row: adw::ActionRow,
    /// Row carrying the Username title — see [`Self::host_row`].
    pub username_row: adw::ActionRow,
    /// Row carrying the Tags title — see [`Self::host_row`].
    pub tags_row: adw::ActionRow,
    /// Password source dropdown
    pub password_source_dropdown: DropDown,
    /// Row carrying the Password Source title — see [`Self::host_row`].
    pub pw_source_row: adw::ActionRow,
    /// Inline password entry row, shown by the password source dropdown
    pub password_row: GtkBox,
    /// Row carrying the Domain title (RDP only) — see [`Self::host_row`].
    pub domain_row: adw::ActionRow,
    /// MOSH-specific settings group
    pub mosh_settings_group: adw::PreferencesGroup,
    /// Keyboard group on the SSH page, hidden for SFTP (issue #271)
    pub ssh_keyboard_group: adw::PreferencesGroup,
}

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

    /// Connects the protocol dropdown to update the stack and port.
    ///
    /// The Zero Trust provider dropdown is wired to the same visibility pass:
    /// Custom Command (the `Generic` provider) is the one Zero Trust variant that
    /// does use Host/Port/Username and a stored password, because its template
    /// resolves `${host}`, `${port}`, `${username}` and `${password}` (issue #151).
    pub(super) fn connect_protocol_dropdown(
        dropdown: &DropDown,
        stack: &Stack,
        zt_provider_dropdown: &DropDown,
        fields: &GeneralFields,
    ) {
        let stack_for_protocol = stack.clone();
        let fields_for_protocol = fields.clone();
        let zt_for_protocol = zt_provider_dropdown.clone();

        dropdown.connect_selected_notify(move |dropdown| {
            let Some(protocol_id) = Self::protocol_id_at(dropdown.selected()) else {
                return;
            };
            // SFTP and MOSH reuse SSH config tab
            let stack_name = if protocol_id == "sftp" || protocol_id == "mosh" {
                "ssh"
            } else {
                protocol_id
            };
            stack_for_protocol.set_visible_child_name(stack_name);
            let default_port = Self::get_default_port(protocol_id);
            if Self::is_default_port(fields_for_protocol.port_spin.value()) {
                fields_for_protocol.port_spin.set_value(default_port);
            }
            Self::apply_general_field_visibility(
                &fields_for_protocol,
                protocol_id,
                is_custom_command_provider(&zt_for_protocol),
            );
        });

        // Switching provider inside the Zero Trust page changes which General
        // fields matter, so re-run the same pass. Only meaningful while Zero
        // Trust is the selected protocol; `apply_general_field_visibility`
        // ignores the flag for every other protocol.
        let fields_for_provider = fields.clone();
        let protocol_for_provider = dropdown.clone();
        zt_provider_dropdown.connect_selected_notify(move |zt_dropdown| {
            let Some(protocol_id) = Self::protocol_id_at(protocol_for_provider.selected()) else {
                return;
            };
            Self::apply_general_field_visibility(
                &fields_for_provider,
                protocol_id,
                is_custom_command_provider(zt_dropdown),
            );
        });
    }

    /// Maps a protocol dropdown index to its protocol id.
    pub(super) fn protocol_id_at(selected: u32) -> Option<&'static str> {
        const PROTOCOLS: [&str; 11] = [
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
        PROTOCOLS.get(selected as usize).copied()
    }

    /// Shows or hides the General-tab fields that only apply to some protocols.
    ///
    /// `zt_custom_command` says whether the Zero Trust page currently has the
    /// Custom Command provider selected; it is ignored unless `protocol_id` is
    /// `zerotrust`.
    pub(super) fn apply_general_field_visibility(
        fields: &GeneralFields,
        protocol_id: &str,
        zt_custom_command: bool,
    ) {
        let is_zerotrust = protocol_id == "zerotrust";
        let is_serial = protocol_id == "serial";
        let is_kubernetes = protocol_id == "kubernetes";
        let is_web = protocol_id == "web";
        // Custom Command substitutes the connection fields into its template, so
        // it keeps the network rows the other Zero Trust providers hide (#151).
        let custom_command = is_zerotrust && zt_custom_command;
        let hide_network = (is_zerotrust && !custom_command) || is_serial || is_kubernetes;
        let visible = !hide_network;

        // Visibility is set on the rows, never on the entries they contain: the
        // entry is a suffix of the row, so hiding only the entry leaves a row
        // that still shows its title with nothing to type into.
        fields.host_row.set_visible(visible || is_web);
        fields.port_row.set_visible(visible && !is_web);
        fields.username_row.set_visible(visible);

        // Update host row title and placeholder for Web protocol
        if is_web {
            fields.host_row.set_title(&crate::i18n::i18n("URL"));
            fields
                .host_entry
                .set_placeholder_text(Some(&crate::i18n::i18n("https://example.com")));
        } else {
            fields.host_row.set_title(&crate::i18n::i18n("Host"));
            fields
                .host_entry
                .set_placeholder_text(Some(&crate::i18n::i18n("hostname or IP")));
        }
        // Tags are organisation metadata (search, smart folders) and
        // apply to every protocol — including Custom Command (#151).
        fields.tags_row.set_visible(true);

        // Password source only relevant for protocols that use credentials:
        // SSH, SFTP, RDP, VNC, SPICE, Web, Telnet. Telnet is an interactive
        // login protocol whose password source (typically None or Prompt) must
        // stay selectable — hiding it left older connections stuck on Vault,
        // which triggered a spurious "Vault entry not found" error (issue #210).
        // Custom Command joins them because `${password}` resolves from the
        // stored password (#151). Hidden for Serial, MOSH, Kubernetes and the
        // remaining Zero Trust providers — no stored passwords.
        let uses_password = custom_command
            || matches!(
                protocol_id,
                "ssh" | "sftp" | "rdp" | "vnc" | "spice" | "web" | "telnet"
            );
        fields.pw_source_row.set_visible(uses_password);
        // The inline password row belongs to the Vault source. Recomputed rather
        // than only hidden, so the row reappears when a protocol switch (or the
        // Zero Trust provider switch) re-enables credentials — otherwise the
        // order in which `populate_from_connection` sets the source and the
        // protocol would decide whether an existing password is visible.
        fields.password_row.set_visible(
            uses_password
                && fields.password_source_dropdown.selected() == PASSWORD_SOURCE_VAULT_INDEX,
        );

        // Domain only relevant for RDP (GEN-2)
        let is_rdp = protocol_id == "rdp";
        fields.domain_row.set_visible(is_rdp);

        // MOSH settings group visible only when MOSH is selected
        fields
            .mosh_settings_group
            .set_visible(protocol_id == "mosh");
        // The Keyboard group lives on the SSH page, which SFTP and MOSH share.
        // MOSH is a terminal protocol and honours the choice; SFTP opens an `mc`
        // file-manager tab that never applies it, so offering it there would be
        // a dead control (issue #271).
        fields.ssh_keyboard_group.set_visible(protocol_id != "sftp");
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
