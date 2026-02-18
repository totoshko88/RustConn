//! VNC protocol options for the connection dialog
//!
//! This module provides the VNC-specific UI components including:
//! - Client mode selection (Embedded/External)
//! - Performance mode (Quality/Balanced/Speed)
//! - Encoding preferences
//! - Compression and quality settings
//! - View-only mode, scaling, clipboard sharing

// These functions are prepared for future refactoring when dialog.rs is further modularized
#![allow(dead_code)]

use super::protocol_layout::ProtocolLayoutBuilder;
use super::widgets::{CheckboxRowBuilder, DropdownRowBuilder, EntryRowBuilder, SpinRowBuilder};
use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, CheckButton, DropDown, Entry, SpinButton};
use libadwaita as adw;
use rustconn_core::models::{ScaleOverride, VncClientMode, VncPerformanceMode};

/// Return type for VNC options creation
#[allow(clippy::type_complexity)]
pub type VncOptionsWidgets = (
    GtkBox,
    DropDown,    // client_mode_dropdown
    DropDown,    // performance_mode_dropdown
    Entry,       // encoding_entry
    SpinButton,  // compression_spin
    SpinButton,  // quality_spin
    CheckButton, // view_only_check
    CheckButton, // scaling_check
    CheckButton, // clipboard_check
    DropDown,    // scale_override_dropdown
    Entry,       // custom_args_entry
);

/// Creates the VNC options panel using libadwaita components following GNOME HIG.
#[must_use]
pub fn create_vnc_options() -> VncOptionsWidgets {
    let (container, content) = ProtocolLayoutBuilder::new().build();

    // === Display Group ===
    let (
        display_group,
        client_mode_dropdown,
        performance_mode_dropdown,
        encoding_entry,
        scale_override_dropdown,
    ) = create_display_group();
    content.append(&display_group);

    // === Quality Group ===
    let (quality_group, compression_spin, quality_spin) = create_quality_group();
    content.append(&quality_group);

    // === Features Group ===
    let (features_group, view_only_check, scaling_check, clipboard_check) = create_features_group();
    content.append(&features_group);

    // === Advanced Group ===
    let (advanced_group, custom_args_entry) = create_advanced_group();
    content.append(&advanced_group);

    (
        container,
        client_mode_dropdown,
        performance_mode_dropdown,
        encoding_entry,
        compression_spin,
        quality_spin,
        view_only_check,
        scaling_check,
        clipboard_check,
        scale_override_dropdown,
        custom_args_entry,
    )
}

/// Creates the Display preferences group
fn create_display_group() -> (adw::PreferencesGroup, DropDown, DropDown, Entry, DropDown) {
    let display_group = adw::PreferencesGroup::builder().title("Display").build();

    // Client mode dropdown
    let (client_mode_row, client_mode_dropdown) = DropdownRowBuilder::new("Client Mode")
        .subtitle("Embedded renders in tab, External opens separate window")
        .items(&[
            VncClientMode::Embedded.display_name(),
            VncClientMode::External.display_name(),
        ])
        .build();
    display_group.add(&client_mode_row);

    // Performance mode dropdown
    let (perf_row, performance_mode_dropdown) = DropdownRowBuilder::new("Performance Mode")
        .subtitle("Quality/speed tradeoff for image rendering")
        .items(&[
            VncPerformanceMode::Quality.display_name(),
            VncPerformanceMode::Balanced.display_name(),
            VncPerformanceMode::Speed.display_name(),
        ])
        .selected(1) // Default to Balanced
        .build();
    display_group.add(&perf_row);

    // Scale override dropdown (for embedded mode)
    let scale_items: Vec<&str> = ScaleOverride::all()
        .iter()
        .map(|s| s.display_name())
        .collect();
    let (scale_row, scale_override_dropdown) = DropdownRowBuilder::new("Display Scale")
        .subtitle("Override HiDPI scaling for embedded viewer")
        .items(&scale_items)
        .build();
    display_group.add(&scale_row);

    // Encoding
    let (encoding_row, encoding_entry) = EntryRowBuilder::new("Encoding")
        .subtitle("Preferred encoding methods (comma-separated)")
        .placeholder("tight, zrle, hextile")
        .build();
    display_group.add(&encoding_row);

    // Toggle scale row visibility based on client mode
    let scale_row_clone = scale_row.clone();
    client_mode_dropdown.connect_selected_notify(move |dropdown| {
        let is_embedded = dropdown.selected() == 0;
        scale_row_clone.set_visible(is_embedded);
    });

    (
        display_group,
        client_mode_dropdown,
        performance_mode_dropdown,
        encoding_entry,
        scale_override_dropdown,
    )
}

/// Creates the Quality preferences group
fn create_quality_group() -> (adw::PreferencesGroup, SpinButton, SpinButton) {
    let quality_group = adw::PreferencesGroup::builder().title("Quality").build();

    // Compression
    let (compression_row, compression_spin) = SpinRowBuilder::new("Compression")
        .subtitle("0 (none) to 9 (maximum)")
        .range(0.0, 9.0)
        .value(6.0)
        .build();
    quality_group.add(&compression_row);

    // Quality
    let (quality_row, quality_spin) = SpinRowBuilder::new("Quality")
        .subtitle("0 (lowest) to 9 (highest)")
        .range(0.0, 9.0)
        .value(6.0)
        .build();
    quality_group.add(&quality_row);

    (quality_group, compression_spin, quality_spin)
}

/// Creates the Features preferences group
fn create_features_group() -> (adw::PreferencesGroup, CheckButton, CheckButton, CheckButton) {
    let features_group = adw::PreferencesGroup::builder().title("Features").build();

    // View-only mode
    let (view_only_row, view_only_check) = CheckboxRowBuilder::new("View-Only Mode")
        .subtitle("Disable keyboard and mouse input")
        .build();
    features_group.add(&view_only_row);

    // Scaling
    let (scaling_row, scaling_check) = CheckboxRowBuilder::new("Scale Display")
        .subtitle("Fit remote desktop to window size")
        .active(true)
        .build();
    features_group.add(&scaling_row);

    // Clipboard sharing
    let (clipboard_row, clipboard_check) = CheckboxRowBuilder::new("Clipboard Sharing")
        .subtitle("Synchronize clipboard with remote")
        .active(true)
        .build();
    features_group.add(&clipboard_row);

    (
        features_group,
        view_only_check,
        scaling_check,
        clipboard_check,
    )
}

/// Creates the Advanced preferences group
fn create_advanced_group() -> (adw::PreferencesGroup, Entry) {
    let advanced_group = adw::PreferencesGroup::builder().title("Advanced").build();

    let (args_row, custom_args_entry) = EntryRowBuilder::new("Custom Arguments")
        .subtitle("Extra command-line options for vncviewer")
        .placeholder("Additional arguments for external client")
        .build();
    advanced_group.add(&args_row);

    (advanced_group, custom_args_entry)
}
