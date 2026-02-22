//! Serial protocol options for the connection dialog
//!
//! UI panel for Serial connections with device path, baud rate,
//! data bits, stop bits, parity, and flow control settings.
//! Serial uses an external `picocom` CLI client via VTE terminal.

use super::protocol_layout::ProtocolLayoutBuilder;
use super::widgets::EntryRowBuilder;
use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, DropDown, Entry, StringList};
use libadwaita as adw;
use rustconn_core::{
    SerialBaudRate, SerialDataBits, SerialFlowControl, SerialParity, SerialStopBits,
};

use crate::i18n::i18n;

/// Return type for Serial options creation
///
/// Contains:
/// - Container box
/// - Device entry
/// - Baud rate dropdown
/// - Data bits dropdown
/// - Stop bits dropdown
/// - Parity dropdown
/// - Flow control dropdown
/// - Custom args entry
pub type SerialOptionsWidgets = (
    GtkBox,
    Entry,
    DropDown,
    DropDown,
    DropDown,
    DropDown,
    DropDown,
    Entry,
);

/// Creates the Serial options panel using libadwaita components.
///
/// The panel has groups for device settings and line parameters.
#[must_use]
pub fn create_serial_options() -> SerialOptionsWidgets {
    let (container, content) = ProtocolLayoutBuilder::new().build();

    // === Device Group ===
    let device_group = adw::PreferencesGroup::builder()
        .title(i18n("Device"))
        .description(i18n("Serial uses picocom as the terminal client"))
        .build();

    let (device_row, device_entry) = EntryRowBuilder::new("Device Path")
        .subtitle("Path to the serial device")
        .placeholder("/dev/ttyUSB0")
        .build();
    device_group.add(&device_row);

    let (custom_args_row, custom_args_entry) = EntryRowBuilder::new("Custom Arguments")
        .subtitle("Additional picocom command-line arguments")
        .placeholder("--noreset --imap lfcrlf")
        .build();
    device_group.add(&custom_args_row);

    content.append(&device_group);

    // === Line Parameters Group ===
    let line_group = adw::PreferencesGroup::builder()
        .title(i18n("Line Parameters"))
        .description(i18n(
            "Standard serial line configuration (default: 115200 8N1)",
        ))
        .build();

    // Baud rate dropdown
    let baud_model = StringList::new(
        &SerialBaudRate::all()
            .iter()
            .map(|b| b.display_name())
            .collect::<Vec<_>>(),
    );
    let baud_dropdown = DropDown::builder()
        .model(&baud_model)
        .selected(SerialBaudRate::default().index())
        .build();
    let baud_row = adw::ActionRow::builder()
        .title(i18n("Baud Rate"))
        .subtitle(i18n("Communication speed in bits per second"))
        .build();
    baud_row.add_suffix(&baud_dropdown);
    baud_row.set_activatable_widget(Some(&baud_dropdown));
    line_group.add(&baud_row);

    // Data bits dropdown
    let data_bits_model = StringList::new(
        &SerialDataBits::all()
            .iter()
            .map(|d| d.display_name())
            .collect::<Vec<_>>(),
    );
    let data_bits_dropdown = DropDown::builder()
        .model(&data_bits_model)
        .selected(SerialDataBits::default().index())
        .build();
    let data_bits_row = adw::ActionRow::builder()
        .title(i18n("Data Bits"))
        .subtitle(i18n("Number of data bits per character"))
        .build();
    data_bits_row.add_suffix(&data_bits_dropdown);
    data_bits_row.set_activatable_widget(Some(&data_bits_dropdown));
    line_group.add(&data_bits_row);

    // Stop bits dropdown
    let stop_bits_model = StringList::new(
        &SerialStopBits::all()
            .iter()
            .map(|s| s.display_name())
            .collect::<Vec<_>>(),
    );
    let stop_bits_dropdown = DropDown::builder()
        .model(&stop_bits_model)
        .selected(SerialStopBits::default().index())
        .build();
    let stop_bits_row = adw::ActionRow::builder()
        .title(i18n("Stop Bits"))
        .subtitle(i18n("Number of stop bits"))
        .build();
    stop_bits_row.add_suffix(&stop_bits_dropdown);
    stop_bits_row.set_activatable_widget(Some(&stop_bits_dropdown));
    line_group.add(&stop_bits_row);

    // Parity dropdown
    let parity_model = StringList::new(
        &SerialParity::all()
            .iter()
            .map(|p| p.display_name())
            .collect::<Vec<_>>(),
    );
    let parity_dropdown = DropDown::builder()
        .model(&parity_model)
        .selected(SerialParity::default().index())
        .build();
    let parity_row = adw::ActionRow::builder()
        .title(i18n("Parity"))
        .subtitle(i18n("Error detection scheme"))
        .build();
    parity_row.add_suffix(&parity_dropdown);
    parity_row.set_activatable_widget(Some(&parity_dropdown));
    line_group.add(&parity_row);

    // Flow control dropdown
    let flow_model = StringList::new(
        &SerialFlowControl::all()
            .iter()
            .map(|f| f.display_name())
            .collect::<Vec<_>>(),
    );
    let flow_dropdown = DropDown::builder()
        .model(&flow_model)
        .selected(SerialFlowControl::default().index())
        .build();
    let flow_row = adw::ActionRow::builder()
        .title(i18n("Flow Control"))
        .subtitle(i18n("Data flow management method"))
        .build();
    flow_row.add_suffix(&flow_dropdown);
    flow_row.set_activatable_widget(Some(&flow_dropdown));
    line_group.add(&flow_row);

    content.append(&line_group);

    (
        container,
        device_entry,
        baud_dropdown,
        data_bits_dropdown,
        stop_bits_dropdown,
        parity_dropdown,
        flow_dropdown,
        custom_args_entry,
    )
}
