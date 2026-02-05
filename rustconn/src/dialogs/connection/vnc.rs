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

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, CheckButton, DropDown, Entry, Orientation, ScrolledWindow, SpinButton,
    StringList,
};
use libadwaita as adw;
use rustconn_core::models::{VncClientMode, VncPerformanceMode};

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
    Entry,       // custom_args_entry
);

/// Creates the VNC options panel using libadwaita components following GNOME HIG.
#[must_use]
pub fn create_vnc_options() -> VncOptionsWidgets {
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .vexpand(true)
        .build();

    let clamp = adw::Clamp::builder()
        .maximum_size(600)
        .tightening_threshold(400)
        .build();

    let content = GtkBox::new(Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    // === Display Group ===
    let (display_group, client_mode_dropdown, performance_mode_dropdown, encoding_entry) =
        create_display_group();
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

    clamp.set_child(Some(&content));
    scrolled.set_child(Some(&clamp));

    let vbox = GtkBox::new(Orientation::Vertical, 0);
    vbox.append(&scrolled);

    (
        vbox,
        client_mode_dropdown,
        performance_mode_dropdown,
        encoding_entry,
        compression_spin,
        quality_spin,
        view_only_check,
        scaling_check,
        clipboard_check,
        custom_args_entry,
    )
}

/// Creates the Display preferences group
fn create_display_group() -> (adw::PreferencesGroup, DropDown, DropDown, Entry) {
    let display_group = adw::PreferencesGroup::builder().title("Display").build();

    // Client mode dropdown
    let client_mode_list = StringList::new(&[
        VncClientMode::Embedded.display_name(),
        VncClientMode::External.display_name(),
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
        VncPerformanceMode::Quality.display_name(),
        VncPerformanceMode::Balanced.display_name(),
        VncPerformanceMode::Speed.display_name(),
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

    // Encoding
    let encoding_entry = Entry::builder()
        .hexpand(true)
        .placeholder_text("tight, zrle, hextile")
        .valign(gtk4::Align::Center)
        .build();

    let encoding_row = adw::ActionRow::builder()
        .title("Encoding")
        .subtitle("Preferred encoding methods (comma-separated)")
        .build();
    encoding_row.add_suffix(&encoding_entry);
    display_group.add(&encoding_row);

    (
        display_group,
        client_mode_dropdown,
        performance_mode_dropdown,
        encoding_entry,
    )
}

/// Creates the Quality preferences group
fn create_quality_group() -> (adw::PreferencesGroup, SpinButton, SpinButton) {
    let quality_group = adw::PreferencesGroup::builder().title("Quality").build();

    // Compression
    let compression_adj = gtk4::Adjustment::new(6.0, 0.0, 9.0, 1.0, 1.0, 0.0);
    let compression_spin = SpinButton::builder()
        .adjustment(&compression_adj)
        .climb_rate(1.0)
        .digits(0)
        .valign(gtk4::Align::Center)
        .build();

    let compression_row = adw::ActionRow::builder()
        .title("Compression")
        .subtitle("0 (none) to 9 (maximum)")
        .build();
    compression_row.add_suffix(&compression_spin);
    quality_group.add(&compression_row);

    // Quality
    let quality_adj = gtk4::Adjustment::new(6.0, 0.0, 9.0, 1.0, 1.0, 0.0);
    let quality_spin = SpinButton::builder()
        .adjustment(&quality_adj)
        .climb_rate(1.0)
        .digits(0)
        .valign(gtk4::Align::Center)
        .build();

    let quality_row = adw::ActionRow::builder()
        .title("Quality")
        .subtitle("0 (lowest) to 9 (highest)")
        .build();
    quality_row.add_suffix(&quality_spin);
    quality_group.add(&quality_row);

    (quality_group, compression_spin, quality_spin)
}

/// Creates the Features preferences group
fn create_features_group() -> (adw::PreferencesGroup, CheckButton, CheckButton, CheckButton) {
    let features_group = adw::PreferencesGroup::builder().title("Features").build();

    // View-only mode
    let view_only_check = CheckButton::new();
    let view_only_row = adw::ActionRow::builder()
        .title("View-Only Mode")
        .subtitle("Disable keyboard and mouse input")
        .activatable_widget(&view_only_check)
        .build();
    view_only_row.add_suffix(&view_only_check);
    features_group.add(&view_only_row);

    // Scaling
    let scaling_check = CheckButton::new();
    scaling_check.set_active(true);
    let scaling_row = adw::ActionRow::builder()
        .title("Scale Display")
        .subtitle("Fit remote desktop to window size")
        .activatable_widget(&scaling_check)
        .build();
    scaling_row.add_suffix(&scaling_check);
    features_group.add(&scaling_row);

    // Clipboard sharing
    let clipboard_check = CheckButton::new();
    clipboard_check.set_active(true);
    let clipboard_row = adw::ActionRow::builder()
        .title("Clipboard Sharing")
        .subtitle("Synchronize clipboard with remote")
        .activatable_widget(&clipboard_check)
        .build();
    clipboard_row.add_suffix(&clipboard_check);
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

    let custom_args_entry = Entry::builder()
        .hexpand(true)
        .placeholder_text("Additional arguments for external client")
        .valign(gtk4::Align::Center)
        .build();

    let args_row = adw::ActionRow::builder()
        .title("Custom Arguments")
        .subtitle("Extra command-line options for vncviewer")
        .build();
    args_row.add_suffix(&custom_args_entry);
    advanced_group.add(&args_row);

    (advanced_group, custom_args_entry)
}
