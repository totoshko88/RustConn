//! Terminal settings tab using libadwaita components

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, DropDown, Entry, Orientation, SpinButton, StringList, ToggleButton, gdk,
};
use libadwaita as adw;
use rustconn_core::config::TerminalSettings;
use rustconn_core::terminal_themes::TerminalTheme;

use crate::i18n::i18n;
use crate::i18n::i18n_f;

/// Creates the terminal settings page using AdwPreferencesPage
#[expect(
    clippy::type_complexity,
    reason = "internal helper signature documents the exact tuple layout used by the caller; aliasing would obscure the data flow"
)]
pub fn create_terminal_page() -> (
    adw::PreferencesPage,
    Entry,
    SpinButton,
    SpinButton,
    DropDown,
    GtkBox, // cursor shape buttons container
    GtkBox, // cursor blink buttons container
    adw::SwitchRow,
    adw::SwitchRow,
    adw::SwitchRow,
    adw::SwitchRow,
    adw::SwitchRow,
    adw::SwitchRow, // sftp_use_mc
    adw::SwitchRow, // copy_on_select
    adw::SwitchRow, // show_scrollbar
    Entry,          // local_shell_command
    adw::SwitchRow, // close_on_clean_exit
) {
    let page = adw::PreferencesPage::builder()
        .title(i18n("Terminal"))
        .icon_name("utilities-terminal-symbolic")
        .build();

    // === Font Group ===
    let font_group = adw::PreferencesGroup::builder().title(i18n("Font")).build();

    // Font family row - simplified title
    let font_family_entry = Entry::builder()
        .text("Monospace")
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .build();
    let font_family_row = adw::ActionRow::builder().title(i18n("Family")).build();
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
    let font_size_row = adw::ActionRow::builder().title(i18n("Size")).build();
    font_size_row.add_suffix(&font_size_spin);
    font_size_row.set_activatable_widget(Some(&font_size_spin));
    font_group.add(&font_size_row);

    page.add(&font_group);

    // === Colors Group ===
    let colors_group = adw::PreferencesGroup::builder()
        .title(i18n("Colors"))
        .build();

    let theme_names = TerminalTheme::theme_names();
    let theme_list = StringList::new(&theme_names.iter().map(String::as_str).collect::<Vec<_>>());
    let color_theme_dropdown = DropDown::builder()
        .model(&theme_list)
        .selected(0)
        .valign(gtk4::Align::Center)
        .build();
    let color_theme_row = adw::ActionRow::builder().title(i18n("Theme")).build();
    color_theme_row.add_suffix(&color_theme_dropdown);
    color_theme_row.set_activatable_widget(Some(&color_theme_dropdown));
    colors_group.add(&color_theme_row);

    // Custom theme management buttons
    let theme_buttons_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .valign(gtk4::Align::Center)
        .build();

    let new_theme_btn = gtk4::Button::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text(i18n("New custom theme"))
        .css_classes(["flat"])
        .build();
    new_theme_btn.update_property(&[gtk4::accessible::Property::Label(&i18n("New custom theme"))]);

    let edit_theme_btn = gtk4::Button::builder()
        .icon_name("document-edit-symbolic")
        .tooltip_text(i18n("Edit custom theme"))
        .css_classes(["flat"])
        .sensitive(false)
        .build();
    edit_theme_btn.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Edit custom theme",
    ))]);

    let delete_theme_btn = gtk4::Button::builder()
        .icon_name("user-trash-symbolic")
        .tooltip_text(i18n("Delete custom theme"))
        .css_classes(["flat", "destructive-action"])
        .sensitive(false)
        .build();
    delete_theme_btn.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Delete custom theme",
    ))]);

    theme_buttons_box.append(&new_theme_btn);
    theme_buttons_box.append(&edit_theme_btn);
    theme_buttons_box.append(&delete_theme_btn);

    let manage_row = adw::ActionRow::builder()
        .title(i18n("Custom themes"))
        .build();
    manage_row.add_suffix(&theme_buttons_box);
    colors_group.add(&manage_row);

    // Enable edit/delete only for custom themes
    {
        let edit_btn = edit_theme_btn.clone();
        let del_btn = delete_theme_btn.clone();
        let dropdown = color_theme_dropdown.clone();
        dropdown.connect_selected_notify(move |dd| {
            let idx = dd.selected() as usize;
            let names = TerminalTheme::theme_names();
            let is_custom = names
                .get(idx)
                .is_some_and(|n| !TerminalTheme::is_builtin(n));
            edit_btn.set_sensitive(is_custom);
            del_btn.set_sensitive(is_custom);
        });
    }

    // "New" button — prompt for name, create custom theme, open editor
    {
        let dropdown = color_theme_dropdown.clone();
        let edit_btn_c = edit_theme_btn.clone();
        let del_btn_c = delete_theme_btn.clone();
        new_theme_btn.connect_clicked(move |btn| {
            let win = btn.root().and_then(|r| r.downcast::<gtk4::Window>().ok());
            let dropdown_c = dropdown.clone();
            let edit_btn_cc = edit_btn_c.clone();
            let del_btn_cc = del_btn_c.clone();
            prompt_new_theme_name(win.as_ref(), move |name| {
                let theme = TerminalTheme::new_custom(&name);
                TerminalTheme::save_custom_theme(theme.clone());
                refresh_theme_dropdown(&dropdown_c, &name);
                show_theme_editor(None, &theme, &dropdown_c, &edit_btn_cc, &del_btn_cc);
            });
        });
    }

    // "Edit" button — open editor for selected custom theme
    {
        let dropdown = color_theme_dropdown.clone();
        let edit_btn_c = edit_theme_btn.clone();
        let del_btn_c = delete_theme_btn.clone();
        edit_theme_btn.connect_clicked(move |btn| {
            let idx = dropdown.selected() as usize;
            let names = TerminalTheme::theme_names();
            if let Some(name) = names.get(idx)
                && let Some(theme) = TerminalTheme::by_name(name)
            {
                let win = btn.root().and_then(|r| r.downcast::<gtk4::Window>().ok());
                show_theme_editor(win.as_ref(), &theme, &dropdown, &edit_btn_c, &del_btn_c);
            }
        });
    }

    // "Delete" button — remove selected custom theme
    {
        let dropdown = color_theme_dropdown.clone();
        let edit_btn_c = edit_theme_btn.clone();
        let del_btn_c = delete_theme_btn.clone();
        delete_theme_btn.connect_clicked(move |_| {
            let idx = dropdown.selected() as usize;
            let names = TerminalTheme::theme_names();
            if let Some(name) = names.get(idx)
                && !TerminalTheme::is_builtin(name)
            {
                TerminalTheme::remove_custom_theme(name);
                refresh_theme_dropdown(&dropdown, "Dark");
                edit_btn_c.set_sensitive(false);
                del_btn_c.set_sensitive(false);
            }
        });
    }

    page.add(&colors_group);

    // === Cursor Group ===
    let cursor_group = adw::PreferencesGroup::builder()
        .title(i18n("Cursor"))
        .build();

    // Cursor shape
    #[cfg(feature = "adw-1-7")]
    let shape_buttons_box = {
        let toggle_group = adw::ToggleGroup::new();
        toggle_group.add(adw::Toggle::builder().label(i18n("Block")).build());
        toggle_group.add(adw::Toggle::builder().label(i18n("IBeam")).build());
        toggle_group.add(adw::Toggle::builder().label(i18n("Underline")).build());
        toggle_group.set_active(0);

        // Use GtkBox as the suffix container holding the ToggleGroup
        let wrapper = GtkBox::new(Orientation::Horizontal, 0);
        wrapper.set_valign(gtk4::Align::Center);
        wrapper.set_widget_name("cursor-shape-toggle-group");
        wrapper.append(&toggle_group);

        let cursor_shape_row = adw::ActionRow::builder().title(i18n("Shape")).build();
        cursor_shape_row.add_suffix(&wrapper);
        cursor_group.add(&cursor_shape_row);

        wrapper
    };

    #[cfg(not(feature = "adw-1-7"))]
    let shape_buttons_box = {
        let buttons_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(0)
            .valign(gtk4::Align::Center)
            .css_classes(["linked"])
            .width_request(240)
            .build();

        let shape_block_btn = ToggleButton::builder()
            .label(i18n("Block"))
            .active(true)
            .hexpand(true)
            .build();
        let shape_ibeam_btn = ToggleButton::builder()
            .label(i18n("IBeam"))
            .group(&shape_block_btn)
            .hexpand(true)
            .build();
        let shape_underline_btn = ToggleButton::builder()
            .label(i18n("Underline"))
            .group(&shape_block_btn)
            .hexpand(true)
            .build();

        buttons_box.append(&shape_block_btn);
        buttons_box.append(&shape_ibeam_btn);
        buttons_box.append(&shape_underline_btn);

        let cursor_shape_row = adw::ActionRow::builder().title(i18n("Shape")).build();
        cursor_shape_row.add_suffix(&buttons_box);
        cursor_group.add(&cursor_shape_row);

        buttons_box
    };

    // Cursor blink
    #[cfg(feature = "adw-1-7")]
    let blink_buttons_box = {
        let toggle_group = adw::ToggleGroup::new();
        toggle_group.add(adw::Toggle::builder().label(i18n("On")).build());
        toggle_group.add(adw::Toggle::builder().label(i18n("Off")).build());
        toggle_group.add(adw::Toggle::builder().label(i18n("System")).build());
        toggle_group.set_active(0);

        // Use GtkBox as the suffix container holding the ToggleGroup
        let wrapper = GtkBox::new(Orientation::Horizontal, 0);
        wrapper.set_valign(gtk4::Align::Center);
        wrapper.set_widget_name("cursor-blink-toggle-group");
        wrapper.append(&toggle_group);

        let cursor_blink_row = adw::ActionRow::builder().title(i18n("Blink")).build();
        cursor_blink_row.add_suffix(&wrapper);
        cursor_group.add(&cursor_blink_row);

        wrapper
    };

    #[cfg(not(feature = "adw-1-7"))]
    let blink_buttons_box = {
        let buttons_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(0)
            .valign(gtk4::Align::Center)
            .css_classes(["linked"])
            .width_request(240)
            .build();

        let blink_on_btn = ToggleButton::builder()
            .label(i18n("On"))
            .active(true)
            .hexpand(true)
            .build();
        let blink_off_btn = ToggleButton::builder()
            .label(i18n("Off"))
            .group(&blink_on_btn)
            .hexpand(true)
            .build();
        let blink_system_btn = ToggleButton::builder()
            .label(i18n("System"))
            .group(&blink_on_btn)
            .hexpand(true)
            .build();

        buttons_box.append(&blink_on_btn);
        buttons_box.append(&blink_off_btn);
        buttons_box.append(&blink_system_btn);

        let cursor_blink_row = adw::ActionRow::builder().title(i18n("Blink")).build();
        cursor_blink_row.add_suffix(&buttons_box);
        cursor_group.add(&cursor_blink_row);

        buttons_box
    };

    page.add(&cursor_group);

    // === Scrolling Group ===
    let scrolling_group = adw::PreferencesGroup::builder()
        .title(i18n("Scrolling"))
        .build();

    // Scrollback lines - simplified title
    let scrollback_adj = gtk4::Adjustment::new(10000.0, 100.0, 1_000_000.0, 100.0, 1000.0, 0.0);
    let scrollback_spin = SpinButton::builder()
        .adjustment(&scrollback_adj)
        .climb_rate(100.0)
        .digits(0)
        .valign(gtk4::Align::Center)
        .build();
    let scrollback_row = adw::ActionRow::builder()
        .title(i18n("History"))
        .subtitle(i18n("Number of lines to keep in scrollback"))
        .build();
    scrollback_row.add_suffix(&scrollback_spin);
    scrollback_row.set_activatable_widget(Some(&scrollback_spin));
    scrolling_group.add(&scrollback_row);

    // Scroll on output
    let scroll_on_output_row = adw::SwitchRow::builder()
        .title(i18n("On output"))
        .subtitle(i18n("Scroll to bottom when new output appears"))
        .build();
    scrolling_group.add(&scroll_on_output_row);

    // Scroll on keystroke
    let scroll_on_keystroke_row = adw::SwitchRow::builder()
        .title(i18n("On keystroke"))
        .subtitle(i18n("Scroll to bottom when typing"))
        .active(true)
        .build();
    scrolling_group.add(&scroll_on_keystroke_row);

    // Show scrollbar
    let show_scrollbar_row = adw::SwitchRow::builder()
        .title(i18n("Scrollbar"))
        .subtitle(i18n("Show a scrollbar next to the terminal"))
        .active(true)
        .build();
    scrolling_group.add(&show_scrollbar_row);

    page.add(&scrolling_group);

    // === Behavior Group ===
    let behavior_group = adw::PreferencesGroup::builder()
        .title(i18n("Behavior"))
        .build();

    // Allow hyperlinks
    let allow_hyperlinks_row = adw::SwitchRow::builder()
        .title(i18n("Hyperlinks"))
        .subtitle(i18n("Allow clickable URLs in terminal"))
        .active(true)
        .build();
    behavior_group.add(&allow_hyperlinks_row);

    // Hide mouse when typing
    let mouse_autohide_row = adw::SwitchRow::builder()
        .title(i18n("Hide pointer"))
        .subtitle(i18n("Hide mouse cursor when typing"))
        .active(true)
        .build();
    behavior_group.add(&mouse_autohide_row);

    // Audible bell
    let audible_bell_row = adw::SwitchRow::builder()
        .title(i18n("Bell"))
        .subtitle(i18n("Play sound on terminal bell"))
        .build();
    behavior_group.add(&audible_bell_row);

    // SFTP via Midnight Commander
    let sftp_use_mc_row = adw::SwitchRow::builder()
        .title(i18n("SFTP via mc"))
        .subtitle(i18n("Open SFTP in Midnight Commander (local shell tab)"))
        .build();

    // In Flatpak, warn when user disables mc — external file managers
    // cannot access the sandbox SSH agent.
    if rustconn_core::flatpak::is_flatpak() {
        let row_clone = sftp_use_mc_row.clone();
        sftp_use_mc_row.connect_active_notify(move |row| {
            if row.is_active() {
                row_clone.set_subtitle(&i18n("Open SFTP in Midnight Commander (local shell tab)"));
            } else {
                row_clone.set_subtitle(&i18n(
                    "⚠ External file managers cannot access SSH agent in Flatpak sandbox",
                ));
            }
        });
    }

    behavior_group.add(&sftp_use_mc_row);

    // Copy on select (X11-style)
    let copy_on_select_row = adw::SwitchRow::builder()
        .title(i18n("Copy on select"))
        .subtitle(i18n("Automatically copy selected text to clipboard"))
        .build();
    behavior_group.add(&copy_on_select_row);

    // Close tab on clean exit
    let close_on_clean_exit_row = adw::SwitchRow::builder()
        .title(i18n("Close tab on clean exit"))
        .subtitle(i18n(
            "Automatically close the tab when the session exits normally",
        ))
        .build();
    behavior_group.add(&close_on_clean_exit_row);

    page.add(&behavior_group);

    // === Local Shell Group ===
    let local_shell_group = adw::PreferencesGroup::builder()
        .title(i18n("Local Shell"))
        .description(i18n("Custom command for the local shell tab"))
        .build();

    let local_shell_command_entry = Entry::builder()
        .placeholder_text(i18n("Default system shell"))
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .build();
    let local_shell_command_row = adw::ActionRow::builder()
        .title(i18n("Command"))
        .subtitle(i18n("e.g. fish, bash --norc, neofetch &amp;&amp; bash"))
        .build();
    local_shell_command_row.add_suffix(&local_shell_command_entry);
    local_shell_command_row.set_activatable_widget(Some(&local_shell_command_entry));
    local_shell_group.add(&local_shell_command_row);

    page.add(&local_shell_group);

    (
        page,
        font_family_entry,
        font_size_spin,
        scrollback_spin,
        color_theme_dropdown,
        shape_buttons_box,
        blink_buttons_box,
        scroll_on_output_row,
        scroll_on_keystroke_row,
        allow_hyperlinks_row,
        mouse_autohide_row,
        audible_bell_row,
        sftp_use_mc_row,
        copy_on_select_row,
        show_scrollbar_row,
        local_shell_command_entry,
        close_on_clean_exit_row,
    )
}

/// Loads terminal settings into UI controls
#[expect(
    clippy::too_many_arguments,
    reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
)]
pub fn load_terminal_settings(
    font_family_entry: &Entry,
    font_size_spin: &SpinButton,
    scrollback_spin: &SpinButton,
    color_theme_dropdown: &DropDown,
    cursor_shape_buttons: &GtkBox,
    cursor_blink_buttons: &GtkBox,
    scroll_on_output_row: &adw::SwitchRow,
    scroll_on_keystroke_row: &adw::SwitchRow,
    allow_hyperlinks_row: &adw::SwitchRow,
    mouse_autohide_row: &adw::SwitchRow,
    audible_bell_row: &adw::SwitchRow,
    sftp_use_mc_row: &adw::SwitchRow,
    copy_on_select_row: &adw::SwitchRow,
    show_scrollbar_row: &adw::SwitchRow,
    local_shell_command_entry: &Entry,
    close_on_clean_exit_row: &adw::SwitchRow,
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
    set_toggle_index(cursor_shape_buttons, cursor_shape_index);

    // Set cursor blink
    let cursor_blink_index = match settings.cursor_blink.as_str() {
        "On" => 0,
        "Off" => 1,
        "System" => 2,
        _ => 0,
    };
    set_toggle_index(cursor_blink_buttons, cursor_blink_index);

    scroll_on_output_row.set_active(settings.scroll_on_output);
    scroll_on_keystroke_row.set_active(settings.scroll_on_keystroke);
    allow_hyperlinks_row.set_active(settings.allow_hyperlinks);
    mouse_autohide_row.set_active(settings.mouse_autohide);
    audible_bell_row.set_active(settings.audible_bell);
    sftp_use_mc_row.set_active(settings.sftp_use_mc);
    copy_on_select_row.set_active(settings.copy_on_select);
    show_scrollbar_row.set_active(settings.show_scrollbar);
    local_shell_command_entry.set_text(&settings.local_shell_command);
    close_on_clean_exit_row.set_active(settings.close_on_clean_exit);
}

/// Sets the active toggle index.
///
/// On adw-1-7+ the wrapper `GtkBox` holds an `AdwToggleGroup` child as a
/// reference. On older libadwaita the `GtkBox` contains linked `ToggleButton`s.
fn set_toggle_index(button_box: &GtkBox, index: usize) {
    #[cfg(feature = "adw-1-7")]
    {
        if let Some(toggle_group) = find_toggle_group_in_box(button_box) {
            toggle_group.set_active(index as u32);
            return;
        }
        // Fallback to legacy ToggleButton approach
        if let Some(btn) = get_toggle_button_at_index(button_box, index) {
            btn.set_active(true);
        }
    }
    #[cfg(not(feature = "adw-1-7"))]
    {
        if let Some(btn) = get_toggle_button_at_index(button_box, index) {
            btn.set_active(true);
        }
    }
}

/// Gets the active toggle index.
///
/// See [`set_toggle_index`] for the dual-path logic.
fn get_toggle_index(button_box: &GtkBox) -> usize {
    #[cfg(feature = "adw-1-7")]
    {
        if let Some(toggle_group) = find_toggle_group_in_box(button_box) {
            return toggle_group.active() as usize;
        }
        get_active_toggle_index(button_box)
    }
    #[cfg(not(feature = "adw-1-7"))]
    {
        get_active_toggle_index(button_box)
    }
}

/// Finds the `AdwToggleGroup` stored as a child of the marker `GtkBox`.
#[cfg(feature = "adw-1-7")]
fn find_toggle_group_in_box(marker: &GtkBox) -> Option<adw::ToggleGroup> {
    let child = marker.first_child()?;
    child.downcast::<adw::ToggleGroup>().ok()
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
        if let Ok(btn) = widget.clone().downcast::<ToggleButton>()
            && btn.is_active()
        {
            return i;
        }
        child = widget.next_sibling();
        i += 1;
    }
    0
}

/// Collects terminal settings from UI controls
#[expect(
    clippy::too_many_arguments,
    reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
)]
pub fn collect_terminal_settings(
    font_family_entry: &Entry,
    font_size_spin: &SpinButton,
    scrollback_spin: &SpinButton,
    color_theme_dropdown: &DropDown,
    cursor_shape_buttons: &GtkBox,
    cursor_blink_buttons: &GtkBox,
    scroll_on_output_row: &adw::SwitchRow,
    scroll_on_keystroke_row: &adw::SwitchRow,
    allow_hyperlinks_row: &adw::SwitchRow,
    mouse_autohide_row: &adw::SwitchRow,
    audible_bell_row: &adw::SwitchRow,
    sftp_use_mc_row: &adw::SwitchRow,
    copy_on_select_row: &adw::SwitchRow,
    show_scrollbar_row: &adw::SwitchRow,
    local_shell_command_entry: &Entry,
    close_on_clean_exit_row: &adw::SwitchRow,
    log_timestamps: bool,
) -> TerminalSettings {
    let theme_names = TerminalTheme::theme_names();
    let color_theme = theme_names
        .get(color_theme_dropdown.selected() as usize)
        .cloned()
        .unwrap_or_else(|| "Dark".to_string());

    let cursor_shapes = ["Block", "IBeam", "Underline"];
    let cursor_shape = cursor_shapes
        .get(get_toggle_index(cursor_shape_buttons))
        .unwrap_or(&"Block")
        .to_string();

    let cursor_blink_modes = ["On", "Off", "System"];
    let cursor_blink_mode = cursor_blink_modes
        .get(get_toggle_index(cursor_blink_buttons))
        .unwrap_or(&"On")
        .to_string();

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "value range fits the target type and is non-negative by construction in this code path"
    )]
    TerminalSettings {
        font_family: font_family_entry.text().to_string(),
        font_size: (font_size_spin.value() as u32).max(1),
        scrollback_lines: scrollback_spin.value() as u32,
        color_theme,
        cursor_shape,
        cursor_blink: cursor_blink_mode,
        scroll_on_output: scroll_on_output_row.is_active(),
        scroll_on_keystroke: scroll_on_keystroke_row.is_active(),
        allow_hyperlinks: allow_hyperlinks_row.is_active(),
        mouse_autohide: mouse_autohide_row.is_active(),
        audible_bell: audible_bell_row.is_active(),
        log_timestamps,
        sftp_use_mc: sftp_use_mc_row.is_active(),
        copy_on_select: copy_on_select_row.is_active(),
        show_scrollbar: show_scrollbar_row.is_active(),
        local_shell_command: local_shell_command_entry.text().trim().to_string(),
        close_on_clean_exit: close_on_clean_exit_row.is_active(),
    }
}

// ========================================================================
// Custom theme helpers
// ========================================================================

/// Refreshes the theme dropdown model and selects the given theme name.
fn refresh_theme_dropdown(dropdown: &DropDown, select_name: &str) {
    let names = TerminalTheme::theme_names();
    let list = StringList::new(&names.iter().map(String::as_str).collect::<Vec<_>>());
    dropdown.set_model(Some(&list));
    let idx = names.iter().position(|n| n == select_name).unwrap_or(0);
    dropdown.set_selected(idx as u32);
}

/// Prompts the user for a new theme name via an `AdwAlertDialog`.
fn prompt_new_theme_name<F>(parent: Option<&gtk4::Window>, on_accept: F)
where
    F: Fn(String) + 'static,
{
    let dialog = adw::AlertDialog::builder()
        .heading(i18n("New Custom Theme"))
        .body(i18n("Enter a name for the new theme"))
        .close_response("cancel")
        .default_response("create")
        .build();

    dialog.add_response("cancel", &i18n("Cancel"));
    dialog.add_response("create", &i18n("Create"));
    dialog.set_response_appearance("create", adw::ResponseAppearance::Suggested);

    let entry = Entry::builder()
        .placeholder_text(i18n("Theme name"))
        .hexpand(true)
        .build();
    dialog.set_extra_child(Some(&entry));

    let entry_c = entry.clone();
    dialog.connect_response(None, move |_, response| {
        if response == "create" {
            let name = entry_c.text().trim().to_string();
            if !name.is_empty() {
                on_accept(name);
            }
        }
    });

    if let Some(win) = parent {
        dialog.present(Some(win));
    } else {
        dialog.present(gtk4::Widget::NONE);
    }
}

/// Shows a theme editor dialog for the given theme.
///
/// The editor lets the user pick background, foreground, and cursor colors.
/// On save the theme is persisted and the dropdown is refreshed.
#[expect(
    clippy::too_many_lines,
    reason = "long match/dispatch over many enum variants; splitting per variant only relocates the boilerplate"
)]
fn show_theme_editor(
    parent: Option<&gtk4::Window>,
    theme: &TerminalTheme,
    dropdown: &DropDown,
    edit_btn: &gtk4::Button,
    delete_btn: &gtk4::Button,
) {
    use rustconn_core::terminal_themes::Color;

    let dialog = adw::AlertDialog::builder()
        .heading(i18n_f("{} — Theme Editor", &[&theme.name]))
        .close_response("cancel")
        .default_response("save")
        .build();

    dialog.add_response("cancel", &i18n("Cancel"));
    dialog.add_response("save", &i18n("Save"));
    dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);

    let content = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .build();

    // Helper to create a color row with a ColorDialogButton
    let make_color_row =
        |label_text: &str, color: &Color| -> (adw::ActionRow, gtk4::ColorDialogButton) {
            let rgba = gdk::RGBA::new(color.r, color.g, color.b, 1.0);
            let color_dialog = gtk4::ColorDialog::builder().with_alpha(false).build();
            let btn = gtk4::ColorDialogButton::builder()
                .dialog(&color_dialog)
                .rgba(&rgba)
                .valign(gtk4::Align::Center)
                .build();
            let row = adw::ActionRow::builder().title(label_text).build();
            row.add_suffix(&btn);
            (row, btn)
        };

    let (bg_row, bg_btn) = make_color_row(&i18n("Background"), &theme.background);
    let (fg_row, fg_btn) = make_color_row(&i18n("Foreground"), &theme.foreground);
    let (cur_row, cur_btn) = make_color_row(&i18n("Cursor"), &theme.cursor);

    let group = adw::PreferencesGroup::builder()
        .title(i18n("Colors"))
        .build();
    group.add(&bg_row);
    group.add(&fg_row);
    group.add(&cur_row);

    // Palette colors (16 ANSI colors)
    let palette_group = adw::PreferencesGroup::builder()
        .title(i18n("Palette"))
        .build();

    let palette_labels = [
        i18n("Black"),
        i18n("Red"),
        i18n("Green"),
        i18n("Yellow"),
        i18n("Blue"),
        i18n("Magenta"),
        i18n("Cyan"),
        i18n("White"),
        i18n("Bright Black"),
        i18n("Bright Red"),
        i18n("Bright Green"),
        i18n("Bright Yellow"),
        i18n("Bright Blue"),
        i18n("Bright Magenta"),
        i18n("Bright Cyan"),
        i18n("Bright White"),
    ];

    let palette_btns: Vec<gtk4::ColorDialogButton> = theme
        .palette
        .iter()
        .enumerate()
        .map(|(i, color)| {
            let label = palette_labels.get(i).map(String::as_str).unwrap_or("Color");
            let (row, btn) = make_color_row(label, color);
            palette_group.add(&row);
            btn
        })
        .collect();

    content.append(&group);
    content.append(&palette_group);

    let scrolled = gtk4::ScrolledWindow::builder()
        .child(&content)
        .min_content_height(400)
        .max_content_height(500)
        .propagate_natural_height(true)
        .build();

    dialog.set_extra_child(Some(&scrolled));

    let theme_name = theme.name.clone();
    let dropdown_c = dropdown.clone();
    let edit_btn_c = edit_btn.clone();
    let del_btn_c = delete_btn.clone();
    dialog.connect_response(None, move |_, response| {
        if response != "save" {
            return;
        }

        let rgba_to_color = |rgba: gdk::RGBA| Color::new(rgba.red(), rgba.green(), rgba.blue());

        let mut palette = TerminalTheme::dark_theme().palette;
        for (i, btn) in palette_btns.iter().enumerate() {
            palette[i] = rgba_to_color(btn.rgba());
        }

        let updated = TerminalTheme {
            name: theme_name.clone(),
            background: rgba_to_color(bg_btn.rgba()),
            foreground: rgba_to_color(fg_btn.rgba()),
            cursor: rgba_to_color(cur_btn.rgba()),
            palette,
            is_custom: true,
        };
        TerminalTheme::save_custom_theme(updated);
        refresh_theme_dropdown(&dropdown_c, &theme_name);

        let names = TerminalTheme::theme_names();
        let is_custom = names
            .get(dropdown_c.selected() as usize)
            .is_some_and(|n| !TerminalTheme::is_builtin(n));
        edit_btn_c.set_sensitive(is_custom);
        del_btn_c.set_sensitive(is_custom);
    });

    if let Some(win) = parent {
        dialog.present(Some(win));
    } else {
        dialog.present(gtk4::Widget::NONE);
    }
}
