//! WinRM protocol options for the connection dialog
//!
//! WinRM sessions run the external `evil-winrm` client in a VTE terminal.

use adw::prelude::*;
use gtk4::{Box as GtkBox, DropDown, Entry, StringList};
use libadwaita as adw;

use super::protocol_layout::ProtocolLayoutBuilder;
use super::widgets::{EntryRowBuilder, SwitchRowBuilder};
use crate::i18n::i18n;

/// Container box, HTTPS switch, certificate switch, jump host dropdown and custom
/// arguments entry.
pub type WinRmOptionsWidgets = (GtkBox, adw::SwitchRow, adw::SwitchRow, DropDown, Entry);

/// Creates the WinRM options panel.
#[must_use]
pub fn create_winrm_options() -> WinRmOptionsWidgets {
    let (container, content) = ProtocolLayoutBuilder::new().build();

    let group = adw::PreferencesGroup::builder()
        .title(i18n("Connection"))
        .description(i18n(
            "Opens a PowerShell session using the evil-winrm client, which must be installed. Your password is answered at its prompt and never appears on a command line.",
        ))
        .build();

    let ssl_switch = SwitchRowBuilder::new(i18n("Use HTTPS"))
        .subtitle(i18n(
            "Encrypts the connection. WinRM over HTTPS usually listens on port 5986",
        ))
        .active(false)
        .build();
    group.add(&ssl_switch);

    let accept_certs_switch = SwitchRowBuilder::new(i18n("Accept Invalid Certificates"))
        .subtitle(i18n(
            "Needed for a self-signed certificate or a jump host when running scripts. Terminal sessions never verify it",
        ))
        .active(false)
        .build();
    group.add(&accept_certs_switch);

    let none = i18n("(None)");
    let jump_host_dropdown = DropDown::new(
        Some(StringList::new(&[none.as_str()])),
        gtk4::Expression::NONE,
    );
    jump_host_dropdown.set_selected(0);
    crate::dialogs::widgets::enable_string_search(&jump_host_dropdown);
    jump_host_dropdown.set_size_request(200, -1);
    jump_host_dropdown.set_hexpand(false);
    jump_host_dropdown.set_valign(gtk4::Align::Center);
    jump_host_dropdown.update_property(&[gtk4::accessible::Property::Label(&i18n("Jump Host"))]);
    let jump_host_row = adw::ActionRow::builder()
        .title(i18n("Jump Host"))
        .subtitle(i18n("Tunnel WinRM through an SSH connection"))
        .build();
    jump_host_row.add_suffix(&jump_host_dropdown);
    group.add(&jump_host_row);

    let (custom_args_row, custom_args_entry) = EntryRowBuilder::new(i18n("Custom Arguments"))
        .subtitle(i18n("Additional command-line arguments"))
        .placeholder("-s /path/to/scripts")
        .build();
    group.add(&custom_args_row);

    content.append(&group);
    (
        container,
        ssl_switch,
        accept_certs_switch,
        jump_host_dropdown,
        custom_args_entry,
    )
}
