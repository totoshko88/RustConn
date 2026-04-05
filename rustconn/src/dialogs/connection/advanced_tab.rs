//! Advanced tab for the connection dialog
//!
//! Contains the Window Mode (embedded/external/fullscreen),
//! Wake-on-LAN configuration, and Terminal Theme override sections.

use crate::i18n::i18n;
use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, CheckButton, ColorDialogButton, DrawingArea, DropDown, Entry, Label,
    ListBox, Orientation, ScrolledWindow, SpinButton, StringList,
};
use libadwaita as adw;
use rustconn_core::wol::{DEFAULT_BROADCAST_ADDRESS, DEFAULT_WOL_PORT, DEFAULT_WOL_WAIT_SECONDS};

/// Creates the Advanced tab combining Display, WOL, and Terminal Theme settings.
///
/// Uses libadwaita components following GNOME HIG.
#[allow(clippy::type_complexity, clippy::similar_names)]
pub(super) fn create_advanced_tab() -> (
    GtkBox,
    DropDown,
    CheckButton,
    CheckButton,
    Entry,
    Entry,
    SpinButton,
    SpinButton,
    ColorDialogButton,
    ColorDialogButton,
    ColorDialogButton,
    Button,
    DrawingArea,
    adw::SwitchRow,
    ListBox,
    Button,
    DropDown,
    adw::ComboRow,
    adw::SpinRow,
    adw::SpinRow,
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

    // === Terminal Theme Section ===
    let theme_group = adw::PreferencesGroup::builder()
        .title(i18n("Terminal Theme"))
        .description(i18n("Override terminal colors for this connection"))
        .build();

    // Preset dropdown: DEV / QA / STAGE / PROD / DEMO / CUSTOM
    let preset_items = [
        i18n("Custom"),
        i18n("DEV"),
        i18n("QA"),
        i18n("STAGE"),
        i18n("PROD"),
        i18n("DEMO"),
    ];
    let preset_strs: Vec<&str> = preset_items.iter().map(String::as_str).collect();
    let preset_model = StringList::new(&preset_strs);
    let theme_preset_dropdown = DropDown::builder().model(&preset_model).selected(0).build();
    let preset_row = adw::ActionRow::builder()
        .title(i18n("Preset"))
        .subtitle(i18n("Quick color presets for environment identification"))
        .build();
    preset_row.add_suffix(&theme_preset_dropdown);
    theme_group.add(&preset_row);

    let color_dialog = gtk4::ColorDialog::new();

    // Default colors: black bg, white fg, white cursor (not GTK red)
    let default_black = gtk4::gdk::RGBA::new(0.0, 0.0, 0.0, 1.0);
    let default_white = gtk4::gdk::RGBA::new(1.0, 1.0, 1.0, 1.0);

    let theme_bg_button = ColorDialogButton::new(Some(color_dialog.clone()));
    theme_bg_button.set_valign(gtk4::Align::Center);
    theme_bg_button.set_rgba(&default_black);

    let bg_row = adw::ActionRow::builder().title(i18n("Background")).build();
    bg_row.add_suffix(&theme_bg_button);
    theme_group.add(&bg_row);

    let theme_fg_button = ColorDialogButton::new(Some(color_dialog.clone()));
    theme_fg_button.set_valign(gtk4::Align::Center);
    theme_fg_button.set_rgba(&default_white);

    let fg_row = adw::ActionRow::builder().title(i18n("Foreground")).build();
    fg_row.add_suffix(&theme_fg_button);
    theme_group.add(&fg_row);

    let theme_cursor_button = ColorDialogButton::new(Some(color_dialog));
    theme_cursor_button.set_valign(gtk4::Align::Center);
    theme_cursor_button.set_rgba(&default_white);

    let cursor_row = adw::ActionRow::builder()
        .title(i18n("Cursor Color"))
        .build();
    cursor_row.add_suffix(&theme_cursor_button);
    theme_group.add(&cursor_row);

    // Wire preset dropdown to apply colors
    {
        let bg_btn = theme_bg_button.clone();
        let fg_btn = theme_fg_button.clone();
        let cur_btn = theme_cursor_button.clone();
        theme_preset_dropdown.connect_selected_notify(move |dropdown| {
            let (bg_hex, fg_hex, cur_hex) = match dropdown.selected() {
                1 => ("#1a2b1a", "#d0e8d0", "#50c050"), // DEV — green
                2 => ("#1a1a2b", "#d0d0e8", "#5080e0"), // QA — blue
                3 => ("#2b2b1a", "#e8e8d0", "#e0c050"), // STAGE — yellow
                4 => ("#2b1a1a", "#e8d0d0", "#e05050"), // PROD — red
                5 => ("#2b1a2b", "#e8d0e8", "#c050c0"), // DEMO — purple
                _ => return,                            // CUSTOM — no changes
            };
            if let Some(c) = hex_to_rgba(bg_hex) {
                bg_btn.set_rgba(&c);
            }
            if let Some(c) = hex_to_rgba(fg_hex) {
                fg_btn.set_rgba(&c);
            }
            if let Some(c) = hex_to_rgba(cur_hex) {
                cur_btn.set_rgba(&c);
            }
        });
    }

    // Preview mini-rectangle showing chosen colors
    let theme_preview = DrawingArea::builder().height_request(40).build();

    // Wire preview to redraw when any color button changes
    {
        let preview = theme_preview.clone();
        let bg_btn = theme_bg_button.clone();
        let fg_btn = theme_fg_button.clone();
        let cur_btn = theme_cursor_button.clone();

        preview.set_draw_func(move |_area, cr, width, _height| {
            let bg = bg_btn.rgba();
            cr.set_source_rgba(
                f64::from(bg.red()),
                f64::from(bg.green()),
                f64::from(bg.blue()),
                f64::from(bg.alpha()),
            );
            cr.paint().ok();

            // Draw sample text "abc" in foreground color
            let fg = fg_btn.rgba();
            cr.set_source_rgba(
                f64::from(fg.red()),
                f64::from(fg.green()),
                f64::from(fg.blue()),
                f64::from(fg.alpha()),
            );
            cr.set_font_size(16.0);
            cr.move_to(8.0, 26.0);
            cr.show_text("abc").ok();

            // Draw cursor block on the right side
            let cur = cur_btn.rgba();
            cr.set_source_rgba(
                f64::from(cur.red()),
                f64::from(cur.green()),
                f64::from(cur.blue()),
                f64::from(cur.alpha()),
            );
            let cursor_x = f64::from(width) - 20.0;
            cr.rectangle(cursor_x, 8.0, 10.0, 24.0);
            cr.fill().ok();
        });
    }

    // Redraw preview when colors change
    {
        let preview_clone = theme_preview.clone();
        theme_bg_button.connect_rgba_notify(move |_| {
            preview_clone.queue_draw();
        });
    }
    {
        let preview_clone = theme_preview.clone();
        theme_fg_button.connect_rgba_notify(move |_| {
            preview_clone.queue_draw();
        });
    }
    {
        let preview_clone = theme_preview.clone();
        theme_cursor_button.connect_rgba_notify(move |_| {
            preview_clone.queue_draw();
        });
    }

    let preview_row = adw::ActionRow::builder().title(i18n("Preview")).build();
    preview_row.add_suffix(&theme_preview);
    theme_group.add(&preview_row);

    // Reset button to clear all theme overrides
    let theme_reset_button = Button::builder()
        .label(i18n("Reset Theme"))
        .css_classes(["destructive-action"])
        .valign(gtk4::Align::Center)
        .build();

    {
        let bg_btn = theme_bg_button.clone();
        let fg_btn = theme_fg_button.clone();
        let cur_btn = theme_cursor_button.clone();
        let preview = theme_preview.clone();
        theme_reset_button.connect_clicked(move |_| {
            let black = gtk4::gdk::RGBA::new(0.0, 0.0, 0.0, 1.0);
            let white = gtk4::gdk::RGBA::new(1.0, 1.0, 1.0, 1.0);
            bg_btn.set_rgba(&black);
            fg_btn.set_rgba(&white);
            cur_btn.set_rgba(&white);
            preview.queue_draw();
        });
    }

    let reset_row = adw::ActionRow::builder()
        .title(i18n("Reset"))
        .subtitle(i18n("Clear color overrides"))
        .activatable_widget(&theme_reset_button)
        .build();
    reset_row.add_suffix(&theme_reset_button);
    theme_group.add(&reset_row);

    content.append(&theme_group);

    // === Session Recording Section ===
    let recording_group = adw::PreferencesGroup::builder()
        .title(i18n("Session Recording"))
        .build();

    let recording_toggle = adw::SwitchRow::builder()
        .title(i18n("Record Session"))
        .subtitle(i18n("Save terminal output to file"))
        .build();
    recording_group.add(&recording_toggle);

    content.append(&recording_group);

    // === Activity Monitor Section ===
    let activity_monitor_group = adw::PreferencesGroup::builder()
        .title(i18n("Activity Monitor"))
        .description(i18n("Detect terminal output activity or silence"))
        .build();

    let mode_items = StringList::new(&[&i18n("Off"), &i18n("Activity"), &i18n("Silence")]);
    let activity_mode_combo = adw::ComboRow::builder()
        .title(i18n("Mode"))
        .subtitle(i18n("Select monitoring mode for this connection"))
        .model(&mode_items)
        .selected(0)
        .build();
    activity_monitor_group.add(&activity_mode_combo);

    let quiet_period_adj = gtk4::Adjustment::new(10.0, 1.0, 300.0, 1.0, 10.0, 0.0);
    let quiet_period_spin = adw::SpinRow::builder()
        .title(i18n("Quiet Period"))
        .subtitle(i18n("Seconds of silence before activity notification"))
        .adjustment(&quiet_period_adj)
        .visible(false)
        .build();
    activity_monitor_group.add(&quiet_period_spin);

    let silence_timeout_adj = gtk4::Adjustment::new(30.0, 1.0, 600.0, 1.0, 10.0, 0.0);
    let silence_timeout_spin = adw::SpinRow::builder()
        .title(i18n("Silence Timeout"))
        .subtitle(i18n("Seconds of no output before silence notification"))
        .adjustment(&silence_timeout_adj)
        .visible(false)
        .build();
    activity_monitor_group.add(&silence_timeout_spin);

    // Wire sensitivity: show/hide spin rows based on mode selection
    {
        let quiet_spin = quiet_period_spin.clone();
        let silence_spin = silence_timeout_spin.clone();
        activity_mode_combo.connect_selected_notify(move |combo| {
            match combo.selected() {
                1 => {
                    // Activity
                    quiet_spin.set_visible(true);
                    silence_spin.set_visible(false);
                }
                2 => {
                    // Silence
                    quiet_spin.set_visible(false);
                    silence_spin.set_visible(true);
                }
                _ => {
                    // Off
                    quiet_spin.set_visible(false);
                    silence_spin.set_visible(false);
                }
            }
        });
    }

    content.append(&activity_monitor_group);

    // === Highlight Rules Section ===
    let highlight_group = adw::PreferencesGroup::builder()
        .title(i18n("Highlight Rules"))
        .description(i18n("Regex-based text highlighting for this connection"))
        .build();

    let highlight_scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .min_content_height(120)
        .build();

    let highlight_rules_list = ListBox::builder()
        .selection_mode(gtk4::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    highlight_rules_list.set_placeholder(Some(&Label::new(Some(&i18n("No highlight rules")))));
    highlight_scrolled.set_child(Some(&highlight_rules_list));

    highlight_group.add(&highlight_scrolled);

    let hl_button_box = GtkBox::new(Orientation::Horizontal, 8);
    hl_button_box.set_halign(gtk4::Align::End);
    hl_button_box.set_margin_top(8);

    let add_highlight_rule_button = Button::builder()
        .label(&i18n("Add Rule"))
        .css_classes(["suggested-action"])
        .build();
    hl_button_box.append(&add_highlight_rule_button);

    highlight_group.add(&hl_button_box);
    content.append(&highlight_group);

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

    // MAC address format validation
    {
        let entry_clone = mac_entry.clone();
        mac_entry.connect_changed(move |_| {
            let text = entry_clone.text();
            let text = text.trim();
            if text.is_empty() {
                entry_clone.remove_css_class("error");
                entry_clone.set_tooltip_text(None);
                return;
            }
            let mac_re = text.split(':').collect::<Vec<_>>();
            let valid = mac_re.len() == 6
                && mac_re
                    .iter()
                    .all(|b| b.len() == 2 && b.chars().all(|c| c.is_ascii_hexdigit()));
            if valid {
                entry_clone.remove_css_class("error");
                entry_clone.set_tooltip_text(None);
            } else {
                entry_clone.add_css_class("error");
                entry_clone.set_tooltip_text(Some(&i18n("Format: AA:BB:CC:DD:EE:FF")));
            }
        });
    }

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
        theme_bg_button,
        theme_fg_button,
        theme_cursor_button,
        theme_reset_button,
        theme_preview,
        recording_toggle,
        highlight_rules_list,
        add_highlight_rule_button,
        theme_preset_dropdown,
        activity_mode_combo,
        quiet_period_spin,
        silence_timeout_spin,
    )
}

/// Converts a hex color string (`#RRGGBB` or `#RRGGBBAA`) to a GDK RGBA value.
///
/// Returns `None` if the string is not a valid hex color.
pub(super) fn hex_to_rgba(hex: &str) -> Option<gtk4::gdk::RGBA> {
    let hex = hex.strip_prefix('#')?;
    let (r, g, b, a) = match hex.len() {
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            (r, g, b, 255u8)
        }
        8 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
            (r, g, b, a)
        }
        _ => return None,
    };
    Some(gtk4::gdk::RGBA::new(
        f32::from(r) / 255.0,
        f32::from(g) / 255.0,
        f32::from(b) / 255.0,
        f32::from(a) / 255.0,
    ))
}

/// Converts a GDK RGBA value to a hex color string (`#RRGGBB`).
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_lossless
)]
pub(super) fn rgba_to_hex(rgba: &gtk4::gdk::RGBA) -> String {
    let r = (rgba.red() * 255.0).round() as u8;
    let g = (rgba.green() * 255.0).round() as u8;
    let b = (rgba.blue() * 255.0).round() as u8;
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Represents a highlight rule row in the connection dialog
pub(super) struct HighlightRuleRow {
    /// The row widget
    pub row: gtk4::ListBoxRow,
    /// Rule ID
    pub id: uuid::Uuid,
    /// Entry for rule name
    pub name_entry: Entry,
    /// Entry for regex pattern
    pub pattern_entry: Entry,
    /// Enabled switch
    pub enabled_check: CheckButton,
    /// Delete button
    pub delete_button: Button,
}

/// Creates a highlight rule row widget for the ListBox.
pub(super) fn create_highlight_rule_row(
    rule: Option<&rustconn_core::models::HighlightRule>,
) -> HighlightRuleRow {
    let id = rule.map_or_else(uuid::Uuid::new_v4, |r| r.id);

    let row = gtk4::ListBoxRow::builder()
        .activatable(false)
        .selectable(false)
        .build();

    let hbox = GtkBox::new(Orientation::Horizontal, 8);
    hbox.set_margin_top(6);
    hbox.set_margin_bottom(6);
    hbox.set_margin_start(8);
    hbox.set_margin_end(8);

    let name_entry = Entry::builder()
        .placeholder_text(i18n("Name"))
        .width_chars(12)
        .tooltip_text(i18n("Rule name"))
        .build();
    if let Some(r) = rule {
        name_entry.set_text(&r.name);
    }

    let pattern_entry = Entry::builder()
        .placeholder_text(i18n("Pattern (regex)"))
        .hexpand(true)
        .tooltip_text(i18n("Regex pattern to match"))
        .build();
    if let Some(r) = rule {
        pattern_entry.set_text(&r.pattern);
    }

    let enabled_check = CheckButton::builder()
        .active(rule.is_none_or(|r| r.enabled))
        .tooltip_text(i18n("Enable rule"))
        .valign(gtk4::Align::Center)
        .build();

    let delete_button = Button::builder()
        .icon_name("user-trash-symbolic")
        .css_classes(["flat"])
        .tooltip_text(i18n("Delete rule"))
        .valign(gtk4::Align::Center)
        .build();

    hbox.append(&name_entry);
    hbox.append(&pattern_entry);
    hbox.append(&enabled_check);
    hbox.append(&delete_button);

    row.set_child(Some(&hbox));

    HighlightRuleRow {
        row,
        id,
        name_entry,
        pattern_entry,
        enabled_check,
        delete_button,
    }
}
