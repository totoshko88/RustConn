//! VNC protocol options for the connection dialog
//!
//! This module provides the VNC-specific UI components including:
//! - Client mode selection (Embedded/External)
//! - Performance mode (Quality/Balanced/Speed)
//! - Encoding preferences
//! - Compression and quality settings
//! - View-only mode, scaling, clipboard sharing
//! - Jump host selection

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, DropDown, Entry, Orientation, ScrolledWindow, SpinButton, StringList};
use libadwaita as adw;
use rustconn_core::models::{ScaleOverride, VncClientMode, VncPerformanceMode};

use crate::i18n::i18n;

/// Creates the VNC options panel using libadwaita components following GNOME HIG.
#[expect(
    clippy::type_complexity,
    reason = "internal helper signature documents the exact tuple layout used by the caller; aliasing would obscure the data flow"
)]
pub(super) fn create_vnc_options() -> (
    GtkBox,
    DropDown,
    DropDown,
    DropDown,
    SpinButton,
    SpinButton,
    adw::SwitchRow,
    adw::SwitchRow,
    adw::SwitchRow,
    adw::SwitchRow,
    DropDown,
    Entry,
    DropDown,
) {
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
    let display_group = adw::PreferencesGroup::builder()
        .title(i18n("Display"))
        .build();

    // Client mode dropdown
    let vnc_mode_items: Vec<String> = vec![
        i18n(VncClientMode::Embedded.display_name()),
        i18n(VncClientMode::External.display_name()),
    ];
    let vnc_mode_strs: Vec<&str> = vnc_mode_items.iter().map(String::as_str).collect();
    let client_mode_list = StringList::new(&vnc_mode_strs);
    let client_mode_dropdown = DropDown::builder()
        .model(&client_mode_list)
        .valign(gtk4::Align::Center)
        .build();

    let client_mode_row = adw::ActionRow::builder()
        .title(i18n("Client Mode"))
        .subtitle(i18n(
            "Embedded renders in tab, External opens separate window",
        ))
        .build();
    client_mode_row.add_suffix(&client_mode_dropdown);
    display_group.add(&client_mode_row);

    // Performance mode dropdown
    let vnc_perf_items: Vec<String> = vec![
        i18n(VncPerformanceMode::Quality.display_name()),
        i18n(VncPerformanceMode::Balanced.display_name()),
        i18n(VncPerformanceMode::Speed.display_name()),
    ];
    let vnc_perf_strs: Vec<&str> = vnc_perf_items.iter().map(String::as_str).collect();
    let performance_mode_list = StringList::new(&vnc_perf_strs);
    let performance_mode_dropdown = DropDown::builder()
        .model(&performance_mode_list)
        .valign(gtk4::Align::Center)
        .build();
    performance_mode_dropdown.set_selected(1); // Default to Balanced

    let performance_mode_row = adw::ActionRow::builder()
        .title(i18n("Performance Mode"))
        .subtitle(i18n("Quality/speed tradeoff for image rendering"))
        .build();
    performance_mode_row.add_suffix(&performance_mode_dropdown);
    display_group.add(&performance_mode_row);

    // VNC-1: Encoding dropdown instead of free text entry
    let encoding_items: Vec<String> = vec![
        i18n("Auto"),
        "Tight".to_string(),
        "ZRLE".to_string(),
        "Hextile".to_string(),
        "Raw".to_string(),
        "CopyRect".to_string(),
    ];
    let encoding_strs: Vec<&str> = encoding_items.iter().map(String::as_str).collect();
    let encoding_list = StringList::new(&encoding_strs);
    let encoding_dropdown = DropDown::builder()
        .model(&encoding_list)
        .valign(gtk4::Align::Center)
        .build();

    let encoding_row = adw::ActionRow::builder()
        .title(i18n("Encoding"))
        .subtitle(i18n(
            "Preferred encoding method (overrides Performance Mode)",
        ))
        .build();
    encoding_row.add_suffix(&encoding_dropdown);
    display_group.add(&encoding_row);

    // Scale override dropdown (for embedded mode)
    let scale_items: Vec<String> = ScaleOverride::all()
        .iter()
        .map(|s| i18n(s.display_name()))
        .collect();
    let scale_strs: Vec<&str> = scale_items.iter().map(String::as_str).collect();
    let scale_list = StringList::new(&scale_strs);
    let scale_override_dropdown = DropDown::builder()
        .model(&scale_list)
        .valign(gtk4::Align::Center)
        .build();
    let scale_row = adw::ActionRow::builder()
        .title(i18n("Display Scale"))
        .subtitle(i18n("Override HiDPI scaling for embedded viewer"))
        .build();
    scale_row.add_suffix(&scale_override_dropdown);
    display_group.add(&scale_row);

    // Show scale row only in embedded mode
    let scale_row_clone = scale_row.clone();
    client_mode_dropdown.connect_selected_notify(move |dropdown| {
        let is_embedded = dropdown.selected() == 0;
        scale_row_clone.set_visible(is_embedded);
    });
    scale_row.set_visible(true); // Default: embedded

    content.append(&display_group);

    // === Quality Group ===
    let quality_group = adw::PreferencesGroup::builder()
        .title(i18n("Quality"))
        .build();

    // Compression
    let compression_adj = gtk4::Adjustment::new(6.0, 0.0, 9.0, 1.0, 1.0, 0.0);
    let compression_spin = SpinButton::builder()
        .adjustment(&compression_adj)
        .climb_rate(1.0)
        .digits(0)
        .valign(gtk4::Align::Center)
        .build();

    let compression_row = adw::ActionRow::builder()
        .title(i18n("Compression"))
        .subtitle(i18n("0 (none) to 9 (maximum)"))
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
        .title(i18n("Quality"))
        .subtitle(i18n("0 (lowest) to 9 (highest)"))
        .build();
    quality_row.add_suffix(&quality_spin);
    quality_group.add(&quality_row);

    // VNC-2: Sync compression/quality with Performance Mode changes
    let compression_spin_sync = compression_spin.clone();
    let quality_spin_sync = quality_spin.clone();
    performance_mode_dropdown.connect_selected_notify(move |dropdown| {
        let (comp, qual) = match dropdown.selected() {
            0 => (0.0, 9.0), // Quality
            2 => (9.0, 0.0), // Speed
            _ => (5.0, 5.0), // Balanced
        };
        compression_spin_sync.set_value(comp);
        quality_spin_sync.set_value(qual);
    });

    content.append(&quality_group);

    // === Features Group ===
    let features_group = adw::PreferencesGroup::builder()
        .title(i18n("Features"))
        .build();

    // View-only mode
    let view_only_switch = adw::SwitchRow::builder()
        .title(i18n("View-Only Mode"))
        .subtitle(i18n("Disable keyboard and mouse input"))
        .active(false)
        .build();
    features_group.add(&view_only_switch);

    // Scaling
    let scaling_switch = adw::SwitchRow::builder()
        .title(i18n("Scale Display"))
        .subtitle(i18n("Fit remote desktop to window size"))
        .active(true)
        .build();
    features_group.add(&scaling_switch);

    // Clipboard sharing
    let clipboard_switch = adw::SwitchRow::builder()
        .title(i18n("Clipboard Sharing"))
        .subtitle(i18n("Synchronize clipboard with remote"))
        .active(true)
        .build();
    features_group.add(&clipboard_switch);

    // Show local cursor
    let show_local_cursor_switch = adw::SwitchRow::builder()
        .title(i18n("Show Local Cursor"))
        .subtitle(i18n("Hide to avoid double cursor in embedded mode"))
        .active(true)
        .build();
    features_group.add(&show_local_cursor_switch);

    // VNC-3: Password info row
    let password_info_row = adw::ActionRow::builder()
        .title(i18n("Authentication"))
        .subtitle(i18n("VNC uses the connection password for authentication"))
        .activatable(false)
        .build();
    password_info_row.add_prefix(&gtk4::Image::from_icon_name("dialog-information-symbolic"));
    features_group.add(&password_info_row);

    content.append(&features_group);

    // === Advanced Group ===
    let advanced_group = adw::PreferencesGroup::builder()
        .title(i18n("Advanced"))
        .build();

    let custom_args_entry = Entry::builder()
        .hexpand(true)
        .placeholder_text(i18n("Additional arguments for external client"))
        .valign(gtk4::Align::Center)
        .build();

    let args_row = adw::ActionRow::builder()
        .title(i18n("Custom Arguments"))
        .subtitle(i18n("Extra command-line options for vncviewer"))
        .build();
    args_row.add_suffix(&custom_args_entry);
    advanced_group.add(&args_row);

    content.append(&advanced_group);

    // === Connection Group (Jump Host) ===
    let vnc_connection_group = adw::PreferencesGroup::builder()
        .title(i18n("Connection"))
        .build();

    let none_items: Vec<String> = vec![i18n("(None)")];
    let none_refs: Vec<&str> = none_items.iter().map(String::as_str).collect();
    let vnc_jump_host_list = StringList::new(&none_refs);
    let vnc_jump_host_dropdown = DropDown::new(Some(vnc_jump_host_list), gtk4::Expression::NONE);
    vnc_jump_host_dropdown.set_selected(0);
    vnc_jump_host_dropdown.set_enable_search(true);
    vnc_jump_host_dropdown.set_size_request(200, -1);
    vnc_jump_host_dropdown.set_hexpand(false);

    let vnc_jump_host_row = adw::ActionRow::builder()
        .title(i18n("Jump Host"))
        .subtitle(i18n("Tunnel VNC through an SSH connection"))
        .build();
    vnc_jump_host_row.add_suffix(&vnc_jump_host_dropdown);
    vnc_connection_group.add(&vnc_jump_host_row);

    content.append(&vnc_connection_group);

    clamp.set_child(Some(&content));
    scrolled.set_child(Some(&clamp));

    let vbox = GtkBox::new(Orientation::Vertical, 0);
    vbox.append(&scrolled);

    (
        vbox,
        client_mode_dropdown,
        performance_mode_dropdown,
        encoding_dropdown,
        compression_spin,
        quality_spin,
        view_only_switch,
        scaling_switch,
        clipboard_switch,
        show_local_cursor_switch,
        scale_override_dropdown,
        custom_args_entry,
        vnc_jump_host_dropdown,
    )
}
