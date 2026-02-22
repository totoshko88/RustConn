//! UI settings tab using libadwaita components

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, CheckButton, DropDown, StringList, ToggleButton};
use libadwaita as adw;
use rustconn_core::config::{ColorScheme, SessionRestoreSettings, StartupAction, UiSettings};
use rustconn_core::models::Connection;

use crate::i18n::i18n;

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
    DropDown,
) {
    let page = adw::PreferencesPage::builder()
        .title(i18n("Interface"))
        .icon_name("applications-graphics-symbolic")
        .build();

    // === Appearance Group ===
    let appearance_group = adw::PreferencesGroup::builder()
        .title(i18n("Appearance"))
        .build();

    // Color scheme row with toggle buttons
    let color_scheme_box = GtkBox::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(0)
        .valign(gtk4::Align::Center)
        .css_classes(["linked"])
        .width_request(255)
        .build();

    let system_btn = ToggleButton::builder()
        .label(i18n("System"))
        .hexpand(true)
        .build();
    let light_btn = ToggleButton::builder()
        .label(i18n("Light"))
        .hexpand(true)
        .build();
    let dark_btn = ToggleButton::builder()
        .label(i18n("Dark"))
        .hexpand(true)
        .build();

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

    let color_scheme_row = adw::ActionRow::builder().title(i18n("Theme")).build();
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
        .title(i18n("Language"))
        .subtitle(i18n("Restart required to apply"))
        .build();
    language_row.add_suffix(&language_dropdown);
    appearance_group.add(&language_row);

    page.add(&appearance_group);

    // === Window Group ===
    let window_group = adw::PreferencesGroup::builder()
        .title(i18n("Window"))
        .build();

    let remember_geometry = CheckButton::builder().valign(gtk4::Align::Center).build();
    let remember_geometry_row = adw::ActionRow::builder()
        .title(i18n("Remember size"))
        .subtitle(i18n("Restore window geometry on startup"))
        .activatable_widget(&remember_geometry)
        .build();
    remember_geometry_row.add_prefix(&remember_geometry);
    window_group.add(&remember_geometry_row);

    page.add(&window_group);

    // === Startup Group ===
    let startup_group = adw::PreferencesGroup::builder()
        .title(i18n("Startup"))
        .description(i18n("Action to perform when the application starts"))
        .build();

    // Startup action dropdown — populated with connections in load_ui_settings
    let startup_action_dropdown = DropDown::builder()
        .model(&StringList::new(&[
            &i18n("Do nothing"),
            &i18n("Local Shell"),
        ]))
        .valign(gtk4::Align::Center)
        .build();

    let startup_action_row = adw::ActionRow::builder()
        .title(i18n("On startup"))
        .subtitle(i18n("Open session automatically"))
        .build();
    startup_action_row.add_suffix(&startup_action_dropdown);
    startup_group.add(&startup_action_row);

    page.add(&startup_group);

    // === System Tray Group ===
    let tray_group = adw::PreferencesGroup::builder()
        .title(i18n("System Tray"))
        .description(i18n("Requires desktop environment with tray support"))
        .build();

    let enable_tray_icon = CheckButton::builder().valign(gtk4::Align::Center).build();
    let enable_tray_row = adw::ActionRow::builder()
        .title(i18n("Show icon"))
        .subtitle(i18n("Display icon in system tray"))
        .activatable_widget(&enable_tray_icon)
        .build();
    enable_tray_row.add_prefix(&enable_tray_icon);
    tray_group.add(&enable_tray_row);

    let minimize_to_tray = CheckButton::builder().valign(gtk4::Align::Center).build();
    let minimize_to_tray_row = adw::ActionRow::builder()
        .title(i18n("Minimize to tray"))
        .subtitle(i18n("Hide window instead of closing"))
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
        .title(i18n("Session Restore"))
        .description(i18n("Restore previous connections on startup"))
        .build();

    let session_restore_enabled = CheckButton::builder().valign(gtk4::Align::Center).build();
    let session_restore_row = adw::ActionRow::builder()
        .title(i18n("Enabled"))
        .subtitle(i18n("Reconnect to previous sessions on startup"))
        .activatable_widget(&session_restore_enabled)
        .build();
    session_restore_row.add_prefix(&session_restore_enabled);
    session_group.add(&session_restore_row);

    let prompt_on_restore = CheckButton::builder().valign(gtk4::Align::Center).build();
    let prompt_on_restore_row = adw::ActionRow::builder()
        .title(i18n("Ask first"))
        .subtitle(i18n("Prompt before restoring sessions"))
        .activatable_widget(&prompt_on_restore)
        .build();
    prompt_on_restore_row.add_prefix(&prompt_on_restore);
    session_group.add(&prompt_on_restore_row);

    let max_age_row = adw::SpinRow::builder()
        .title(i18n("Max age"))
        .subtitle(i18n("Hours before sessions expire"))
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
        startup_action_dropdown,
    )
}

/// Connection entry in the startup action dropdown.
///
/// Indices 0 and 1 are reserved for "Do nothing" and "Local Shell".
/// Indices 2+ map to connections sorted alphabetically.
struct StartupConnectionEntry {
    id: uuid::Uuid,
    display_name: String,
}

/// Builds the sorted list of connections for the startup dropdown.
fn build_startup_entries(connections: &[&Connection]) -> Vec<StartupConnectionEntry> {
    let mut entries: Vec<StartupConnectionEntry> = connections
        .iter()
        .map(|c| StartupConnectionEntry {
            id: c.id,
            display_name: format!("{} ({})", c.name, c.protocol),
        })
        .collect();
    entries.sort_by(|a, b| {
        a.display_name
            .to_lowercase()
            .cmp(&b.display_name.to_lowercase())
    });
    entries
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
    startup_action_dropdown: &DropDown,
    settings: &UiSettings,
    connections: &[&Connection],
) {
    let target_index = match settings.color_scheme {
        ColorScheme::System => 0,
        ColorScheme::Light => 1,
        ColorScheme::Dark => 2,
    };

    let mut child = color_scheme_box.first_child();
    let mut index = 0;
    while let Some(widget) = child {
        if let Some(btn) = widget.downcast_ref::<ToggleButton>()
            && index == target_index
        {
            btn.set_active(true);
            crate::app::apply_color_scheme(settings.color_scheme);
            break;
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

    // Populate startup action dropdown with connections
    let entries = build_startup_entries(connections);
    let mut labels: Vec<String> = vec![i18n("Do nothing"), i18n("Local Shell")];
    for entry in &entries {
        labels.push(entry.display_name.clone());
    }
    let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    startup_action_dropdown.set_model(Some(&StringList::new(&label_refs)));

    // Select the current startup action
    let selected = match &settings.startup_action {
        StartupAction::None => 0,
        StartupAction::LocalShell => 1,
        StartupAction::Connection(id) => entries
            .iter()
            .position(|e| e.id == *id)
            .map_or(0, |pos| pos + 2),
    };
    #[allow(clippy::cast_possible_truncation)]
    startup_action_dropdown.set_selected(selected as u32);
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
    startup_action_dropdown: &DropDown,
    connections: &[&Connection],
) -> UiSettings {
    let mut selected_scheme = ColorScheme::System;
    let mut child = color_scheme_box.first_child();
    let mut index = 0;
    while let Some(widget) = child {
        if let Some(btn) = widget.downcast_ref::<ToggleButton>()
            && btn.is_active()
        {
            selected_scheme = match index {
                0 => ColorScheme::System,
                1 => ColorScheme::Light,
                2 => ColorScheme::Dark,
                _ => ColorScheme::System,
            };
            break;
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

    // Resolve startup action from dropdown index
    let entries = build_startup_entries(connections);
    let startup_idx = startup_action_dropdown.selected() as usize;
    let startup_action = match startup_idx {
        0 => StartupAction::None,
        1 => StartupAction::LocalShell,
        n => entries
            .get(n - 2)
            .map_or(StartupAction::None, |e| StartupAction::Connection(e.id)),
    };

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
        startup_action,
    }
}
