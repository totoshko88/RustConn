//! UI settings tab using libadwaita components

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, CheckButton, DropDown, StringList, ToggleButton};
use libadwaita as adw;
use rustconn_core::config::{ColorScheme, SessionRestoreSettings, UiSettings};

/// Creates the UI settings page using AdwPreferencesPage
#[allow(clippy::type_complexity)]
pub fn create_ui_page() -> (
    adw::PreferencesPage,
    GtkBox,
    DropDown,
    CheckButton,
    CheckButton,
    CheckButton,
    CheckButton,
    CheckButton,
    adw::SpinRow,
) {
    let page = adw::PreferencesPage::builder()
        .title("Interface")
        .icon_name("applications-graphics-symbolic")
        .build();

    // === Appearance Group ===
    let appearance_group = adw::PreferencesGroup::builder().title("Appearance").build();

    // Color scheme row with toggle buttons
    let color_scheme_box = GtkBox::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(0)
        .valign(gtk4::Align::Center)
        .css_classes(["linked"])
        .width_request(255)
        .build();

    let system_btn = ToggleButton::builder()
        .label("System")
        .hexpand(true)
        .build();
    let light_btn = ToggleButton::builder().label("Light").hexpand(true).build();
    let dark_btn = ToggleButton::builder().label("Dark").hexpand(true).build();

    light_btn.set_group(Some(&system_btn));
    dark_btn.set_group(Some(&system_btn));
    system_btn.set_active(true);

    system_btn.connect_toggled(|btn| {
        if btn.is_active() {
            crate::app::apply_color_scheme(ColorScheme::System);
        }
    });

    light_btn.connect_toggled(|btn| {
        if btn.is_active() {
            crate::app::apply_color_scheme(ColorScheme::Light);
        }
    });

    dark_btn.connect_toggled(|btn| {
        if btn.is_active() {
            crate::app::apply_color_scheme(ColorScheme::Dark);
        }
    });

    color_scheme_box.append(&system_btn);
    color_scheme_box.append(&light_btn);
    color_scheme_box.append(&dark_btn);

    let color_scheme_row = adw::ActionRow::builder().title("Theme").build();
    color_scheme_row.add_suffix(&color_scheme_box);
    appearance_group.add(&color_scheme_row);

    // Language selector dropdown
    let languages = crate::i18n::available_languages();
    let display_names: Vec<&str> = languages.iter().map(|(_, name)| *name).collect();
    let string_list = StringList::new(&display_names);
    let language_dropdown = DropDown::builder()
        .model(&string_list)
        .valign(gtk4::Align::Center)
        .build();

    let language_row = adw::ActionRow::builder()
        .title("Language")
        .subtitle("Restart required to apply")
        .build();
    language_row.add_suffix(&language_dropdown);
    appearance_group.add(&language_row);

    page.add(&appearance_group);

    // === Window Group ===
    let window_group = adw::PreferencesGroup::builder().title("Window").build();

    let remember_geometry = CheckButton::builder().valign(gtk4::Align::Center).build();
    let remember_geometry_row = adw::ActionRow::builder()
        .title("Remember size")
        .subtitle("Restore window geometry on startup")
        .activatable_widget(&remember_geometry)
        .build();
    remember_geometry_row.add_prefix(&remember_geometry);
    window_group.add(&remember_geometry_row);

    page.add(&window_group);

    // === System Tray Group ===
    let tray_group = adw::PreferencesGroup::builder()
        .title("System Tray")
        .description("Requires desktop environment with tray support")
        .build();

    let enable_tray_icon = CheckButton::builder().valign(gtk4::Align::Center).build();
    let enable_tray_row = adw::ActionRow::builder()
        .title("Show icon")
        .subtitle("Display icon in system tray")
        .activatable_widget(&enable_tray_icon)
        .build();
    enable_tray_row.add_prefix(&enable_tray_icon);
    tray_group.add(&enable_tray_row);

    let minimize_to_tray = CheckButton::builder().valign(gtk4::Align::Center).build();
    let minimize_to_tray_row = adw::ActionRow::builder()
        .title("Minimize to tray")
        .subtitle("Hide window instead of closing")
        .activatable_widget(&minimize_to_tray)
        .build();
    minimize_to_tray_row.add_prefix(&minimize_to_tray);
    tray_group.add(&minimize_to_tray_row);

    // Make minimize_to_tray sensitive based on enable_tray_icon
    let minimize_to_tray_clone = minimize_to_tray.clone();
    enable_tray_icon.connect_toggled(move |check| {
        minimize_to_tray_clone.set_sensitive(check.is_active());
    });

    page.add(&tray_group);

    // === Session Restore Group ===
    let session_group = adw::PreferencesGroup::builder()
        .title("Session Restore")
        .description("Restore previous connections on startup")
        .build();

    let session_restore_enabled = CheckButton::builder().valign(gtk4::Align::Center).build();
    let session_restore_row = adw::ActionRow::builder()
        .title("Enabled")
        .subtitle("Reconnect to previous sessions on startup")
        .activatable_widget(&session_restore_enabled)
        .build();
    session_restore_row.add_prefix(&session_restore_enabled);
    session_group.add(&session_restore_row);

    let prompt_on_restore = CheckButton::builder().valign(gtk4::Align::Center).build();
    let prompt_on_restore_row = adw::ActionRow::builder()
        .title("Ask first")
        .subtitle("Prompt before restoring sessions")
        .activatable_widget(&prompt_on_restore)
        .build();
    prompt_on_restore_row.add_prefix(&prompt_on_restore);
    session_group.add(&prompt_on_restore_row);

    let max_age_row = adw::SpinRow::builder()
        .title("Max age")
        .subtitle("Hours before sessions expire")
        .adjustment(&gtk4::Adjustment::new(24.0, 1.0, 168.0, 1.0, 24.0, 0.0))
        .build();
    session_group.add(&max_age_row);

    // Make session options sensitive based on session_restore_enabled
    let prompt_on_restore_clone = prompt_on_restore.clone();
    let max_age_row_clone = max_age_row.clone();
    session_restore_enabled.connect_toggled(move |check| {
        let active = check.is_active();
        prompt_on_restore_clone.set_sensitive(active);
        max_age_row_clone.set_sensitive(active);
    });

    page.add(&session_group);

    (
        page,
        color_scheme_box,
        language_dropdown,
        remember_geometry,
        enable_tray_icon,
        minimize_to_tray,
        session_restore_enabled,
        prompt_on_restore,
        max_age_row,
    )
}

/// Loads UI settings into UI controls
#[allow(clippy::too_many_arguments)]
pub fn load_ui_settings(
    color_scheme_box: &GtkBox,
    language_dropdown: &DropDown,
    remember_geometry: &CheckButton,
    enable_tray_icon: &CheckButton,
    minimize_to_tray: &CheckButton,
    session_restore_enabled: &CheckButton,
    prompt_on_restore: &CheckButton,
    max_age_row: &adw::SpinRow,
    settings: &UiSettings,
) {
    let target_index = match settings.color_scheme {
        ColorScheme::System => 0,
        ColorScheme::Light => 1,
        ColorScheme::Dark => 2,
    };

    let mut child = color_scheme_box.first_child();
    let mut index = 0;
    while let Some(widget) = child {
        if let Some(btn) = widget.downcast_ref::<ToggleButton>() {
            if index == target_index {
                btn.set_active(true);
                crate::app::apply_color_scheme(settings.color_scheme);
                break;
            }
        }
        child = widget.next_sibling();
        index += 1;
    }

    // Set language dropdown to saved language
    let languages = crate::i18n::available_languages();
    let lang_index = languages
        .iter()
        .position(|(code, _)| *code == settings.language)
        .unwrap_or(0); // Default to "System" if not found
    language_dropdown.set_selected(lang_index as u32);

    remember_geometry.set_active(settings.remember_window_geometry);
    enable_tray_icon.set_active(settings.enable_tray_icon);
    minimize_to_tray.set_active(settings.minimize_to_tray);
    minimize_to_tray.set_sensitive(settings.enable_tray_icon);

    session_restore_enabled.set_active(settings.session_restore.enabled);
    prompt_on_restore.set_active(settings.session_restore.prompt_on_restore);
    max_age_row.set_value(f64::from(settings.session_restore.max_age_hours));

    prompt_on_restore.set_sensitive(settings.session_restore.enabled);
    max_age_row.set_sensitive(settings.session_restore.enabled);
}

/// Collects UI settings from UI controls
#[allow(clippy::too_many_arguments)]
pub fn collect_ui_settings(
    color_scheme_box: &GtkBox,
    language_dropdown: &DropDown,
    remember_geometry: &CheckButton,
    enable_tray_icon: &CheckButton,
    minimize_to_tray: &CheckButton,
    session_restore_enabled: &CheckButton,
    prompt_on_restore: &CheckButton,
    max_age_row: &adw::SpinRow,
) -> UiSettings {
    let mut selected_scheme = ColorScheme::System;
    let mut child = color_scheme_box.first_child();
    let mut index = 0;
    while let Some(widget) = child {
        if let Some(btn) = widget.downcast_ref::<ToggleButton>() {
            if btn.is_active() {
                selected_scheme = match index {
                    0 => ColorScheme::System,
                    1 => ColorScheme::Light,
                    2 => ColorScheme::Dark,
                    _ => ColorScheme::System,
                };
                break;
            }
        }
        child = widget.next_sibling();
        index += 1;
    }

    // Get selected language code
    let languages = crate::i18n::available_languages();
    let lang_idx = language_dropdown.selected() as usize;
    let language = languages
        .get(lang_idx)
        .map_or_else(|| "system".to_string(), |(code, _)| (*code).to_string());

    UiSettings {
        color_scheme: selected_scheme,
        language,
        remember_window_geometry: remember_geometry.is_active(),
        window_width: None,
        window_height: None,
        sidebar_width: None,
        enable_tray_icon: enable_tray_icon.is_active(),
        minimize_to_tray: minimize_to_tray.is_active(),
        expanded_groups: std::collections::HashSet::new(),
        session_restore: SessionRestoreSettings {
            enabled: session_restore_enabled.is_active(),
            prompt_on_restore: prompt_on_restore.is_active(),
            #[allow(clippy::cast_sign_loss)]
            max_age_hours: max_age_row.value().max(0.0) as u32,
            saved_sessions: Vec::new(),
        },
        search_history: Vec::new(), // Preserve existing history from current settings
    }
}
