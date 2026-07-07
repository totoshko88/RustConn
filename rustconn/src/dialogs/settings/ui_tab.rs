//! UI settings tab using libadwaita components

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, DropDown, StringList};
use libadwaita as adw;
use rustconn_core::config::{ColorScheme, SessionRestoreSettings, StartupAction, UiSettings};
use rustconn_core::models::Connection;

use crate::i18n::i18n;

/// Creates the UI settings page using AdwPreferencesPage
#[expect(
    clippy::type_complexity,
    reason = "internal helper signature documents the exact tuple layout used by the caller; aliasing would obscure the data flow"
)]
pub fn create_ui_page() -> (
    adw::PreferencesPage,
    GtkBox,
    DropDown,
    adw::SwitchRow,
    adw::SwitchRow,
    adw::SwitchRow,
    adw::SwitchRow,
    adw::SwitchRow,
    adw::SpinRow,
    DropDown,
    adw::SwitchRow,
    adw::SwitchRow,
    adw::SpinRow,
    adw::SwitchRow,
    adw::SwitchRow,
    adw::SwitchRow,
) {
    let page = adw::PreferencesPage::builder()
        .title(i18n("Interface"))
        .icon_name("applications-graphics-symbolic")
        .build();

    // === Appearance Group ===
    let appearance_group = adw::PreferencesGroup::builder()
        .title(i18n("Appearance"))
        .build();

    // Color scheme selector
    #[cfg(feature = "adw-1-7")]
    let color_scheme_box = {
        let toggle_group = adw::ToggleGroup::new();
        toggle_group.add(adw::Toggle::builder().label(i18n("System")).build());
        toggle_group.add(adw::Toggle::builder().label(i18n("Light")).build());
        toggle_group.add(adw::Toggle::builder().label(i18n("Dark")).build());
        toggle_group.set_active(0);

        toggle_group.connect_active_notify(|tg| {
            let scheme = match tg.active() {
                1 => ColorScheme::Light,
                2 => ColorScheme::Dark,
                _ => ColorScheme::System,
            };
            crate::app::apply_color_scheme(scheme);
        });

        // Wrap the ToggleGroup in a GtkBox so load/collect can locate it via
        // first_child() — same reparent-safe pattern as the cursor shape/blink
        // toggles. The wrapper IS the suffix added to the row (it must hold the
        // toggle group, not be an empty placeholder), otherwise load cannot
        // sync the segmented control to the saved scheme.
        let wrapper = GtkBox::new(gtk4::Orientation::Horizontal, 0);
        wrapper.set_valign(gtk4::Align::Center);
        wrapper.set_widget_name("color-scheme-toggle-group");
        wrapper.append(&toggle_group);

        let color_scheme_row = adw::ActionRow::builder().title(i18n("Theme")).build();
        color_scheme_row.add_suffix(&wrapper);
        appearance_group.add(&color_scheme_row);

        wrapper
    };

    #[cfg(not(feature = "adw-1-7"))]
    let color_scheme_box = {
        let buttons_box = GtkBox::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(0)
            .valign(gtk4::Align::Center)
            .css_classes(["linked"])
            .build();

        let system_btn = gtk4::ToggleButton::builder()
            .label(i18n("System"))
            .active(true)
            .hexpand(true)
            .build();
        let light_btn = gtk4::ToggleButton::builder()
            .label(i18n("Light"))
            .group(&system_btn)
            .hexpand(true)
            .build();
        let dark_btn = gtk4::ToggleButton::builder()
            .label(i18n("Dark"))
            .group(&system_btn)
            .hexpand(true)
            .build();

        buttons_box.append(&system_btn);
        buttons_box.append(&light_btn);
        buttons_box.append(&dark_btn);

        // Apply color scheme on toggle change
        let system_btn_c = system_btn.clone();
        let light_btn_c = light_btn.clone();
        system_btn_c.connect_toggled(move |btn| {
            if btn.is_active() {
                crate::app::apply_color_scheme(ColorScheme::System);
            }
        });
        light_btn_c.connect_toggled(move |btn| {
            if btn.is_active() {
                crate::app::apply_color_scheme(ColorScheme::Light);
            }
        });
        dark_btn.connect_toggled(move |btn| {
            if btn.is_active() {
                crate::app::apply_color_scheme(ColorScheme::Dark);
            }
        });

        let color_scheme_row = adw::ActionRow::builder().title(i18n("Theme")).build();
        color_scheme_row.add_suffix(&buttons_box);
        appearance_group.add(&color_scheme_row);

        buttons_box
    };

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
    language_row.set_activatable_widget(Some(&language_dropdown));
    appearance_group.add(&language_row);

    // Color tabs by protocol toggle
    let color_tabs_by_protocol = adw::SwitchRow::builder()
        .title(i18n("Color tabs by protocol"))
        .subtitle(i18n(
            "Show colored indicator on tabs based on protocol type",
        ))
        .build();
    appearance_group.add(&color_tabs_by_protocol);

    // Show protocol filters toggle
    let show_protocol_filters = adw::SwitchRow::builder()
        .title(i18n("Show protocol filters"))
        .subtitle(i18n("Display protocol filter bar in sidebar"))
        .build();
    appearance_group.add(&show_protocol_filters);

    // Sidebar width SpinRow
    let sidebar_width_row = adw::SpinRow::builder()
        .title(i18n("Sidebar width"))
        .subtitle(i18n("Width of the connection sidebar in pixels"))
        .adjustment(&gtk4::Adjustment::new(320.0, 180.0, 500.0, 10.0, 50.0, 0.0))
        .build();
    appearance_group.add(&sidebar_width_row);

    // Compact interface toggle — denser header bar, tabs and buttons.
    // Live preview: applies the .compact CSS class to all windows on toggle,
    // so the user sees the effect before clicking Save.
    let compact_ui = adw::SwitchRow::builder()
        .title(i18n("Compact interface"))
        .subtitle(i18n(
            "Reduce header bar, tab bar, and toolbar height. Applied instantly; useful on small screens, macOS and KDE",
        ))
        .build();
    appearance_group.add(&compact_ui);

    // Automatic compact — engage compact chrome only while the window is small.
    let compact_auto = adw::SwitchRow::builder()
        .title(i18n("Compact automatically on small windows"))
        .subtitle(i18n(
            "Switch to compact chrome when the window becomes short or narrow, and back when it grows",
        ))
        .build();
    appearance_group.add(&compact_auto);

    // Live preview: both toggles feed the same recompute so the effect is
    // visible before the dialog is saved.
    {
        let auto = compact_auto.clone();
        compact_ui.connect_active_notify(move |row| {
            crate::app::set_compact_prefs(row.is_active(), auto.is_active());
        });
    }
    {
        let manual = compact_ui.clone();
        compact_auto.connect_active_notify(move |row| {
            crate::app::set_compact_prefs(manual.is_active(), row.is_active());
        });
    }

    // Send terminal control shortcuts to the session toggle — when on, the
    // focus-based accelerator suspend is active (single-Ctrl chords reach the
    // focused terminal/viewer instead of the app); when off, accelerators stay
    // always-active (the old behavior). Independent of the global passthrough.
    let terminal_passthrough_ctrl = adw::SwitchRow::builder()
        .title(i18n("Send terminal control shortcuts to the session"))
        .subtitle(i18n(
            "While a terminal or remote viewer is focused, let Ctrl+F/P/N and similar chords reach the session instead of the app",
        ))
        .build();
    appearance_group.add(&terminal_passthrough_ctrl);

    page.add(&appearance_group);

    // === Window Group ===
    let window_group = adw::PreferencesGroup::builder()
        .title(i18n("Window"))
        .build();

    let remember_geometry = adw::SwitchRow::builder()
        .title(i18n("Remember size"))
        .subtitle(i18n("Restore window geometry on startup"))
        .build();
    window_group.add(&remember_geometry);

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
    startup_action_row.set_activatable_widget(Some(&startup_action_dropdown));
    startup_group.add(&startup_action_row);

    page.add(&startup_group);

    // === System Tray Group (collapsible) ===
    let tray_group = adw::PreferencesGroup::builder().build();

    let tray_expander = adw::ExpanderRow::builder()
        .title(i18n("System Tray"))
        .subtitle(i18n("Requires desktop environment with tray support"))
        .show_enable_switch(false)
        .build();

    let enable_tray_icon = adw::SwitchRow::builder()
        .title(i18n("Show icon"))
        .subtitle(i18n("Display icon in system tray"))
        .build();
    tray_expander.add_row(&enable_tray_icon);

    let minimize_to_tray = adw::SwitchRow::builder()
        .title(i18n("Minimize to tray"))
        .subtitle(i18n("Hide window instead of closing"))
        .build();
    tray_expander.add_row(&minimize_to_tray);

    // Make minimize_to_tray sensitive based on enable_tray_icon
    let minimize_to_tray_clone = minimize_to_tray.clone();
    enable_tray_icon.connect_active_notify(move |row| {
        minimize_to_tray_clone.set_sensitive(row.is_active());
    });

    tray_group.add(&tray_expander);
    page.add(&tray_group);

    // === Session Restore Group (collapsible) ===
    let session_group = adw::PreferencesGroup::builder().build();

    let session_expander = adw::ExpanderRow::builder()
        .title(i18n("Session Restore"))
        .subtitle(i18n("Restore previous connections on startup"))
        .show_enable_switch(false)
        .build();

    let session_restore_enabled = adw::SwitchRow::builder()
        .title(i18n("Enabled"))
        .subtitle(i18n("Reconnect to previous sessions on startup"))
        .build();
    session_expander.add_row(&session_restore_enabled);

    let prompt_on_restore = adw::SwitchRow::builder()
        .title(i18n("Ask first"))
        .subtitle(i18n("Prompt before restoring sessions"))
        .build();
    session_expander.add_row(&prompt_on_restore);

    let max_age_row = adw::SpinRow::builder()
        .title(i18n("Max age"))
        .subtitle(i18n("Hours before sessions expire"))
        .adjustment(&gtk4::Adjustment::new(24.0, 1.0, 168.0, 1.0, 24.0, 0.0))
        .build();
    session_expander.add_row(&max_age_row);

    // Make session options sensitive based on session_restore_enabled
    let prompt_on_restore_clone = prompt_on_restore.clone();
    let max_age_row_clone = max_age_row.clone();
    session_restore_enabled.connect_active_notify(move |row| {
        let active = row.is_active();
        prompt_on_restore_clone.set_sensitive(active);
        max_age_row_clone.set_sensitive(active);
    });

    session_group.add(&session_expander);
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
        color_tabs_by_protocol,
        show_protocol_filters,
        sidebar_width_row,
        compact_ui,
        compact_auto,
        terminal_passthrough_ctrl,
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
#[expect(
    clippy::too_many_arguments,
    reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
)]
pub fn load_ui_settings(
    color_scheme_box: &GtkBox,
    language_dropdown: &DropDown,
    remember_geometry: &adw::SwitchRow,
    enable_tray_icon: &adw::SwitchRow,
    minimize_to_tray: &adw::SwitchRow,
    session_restore_enabled: &adw::SwitchRow,
    prompt_on_restore: &adw::SwitchRow,
    max_age_row: &adw::SpinRow,
    startup_action_dropdown: &DropDown,
    color_tabs_by_protocol: &adw::SwitchRow,
    show_protocol_filters: &adw::SwitchRow,
    sidebar_width_row: &adw::SpinRow,
    compact_ui: &adw::SwitchRow,
    compact_auto: &adw::SwitchRow,
    terminal_passthrough_ctrl: &adw::SwitchRow,
    settings: &UiSettings,
    connections: &[&Connection],
) {
    let target_index = match settings.color_scheme {
        ColorScheme::System => 0,
        ColorScheme::Light => 1,
        ColorScheme::Dark => 2,
    };

    // Set the color scheme toggle to the saved value
    #[cfg(feature = "adw-1-7")]
    {
        // The wrapper GtkBox holds the AdwToggleGroup as its only child.
        // Syncing it keeps the segmented control in step with the saved scheme.
        if let Some(child) = color_scheme_box.first_child()
            && let Ok(toggle_group) = child.downcast::<adw::ToggleGroup>()
        {
            toggle_group.set_active(u32::try_from(target_index).unwrap_or(0));
        }
    }
    #[cfg(not(feature = "adw-1-7"))]
    {
        // The color_scheme_box is the linked ToggleButton container
        let mut child = color_scheme_box.first_child();
        let mut i = 0;
        while let Some(widget) = child {
            if i == target_index {
                if let Ok(btn) = widget.downcast::<gtk4::ToggleButton>() {
                    btn.set_active(true);
                }
                break;
            }
            child = widget.next_sibling();
            i += 1;
        }
    }
    crate::app::apply_color_scheme(settings.color_scheme);

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

    color_tabs_by_protocol.set_active(settings.color_tabs_by_protocol);

    show_protocol_filters.set_active(settings.show_protocol_filters);

    // Load sidebar width (default 320 if not set)
    let sidebar_w = settings.sidebar_width.unwrap_or(320);
    sidebar_width_row.set_value(f64::from(sidebar_w.clamp(180, 500)));

    // Load compact interface — apply CSS immediately so it persists at startup.
    compact_ui.set_active(settings.compact_ui);
    compact_auto.set_active(settings.compact_auto);
    crate::app::set_compact_prefs(settings.compact_ui, settings.compact_auto);

    terminal_passthrough_ctrl.set_active(settings.terminal_passthrough_ctrl);

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
        StartupAction::None | StartupAction::RdpFile(_) | StartupAction::VvFile(_) => 0,
        StartupAction::LocalShell => 1,
        StartupAction::Connection(id) => entries
            .iter()
            .position(|e| e.id == *id)
            .map_or(0, |pos| pos + 2),
    };
    #[expect(
        clippy::cast_possible_truncation,
        reason = "value range fits the target type by construction in this code path"
    )]
    startup_action_dropdown.set_selected(selected as u32);
}

/// Collects UI settings from UI controls
#[expect(
    clippy::too_many_arguments,
    reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
)]
pub fn collect_ui_settings(
    color_scheme_box: &GtkBox,
    language_dropdown: &DropDown,
    remember_geometry: &adw::SwitchRow,
    enable_tray_icon: &adw::SwitchRow,
    minimize_to_tray: &adw::SwitchRow,
    session_restore_enabled: &adw::SwitchRow,
    prompt_on_restore: &adw::SwitchRow,
    max_age_row: &adw::SpinRow,
    startup_action_dropdown: &DropDown,
    color_tabs_by_protocol: &adw::SwitchRow,
    show_protocol_filters: &adw::SwitchRow,
    sidebar_width_row: &adw::SpinRow,
    compact_ui: &adw::SwitchRow,
    compact_auto: &adw::SwitchRow,
    terminal_passthrough_ctrl: &adw::SwitchRow,
    connections: &[&Connection],
) -> UiSettings {
    let mut selected_scheme = ColorScheme::System;
    // Read color scheme from StyleManager (the widget applies changes live)
    let _ = color_scheme_box; // marker only
    let style_manager = adw::StyleManager::default();
    if style_manager.color_scheme() == adw::ColorScheme::ForceLight {
        selected_scheme = ColorScheme::Light;
    } else if style_manager.color_scheme() == adw::ColorScheme::ForceDark {
        selected_scheme = ColorScheme::Dark;
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
        window_maximized: false,
        sidebar_width: Some(sidebar_width_row.value().clamp(180.0, 500.0) as i32),
        enable_tray_icon: enable_tray_icon.is_active(),
        minimize_to_tray: minimize_to_tray.is_active(),
        expanded_groups: std::collections::HashSet::new(),
        session_restore: SessionRestoreSettings {
            enabled: session_restore_enabled.is_active(),
            prompt_on_restore: prompt_on_restore.is_active(),
            #[expect(
                clippy::cast_sign_loss,
                reason = "value is non-negative by construction in this code path"
            )]
            max_age_hours: max_age_row.value().max(0.0) as u32,
            saved_sessions: Vec::new(),
        },
        search_history: Vec::new(), // Preserve existing history from current settings
        startup_action,
        color_tabs_by_protocol: color_tabs_by_protocol.is_active(),
        show_protocol_filters: show_protocol_filters.is_active(),
        show_smart_folders: false, // Preserved via toggle button, not settings dialog
        compact_ui: compact_ui.is_active(),
        compact_auto: compact_auto.is_active(),
        terminal_passthrough_ctrl: terminal_passthrough_ctrl.is_active(),
    }
}
