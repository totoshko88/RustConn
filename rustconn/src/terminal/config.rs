//! Terminal configuration
//!
//! This module handles VTE terminal appearance and behavior configuration.

use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{gdk, glib};
use rustconn_core::config::TerminalSettings;
use rustconn_core::models::ConnectionThemeOverride;
use rustconn_core::terminal_themes::{Color, TerminalTheme};
use vte4::prelude::*;
use vte4::{CursorBlinkMode, CursorShape, Terminal};

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

    // Fallback scrolling lets VTE scroll the scrollback buffer with the
    // mouse wheel when the running program has NOT requested mouse tracking.
    // Programs that *do* request mouse tracking (mc, htop, vim) receive
    // scroll events directly from VTE regardless of this setting.
    // Always enable so that normal shell sessions can scroll (#121).
    terminal.set_enable_fallback_scrolling(true);

    // Bell
    terminal.set_audible_bell(settings.audible_bell);

    // Copy on select (X11-style auto-copy)
    if settings.copy_on_select {
        setup_copy_on_select(terminal);
    }

    // Keyboard shortcuts (Copy/Paste + font zoom)
    setup_keyboard_shortcuts(terminal);
    setup_font_zoom(terminal);

    // macOS: Option key composed text handler (when option_is_meta is false)
    #[cfg(target_os = "macos")]
    if !settings.option_is_meta {
        setup_macos_option_key_handler(terminal);
    }

    // Context menu (Right click) — attached to the container, NOT the
    // terminal, to avoid interfering with VTE's internal mouse handling.
    // The context menu is set up separately after the terminal is created
    // (see `setup_context_menu`).

    // Colors and font
    setup_colors_with_theme(terminal, &settings.color_theme);
    setup_font_with_settings(terminal, settings);
}

/// Automatically copies selected text to the clipboard when the user
/// finishes a selection (X11-style "copy on select").
fn setup_copy_on_select(terminal: &Terminal) {
    let term = terminal.clone();
    terminal.connect_selection_changed(move |_| {
        if term.has_selection() {
            // Use text_selected + clipboard().set_text instead of
            // copy_clipboard_format to avoid the race where VTE clears
            // the selection during reparenting, triggering
            // `gdk_clipboard_write_async: mime_type != NULL`.
            if let Some(text) = term.text_selected(vte4::Format::Text) {
                term.display().clipboard().set_text(&text);
            }
        }
    });
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
                    if let Some(text) = term.text_selected(vte4::Format::Text) {
                        term.display().clipboard().set_text(&text);
                    }
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

/// Sets up context menu using VTE's native `set_context_menu_model` API.
///
/// VTE 0.76+ handles right-click internally: it preserves the text
/// selection, positions the popover correctly, and routes actions through
/// the widget hierarchy. The previous approach (a `GestureClick` on the
/// container) broke Copy/Paste because the popover stole focus from VTE
/// before the action callbacks could run (#84).
///
/// Copy uses `text_selected()` to snapshot the selection text before VTE
/// clears it, avoiding the `gdk_clipboard_write_async: mime_type != NULL`
/// assertion that `copy_clipboard_format` triggers on an empty selection.
///
/// The `snippet_section` is a shared live `gio::Menu` model — all terminals
/// reference the same instance so snippet changes propagate automatically.
pub fn setup_context_menu(terminal: &Terminal, snippet_section: &Rc<gtk4::gio::Menu>) {
    use std::cell::RefCell;

    use gtk4::gio;

    // Cache the last selection so Copy still works after VTE clears it
    // on right-click.
    let last_selection: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

    // Snapshot selection whenever it changes (including when it appears
    // and when VTE clears it on right-click).
    let sel = last_selection.clone();
    let term_sel = terminal.clone();
    terminal.connect_selection_changed(move |_| {
        if term_sel.has_selection() {
            *sel.borrow_mut() = term_sel
                .text_selected(vte4::Format::Text)
                .map(|s| s.to_string());
        }
        // When selection is cleared we intentionally keep the cached
        // value so the Copy action can still use it.
    });

    // Build the menu model (shared across all right-click invocations)
    let menu = gio::Menu::new();
    let clipboard_section = gio::Menu::new();
    clipboard_section.append(Some(&crate::i18n::i18n("Copy")), Some("terminal.copy"));
    clipboard_section.append(Some(&crate::i18n::i18n("Paste")), Some("terminal.paste"));
    clipboard_section.append(
        Some(&crate::i18n::i18n("Select All")),
        Some("terminal.select-all"),
    );
    menu.append_section(None, &clipboard_section);

    // Snippet section — shared live model, updated externally
    menu.append_section(None, snippet_section.as_ref());

    // Register the action group on the container so VTE's popover can
    // resolve "terminal.*" actions by walking up the widget tree.
    let action_group = gio::SimpleActionGroup::new();

    let term_copy = terminal.clone();
    let sel_copy = last_selection;
    let action_copy = gio::SimpleAction::new("copy", None);
    action_copy.connect_activate(move |_, _| {
        // Always use cached selection to avoid the race where VTE clears
        // the selection between has_selection() and copy_clipboard_format(),
        // triggering `gdk_clipboard_write_async: mime_type != NULL`.
        // Prefer a fresh snapshot if selection is still live.
        let text = if term_copy.has_selection() {
            term_copy
                .text_selected(vte4::Format::Text)
                .map(|s| s.to_string())
                .or_else(|| sel_copy.borrow().clone())
        } else {
            sel_copy.borrow().clone()
        };
        if let Some(text) = text {
            let display = term_copy.display();
            display.clipboard().set_text(&text);
        }
    });
    action_group.add_action(&action_copy);

    let term_paste = terminal.clone();
    let action_paste = gio::SimpleAction::new("paste", None);
    action_paste.connect_activate(move |_, _| {
        term_paste.paste_clipboard();
    });
    action_group.add_action(&action_paste);

    let term_select = terminal.clone();
    let action_select = gio::SimpleAction::new("select-all", None);
    action_select.connect_activate(move |_, _| {
        term_select.select_all();
    });
    action_group.add_action(&action_select);

    // Install on the terminal itself so the action group follows the
    // widget when it is reparented between TabView and split view panels.
    terminal.insert_action_group("terminal", Some(&action_group));

    // Let VTE handle the right-click popover natively.
    terminal.set_context_menu_model(Some(&menu));
}

/// Maximum number of snippets to show inline in the context menu.
/// If more exist, a single "Execute Snippet…" item opens the picker.
pub const SNIPPET_INLINE_LIMIT: usize = 5;

/// Rebuilds the snippet section of the terminal context menu.
///
/// Only shows snippets with `target` compatible with VTE terminals
/// (i.e. `Terminal` or `Any`; Windows-only snippets are excluded).
///
/// - ≤ `SNIPPET_INLINE_LIMIT` snippets → each shown as a direct menu item
///   with action `win.run-snippet-direct('uuid')`.
/// - More than limit → single "Execute Snippet…" item opening the picker.
/// - No snippets → single "Execute Snippet…" item (opens manager).
pub fn rebuild_snippet_menu_section(
    section: &gtk4::gio::Menu,
    state: &crate::state::SharedAppState,
) {
    section.remove_all();

    let state_ref = state.borrow();
    let snippets: Vec<_> = state_ref
        .list_snippets()
        .into_iter()
        .filter(|s| s.target.is_terminal_compatible())
        .collect();

    if snippets.len() <= SNIPPET_INLINE_LIMIT && !snippets.is_empty() {
        // Few snippets — show each inline for quick access
        for snippet in snippets {
            let action = format!("win.run-snippet-direct('{}')", snippet.id);
            section.append(Some(&snippet.name), Some(&action));
        }
    } else {
        // No snippets or too many — show picker
        section.append(
            Some(&crate::i18n::i18n("Execute Snippet…")),
            Some("win.execute-snippet"),
        );
    }
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

/// Minimum font scale factor (roughly 50% of base size)
const FONT_SCALE_MIN: f64 = 0.5;
/// Maximum font scale factor (roughly 400% of base size)
const FONT_SCALE_MAX: f64 = 4.0;
/// Step for each zoom increment/decrement
const FONT_SCALE_STEP: f64 = 0.1;

/// Sets up font zoom via Ctrl+Scroll, Ctrl+Plus/Minus, and Ctrl+0 to reset.
///
/// Uses VTE's built-in `set_font_scale()` which scales the configured font
/// without changing the underlying `FontDescription`. This means the zoom
/// level is per-terminal and resets when a new session is created.
fn setup_font_zoom(terminal: &Terminal) {
    // Ctrl+Scroll wheel zoom
    let scroll_controller =
        gtk4::EventControllerScroll::new(gtk4::EventControllerScrollFlags::VERTICAL);
    let term_scroll = terminal.clone();
    scroll_controller.connect_scroll(move |_, _, dy| {
        let state = gdk::Display::default()
            .and_then(|d| d.default_seat())
            .and_then(|s| s.keyboard())
            .map(|k| k.modifier_state())
            .unwrap_or_else(gdk::ModifierType::empty);

        if !state.contains(gdk::ModifierType::CONTROL_MASK) {
            return glib::Propagation::Proceed;
        }

        let current = term_scroll.font_scale();
        let new_scale = if dy < 0.0 {
            (current + FONT_SCALE_STEP).min(FONT_SCALE_MAX)
        } else {
            (current - FONT_SCALE_STEP).max(FONT_SCALE_MIN)
        };
        term_scroll.set_font_scale(new_scale);
        glib::Propagation::Stop
    });
    terminal.add_controller(scroll_controller);

    // Ctrl+Plus / Ctrl+Minus / Ctrl+0 keyboard zoom
    let key_controller = gtk4::EventControllerKey::new();
    let term_key = terminal.clone();
    key_controller.connect_key_pressed(move |_, key, _, state| {
        if !state.contains(gdk::ModifierType::CONTROL_MASK) {
            return glib::Propagation::Proceed;
        }

        match key.name().as_deref() {
            Some("plus" | "equal" | "KP_Add") => {
                let s = (term_key.font_scale() + FONT_SCALE_STEP).min(FONT_SCALE_MAX);
                term_key.set_font_scale(s);
                glib::Propagation::Stop
            }
            Some("minus" | "KP_Subtract") => {
                let s = (term_key.font_scale() - FONT_SCALE_STEP).max(FONT_SCALE_MIN);
                term_key.set_font_scale(s);
                glib::Propagation::Stop
            }
            Some("0" | "KP_0") => {
                term_key.set_font_scale(1.0);
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    });
    terminal.add_controller(key_controller);
}

/// Sets up terminal font with settings
fn setup_font_with_settings(terminal: &Terminal, settings: &TerminalSettings) {
    // Guard against zero/invalid font size that causes Pango assertion failures
    let font_size = if settings.font_size == 0 {
        12
    } else {
        settings.font_size
    };
    let font_desc = gtk4::pango::FontDescription::from_string(&format!(
        "{} {}",
        settings.font_family, font_size
    ));
    terminal.set_font(Some(&font_desc));
}

/// Converts a hex color string (`#RRGGBB` or `#RRGGBBAA`) to a GDK RGBA value.
///
/// Returns `None` if the string is not a valid hex color.
fn hex_to_rgba(hex: &str) -> Option<gdk::RGBA> {
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
    Some(gdk::RGBA::new(
        f32::from(r) / 255.0,
        f32::from(g) / 255.0,
        f32::from(b) / 255.0,
        f32::from(a) / 255.0,
    ))
}

/// Applies per-connection theme override colors to a VTE terminal.
///
/// Applies per-connection theme override colors to a VTE terminal.
///
/// Rebuilds the full palette via `set_colors()` so that both the explicit
/// foreground/background AND the ANSI palette entries (7=white, 15=bright
/// white for foreground; 0=black, 8=bright black for background) are
/// consistent with the override. Without this, shells that emit ANSI color
/// codes (e.g. palette index 7 for "white") would still show the global
/// theme's grey instead of the user's chosen white (#145).
///
/// The `base_theme` provides the palette skeleton — only entries affected
/// by the override are replaced.
pub fn apply_theme_override_with_base(
    terminal: &Terminal,
    theme_override: &ConnectionThemeOverride,
    base_theme: &TerminalTheme,
) {
    let fg_rgba = theme_override
        .foreground
        .as_deref()
        .and_then(hex_to_rgba)
        .unwrap_or_else(|| color_to_rgba(&base_theme.foreground));

    let bg_rgba = theme_override
        .background
        .as_deref()
        .and_then(hex_to_rgba)
        .unwrap_or_else(|| color_to_rgba(&base_theme.background));

    // Build palette from base theme, overriding relevant ANSI entries
    let mut palette_rgba: Vec<gdk::RGBA> = base_theme.palette.iter().map(color_to_rgba).collect();

    // If foreground is overridden, also update palette white (7) and
    // bright white (15) so ANSI-colored output matches the custom color.
    if theme_override.foreground.is_some() {
        palette_rgba[7] = fg_rgba;
        palette_rgba[15] = fg_rgba;
    }

    // If background is overridden, also update palette black (0) and
    // bright black (8) for consistency.
    if theme_override.background.is_some() {
        palette_rgba[0] = bg_rgba;
        palette_rgba[8] = bg_rgba;
    }

    let palette_refs: Vec<&gdk::RGBA> = palette_rgba.iter().collect();
    terminal.set_colors(Some(&fg_rgba), Some(&bg_rgba), &palette_refs);

    // Cursor override (independent of palette)
    if let Some(ref cursor) = theme_override.cursor
        && let Some(rgba) = hex_to_rgba(cursor)
    {
        terminal.set_color_cursor(Some(&rgba));
    }
}

/// macOS: intercepts Option+key combinations in Capture phase and sends the
/// composed Unicode character directly to the PTY instead of letting VTE
/// interpret it as Alt/Meta + escape sequence.
///
/// On macOS with non-US keyboard layouts (German, French, etc.), the Option
/// key is used to type common characters like @ (Option+L on German) or €
/// (Option+E on many layouts). VTE treats Option as Alt/Meta and sends
/// `ESC + keycode`, which is incorrect for composed text input.
///
/// This handler checks whether the GDK keyval produced by the macOS IM has
/// a printable Unicode mapping. If so, it feeds the character directly to
/// the terminal and stops propagation, preventing VTE's internal Alt handler.
///
/// When `option_is_meta` is `true` in settings, this handler is NOT installed
/// and VTE's default behavior (ESC prefix) is used — suitable for vim/emacs.
#[cfg(target_os = "macos")]
fn setup_macos_option_key_handler(terminal: &Terminal) {
    let controller = gtk4::EventControllerKey::new();
    // Capture phase — intercept BEFORE VTE's internal key handler
    controller.set_propagation_phase(gtk4::PropagationPhase::Capture);

    let term = terminal.clone();
    controller.connect_key_pressed(move |_ctrl, keyval, _keycode, modifiers| {
        // Only act when Option/Alt is pressed
        if !modifiers.contains(gdk::ModifierType::ALT_MASK) {
            return glib::Propagation::Proceed;
        }
        // Do not intercept Ctrl+Option or Super+Option combos
        if modifiers.contains(gdk::ModifierType::CONTROL_MASK)
            || modifiers.contains(gdk::ModifierType::SUPER_MASK)
        {
            return glib::Propagation::Proceed;
        }
        // Do not intercept Shift+Option (uppercase compose is handled by the
        // same logic — the keyval already reflects the shifted character)
        // but we do NOT block it, Shift+Option is also a compose trigger.

        // If GDK resolved this to a printable Unicode character, it came from
        // the macOS IMContext compose path. Feed it as text to the PTY.
        if let Some(ch) = keyval.to_unicode() {
            if ch.is_control() || ch == '\u{ffff}' {
                // Non-printable or invalid — let VTE handle as Alt+key
                return glib::Propagation::Proceed;
            }
            let mut buf = [0u8; 4];
            let text = ch.encode_utf8(&mut buf);
            term.feed_child(text.as_bytes());
            return glib::Propagation::Stop;
        }

        glib::Propagation::Proceed
    });
    terminal.add_controller(controller);
}
