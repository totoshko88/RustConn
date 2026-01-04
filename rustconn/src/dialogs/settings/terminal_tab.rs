//! Terminal settings tab

use gtk4::prelude::*;
use gtk4::{
    CheckButton, DropDown, Entry, Frame, Grid, Label, ScrolledWindow, SpinButton, StringList,
};
use rustconn_core::config::TerminalSettings;
use rustconn_core::terminal_themes::TerminalTheme;

/// Creates the terminal settings tab
#[allow(clippy::type_complexity)]
pub fn create_terminal_tab() -> (
    Frame,
    Entry,
    SpinButton,
    SpinButton,
    DropDown,
    DropDown,
    DropDown,
    CheckButton,
    CheckButton,
    CheckButton,
    CheckButton,
    CheckButton,
) {
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .vexpand(true)
        .build();

    let grid = Grid::builder()
        .row_spacing(8)
        .column_spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    let mut row = 0;

    // === Font Settings ===
    let font_header = Label::builder()
        .label("Font")
        .halign(gtk4::Align::Start)
        .css_classes(["heading"])
        .margin_top(6)
        .build();
    grid.attach(&font_header, 0, row, 2, 1);
    row += 1;

    // Font family
    let font_label = Label::builder()
        .label("Font Family:")
        .halign(gtk4::Align::End)
        .build();
    let font_family_entry = Entry::builder().hexpand(true).text("Monospace").build();
    grid.attach(&font_label, 0, row, 1, 1);
    grid.attach(&font_family_entry, 1, row, 1, 1);
    row += 1;

    // Font size
    let size_label = Label::builder()
        .label("Font Size:")
        .halign(gtk4::Align::End)
        .build();
    let size_adj = gtk4::Adjustment::new(12.0, 6.0, 72.0, 1.0, 2.0, 0.0);
    let font_size_spin = SpinButton::builder()
        .adjustment(&size_adj)
        .climb_rate(1.0)
        .digits(0)
        .build();
    grid.attach(&size_label, 0, row, 1, 1);
    grid.attach(&font_size_spin, 1, row, 1, 1);
    row += 1;

    // === Color Theme ===
    let color_header = Label::builder()
        .label("Colors")
        .halign(gtk4::Align::Start)
        .css_classes(["heading"])
        .margin_top(12)
        .build();
    grid.attach(&color_header, 0, row, 2, 1);
    row += 1;

    // Color theme dropdown
    let theme_label = Label::builder()
        .label("Color Theme:")
        .halign(gtk4::Align::End)
        .build();
    let theme_names = TerminalTheme::theme_names();
    let theme_list = StringList::new(&theme_names.iter().map(String::as_str).collect::<Vec<_>>());
    let color_theme_dropdown = DropDown::builder()
        .model(&theme_list)
        .selected(0) // Default to first theme (Dark)
        .build();
    grid.attach(&theme_label, 0, row, 1, 1);
    grid.attach(&color_theme_dropdown, 1, row, 1, 1);
    row += 1;

    // === Cursor Settings ===
    let cursor_header = Label::builder()
        .label("Cursor")
        .halign(gtk4::Align::Start)
        .css_classes(["heading"])
        .margin_top(12)
        .build();
    grid.attach(&cursor_header, 0, row, 2, 1);
    row += 1;

    // Cursor shape
    let cursor_shape_label = Label::builder()
        .label("Cursor Shape:")
        .halign(gtk4::Align::End)
        .build();
    let cursor_shapes = ["Block", "IBeam", "Underline"];
    let cursor_shape_list = StringList::new(&cursor_shapes);
    let cursor_shape_dropdown = DropDown::builder()
        .model(&cursor_shape_list)
        .selected(0) // Default to Block
        .build();
    grid.attach(&cursor_shape_label, 0, row, 1, 1);
    grid.attach(&cursor_shape_dropdown, 1, row, 1, 1);
    row += 1;

    // Cursor blink
    let cursor_blink_label = Label::builder()
        .label("Cursor Blink:")
        .halign(gtk4::Align::End)
        .build();
    let cursor_blink_modes = ["On", "Off", "System"];
    let cursor_blink_list = StringList::new(&cursor_blink_modes);
    let cursor_blink_dropdown = DropDown::builder()
        .model(&cursor_blink_list)
        .selected(0) // Default to On
        .build();
    grid.attach(&cursor_blink_label, 0, row, 1, 1);
    grid.attach(&cursor_blink_dropdown, 1, row, 1, 1);
    row += 1;

    // === Scrolling Settings ===
    let scroll_header = Label::builder()
        .label("Scrolling")
        .halign(gtk4::Align::Start)
        .css_classes(["heading"])
        .margin_top(12)
        .build();
    grid.attach(&scroll_header, 0, row, 2, 1);
    row += 1;

    // Scrollback lines
    let scrollback_label = Label::builder()
        .label("Scrollback Lines:")
        .halign(gtk4::Align::End)
        .build();
    let scrollback_adj = gtk4::Adjustment::new(10000.0, 100.0, 1_000_000.0, 100.0, 1000.0, 0.0);
    let scrollback_spin = SpinButton::builder()
        .adjustment(&scrollback_adj)
        .climb_rate(100.0)
        .digits(0)
        .build();
    grid.attach(&scrollback_label, 0, row, 1, 1);
    grid.attach(&scrollback_spin, 1, row, 1, 1);
    row += 1;

    // Scroll on output
    let scroll_on_output_check = CheckButton::builder()
        .label("Scroll on output")
        .active(false)
        .build();
    grid.attach(&scroll_on_output_check, 0, row, 2, 1);
    row += 1;

    // Scroll on keystroke
    let scroll_on_keystroke_check = CheckButton::builder()
        .label("Scroll on keystroke")
        .active(true)
        .build();
    grid.attach(&scroll_on_keystroke_check, 0, row, 2, 1);
    row += 1;

    // === Behavior Settings ===
    let behavior_header = Label::builder()
        .label("Behavior")
        .halign(gtk4::Align::Start)
        .css_classes(["heading"])
        .margin_top(12)
        .build();
    grid.attach(&behavior_header, 0, row, 2, 1);
    row += 1;

    // Allow hyperlinks
    let allow_hyperlinks_check = CheckButton::builder()
        .label("Allow hyperlinks")
        .active(true)
        .build();
    grid.attach(&allow_hyperlinks_check, 0, row, 2, 1);
    row += 1;

    // Mouse autohide
    let mouse_autohide_check = CheckButton::builder()
        .label("Hide mouse when typing")
        .active(true)
        .build();
    grid.attach(&mouse_autohide_check, 0, row, 2, 1);
    row += 1;

    // Audible bell
    let audible_bell_check = CheckButton::builder()
        .label("Audible bell")
        .active(false)
        .build();
    grid.attach(&audible_bell_check, 0, row, 2, 1);

    scrolled.set_child(Some(&grid));

    let frame = Frame::builder()
        .label("Terminal Settings")
        .child(&scrolled)
        .margin_top(12)
        .valign(gtk4::Align::Fill)
        .vexpand(true)
        .build();

    (
        frame,
        font_family_entry,
        font_size_spin,
        scrollback_spin,
        color_theme_dropdown,
        cursor_shape_dropdown,
        cursor_blink_dropdown,
        scroll_on_output_check,
        scroll_on_keystroke_check,
        allow_hyperlinks_check,
        mouse_autohide_check,
        audible_bell_check,
    )
}

/// Loads terminal settings into UI controls
#[allow(clippy::too_many_arguments)]
pub fn load_terminal_settings(
    font_family_entry: &Entry,
    font_size_spin: &SpinButton,
    scrollback_spin: &SpinButton,
    color_theme_dropdown: &DropDown,
    cursor_shape_dropdown: &DropDown,
    cursor_blink_dropdown: &DropDown,
    scroll_on_output_check: &CheckButton,
    scroll_on_keystroke_check: &CheckButton,
    allow_hyperlinks_check: &CheckButton,
    mouse_autohide_check: &CheckButton,
    audible_bell_check: &CheckButton,
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

    // Set cursor shape
    let cursor_shape_index = match settings.cursor_shape.as_str() {
        "Block" => 0,
        "IBeam" => 1,
        "Underline" => 2,
        _ => 0,
    };
    cursor_shape_dropdown.set_selected(cursor_shape_index);

    // Set cursor blink
    let cursor_blink_index = match settings.cursor_blink.as_str() {
        "On" => 0,
        "Off" => 1,
        "System" => 2,
        _ => 0,
    };
    cursor_blink_dropdown.set_selected(cursor_blink_index);

    scroll_on_output_check.set_active(settings.scroll_on_output);
    scroll_on_keystroke_check.set_active(settings.scroll_on_keystroke);
    allow_hyperlinks_check.set_active(settings.allow_hyperlinks);
    mouse_autohide_check.set_active(settings.mouse_autohide);
    audible_bell_check.set_active(settings.audible_bell);
}

/// Collects terminal settings from UI controls
#[allow(clippy::too_many_arguments)]
pub fn collect_terminal_settings(
    font_family_entry: &Entry,
    font_size_spin: &SpinButton,
    scrollback_spin: &SpinButton,
    color_theme_dropdown: &DropDown,
    cursor_shape_dropdown: &DropDown,
    cursor_blink_dropdown: &DropDown,
    scroll_on_output_check: &CheckButton,
    scroll_on_keystroke_check: &CheckButton,
    allow_hyperlinks_check: &CheckButton,
    mouse_autohide_check: &CheckButton,
    audible_bell_check: &CheckButton,
) -> TerminalSettings {
    let theme_names = TerminalTheme::theme_names();
    let color_theme = theme_names
        .get(color_theme_dropdown.selected() as usize)
        .cloned()
        .unwrap_or_else(|| "Dark".to_string());

    let cursor_shapes = ["Block", "IBeam", "Underline"];
    let cursor_shape = cursor_shapes
        .get(cursor_shape_dropdown.selected() as usize)
        .unwrap_or(&"Block")
        .to_string();

    let cursor_blink_modes = ["On", "Off", "System"];
    let cursor_blink_mode = cursor_blink_modes
        .get(cursor_blink_dropdown.selected() as usize)
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
    }
}
