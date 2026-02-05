//! RDP protocol options for the connection dialog
//!
//! This module provides the RDP-specific UI components including:
//! - Client mode selection (Embedded/External)
//! - Performance mode (Quality/Balanced/Speed)
//! - Resolution and color depth settings
//! - Audio redirection
//! - RDP Gateway configuration
//! - Shared folders management

// These functions are prepared for future refactoring when dialog.rs is further modularized
#![allow(dead_code)]

use super::protocol_layout::ProtocolLayoutBuilder;
use super::shared_folders;
use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, CheckButton, DropDown, Entry, Label, Orientation, SpinButton, StringList,
};
use libadwaita as adw;
use rustconn_core::models::{RdpClientMode, RdpPerformanceMode, SharedFolder};
use std::cell::RefCell;
use std::rc::Rc;

/// Return type for RDP options creation
#[allow(clippy::type_complexity)]
pub type RdpOptionsWidgets = (
    GtkBox,
    DropDown,                       // client_mode_dropdown
    DropDown,                       // performance_mode_dropdown
    SpinButton,                     // width_spin
    SpinButton,                     // height_spin
    DropDown,                       // color_dropdown
    CheckButton,                    // audio_check
    Entry,                          // gateway_entry
    Rc<RefCell<Vec<SharedFolder>>>, // shared_folders
    gtk4::ListBox,                  // folders_list
    Entry,                          // custom_args_entry
);

/// Creates the RDP options panel using libadwaita components following GNOME HIG.
#[must_use]
pub fn create_rdp_options() -> RdpOptionsWidgets {
    let (container, content) = ProtocolLayoutBuilder::new().build();

    // === Display Group ===
    let (
        display_group,
        client_mode_dropdown,
        performance_mode_dropdown,
        width_spin,
        height_spin,
        color_dropdown,
    ) = create_display_group();
    content.append(&display_group);

    // === Features Group ===
    let (features_group, audio_check, gateway_entry) = create_features_group();
    content.append(&features_group);

    // === Shared Folders Group ===
    let (folders_group, shared_folders, folders_list) =
        shared_folders::create_shared_folders_group();
    content.append(&folders_group);

    // === Advanced Group ===
    let (advanced_group, args_entry) = create_advanced_group();
    content.append(&advanced_group);

    (
        container,
        client_mode_dropdown,
        performance_mode_dropdown,
        width_spin,
        height_spin,
        color_dropdown,
        audio_check,
        gateway_entry,
        shared_folders,
        folders_list,
        args_entry,
    )
}

/// Creates the Display preferences group
#[allow(clippy::type_complexity)]
fn create_display_group() -> (
    adw::PreferencesGroup,
    DropDown,
    DropDown,
    SpinButton,
    SpinButton,
    DropDown,
) {
    let display_group = adw::PreferencesGroup::builder().title("Display").build();

    // Client mode dropdown
    let client_mode_list = StringList::new(&[
        RdpClientMode::Embedded.display_name(),
        RdpClientMode::External.display_name(),
    ]);
    let client_mode_dropdown = DropDown::builder()
        .model(&client_mode_list)
        .valign(gtk4::Align::Center)
        .build();

    let client_mode_row = adw::ActionRow::builder()
        .title("Client Mode")
        .subtitle("Embedded renders in tab, External opens separate window")
        .build();
    client_mode_row.add_suffix(&client_mode_dropdown);
    display_group.add(&client_mode_row);

    // Performance mode dropdown
    let performance_mode_list = StringList::new(&[
        RdpPerformanceMode::Quality.display_name(),
        RdpPerformanceMode::Balanced.display_name(),
        RdpPerformanceMode::Speed.display_name(),
    ]);
    let performance_mode_dropdown = DropDown::builder()
        .model(&performance_mode_list)
        .valign(gtk4::Align::Center)
        .build();
    performance_mode_dropdown.set_selected(1); // Default to Balanced

    let performance_mode_row = adw::ActionRow::builder()
        .title("Performance Mode")
        .subtitle("Quality/speed tradeoff for image rendering")
        .build();
    performance_mode_row.add_suffix(&performance_mode_dropdown);
    display_group.add(&performance_mode_row);

    // Resolution
    let res_box = GtkBox::new(Orientation::Horizontal, 4);
    res_box.set_valign(gtk4::Align::Center);
    let width_adj = gtk4::Adjustment::new(1920.0, 640.0, 7680.0, 1.0, 100.0, 0.0);
    let width_spin = SpinButton::builder()
        .adjustment(&width_adj)
        .climb_rate(1.0)
        .digits(0)
        .build();
    let x_label = Label::new(Some("×"));
    let height_adj = gtk4::Adjustment::new(1080.0, 480.0, 4320.0, 1.0, 100.0, 0.0);
    let height_spin = SpinButton::builder()
        .adjustment(&height_adj)
        .climb_rate(1.0)
        .digits(0)
        .build();
    res_box.append(&width_spin);
    res_box.append(&x_label);
    res_box.append(&height_spin);

    let resolution_row = adw::ActionRow::builder()
        .title("Resolution")
        .subtitle("Width × Height in pixels")
        .build();
    resolution_row.add_suffix(&res_box);
    display_group.add(&resolution_row);

    // Color depth
    let color_list = StringList::new(&[
        "32-bit (True Color)",
        "24-bit",
        "16-bit (High Color)",
        "15-bit",
        "8-bit",
    ]);
    let color_dropdown = DropDown::new(Some(color_list), gtk4::Expression::NONE);
    color_dropdown.set_selected(0);
    color_dropdown.set_valign(gtk4::Align::Center);

    let color_row = adw::ActionRow::builder()
        .title("Color Depth")
        .subtitle("Higher values provide better quality")
        .build();
    color_row.add_suffix(&color_dropdown);
    display_group.add(&color_row);

    // Connect client mode dropdown to show/hide resolution/color rows
    let resolution_row_clone = resolution_row.clone();
    let color_row_clone = color_row.clone();
    client_mode_dropdown.connect_selected_notify(move |dropdown| {
        let is_embedded = dropdown.selected() == 0;
        resolution_row_clone.set_visible(!is_embedded);
        color_row_clone.set_visible(!is_embedded);
    });

    // Set initial state (Embedded - hide resolution/color)
    resolution_row.set_visible(false);
    color_row.set_visible(false);

    (
        display_group,
        client_mode_dropdown,
        performance_mode_dropdown,
        width_spin,
        height_spin,
        color_dropdown,
    )
}

/// Creates the Features preferences group
fn create_features_group() -> (adw::PreferencesGroup, CheckButton, Entry) {
    let features_group = adw::PreferencesGroup::builder().title("Features").build();

    // Audio redirect
    let audio_check = CheckButton::new();
    let audio_row = adw::ActionRow::builder()
        .title("Audio Redirection")
        .subtitle("Play remote audio locally")
        .activatable_widget(&audio_check)
        .build();
    audio_row.add_suffix(&audio_check);
    features_group.add(&audio_row);

    // Gateway
    let gateway_entry = Entry::builder()
        .hexpand(true)
        .placeholder_text("gateway.example.com")
        .valign(gtk4::Align::Center)
        .build();

    let gateway_row = adw::ActionRow::builder()
        .title("RDP Gateway")
        .subtitle("Remote Desktop Gateway server")
        .build();
    gateway_row.add_suffix(&gateway_entry);
    features_group.add(&gateway_row);

    (features_group, audio_check, gateway_entry)
}

/// Creates the Advanced preferences group
fn create_advanced_group() -> (adw::PreferencesGroup, Entry) {
    let advanced_group = adw::PreferencesGroup::builder().title("Advanced").build();

    let args_entry = Entry::builder()
        .hexpand(true)
        .placeholder_text("Additional command-line arguments")
        .valign(gtk4::Align::Center)
        .build();

    let args_row = adw::ActionRow::builder()
        .title("Custom Arguments")
        .subtitle("Extra FreeRDP command-line options")
        .build();
    args_row.add_suffix(&args_entry);
    advanced_group.add(&args_row);

    (advanced_group, args_entry)
}
