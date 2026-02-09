//! Telnet protocol options for the connection dialog
//!
//! Minimal UI panel for Telnet connections. Telnet uses an external
//! `telnet` CLI client via VTE terminal (similar to SSH pattern).

use super::protocol_layout::ProtocolLayoutBuilder;
use super::widgets::EntryRowBuilder;
use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Entry};
use libadwaita as adw;

/// Return type for Telnet options creation
///
/// Contains:
/// - Container box
/// - Custom args entry
pub type TelnetOptionsWidgets = (GtkBox, Entry);

/// Creates the Telnet options panel using libadwaita components.
///
/// The panel has a single group for custom arguments since Telnet
/// is a simple protocol with no authentication options.
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

    (container, custom_args_entry)
}
