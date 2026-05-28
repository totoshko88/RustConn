//! Logging settings tab using libadwaita components

use gtk4::prelude::*;
use gtk4::{Entry, SpinButton};
use libadwaita as adw;
use rustconn_core::config::LoggingSettings;

use crate::i18n::i18n;

/// Creates the logging settings widgets (not added to any page).
///
/// Returns the individual widgets for embedding into a collapsible expander
/// on the Terminal page.
#[allow(
    clippy::type_complexity,
    reason = "internal helper signature documents the exact tuple layout used by the caller; aliasing would obscure the data flow"
)]
pub fn create_logging_page() -> (
    adw::PreferencesPage,
    adw::SwitchRow,
    Entry,
    SpinButton,
    adw::SwitchRow,
    adw::SwitchRow,
    adw::SwitchRow,
    adw::SwitchRow, // log_timestamps
) {
    // We still create a dummy page for API compatibility, but the widgets
    // are NOT added to it — they will be placed into an ExpanderRow by mod.rs.
    let page = adw::PreferencesPage::builder()
        .title(i18n("Logging"))
        .icon_name("document-open-recent-symbolic")
        .build();

    // Enable logging switch
    let logging_enabled_row = adw::SwitchRow::builder()
        .title(i18n("Persist logs"))
        .subtitle(i18n("Save session logs to disk"))
        .build();

    // Log directory
    let log_dir_entry = Entry::builder()
        .text("logs")
        .placeholder_text(i18n("Relative to config dir or absolute path"))
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .sensitive(false)
        .build();

    // Retention days
    let retention_adj = gtk4::Adjustment::new(30.0, 1.0, 365.0, 1.0, 7.0, 0.0);
    let retention_spin = SpinButton::builder()
        .adjustment(&retention_adj)
        .climb_rate(1.0)
        .digits(0)
        .valign(gtk4::Align::Center)
        .sensitive(false)
        .build();

    // Activity logging
    let log_activity_row = adw::SwitchRow::builder()
        .title(i18n("Activity"))
        .subtitle(i18n("Connection events and change counts"))
        .active(true)
        .sensitive(false)
        .build();

    // Input logging
    let log_input_row = adw::SwitchRow::builder()
        .title(i18n("User Input"))
        .subtitle(i18n("Commands and keystrokes"))
        .sensitive(false)
        .build();

    // Output logging
    let log_output_row = adw::SwitchRow::builder()
        .title(i18n("Terminal Output"))
        .subtitle(i18n("Full session transcript"))
        .sensitive(false)
        .build();

    // Timestamps
    let log_timestamps_row = adw::SwitchRow::builder()
        .title(i18n("Timestamps"))
        .subtitle(i18n("Prepend [HH:MM:SS] to each line in session logs"))
        .build();

    // Connect switch to enable/disable other controls
    let log_dir_entry_clone = log_dir_entry.clone();
    let retention_clone = retention_spin.clone();
    let log_activity_clone = log_activity_row.clone();
    let log_input_clone = log_input_row.clone();
    let log_output_clone = log_output_row.clone();
    logging_enabled_row.connect_active_notify(move |row| {
        let state = row.is_active();
        log_dir_entry_clone.set_sensitive(state);
        retention_clone.set_sensitive(state);
        log_activity_clone.set_sensitive(state);
        log_input_clone.set_sensitive(state);
        log_output_clone.set_sensitive(state);
    });

    (
        page,
        logging_enabled_row,
        log_dir_entry,
        retention_spin,
        log_activity_row,
        log_input_row,
        log_output_row,
        log_timestamps_row,
    )
}

/// Loads logging settings into UI controls
#[expect(
    clippy::too_many_arguments,
    reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
)]
pub fn load_logging_settings(
    logging_enabled_row: &adw::SwitchRow,
    log_dir_entry: &Entry,
    retention_spin: &SpinButton,
    log_activity_row: &adw::SwitchRow,
    log_input_row: &adw::SwitchRow,
    log_output_row: &adw::SwitchRow,
    log_timestamps_row: &adw::SwitchRow,
    settings: &LoggingSettings,
    log_timestamps: bool,
) {
    logging_enabled_row.set_active(settings.enabled);
    log_dir_entry.set_text(&settings.log_directory.display().to_string());
    retention_spin.set_value(f64::from(settings.retention_days));
    log_activity_row.set_active(settings.log_activity);
    log_input_row.set_active(settings.log_input);
    log_output_row.set_active(settings.log_output);
    log_timestamps_row.set_active(log_timestamps);

    // Update sensitivity based on enabled state
    let enabled = settings.enabled;
    log_dir_entry.set_sensitive(enabled);
    retention_spin.set_sensitive(enabled);
    log_activity_row.set_sensitive(enabled);
    log_input_row.set_sensitive(enabled);
    log_output_row.set_sensitive(enabled);
}

/// Collects logging settings from UI controls
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "value range fits the target type and is non-negative by construction in this code path"
)]
pub fn collect_logging_settings(
    logging_enabled_row: &adw::SwitchRow,
    log_dir_entry: &Entry,
    retention_spin: &SpinButton,
    log_activity_row: &adw::SwitchRow,
    log_input_row: &adw::SwitchRow,
    log_output_row: &adw::SwitchRow,
) -> LoggingSettings {
    LoggingSettings {
        enabled: logging_enabled_row.is_active(),
        log_directory: std::path::PathBuf::from(log_dir_entry.text().as_str()),
        retention_days: retention_spin.value() as u32,
        log_activity: log_activity_row.is_active(),
        log_input: log_input_row.is_active(),
        log_output: log_output_row.is_active(),
    }
}
