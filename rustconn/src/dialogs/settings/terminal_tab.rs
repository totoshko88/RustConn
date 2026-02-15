//! Terminal settings tab using libadwaita components

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, CheckButton, DropDown, Entry, Orientation, SpinButton, StringList, ToggleButton,
};
use libadwaita as adw;
use rustconn_core::config::TerminalSettings;
use rustconn_core::terminal_themes::TerminalTheme;

/// Creates the terminal settings page using AdwPreferencesPage
#[allow(clippy::type_complexity)]
pub fn create_terminal_page() -> (
    adw::PreferencesPage,
    Entry,
    SpinButton,
    SpinButton,
    DropDown,
    GtkBox, // cursor shape buttons container
    GtkBox, // cursor blink buttons container
    CheckButton,
    CheckButton,
    CheckButton,
    CheckButton,
    CheckButton,
    CheckButton, // sftp_use_mc
) {
    let page = adw::PreferencesPage::builder()
        .title("Terminal")
        .icon_name("utilities-terminal-symbolic")
        .build();

    // === Font Group ===
    let font_group = adw::PreferencesGroup::builder().title("Font").build();

    // Font family row - simplified title
    let font_family_entry = Entry::builder()
        .text("Monospace")
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .build();
    let font_family_row = adw::ActionRow::builder().title("Family").build();
    font_family_row.add_suffix(&font_family_entry);
    font_family_row.set_activatable_widget(Some(&font_family_entry));
    font_group.add(&font_family_row);

    // Font size row - simplified title
    let size_adj = gtk4::Adjustment::new(12.0, 6.0, 72.0, 1.0, 2.0, 0.0);
    let font_size_spin = SpinButton::builder()
        .adjustment(&size_adj)
        .climb_rate(1.0)
        .digits(0)
        .valign(gtk4::Align::Center)
        .build();
    let font_size_row = adw::ActionRow::builder().title("Size").build();
    font_size_row.add_suffix(&font_size_spin);
    font_size_row.set_activatable_widget(Some(&font_size_spin));
    font_group.add(&font_size_row);

    page.add(&font_group);

    // === Colors Group ===
    let colors_group = adw::PreferencesGroup::builder().title("Colors").build();

    let theme_names = TerminalTheme::theme_names();
    let theme_list = StringList::new(&theme_names.iter().map(String::as_str).collect::<Vec<_>>());
    let color_theme_dropdown = DropDown::builder()
        .model(&theme_list)
        .selected(0)
        .valign(gtk4::Align::Center)
        .build();
    let color_theme_row = adw::ActionRow::builder().title("Theme").build();
    color_theme_row.add_suffix(&color_theme_dropdown);
    color_theme_row.set_activatable_widget(Some(&color_theme_dropdown));
    colors_group.add(&color_theme_row);

    page.add(&colors_group);

    // === Cursor Group ===
    let cursor_group = adw::PreferencesGroup::builder().title("Cursor").build();

    // Cursor shape - toggle buttons
    let shape_buttons_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(0)
        .valign(gtk4::Align::Center)
        .css_classes(["linked"])
        .width_request(240)
        .build();

    let shape_block_btn = ToggleButton::builder()
        .label("Block")
        .active(true)
        .hexpand(true)
        .build();
    let shape_ibeam_btn = ToggleButton::builder()
        .label("IBeam")
        .group(&shape_block_btn)
        .hexpand(true)
        .build();
    let shape_underline_btn = ToggleButton::builder()
        .label("Underline")
        .group(&shape_block_btn)
        .hexpand(true)
        .build();

    shape_buttons_box.append(&shape_block_btn);
    shape_buttons_box.append(&shape_ibeam_btn);
    shape_buttons_box.append(&shape_underline_btn);

    let cursor_shape_row = adw::ActionRow::builder().title("Shape").build();
    cursor_shape_row.add_suffix(&shape_buttons_box);
    cursor_group.add(&cursor_shape_row);

    // Cursor blink - toggle buttons
    let blink_buttons_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(0)
        .valign(gtk4::Align::Center)
        .css_classes(["linked"])
        .width_request(240)
        .build();

    let blink_on_btn = ToggleButton::builder()
        .label("On")
        .active(true)
        .hexpand(true)
        .build();
    let blink_off_btn = ToggleButton::builder()
        .label("Off")
        .group(&blink_on_btn)
        .hexpand(true)
        .build();
    let blink_system_btn = ToggleButton::builder()
        .label("System")
        .group(&blink_on_btn)
        .hexpand(true)
        .build();

    blink_buttons_box.append(&blink_on_btn);
    blink_buttons_box.append(&blink_off_btn);
    blink_buttons_box.append(&blink_system_btn);

    let cursor_blink_row = adw::ActionRow::builder().title("Blink").build();
    cursor_blink_row.add_suffix(&blink_buttons_box);
    cursor_group.add(&cursor_blink_row);

    page.add(&cursor_group);

    // === Scrolling Group ===
    let scrolling_group = adw::PreferencesGroup::builder().title("Scrolling").build();

    // Scrollback lines - simplified title
    let scrollback_adj = gtk4::Adjustment::new(10000.0, 100.0, 1_000_000.0, 100.0, 1000.0, 0.0);
    let scrollback_spin = SpinButton::builder()
        .adjustment(&scrollback_adj)
        .climb_rate(100.0)
        .digits(0)
        .valign(gtk4::Align::Center)
        .build();
    let scrollback_row = adw::ActionRow::builder()
        .title("History")
        .subtitle("Number of lines to keep in scrollback")
        .build();
    scrollback_row.add_suffix(&scrollback_spin);
    scrollback_row.set_activatable_widget(Some(&scrollback_spin));
    scrolling_group.add(&scrollback_row);

    // Scroll on output
    let scroll_on_output_check = CheckButton::builder().valign(gtk4::Align::Center).build();
    let scroll_on_output_row = adw::ActionRow::builder()
        .title("On output")
        .subtitle("Scroll to bottom when new output appears")
        .activatable_widget(&scroll_on_output_check)
        .build();
    scroll_on_output_row.add_prefix(&scroll_on_output_check);
    scrolling_group.add(&scroll_on_output_row);

    // Scroll on keystroke
    let scroll_on_keystroke_check = CheckButton::builder()
        .active(true)
        .valign(gtk4::Align::Center)
        .build();
    let scroll_on_keystroke_row = adw::ActionRow::builder()
        .title("On keystroke")
        .subtitle("Scroll to bottom when typing")
        .activatable_widget(&scroll_on_keystroke_check)
        .build();
    scroll_on_keystroke_row.add_prefix(&scroll_on_keystroke_check);
    scrolling_group.add(&scroll_on_keystroke_row);

    page.add(&scrolling_group);

    // === Behavior Group ===
    let behavior_group = adw::PreferencesGroup::builder().title("Behavior").build();

    // Allow hyperlinks
    let allow_hyperlinks_check = CheckButton::builder()
        .active(true)
        .valign(gtk4::Align::Center)
        .build();
    let allow_hyperlinks_row = adw::ActionRow::builder()
        .title("Hyperlinks")
        .subtitle("Allow clickable URLs in terminal")
        .activatable_widget(&allow_hyperlinks_check)
        .build();
    allow_hyperlinks_row.add_prefix(&allow_hyperlinks_check);
    behavior_group.add(&allow_hyperlinks_row);

    // Hide mouse when typing
    let mouse_autohide_check = CheckButton::builder()
        .active(true)
        .valign(gtk4::Align::Center)
        .build();
    let mouse_autohide_row = adw::ActionRow::builder()
        .title("Hide pointer")
        .subtitle("Hide mouse cursor when typing")
        .activatable_widget(&mouse_autohide_check)
        .build();
    mouse_autohide_row.add_prefix(&mouse_autohide_check);
    behavior_group.add(&mouse_autohide_row);

    // Audible bell
    let audible_bell_check = CheckButton::builder().valign(gtk4::Align::Center).build();
    let audible_bell_row = adw::ActionRow::builder()
        .title("Bell")
        .subtitle("Play sound on terminal bell")
        .activatable_widget(&audible_bell_check)
        .build();
    audible_bell_row.add_prefix(&audible_bell_check);
    behavior_group.add(&audible_bell_row);

    // SFTP via Midnight Commander
    let sftp_use_mc_check = CheckButton::builder().valign(gtk4::Align::Center).build();
    let sftp_use_mc_row = adw::ActionRow::builder()
        .title("SFTP via mc")
        .subtitle("Open SFTP in Midnight Commander (local shell tab)")
        .activatable_widget(&sftp_use_mc_check)
        .build();
    sftp_use_mc_row.add_prefix(&sftp_use_mc_check);
    behavior_group.add(&sftp_use_mc_row);

    page.add(&behavior_group);

    (
        page,
        font_family_entry,
        font_size_spin,
        scrollback_spin,
        color_theme_dropdown,
        shape_buttons_box,
        blink_buttons_box,
        scroll_on_output_check,
        scroll_on_keystroke_check,
        allow_hyperlinks_check,
        mouse_autohide_check,
        audible_bell_check,
        sftp_use_mc_check,
    )
}

/// Loads terminal settings into UI controls
#[allow(clippy::too_many_arguments)]
pub fn load_terminal_settings(
    font_family_entry: &Entry,
    font_size_spin: &SpinButton,
    scrollback_spin: &SpinButton,
    color_theme_dropdown: &DropDown,
    cursor_shape_buttons: &GtkBox,
    cursor_blink_buttons: &GtkBox,
    scroll_on_output_check: &CheckButton,
    scroll_on_keystroke_check: &CheckButton,
    allow_hyperlinks_check: &CheckButton,
    mouse_autohide_check: &CheckButton,
    audible_bell_check: &CheckButton,
    sftp_use_mc_check: &CheckButton,
    settings: &TerminalSettings,
) {
    font_family_entry.set_text(&settings.font_family);
    font_size_spin.set_value(f64::from(settings.font_size));
    scrollback_spin.set_value(f64::from(settings.scrollback_lines));

    // Set color theme
    let theme_names = TerminalTheme::theme_names();
    if let Some(index) = theme_names
        .iter()
        .position(|name| name == &settings.color_theme)
    {
        color_theme_dropdown.set_selected(index as u32);
    }

    // Set cursor shape via toggle buttons
    let cursor_shape_index = match settings.cursor_shape.as_str() {
        "Block" => 0,
        "IBeam" => 1,
        "Underline" => 2,
        _ => 0,
    };
    if let Some(btn) = get_toggle_button_at_index(cursor_shape_buttons, cursor_shape_index) {
        btn.set_active(true);
    }

    // Set cursor blink via toggle buttons
    let cursor_blink_index = match settings.cursor_blink.as_str() {
        "On" => 0,
        "Off" => 1,
        "System" => 2,
        _ => 0,
    };
    if let Some(btn) = get_toggle_button_at_index(cursor_blink_buttons, cursor_blink_index) {
        btn.set_active(true);
    }

    scroll_on_output_check.set_active(settings.scroll_on_output);
    scroll_on_keystroke_check.set_active(settings.scroll_on_keystroke);
    allow_hyperlinks_check.set_active(settings.allow_hyperlinks);
    mouse_autohide_check.set_active(settings.mouse_autohide);
    audible_bell_check.set_active(settings.audible_bell);
    sftp_use_mc_check.set_active(settings.sftp_use_mc);
}

/// Gets the toggle button at a specific index in a button box
fn get_toggle_button_at_index(button_box: &GtkBox, index: usize) -> Option<ToggleButton> {
    let mut child = button_box.first_child();
    let mut i = 0;
    while let Some(widget) = child {
        if i == index {
            return widget.downcast::<ToggleButton>().ok();
        }
        child = widget.next_sibling();
        i += 1;
    }
    None
}

/// Gets the index of the active toggle button in a button box
fn get_active_toggle_index(button_box: &GtkBox) -> usize {
    let mut child = button_box.first_child();
    let mut i = 0;
    while let Some(widget) = child {
        if let Ok(btn) = widget.clone().downcast::<ToggleButton>() {
            if btn.is_active() {
                return i;
            }
        }
        child = widget.next_sibling();
        i += 1;
    }
    0
}

/// Collects terminal settings from UI controls
#[allow(clippy::too_many_arguments)]
pub fn collect_terminal_settings(
    font_family_entry: &Entry,
    font_size_spin: &SpinButton,
    scrollback_spin: &SpinButton,
    color_theme_dropdown: &DropDown,
    cursor_shape_buttons: &GtkBox,
    cursor_blink_buttons: &GtkBox,
    scroll_on_output_check: &CheckButton,
    scroll_on_keystroke_check: &CheckButton,
    allow_hyperlinks_check: &CheckButton,
    mouse_autohide_check: &CheckButton,
    audible_bell_check: &CheckButton,
    sftp_use_mc_check: &CheckButton,
    log_timestamps: bool,
) -> TerminalSettings {
    let theme_names = TerminalTheme::theme_names();
    let color_theme = theme_names
        .get(color_theme_dropdown.selected() as usize)
        .cloned()
        .unwrap_or_else(|| "Dark".to_string());

    let cursor_shapes = ["Block", "IBeam", "Underline"];
    let cursor_shape = cursor_shapes
        .get(get_active_toggle_index(cursor_shape_buttons))
        .unwrap_or(&"Block")
        .to_string();

    let cursor_blink_modes = ["On", "Off", "System"];
    let cursor_blink_mode = cursor_blink_modes
        .get(get_active_toggle_index(cursor_blink_buttons))
        .unwrap_or(&"On")
        .to_string();

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    TerminalSettings {
        font_family: font_family_entry.text().to_string(),
        font_size: font_size_spin.value() as u32,
        scrollback_lines: scrollback_spin.value() as u32,
        color_theme,
        cursor_shape,
        cursor_blink: cursor_blink_mode,
        scroll_on_output: scroll_on_output_check.is_active(),
        scroll_on_keystroke: scroll_on_keystroke_check.is_active(),
        allow_hyperlinks: allow_hyperlinks_check.is_active(),
        mouse_autohide: mouse_autohide_check.is_active(),
        audible_bell: audible_bell_check.is_active(),
        log_timestamps,
        sftp_use_mc: sftp_use_mc_check.is_active(),
    }
}
