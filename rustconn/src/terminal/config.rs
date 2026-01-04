//! Terminal configuration
//!
//! This module handles VTE terminal appearance and behavior configuration.

use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use rustconn_core::config::TerminalSettings;
use rustconn_core::terminal_themes::{Color, TerminalTheme};
use vte4::prelude::*;
use vte4::{CursorBlinkMode, CursorShape, Terminal};

/// Configures terminal appearance and behavior
pub fn configure_terminal(terminal: &Terminal) {
    configure_terminal_with_settings(terminal, &TerminalSettings::default());
}

/// Configures terminal with specific settings
pub fn configure_terminal_with_settings(terminal: &Terminal, settings: &TerminalSettings) {
    // Cursor settings
    let cursor_blink = match settings.cursor_blink.as_str() {
        "On" => CursorBlinkMode::On,
        "Off" => CursorBlinkMode::Off,
        "System" => CursorBlinkMode::System,
        _ => CursorBlinkMode::On,
    };
    terminal.set_cursor_blink_mode(cursor_blink);

    let cursor_shape = match settings.cursor_shape.as_str() {
        "Block" => CursorShape::Block,
        "IBeam" => CursorShape::Ibeam,
        "Underline" => CursorShape::Underline,
        _ => CursorShape::Block,
    };
    terminal.set_cursor_shape(cursor_shape);

    // Scrolling behavior
    terminal.set_scroll_on_output(settings.scroll_on_output);
    terminal.set_scroll_on_keystroke(settings.scroll_on_keystroke);
    terminal.set_scrollback_lines(i64::from(settings.scrollback_lines));

    // Input handling
    terminal.set_input_enabled(true);
    terminal.set_allow_hyperlink(settings.allow_hyperlinks);
    terminal.set_mouse_autohide(settings.mouse_autohide);

    // Bold text - VTE4 doesn't have set_allow_bold, remove this setting
    // terminal.set_allow_bold(settings.allow_bold);

    // Bell
    terminal.set_audible_bell(settings.audible_bell);

    // Keyboard shortcuts (Copy/Paste)
    setup_keyboard_shortcuts(terminal);

    // Context menu (Right click)
    setup_context_menu(terminal);

    // Colors and font
    setup_colors_with_theme(terminal, &settings.color_theme);
    setup_font_with_settings(terminal, settings);
}

/// Sets up keyboard shortcuts for copy/paste
fn setup_keyboard_shortcuts(terminal: &Terminal) {
    let controller = gtk4::EventControllerKey::new();
    let term = terminal.clone();
    controller.connect_key_pressed(move |_, key, _, state| {
        let mask = gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK;
        if state.contains(mask) {
            match key.name().as_deref() {
                Some("C" | "c") => {
                    term.copy_clipboard_format(vte4::Format::Text);
                    return glib::Propagation::Stop;
                }
                Some("V" | "v") => {
                    term.paste_clipboard();
                    return glib::Propagation::Stop;
                }
                _ => (),
            }
        }
        glib::Propagation::Proceed
    });
    terminal.add_controller(controller);
}

/// Sets up context menu for right-click
fn setup_context_menu(terminal: &Terminal) {
    use gtk4::gio;
    use gtk4::PopoverMenu;

    let click_controller = gtk4::GestureClick::new();
    click_controller.set_button(3); // Right click
    let term_menu = terminal.clone();
    click_controller.connect_pressed(move |_gesture, _, x, y| {
        let menu = gio::Menu::new();
        menu.append(Some("Copy"), Some("terminal.copy"));
        menu.append(Some("Paste"), Some("terminal.paste"));
        menu.append(Some("Select All"), Some("terminal.select-all"));

        let popover = PopoverMenu::from_model(Some(&menu));
        popover.set_parent(&term_menu);
        popover.set_has_arrow(false);

        // Create action group for the menu
        let action_group = gio::SimpleActionGroup::new();

        let term_copy = term_menu.clone();
        let action_copy = gio::SimpleAction::new("copy", None);
        action_copy.connect_activate(move |_, _| {
            term_copy.copy_clipboard_format(vte4::Format::Text);
        });
        action_group.add_action(&action_copy);

        let term_paste = term_menu.clone();
        let action_paste = gio::SimpleAction::new("paste", None);
        action_paste.connect_activate(move |_, _| {
            term_paste.paste_clipboard();
        });
        action_group.add_action(&action_paste);

        let term_select = term_menu.clone();
        let action_select = gio::SimpleAction::new("select-all", None);
        action_select.connect_activate(move |_, _| {
            term_select.select_all();
        });
        action_group.add_action(&action_select);

        term_menu.insert_action_group("terminal", Some(&action_group));

        let rect = gdk::Rectangle::new(x as i32, y as i32, 1, 1);
        popover.set_pointing_to(Some(&rect));
        popover.popup();
    });
    terminal.add_controller(click_controller);
}

/// Converts Color to gdk::RGBA
fn color_to_rgba(color: &Color) -> gdk::RGBA {
    gdk::RGBA::new(color.r, color.g, color.b, 1.0)
}

/// Sets up terminal colors with theme
fn setup_colors_with_theme(terminal: &Terminal, theme_name: &str) {
    let theme = TerminalTheme::by_name(theme_name).unwrap_or_else(TerminalTheme::dark_theme);

    let bg_color = color_to_rgba(&theme.background);
    let fg_color = color_to_rgba(&theme.foreground);
    let cursor_color = color_to_rgba(&theme.cursor);

    terminal.set_color_background(&bg_color);
    terminal.set_color_foreground(&fg_color);
    terminal.set_color_cursor(Some(&cursor_color));

    // Set up palette colors
    let palette_rgba: Vec<gdk::RGBA> = theme.palette.iter().map(color_to_rgba).collect();
    let palette_refs: Vec<&gdk::RGBA> = palette_rgba.iter().collect();
    terminal.set_colors(Some(&fg_color), Some(&bg_color), &palette_refs);
}

/// Sets up terminal font with settings
fn setup_font_with_settings(terminal: &Terminal, settings: &TerminalSettings) {
    let font_desc = gtk4::pango::FontDescription::from_string(&format!(
        "{} {}",
        settings.font_family, settings.font_size
    ));
    terminal.set_font(Some(&font_desc));
}
