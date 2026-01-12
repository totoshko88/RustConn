//! Logging settings tab using libadwaita components

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Button, CheckButton, Entry, SpinButton, Switch};
use libadwaita as adw;
use rustconn_core::config::LoggingSettings;
use std::path::PathBuf;

/// Creates the logging settings page using AdwPreferencesPage
#[allow(clippy::type_complexity)]
pub fn create_logging_page() -> (
    adw::PreferencesPage,
    Switch,
    Entry,
    SpinButton,
    CheckButton,
    CheckButton,
    CheckButton,
) {
    let page = adw::PreferencesPage::builder()
        .title("Logging")
        .icon_name("document-open-recent-symbolic")
        .build();

    // === General Logging Settings ===
    let general_group = adw::PreferencesGroup::builder()
        .title("General")
        .description("Configure session logging")
        .build();

    // Enable logging switch
    let logging_enabled_switch = Switch::builder().valign(gtk4::Align::Center).build();
    let enable_row = adw::ActionRow::builder()
        .title("Persist logs")
        .subtitle("Save session logs to disk")
        .build();
    enable_row.add_suffix(&logging_enabled_switch);
    enable_row.set_activatable_widget(Some(&logging_enabled_switch));
    general_group.add(&enable_row);

    // Log directory
    let log_dir_entry = Entry::builder()
        .text("logs")
        .placeholder_text("Relative to config dir or absolute path")
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .sensitive(false)
        .build();
    let log_dir_row = adw::ActionRow::builder().title("Directory").build();
    log_dir_row.add_suffix(&log_dir_entry);
    log_dir_row.set_activatable_widget(Some(&log_dir_entry));
    general_group.add(&log_dir_row);

    // Retention days
    let retention_adj = gtk4::Adjustment::new(30.0, 1.0, 365.0, 1.0, 7.0, 0.0);
    let retention_spin = SpinButton::builder()
        .adjustment(&retention_adj)
        .climb_rate(1.0)
        .digits(0)
        .valign(gtk4::Align::Center)
        .sensitive(false)
        .build();
    let retention_row = adw::ActionRow::builder()
        .title("Retention")
        .subtitle("Days to keep logs")
        .build();
    retention_row.add_suffix(&retention_spin);
    retention_row.set_activatable_widget(Some(&retention_spin));
    general_group.add(&retention_row);

    // Open logs directory button
    let open_logs_btn = Button::builder()
        .icon_name("folder-open-symbolic")
        .valign(gtk4::Align::Center)
        .tooltip_text("Open logs directory")
        .sensitive(false)
        .build();
    let open_logs_row = adw::ActionRow::builder()
        .title("Open Logs Directory")
        .activatable(true)
        .build();
    open_logs_row.add_suffix(&open_logs_btn);

    let log_dir_entry_clone = log_dir_entry.clone();
    open_logs_row.connect_activated(move |_| {
        open_logs_directory(&log_dir_entry_clone);
    });

    let log_dir_entry_clone2 = log_dir_entry.clone();
    open_logs_btn.connect_clicked(move |_| {
        open_logs_directory(&log_dir_entry_clone2);
    });

    general_group.add(&open_logs_row);
    page.add(&general_group);

    // === Log Content Group ===
    let content_group = adw::PreferencesGroup::builder()
        .title("Log Content")
        .description("Select what to include in logs")
        .build();

    // Activity logging
    let log_activity_check = CheckButton::builder()
        .active(true)
        .valign(gtk4::Align::Center)
        .sensitive(false)
        .build();
    let log_activity_row = adw::ActionRow::builder()
        .title("Activity")
        .subtitle("Connection events and change counts")
        .activatable_widget(&log_activity_check)
        .build();
    log_activity_row.add_prefix(&log_activity_check);
    content_group.add(&log_activity_row);

    // Input logging
    let log_input_check = CheckButton::builder()
        .valign(gtk4::Align::Center)
        .sensitive(false)
        .build();
    let log_input_row = adw::ActionRow::builder()
        .title("User Input")
        .subtitle("Commands and keystrokes")
        .activatable_widget(&log_input_check)
        .build();
    log_input_row.add_prefix(&log_input_check);
    content_group.add(&log_input_row);

    // Output logging
    let log_output_check = CheckButton::builder()
        .valign(gtk4::Align::Center)
        .sensitive(false)
        .build();
    let log_output_row = adw::ActionRow::builder()
        .title("Terminal Output")
        .subtitle("Full session transcript")
        .activatable_widget(&log_output_check)
        .build();
    log_output_row.add_prefix(&log_output_check);
    content_group.add(&log_output_row);

    page.add(&content_group);

    // Connect switch to enable/disable other controls
    let log_dir_entry_clone = log_dir_entry.clone();
    let retention_clone = retention_spin.clone();
    let open_logs_btn_clone = open_logs_btn.clone();
    let log_activity_clone = log_activity_check.clone();
    let log_input_clone = log_input_check.clone();
    let log_output_clone = log_output_check.clone();
    logging_enabled_switch.connect_state_set(move |_, state| {
        log_dir_entry_clone.set_sensitive(state);
        retention_clone.set_sensitive(state);
        open_logs_btn_clone.set_sensitive(state);
        log_activity_clone.set_sensitive(state);
        log_input_clone.set_sensitive(state);
        log_output_clone.set_sensitive(state);
        gtk4::glib::Propagation::Proceed
    });

    (
        page,
        logging_enabled_switch,
        log_dir_entry,
        retention_spin,
        log_activity_check,
        log_input_check,
        log_output_check,
    )
}

/// Loads logging settings into UI controls
pub fn load_logging_settings(
    logging_enabled_switch: &Switch,
    log_dir_entry: &Entry,
    retention_spin: &SpinButton,
    log_activity_check: &CheckButton,
    log_input_check: &CheckButton,
    log_output_check: &CheckButton,
    settings: &LoggingSettings,
) {
    logging_enabled_switch.set_active(settings.enabled);
    log_dir_entry.set_text(&settings.log_directory.display().to_string());
    retention_spin.set_value(f64::from(settings.retention_days));
    log_activity_check.set_active(settings.log_activity);
    log_input_check.set_active(settings.log_input);
    log_output_check.set_active(settings.log_output);

    // Update sensitivity based on enabled state
    let enabled = settings.enabled;
    log_dir_entry.set_sensitive(enabled);
    retention_spin.set_sensitive(enabled);
    log_activity_check.set_sensitive(enabled);
    log_input_check.set_sensitive(enabled);
    log_output_check.set_sensitive(enabled);
}

/// Collects logging settings from UI controls
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn collect_logging_settings(
    logging_enabled_switch: &Switch,
    log_dir_entry: &Entry,
    retention_spin: &SpinButton,
    log_activity_check: &CheckButton,
    log_input_check: &CheckButton,
    log_output_check: &CheckButton,
) -> LoggingSettings {
    LoggingSettings {
        enabled: logging_enabled_switch.is_active(),
        log_directory: std::path::PathBuf::from(log_dir_entry.text().as_str()),
        retention_days: retention_spin.value() as u32,
        log_activity: log_activity_check.is_active(),
        log_input: log_input_check.is_active(),
        log_output: log_output_check.is_active(),
    }
}

/// Opens the logs directory asynchronously
///
/// Creates the directory if it doesn't exist, then opens it in the file manager.
/// Uses background thread to avoid blocking UI on slow filesystems.
fn open_logs_directory(log_dir_entry: &Entry) {
    let log_dir = log_dir_entry.text().to_string();
    let log_path = if log_dir.starts_with('/') {
        PathBuf::from(&log_dir)
    } else {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("rustconn")
            .join(&log_dir)
    };

    // Run directory creation in background to avoid blocking UI
    crate::utils::spawn_blocking_with_callback(
        move || {
            if !log_path.exists() {
                if let Err(e) = std::fs::create_dir_all(&log_path) {
                    return Err(format!("Failed to create logs directory: {e}"));
                }
            }
            // open::that spawns a process and returns immediately, so it's safe here
            if let Err(e) = open::that(&log_path) {
                return Err(format!("Failed to open logs directory: {e}"));
            }
            Ok(())
        },
        |result: Result<(), String>| {
            if let Err(e) = result {
                tracing::error!("{e}");
            }
        },
    );
}
