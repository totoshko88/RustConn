//! WinRM protocol options for the connection dialog
//!
//! WinRM sessions run the external `evil-winrm` client in a VTE terminal.

use adw::prelude::*;
use gtk4::{Box as GtkBox, Entry};
use libadwaita as adw;

use super::protocol_layout::ProtocolLayoutBuilder;
use super::widgets::{EntryRowBuilder, SwitchRowBuilder};
use crate::i18n::i18n;

/// Container box, HTTPS switch and custom arguments entry.
pub type WinRmOptionsWidgets = (GtkBox, adw::SwitchRow, Entry);

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

    let (custom_args_row, custom_args_entry) = EntryRowBuilder::new(i18n("Custom Arguments"))
        .subtitle(i18n("Additional command-line arguments"))
        .placeholder("-s /path/to/scripts")
        .build();
    group.add(&custom_args_row);

    content.append(&group);
    (container, ssl_switch, custom_args_entry)
}
