//! Advanced tab for the connection dialog
//!
//! Contains Terminal Theme override, Remote Monitoring, Session Recording,
//! Activity Monitor, Highlight Rules, and Wake-on-LAN configuration sections.

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, CheckButton, ColorDialogButton, DrawingArea, DropDown, Entry, Label,
    ListBox, Orientation, ScrolledWindow, SpinButton, StringList,
};
use libadwaita as adw;
use rustconn_core::activity_monitor::MonitorMode;
use rustconn_core::monitoring::MonitoringOverride;
use rustconn_core::wol::{DEFAULT_BROADCAST_ADDRESS, DEFAULT_WOL_PORT, DEFAULT_WOL_WAIT_SECONDS};

use crate::dialogs::widgets::highlight_fields;
use crate::i18n::i18n;

/// Creates the Advanced tab combining Terminal Theme, Monitoring, Recording,
/// Activity Monitor, Highlight Rules, and WOL settings.
///
/// Uses libadwaita components following GNOME HIG.
#[expect(
    clippy::similar_names,
    clippy::type_complexity,
    reason = "internal helper signature documents the tuple layout with paired naming; aliasing would obscure the data flow"
)]
pub(super) fn create_advanced_tab() -> (
    GtkBox,
    CheckButton,
    Entry,  // mac_entry
    Button, // get_mac_button
    Entry,  // broadcast_entry
    SpinButton,
    SpinButton,
    ColorDialogButton,
    ColorDialogButton,
    ColorDialogButton,
    Button,
    DrawingArea,
    adw::ComboRow,
    adw::SwitchRow,
    ListBox,
    Button,
    DropDown,
    adw::ComboRow,
    adw::SpinRow,
    adw::SpinRow,
    adw::SwitchRow,
    adw::SpinRow,
    adw::SpinRow,
    adw::SpinRow,
    adw::SwitchRow,
    gtk4::Entry,
    // SPA fields
    adw::SwitchRow,
    adw::PasswordEntryRow,
    adw::PasswordEntryRow,
    adw::EntryRow,
    adw::SpinRow,
    adw::ComboRow,
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

    // === Terminal Theme Section (collapsible) ===
    let theme_group = adw::PreferencesGroup::builder().build();
    let theme_expander = adw::ExpanderRow::builder()
        .title(i18n("Terminal Theme"))
        .subtitle(i18n("Override terminal colors for this connection"))
        .show_enable_switch(false)
        .build();

    // Preset dropdown: built-in environment presets + user custom themes
    let builtin_presets = [
        i18n("Custom"),
        i18n("DEV"),
        i18n("QA"),
        i18n("STAGE"),
        i18n("PROD"),
        i18n("DEMO"),
    ];
    let custom_theme_names = rustconn_core::terminal_themes::TerminalTheme::custom_theme_names();
    let mut preset_items: Vec<String> = builtin_presets.to_vec();
    preset_items.extend(custom_theme_names.clone());
    let preset_strs: Vec<&str> = preset_items.iter().map(String::as_str).collect();
    let preset_model = StringList::new(&preset_strs);
    let theme_preset_dropdown = DropDown::builder().model(&preset_model).selected(0).build();
    let preset_row = adw::ActionRow::builder()
        .title(i18n("Preset"))
        .subtitle(i18n("Quick color presets for environment identification"))
        .build();
    preset_row.add_suffix(&theme_preset_dropdown);
    theme_expander.add_row(&preset_row);

    let color_dialog = gtk4::ColorDialog::new();

    // Default colors: black bg, white fg, white cursor (not GTK red)
    let default_black = gtk4::gdk::RGBA::new(0.0, 0.0, 0.0, 1.0);
    let default_white = gtk4::gdk::RGBA::new(1.0, 1.0, 1.0, 1.0);

    let theme_bg_button = ColorDialogButton::new(Some(color_dialog.clone()));
    theme_bg_button.set_valign(gtk4::Align::Center);
    theme_bg_button.set_rgba(&default_black);

    let bg_row = adw::ActionRow::builder().title(i18n("Background")).build();
    bg_row.add_suffix(&theme_bg_button);
    theme_expander.add_row(&bg_row);

    let theme_fg_button = ColorDialogButton::new(Some(color_dialog.clone()));
    theme_fg_button.set_valign(gtk4::Align::Center);
    theme_fg_button.set_rgba(&default_white);

    let fg_row = adw::ActionRow::builder().title(i18n("Foreground")).build();
    fg_row.add_suffix(&theme_fg_button);
    theme_expander.add_row(&fg_row);

    let theme_cursor_button = ColorDialogButton::new(Some(color_dialog));
    theme_cursor_button.set_valign(gtk4::Align::Center);
    theme_cursor_button.set_rgba(&default_white);

    let cursor_row = adw::ActionRow::builder()
        .title(i18n("Cursor Color"))
        .build();
    cursor_row.add_suffix(&theme_cursor_button);
    theme_expander.add_row(&cursor_row);

    // Wire preset dropdown to apply colors (built-in + custom themes)
    {
        let bg_btn = theme_bg_button.clone();
        let fg_btn = theme_fg_button.clone();
        let cur_btn = theme_cursor_button.clone();
        let custom_names = custom_theme_names.clone();
        theme_preset_dropdown.connect_selected_notify(move |dropdown| {
            let selected = dropdown.selected();
            let (bg_hex, fg_hex, cur_hex) = match selected {
                1 => ("#1a2b1a", "#d0e8d0", "#50c050"), // DEV — green
                2 => ("#1a1a2b", "#d0d0e8", "#5080e0"), // QA — blue
                3 => ("#2b2b1a", "#e8e8d0", "#e0c050"), // STAGE — yellow
                4 => ("#2b1a1a", "#e8d0d0", "#e05050"), // PROD — red
                5 => ("#2b1a2b", "#e8d0e8", "#c050c0"), // DEMO — purple
                idx if idx >= 6 => {
                    // Custom theme from Settings → Terminal → Colors
                    let theme_idx = (idx - 6) as usize;
                    if let Some(name) = custom_names.get(theme_idx)
                        && let Some(theme) =
                            rustconn_core::terminal_themes::TerminalTheme::by_name(name)
                    {
                        let bg = theme.background.to_hex();
                        let fg = theme.foreground.to_hex();
                        let cur = theme.cursor.to_hex();
                        if let Some(c) = hex_to_rgba(&bg) {
                            bg_btn.set_rgba(&c);
                        }
                        if let Some(c) = hex_to_rgba(&fg) {
                            fg_btn.set_rgba(&c);
                        }
                        if let Some(c) = hex_to_rgba(&cur) {
                            cur_btn.set_rgba(&c);
                        }
                    }
                    return;
                }
                _ => return, // CUSTOM (0) — no changes
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
    // Decorative for a screen reader: the colour buttons above carry the
    // values, so the preview is kept out of the accessibility tree.
    let theme_preview = DrawingArea::builder()
        .height_request(40)
        .accessible_role(gtk4::AccessibleRole::Presentation)
        .build();

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
    theme_expander.add_row(&preview_row);

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
    theme_expander.add_row(&reset_row);

    theme_group.add(&theme_expander);
    content.append(&theme_group);

    // === Remote Monitoring Section ===
    let monitoring_group = adw::PreferencesGroup::builder()
        .title(i18n("Remote Monitoring"))
        .description(i18n(
            "Override global monitoring settings for this connection",
        ))
        .build();

    // Three-way rather than a switch (issue #352): a switch has no "not set",
    // so every saved connection pinned itself on and stopped following the
    // global switch. "Use global setting" stores no override at all.
    let monitoring_labels: Vec<String> = MONITORING_CHOICES
        .into_iter()
        .map(monitoring_choice_label)
        .collect();
    let monitoring_label_refs: Vec<&str> = monitoring_labels.iter().map(String::as_str).collect();
    let monitoring_items = StringList::new(&monitoring_label_refs);
    let monitoring_combo = adw::ComboRow::builder()
        .title(i18n("Enable Monitoring"))
        .subtitle(i18n("Collect CPU, RAM, disk and network metrics via SSH"))
        .model(&monitoring_items)
        .selected(monitoring_choice_index(MonitoringOverride::Inherit))
        .build();
    monitoring_group.add(&monitoring_combo);

    content.append(&monitoring_group);

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

    // === Activity Monitor Section (collapsible) ===
    let activity_monitor_group = adw::PreferencesGroup::builder().build();
    let activity_expander = adw::ExpanderRow::builder()
        .title(i18n("Activity Monitor"))
        .subtitle(i18n("Detect terminal output activity or silence"))
        .show_enable_switch(false)
        .build();

    let mode_labels = crate::monitor_mode::labels();
    let mode_items = StringList::new(&mode_labels.iter().map(String::as_str).collect::<Vec<_>>());
    let activity_mode_combo = adw::ComboRow::builder()
        .title(i18n("Mode"))
        .subtitle(crate::monitor_mode::mode_row_subtitle())
        .model(&mode_items)
        .selected(0)
        .build();
    activity_expander.add_row(&activity_mode_combo);

    let quiet_period_adj = gtk4::Adjustment::new(10.0, 1.0, 300.0, 1.0, 10.0, 0.0);
    let quiet_period_spin = adw::SpinRow::builder()
        .title(i18n("Quiet Period"))
        .subtitle(i18n("Seconds of silence before activity notification"))
        .adjustment(&quiet_period_adj)
        .visible(false)
        .build();
    activity_expander.add_row(&quiet_period_spin);

    let silence_timeout_adj = gtk4::Adjustment::new(30.0, 1.0, 600.0, 1.0, 10.0, 0.0);
    let silence_timeout_spin = adw::SpinRow::builder()
        .title(i18n("Silence Timeout"))
        .subtitle(i18n("Seconds of no output before silence notification"))
        .adjustment(&silence_timeout_adj)
        .visible(false)
        .build();
    activity_expander.add_row(&silence_timeout_spin);

    // Wire sensitivity: show/hide spin rows based on mode selection
    {
        let quiet_spin = quiet_period_spin.clone();
        let silence_spin = silence_timeout_spin.clone();
        activity_mode_combo.connect_selected_notify(move |combo| {
            // Driven by the mode rather than by a bare index, so a new variant added
            // to `MonitorMode` cannot land in the `_` arm and silently hide both
            // spin rows. `uses_timers()` is what actually decides whether either
            // value is read.
            match crate::monitor_mode::from_index(combo.selected()) {
                MonitorMode::Activity => {
                    quiet_spin.set_visible(true);
                    silence_spin.set_visible(false);
                }
                MonitorMode::Silence => {
                    quiet_spin.set_visible(false);
                    silence_spin.set_visible(true);
                }
                // Off and Command are event-driven or inert: neither reads a period.
                MonitorMode::Off | MonitorMode::Command => {
                    quiet_spin.set_visible(false);
                    silence_spin.set_visible(false);
                }
            }
        });
    }

    activity_monitor_group.add(&activity_expander);
    content.append(&activity_monitor_group);

    // === Automatic Reconnection Section (collapsible) ===
    let retry_group = adw::PreferencesGroup::builder().build();
    let retry_expander = adw::ExpanderRow::builder()
        .title(i18n("Automatic Reconnection"))
        .subtitle(i18n("Retry connection with exponential backoff on failure"))
        .show_enable_switch(false)
        .build();

    let retry_enabled_toggle = adw::SwitchRow::builder()
        .title(i18n("Enable auto-reconnect"))
        .subtitle(i18n("Automatically retry when connection drops"))
        .active(true)
        .build();
    retry_expander.add_row(&retry_enabled_toggle);

    let retry_max_attempts_adj = gtk4::Adjustment::new(3.0, 1.0, 10.0, 1.0, 1.0, 0.0);
    let retry_max_attempts_spin = adw::SpinRow::builder()
        .title(i18n("Maximum attempts"))
        .subtitle(i18n("Number of reconnection attempts before giving up"))
        .adjustment(&retry_max_attempts_adj)
        .build();
    retry_expander.add_row(&retry_max_attempts_spin);

    let retry_initial_delay_adj = gtk4::Adjustment::new(1000.0, 100.0, 30000.0, 100.0, 1000.0, 0.0);
    let retry_initial_delay_spin = adw::SpinRow::builder()
        .title(i18n("Initial delay (ms)"))
        .subtitle(i18n("Delay before first reconnection attempt"))
        .adjustment(&retry_initial_delay_adj)
        .build();
    retry_expander.add_row(&retry_initial_delay_spin);

    let retry_max_delay_adj =
        gtk4::Adjustment::new(30000.0, 1000.0, 120_000.0, 1000.0, 5000.0, 0.0);
    let retry_max_delay_spin = adw::SpinRow::builder()
        .title(i18n("Maximum delay (ms)"))
        .subtitle(i18n("Upper limit for backoff delay between attempts"))
        .adjustment(&retry_max_delay_adj)
        .build();
    retry_expander.add_row(&retry_max_delay_spin);

    // Wire sensitivity: show/hide spin rows based on enabled toggle
    {
        let max_attempts = retry_max_attempts_spin.clone();
        let initial_delay = retry_initial_delay_spin.clone();
        let max_delay = retry_max_delay_spin.clone();
        retry_enabled_toggle.connect_active_notify(move |toggle| {
            let active = toggle.is_active();
            max_attempts.set_sensitive(active);
            initial_delay.set_sensitive(active);
            max_delay.set_sensitive(active);
        });
    }

    // Ensure max_delay >= initial_delay: when initial_delay changes,
    // raise max_delay if it's lower
    {
        let max_delay_clone = retry_max_delay_spin.clone();
        retry_initial_delay_spin.connect_changed(move |spin| {
            let initial = spin.value();
            if max_delay_clone.value() < initial {
                max_delay_clone.set_value(initial);
            }
        });
    }

    retry_group.add(&retry_expander);
    content.append(&retry_group);

    // === Connection Behavior Section ===
    let connection_group = adw::PreferencesGroup::builder()
        .title(i18n("Connection Behavior"))
        .build();

    let skip_port_check_toggle = adw::SwitchRow::builder()
        .title(i18n("Skip port check before connecting"))
        .subtitle(i18n(
            "Bypass TCP probe of the host. Useful for low-bandwidth links or hosts only reachable via a gateway.",
        ))
        .active(false)
        .build();
    connection_group.add(&skip_port_check_toggle);

    // Port knock sequence entry with inline validation
    let knock_sequence_entry = gtk4::Entry::builder()
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .placeholder_text(i18n("e.g., 7000 8000/tcp 9000/udp"))
        .build();
    let knock_row = adw::ActionRow::builder()
        .title(i18n("Port Knock Sequence"))
        .subtitle(i18n(
            "Send TCP/UDP packets to open firewall before connecting",
        ))
        .build();
    knock_row.add_suffix(&knock_sequence_entry);
    connection_group.add(&knock_row);

    // Inline validation: highlight invalid knock sequence
    {
        let entry_clone = knock_sequence_entry.clone();
        knock_sequence_entry.connect_changed(move |_| {
            let text = entry_clone.text();
            let text = text.trim();
            if text.is_empty() {
                entry_clone.remove_css_class("error");
                entry_clone.set_tooltip_text(None);
                return;
            }
            if rustconn_core::connection::knock::KnockSequence::parse(text).is_ok() {
                entry_clone.remove_css_class("error");
                entry_clone.set_tooltip_text(None);
            } else {
                entry_clone.add_css_class("error");
                entry_clone
                    .set_tooltip_text(Some(&i18n("Invalid format. Use: 7000 8000/tcp 9000/udp")));
            }
        });
    }

    content.append(&connection_group);

    // === fwknop Single Packet Authorization (SPA) Section ===
    let spa_group = adw::PreferencesGroup::builder().build();
    let spa_expander = adw::ExpanderRow::builder()
        .title(i18n("Single Packet Authorization (fwknop)"))
        .subtitle(i18n(
            "Send encrypted UDP packet to open firewall before connecting",
        ))
        .show_enable_switch(false)
        .build();

    let spa_enabled_toggle = adw::SwitchRow::builder()
        .title(i18n("Enable SPA"))
        .subtitle(i18n("Send fwknop packet before connecting"))
        .active(false)
        .build();
    spa_expander.add_row(&spa_enabled_toggle);

    let spa_rij_key_entry = adw::PasswordEntryRow::builder()
        .title(i18n("Rijndael Key"))
        .build();
    spa_expander.add_row(&spa_rij_key_entry);

    let spa_hmac_key_entry = adw::PasswordEntryRow::builder()
        .title(i18n("HMAC Key"))
        .build();
    spa_expander.add_row(&spa_hmac_key_entry);

    let spa_access_entry = adw::EntryRow::builder()
        .title(i18n("Access"))
        .text("tcp/22")
        .build();
    spa_expander.add_row(&spa_access_entry);

    let spa_port_adj = gtk4::Adjustment::new(62201.0, 1.0, 65535.0, 1.0, 100.0, 0.0);
    let spa_port_spin = adw::SpinRow::builder()
        .title(i18n("Destination Port"))
        .subtitle(i18n("UDP port for the SPA packet (default: 62201)"))
        .adjustment(&spa_port_adj)
        .build();
    spa_expander.add_row(&spa_port_spin);

    let spa_allow_ip_items = StringList::new(&[
        &i18n("Source IP"),
        &i18n("Resolve Public"),
        &i18n("Explicit"),
    ]);
    let spa_allow_ip_combo = adw::ComboRow::builder()
        .title(i18n("Allow IP"))
        .subtitle(i18n("IP address to authorize in the SPA packet"))
        .model(&spa_allow_ip_items)
        .selected(0)
        .build();
    spa_expander.add_row(&spa_allow_ip_combo);

    // Wire sensitivity: show/hide fields based on enabled toggle
    {
        let rij = spa_rij_key_entry.clone();
        let hmac = spa_hmac_key_entry.clone();
        let access = spa_access_entry.clone();
        let port = spa_port_spin.clone();
        let allow_ip = spa_allow_ip_combo.clone();
        spa_enabled_toggle.connect_active_notify(move |toggle| {
            let active = toggle.is_active();
            rij.set_sensitive(active);
            hmac.set_sensitive(active);
            access.set_sensitive(active);
            port.set_sensitive(active);
            allow_ip.set_sensitive(active);
        });
        // Initial state: disabled
        spa_rij_key_entry.set_sensitive(false);
        spa_hmac_key_entry.set_sensitive(false);
        spa_access_entry.set_sensitive(false);
        spa_port_spin.set_sensitive(false);
        spa_allow_ip_combo.set_sensitive(false);
    }

    spa_group.add(&spa_expander);
    content.append(&spa_group);

    // === Highlight Rules Section (collapsible) ===
    let highlight_group = adw::PreferencesGroup::builder().build();
    let highlight_expander = adw::ExpanderRow::builder()
        .title(i18n("Highlight Rules"))
        .subtitle(i18n("Regex-based text highlighting for this connection"))
        .show_enable_switch(false)
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

    let scrolled_wrapper = adw::PreferencesRow::builder().build();
    scrolled_wrapper.set_child(Some(&highlight_scrolled));
    highlight_expander.add_row(&scrolled_wrapper);

    let hl_button_box = GtkBox::new(Orientation::Horizontal, 8);
    hl_button_box.set_halign(gtk4::Align::End);
    hl_button_box.set_margin_top(12);

    let add_highlight_rule_button = Button::builder()
        .label(i18n("Add Rule"))
        .css_classes(["suggested-action"])
        .build();
    hl_button_box.append(&add_highlight_rule_button);

    let button_wrapper = adw::PreferencesRow::builder().build();
    button_wrapper.set_child(Some(&hl_button_box));
    highlight_expander.add_row(&button_wrapper);

    highlight_group.add(&highlight_expander);
    content.append(&highlight_group);

    // === Wake On LAN Section (collapsible, merged) ===
    let wol_group = adw::PreferencesGroup::builder().build();
    let wol_expander = adw::ExpanderRow::builder()
        .title(i18n("Wake On LAN"))
        .subtitle(i18n("Send magic packet before connecting"))
        .show_enable_switch(false)
        .build();

    let wol_enabled_check = CheckButton::builder().valign(gtk4::Align::Center).build();

    let wol_enable_row = adw::ActionRow::builder()
        .title(i18n("Enable WOL"))
        .subtitle(i18n("Send magic packet before connecting"))
        .activatable_widget(&wol_enabled_check)
        .build();
    wol_enable_row.add_suffix(&wol_enabled_check);
    wol_expander.add_row(&wol_enable_row);

    let mac_entry = Entry::builder()
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .placeholder_text(i18n("AA:BB:CC:DD:EE:FF"))
        .build();

    // "Get MAC" reads the address from the local ARP cache for the connection's
    // host, so the user need not look it up by hand. Wired in build.rs, which
    // has the host entry. Only useful on the same LAN segment; off Linux the
    // core lookup returns nothing and the click reports that.
    let get_mac_button = Button::builder()
        .icon_name("edit-find-symbolic")
        .valign(gtk4::Align::Center)
        .tooltip_text(i18n("Detect the MAC address from this network"))
        .build();
    get_mac_button.add_css_class("flat");
    get_mac_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Detect MAC address",
    ))]);

    let mac_row = adw::ActionRow::builder().title(i18n("MAC Address")).build();
    mac_row.add_suffix(&mac_entry);
    mac_row.add_suffix(&get_mac_button);
    wol_expander.add_row(&mac_row);

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
    wol_expander.add_row(&broadcast_row);

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
    wol_expander.add_row(&port_row);

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
    wol_expander.add_row(&wait_row);

    wol_group.add(&wol_expander);
    content.append(&wol_group);

    clamp.set_child(Some(&content));
    scrolled.set_child(Some(&clamp));

    let vbox = GtkBox::new(Orientation::Vertical, 0);
    vbox.append(&scrolled);

    (
        vbox,
        wol_enabled_check,
        mac_entry,
        get_mac_button,
        broadcast_entry,
        port_spin,
        wait_spin,
        theme_bg_button,
        theme_fg_button,
        theme_cursor_button,
        theme_reset_button,
        theme_preview,
        monitoring_combo,
        recording_toggle,
        highlight_rules_list,
        add_highlight_rule_button,
        theme_preset_dropdown,
        activity_mode_combo,
        quiet_period_spin,
        silence_timeout_spin,
        retry_enabled_toggle,
        retry_max_attempts_spin,
        retry_initial_delay_spin,
        retry_max_delay_spin,
        skip_port_check_toggle,
        knock_sequence_entry,
        spa_enabled_toggle,
        spa_rij_key_entry,
        spa_hmac_key_entry,
        spa_access_entry,
        spa_port_spin,
        spa_allow_ip_combo,
    )
}

/// The rows of the Enable Monitoring picker, top to bottom.
///
/// The labels come from this list and so do the indices the editor writes and
/// reads back, so the three cannot drift apart. `Inherit` is first: a picker
/// built with nothing loaded yet must mean "follow the global switch".
const MONITORING_CHOICES: [MonitoringOverride; 3] = [
    MonitoringOverride::Inherit,
    MonitoringOverride::Enabled,
    MonitoringOverride::Disabled,
];

/// Translated label for one row of the Enable Monitoring picker.
fn monitoring_choice_label(choice: MonitoringOverride) -> String {
    match choice {
        MonitoringOverride::Inherit => i18n("Use global setting"),
        MonitoringOverride::Enabled => i18n("Enabled"),
        MonitoringOverride::Disabled => i18n("Disabled"),
    }
}

/// The Enable Monitoring picker index showing `choice`.
///
/// Falls back to 0 (`Inherit`) for a choice somehow absent from the list —
/// showing "Use global setting" is safer than showing a pinned value.
#[must_use]
pub(super) fn monitoring_choice_index(choice: MonitoringOverride) -> u32 {
    let position = MONITORING_CHOICES.iter().position(|c| *c == choice);
    u32::try_from(position.unwrap_or(0)).unwrap_or(0)
}

/// The choice at Enable Monitoring picker index `index`, or `Inherit` when the
/// index is out of range (including `INVALID_LIST_POSITION`, nothing selected).
#[must_use]
pub(super) fn monitoring_choice_from_index(index: u32) -> MonitoringOverride {
    usize::try_from(index)
        .ok()
        .and_then(|i| MONITORING_CHOICES.get(i).copied())
        .unwrap_or_default()
}

/// Converts a hex color string (`#RRGGBB` or `#RRGGBBAA`) to a GDK RGBA value.
///
/// Returns `None` if the string is not a valid hex color. It reads a stored theme
/// override when the editor opens, which an import or a sync can fill with
/// anything, so it uses the shared parser that refuses a multi-byte character
/// instead of panicking on it (issue #343).
pub(super) fn hex_to_rgba(hex: &str) -> Option<gtk4::gdk::RGBA> {
    let [r, g, b, a] = rustconn_core::terminal_themes::parse_hex_channels(hex.strip_prefix('#')?)?;
    Some(gtk4::gdk::RGBA::new(
        f32::from(r) / 255.0,
        f32::from(g) / 255.0,
        f32::from(b) / 255.0,
        f32::from(a) / 255.0,
    ))
}

/// Converts a GDK RGBA value to a hex color string (`#RRGGBB`).
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
    /// Tooltip the pattern entry shows while its pattern is valid
    pub pattern_tooltip: String,
    /// Entry for the foreground colour, `#RRGGBB`, drawn as an underline
    pub foreground_entry: Entry,
    /// Entry for the background colour, `#RRGGBB`
    pub background_entry: Entry,
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
    hbox.set_margin_start(12);
    hbox.set_margin_end(12);

    let name_entry = Entry::builder()
        .placeholder_text(i18n("Name"))
        .width_chars(12)
        .tooltip_text(i18n("Rule name"))
        .build();
    name_entry.update_property(&[gtk4::accessible::Property::Label(&i18n("Rule name"))]);
    if let Some(r) = rule {
        name_entry.set_text(&r.name);
    }

    let pattern_tooltip = i18n(
        "Regular expression, for example (?i)\\bINFO\\b. Set the colour in the fields to the right.",
    );
    let pattern_entry = Entry::builder()
        .placeholder_text(i18n("Pattern (regex)"))
        .hexpand(true)
        .tooltip_text(pattern_tooltip.as_str())
        .build();
    pattern_entry.update_property(&[gtk4::accessible::Property::Label(&i18n("Pattern (regex)"))]);
    if let Some(r) = rule {
        pattern_entry.set_text(&r.pattern);
    }
    highlight_fields::show_pattern_validity(
        &pattern_entry,
        rule.map_or("", |r| r.pattern.as_str()),
        &pattern_tooltip,
    );

    // The foreground colour is drawn as an underline: the overlay cannot
    // recolour the terminal's text (issue #343). The placeholders are short
    // words so they fit the entry; the format is in the tooltip, and the full
    // label goes to screen readers.
    let foreground_entry = Entry::builder()
        .placeholder_text(i18n("Underline"))
        .width_chars(10)
        .tooltip_text(highlight_fields::underline_colour_tooltip())
        .build();
    foreground_entry.update_property(&[gtk4::accessible::Property::Label(
        &highlight_fields::underline_colour_label(),
    )]);
    let foreground_text = rule
        .and_then(|r| r.foreground_color.as_deref())
        .unwrap_or_default();
    foreground_entry.set_text(foreground_text);
    highlight_fields::show_colour_validity(&foreground_entry, foreground_text);

    let background_entry = Entry::builder()
        .placeholder_text(i18n("Background"))
        .width_chars(10)
        .tooltip_text(highlight_fields::background_colour_tooltip())
        .build();
    background_entry.update_property(&[gtk4::accessible::Property::Label(
        &highlight_fields::background_colour_label(),
    )]);
    let background_text = rule
        .and_then(|r| r.background_color.as_deref())
        .unwrap_or_default();
    background_entry.set_text(background_text);
    highlight_fields::show_colour_validity(&background_entry, background_text);

    let enabled_check = CheckButton::builder()
        .active(rule.is_none_or(|r| r.enabled))
        .tooltip_text(i18n("Enable rule"))
        .valign(gtk4::Align::Center)
        .build();
    enabled_check.update_property(&[gtk4::accessible::Property::Label(&i18n("Enable rule"))]);

    let delete_button = Button::builder()
        .icon_name("user-trash-symbolic")
        .css_classes(["flat"])
        .tooltip_text(i18n("Delete rule"))
        .valign(gtk4::Align::Center)
        .build();
    delete_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Delete highlight rule",
    ))]);

    hbox.append(&name_entry);
    hbox.append(&pattern_entry);
    hbox.append(&foreground_entry);
    hbox.append(&background_entry);
    hbox.append(&enabled_check);
    hbox.append(&delete_button);

    row.set_child(Some(&hbox));

    HighlightRuleRow {
        row,
        id,
        name_entry,
        pattern_entry,
        pattern_tooltip,
        foreground_entry,
        background_entry,
        enabled_check,
        delete_button,
    }
}

#[cfg(test)]
mod tests {
    use rustconn_core::monitoring::MonitoringOverride;

    use super::{MONITORING_CHOICES, monitoring_choice_from_index, monitoring_choice_index};

    #[test]
    fn every_monitoring_choice_round_trips_through_its_index() {
        for choice in MONITORING_CHOICES {
            assert_eq!(
                monitoring_choice_from_index(monitoring_choice_index(choice)),
                choice,
                "{choice:?} does not survive the picker"
            );
        }
    }

    #[test]
    fn the_monitoring_picker_starts_with_use_global_setting() {
        // The row is built with nothing loaded, and a new connection is saved
        // from it as it stands, so index 0 has to be "no override" (issue #352).
        for (index, choice) in [
            (0, MonitoringOverride::Inherit),
            (1, MonitoringOverride::Enabled),
            (2, MonitoringOverride::Disabled),
        ] {
            assert_eq!(monitoring_choice_index(choice), index);
            assert_eq!(monitoring_choice_from_index(index), choice);
        }
    }

    #[test]
    fn an_out_of_range_monitoring_index_falls_back_to_use_global_setting() {
        // `u32::MAX` is GTK's `INVALID_LIST_POSITION`: nothing selected.
        for index in [3, u32::MAX] {
            assert_eq!(
                monitoring_choice_from_index(index),
                MonitoringOverride::Inherit,
                "index {index} must fall back to the global setting"
            );
        }
    }
}
