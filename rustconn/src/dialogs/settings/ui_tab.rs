//! UI settings tab

use gtk4::prelude::*;
use gtk4::{Box as GtkBox, CheckButton, Frame, Orientation, SpinButton};
use rustconn_core::config::{SessionRestoreSettings, UiSettings};

/// Creates the UI settings tab
#[allow(clippy::type_complexity)]
pub fn create_ui_tab() -> (
    Frame,
    CheckButton,
    CheckButton,
    CheckButton,
    CheckButton,
    CheckButton,
    SpinButton,
) {
    let main_frame = Frame::builder()
        .label("Interface Settings")
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .valign(gtk4::Align::Start)
        .build();

    let main_vbox = GtkBox::new(Orientation::Vertical, 12);
    main_vbox.set_margin_top(12);
    main_vbox.set_margin_bottom(12);
    main_vbox.set_margin_start(12);
    main_vbox.set_margin_end(12);

    // Window settings
    let window_frame = Frame::builder().label("Window").margin_bottom(12).build();

    let window_vbox = GtkBox::new(Orientation::Vertical, 6);
    window_vbox.set_margin_top(6);
    window_vbox.set_margin_bottom(6);
    window_vbox.set_margin_start(6);
    window_vbox.set_margin_end(6);

    let remember_geometry = CheckButton::with_label("Remember window geometry");
    window_vbox.append(&remember_geometry);

    window_frame.set_child(Some(&window_vbox));

    // Tray settings
    let tray_frame = Frame::builder()
        .label("System Tray")
        .margin_bottom(12)
        .build();

    let tray_vbox = GtkBox::new(Orientation::Vertical, 6);
    tray_vbox.set_margin_top(6);
    tray_vbox.set_margin_bottom(6);
    tray_vbox.set_margin_start(6);
    tray_vbox.set_margin_end(6);

    let enable_tray_icon = CheckButton::with_label("Enable tray icon");
    let minimize_to_tray = CheckButton::with_label("Minimize to tray instead of closing");

    tray_vbox.append(&enable_tray_icon);
    tray_vbox.append(&minimize_to_tray);

    tray_frame.set_child(Some(&tray_vbox));

    // Session restore settings
    let session_frame = Frame::builder()
        .label("Session Restore")
        .margin_bottom(12)
        .build();

    let session_vbox = GtkBox::new(Orientation::Vertical, 6);
    session_vbox.set_margin_top(6);
    session_vbox.set_margin_bottom(6);
    session_vbox.set_margin_start(6);
    session_vbox.set_margin_end(6);

    let session_restore_enabled = CheckButton::with_label("Restore sessions on startup");
    let prompt_on_restore = CheckButton::with_label("Prompt before restoring sessions");

    let max_age_hbox = GtkBox::new(Orientation::Horizontal, 6);
    max_age_hbox.append(&gtk4::Label::new(Some("Maximum session age (hours):")));
    let max_age_spin = SpinButton::new(
        Some(&gtk4::Adjustment::new(24.0, 1.0, 168.0, 1.0, 24.0, 0.0)),
        1.0,
        0,
    );
    max_age_hbox.append(&max_age_spin);

    session_vbox.append(&session_restore_enabled);
    session_vbox.append(&prompt_on_restore);
    session_vbox.append(&max_age_hbox);

    session_frame.set_child(Some(&session_vbox));

    main_vbox.append(&window_frame);
    main_vbox.append(&tray_frame);
    main_vbox.append(&session_frame);

    main_frame.set_child(Some(&main_vbox));

    (
        main_frame,
        remember_geometry,
        enable_tray_icon,
        minimize_to_tray,
        session_restore_enabled,
        prompt_on_restore,
        max_age_spin,
    )
}

/// Loads UI settings into UI controls
pub fn load_ui_settings(
    remember_geometry: &CheckButton,
    enable_tray_icon: &CheckButton,
    minimize_to_tray: &CheckButton,
    session_restore_enabled: &CheckButton,
    prompt_on_restore: &CheckButton,
    max_age_spin: &SpinButton,
    settings: &UiSettings,
) {
    remember_geometry.set_active(settings.remember_window_geometry);
    enable_tray_icon.set_active(settings.enable_tray_icon);
    minimize_to_tray.set_active(settings.minimize_to_tray);

    session_restore_enabled.set_active(settings.session_restore.enabled);
    prompt_on_restore.set_active(settings.session_restore.prompt_on_restore);
    max_age_spin.set_value(f64::from(settings.session_restore.max_age_hours));
}

/// Collects UI settings from UI controls
pub fn collect_ui_settings(
    remember_geometry: &CheckButton,
    enable_tray_icon: &CheckButton,
    minimize_to_tray: &CheckButton,
    session_restore_enabled: &CheckButton,
    prompt_on_restore: &CheckButton,
    max_age_spin: &SpinButton,
) -> UiSettings {
    UiSettings {
        remember_window_geometry: remember_geometry.is_active(),
        window_width: None, // These are managed by the window manager
        window_height: None,
        sidebar_width: None,
        enable_tray_icon: enable_tray_icon.is_active(),
        minimize_to_tray: minimize_to_tray.is_active(),
        expanded_groups: std::collections::HashSet::new(), // Managed separately
        session_restore: SessionRestoreSettings {
            enabled: session_restore_enabled.is_active(),
            prompt_on_restore: prompt_on_restore.is_active(),
            #[allow(clippy::cast_sign_loss)]
            max_age_hours: max_age_spin.value().max(0.0) as u32,
            saved_sessions: Vec::new(), // Managed separately
        },
    }
}
