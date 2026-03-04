//! Advanced tab for the connection dialog
//!
//! Contains the Window Mode (embedded/external/fullscreen) and
//! Wake-on-LAN configuration sections.

use crate::i18n::i18n;
use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, CheckButton, DropDown, Entry, Orientation, ScrolledWindow, SpinButton,
    StringList,
};
use libadwaita as adw;
use rustconn_core::wol::{DEFAULT_BROADCAST_ADDRESS, DEFAULT_WOL_PORT, DEFAULT_WOL_WAIT_SECONDS};

/// Creates the Advanced tab combining Display and WOL settings.
///
/// Uses libadwaita components following GNOME HIG.
#[allow(clippy::type_complexity)]
pub(super) fn create_advanced_tab() -> (
    GtkBox,
    DropDown,
    CheckButton,
    CheckButton,
    Entry,
    Entry,
    SpinButton,
    SpinButton,
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

    // === Window Mode Section ===
    let mode_group = adw::PreferencesGroup::builder()
        .title(i18n("Window Mode"))
        .build();

    let mode_list = StringList::new(&[
        &i18n("Embedded"),
        &i18n("External Window"),
        &i18n("Fullscreen"),
    ]);
    let mode_dropdown = DropDown::new(Some(mode_list), gtk4::Expression::NONE);
    mode_dropdown.set_selected(0);
    mode_dropdown.set_valign(gtk4::Align::Center);

    let mode_row = adw::ActionRow::builder()
        .title(i18n("Display Mode"))
        .subtitle(i18n("Embedded • External • Fullscreen"))
        .build();
    mode_row.add_suffix(&mode_dropdown);
    mode_group.add(&mode_row);

    let remember_check = CheckButton::builder()
        .valign(gtk4::Align::Center)
        .sensitive(false)
        .build();

    let remember_row = adw::ActionRow::builder()
        .title(i18n("Remember Position"))
        .subtitle(i18n("Save window geometry (External mode only)"))
        .activatable_widget(&remember_check)
        .build();
    remember_row.add_suffix(&remember_check);
    mode_group.add(&remember_row);

    let remember_check_clone = remember_check.clone();
    let remember_row_clone = remember_row.clone();
    mode_dropdown.connect_selected_notify(move |dropdown| {
        let is_external = dropdown.selected() == 1;
        remember_check_clone.set_sensitive(is_external);
        remember_row_clone.set_sensitive(is_external);
        if !is_external {
            remember_check_clone.set_active(false);
        }
    });

    content.append(&mode_group);

    // === Wake On LAN Section ===
    let wol_group = adw::PreferencesGroup::builder()
        .title(i18n("Wake On LAN"))
        .build();

    let wol_enabled_check = CheckButton::builder().valign(gtk4::Align::Center).build();

    let wol_enable_row = adw::ActionRow::builder()
        .title(i18n("Enable WOL"))
        .subtitle(i18n("Send magic packet before connecting"))
        .activatable_widget(&wol_enabled_check)
        .build();
    wol_enable_row.add_suffix(&wol_enabled_check);
    wol_group.add(&wol_enable_row);

    content.append(&wol_group);

    // WOL Settings group
    let wol_settings_group = adw::PreferencesGroup::builder()
        .title(i18n("WOL Settings"))
        .sensitive(false)
        .build();

    let mac_entry = Entry::builder()
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .placeholder_text(i18n("AA:BB:CC:DD:EE:FF"))
        .build();

    let mac_row = adw::ActionRow::builder().title(i18n("MAC Address")).build();
    mac_row.add_suffix(&mac_entry);
    wol_settings_group.add(&mac_row);

    let broadcast_entry = Entry::builder()
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .text(DEFAULT_BROADCAST_ADDRESS)
        .build();

    let broadcast_row = adw::ActionRow::builder()
        .title(i18n("Broadcast Address"))
        .build();
    broadcast_row.add_suffix(&broadcast_entry);
    wol_settings_group.add(&broadcast_row);

    let port_adjustment =
        gtk4::Adjustment::new(f64::from(DEFAULT_WOL_PORT), 1.0, 65535.0, 1.0, 10.0, 0.0);
    let port_spin = SpinButton::builder()
        .adjustment(&port_adjustment)
        .digits(0)
        .valign(gtk4::Align::Center)
        .build();

    let port_row = adw::ActionRow::builder()
        .title(i18n("UDP Port"))
        .subtitle(i18n("Default: 9"))
        .build();
    port_row.add_suffix(&port_spin);
    wol_settings_group.add(&port_row);

    let wait_adjustment = gtk4::Adjustment::new(
        f64::from(DEFAULT_WOL_WAIT_SECONDS),
        0.0,
        300.0,
        1.0,
        10.0,
        0.0,
    );
    let wait_spin = SpinButton::builder()
        .adjustment(&wait_adjustment)
        .digits(0)
        .valign(gtk4::Align::Center)
        .build();

    let wait_row = adw::ActionRow::builder()
        .title(i18n("Wait Time (sec)"))
        .subtitle(i18n("Time to wait for boot"))
        .build();
    wait_row.add_suffix(&wait_spin);
    wol_settings_group.add(&wait_row);

    content.append(&wol_settings_group);

    // Connect WOL enabled checkbox
    let wol_settings_group_clone = wol_settings_group.clone();
    wol_enabled_check.connect_toggled(move |check| {
        wol_settings_group_clone.set_sensitive(check.is_active());
    });

    clamp.set_child(Some(&content));
    scrolled.set_child(Some(&clamp));

    let vbox = GtkBox::new(Orientation::Vertical, 0);
    vbox.append(&scrolled);

    (
        vbox,
        mode_dropdown,
        remember_check,
        wol_enabled_check,
        mac_entry,
        broadcast_entry,
        port_spin,
        wait_spin,
    )
}
