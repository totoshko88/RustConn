//! Telnet protocol options for the connection dialog
//!
//! UI panel for Telnet connections with keyboard behavior settings.
//! Telnet uses an external `telnet` CLI client via VTE terminal.

use super::protocol_layout::ProtocolLayoutBuilder;
use super::widgets::EntryRowBuilder;
use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, DropDown, Entry, StringList};
use libadwaita as adw;
use rustconn_core::models::{TelnetBackspaceSends, TelnetDeleteSends};

/// Return type for Telnet options creation
///
/// Contains:
/// - Container box
/// - Custom args entry
/// - Backspace sends dropdown
/// - Delete sends dropdown
pub type TelnetOptionsWidgets = (GtkBox, Entry, DropDown, DropDown);

/// Creates the Telnet options panel using libadwaita components.
///
/// The panel has groups for connection settings and keyboard behavior.
/// Keyboard settings address the common backspace/delete inversion
/// issue with Telnet connections.
#[must_use]
pub fn create_telnet_options() -> TelnetOptionsWidgets {
    let (container, content) = ProtocolLayoutBuilder::new().build();

    // === Connection Group ===
    let connection_group = adw::PreferencesGroup::builder()
        .title("Connection")
        .description("Telnet uses an external telnet client")
        .build();

    let (custom_args_row, custom_args_entry) = EntryRowBuilder::new("Custom Arguments")
        .subtitle("Additional command-line arguments")
        .placeholder("-e ^] -l user")
        .build();
    connection_group.add(&custom_args_row);

    content.append(&connection_group);

    // === Keyboard Group ===
    let keyboard_group = adw::PreferencesGroup::builder()
        .title("Keyboard")
        .description(
            "Configure key behavior for remote systems with \
             inverted backspace/delete",
        )
        .build();

    // Backspace sends dropdown
    let backspace_model = StringList::new(
        &TelnetBackspaceSends::all()
            .iter()
            .map(|m| m.display_name())
            .collect::<Vec<_>>(),
    );
    let backspace_dropdown = DropDown::builder()
        .model(&backspace_model)
        .selected(0)
        .build();
    let backspace_row = adw::ActionRow::builder()
        .title("Backspace sends")
        .subtitle("What the Backspace key sends to the remote")
        .build();
    backspace_row.add_suffix(&backspace_dropdown);
    backspace_row.set_activatable_widget(Some(&backspace_dropdown));
    keyboard_group.add(&backspace_row);

    // Delete sends dropdown
    let delete_model = StringList::new(
        &TelnetDeleteSends::all()
            .iter()
            .map(|m| m.display_name())
            .collect::<Vec<_>>(),
    );
    let delete_dropdown = DropDown::builder().model(&delete_model).selected(0).build();
    let delete_row = adw::ActionRow::builder()
        .title("Delete sends")
        .subtitle("What the Delete key sends to the remote")
        .build();
    delete_row.add_suffix(&delete_dropdown);
    delete_row.set_activatable_widget(Some(&delete_dropdown));
    keyboard_group.add(&delete_row);

    content.append(&keyboard_group);

    (
        container,
        custom_args_entry,
        backspace_dropdown,
        delete_dropdown,
    )
}
