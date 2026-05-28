//! Logging tab for the connection dialog
//!
//! Contains the `LoggingTab` struct that owns all logging-related widgets
//! and provides `set`/`build` methods for `LogConfig`.

use gtk4::prelude::*;
use gtk4::{Box as GtkBox, DropDown, Entry, Orientation, ScrolledWindow, SpinButton, StringList};
use libadwaita as adw;
use libadwaita::prelude::*;
use rustconn_core::session::LogConfig;

use crate::i18n::i18n;

/// Timestamp format options matching the dropdown order
const TIMESTAMP_FORMATS: [&str; 5] = [
    "%Y-%m-%d %H:%M:%S",
    "%H:%M:%S",
    "%Y-%m-%d %H:%M:%S%.3f",
    "[%Y-%m-%d %H:%M:%S]",
    "%d/%m/%Y %H:%M:%S",
];

/// Logging tab widget group
#[allow(dead_code, reason = "Fields kept for GTK widget lifecycle")]
pub struct LoggingTab {
    pub enabled_switch: adw::SwitchRow,
    pub path_entry: Entry,
    pub timestamp_dropdown: DropDown,
    pub max_size_spin: SpinButton,
    pub retention_spin: SpinButton,
    pub log_activity_switch: adw::SwitchRow,
    pub log_input_switch: adw::SwitchRow,
    pub log_output_switch: adw::SwitchRow,
    pub log_timestamps_switch: adw::SwitchRow,
}

impl LoggingTab {
    /// Creates the logging tab UI and returns (container, tab)
    #[must_use]
    pub fn new() -> (GtkBox, Self) {
        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .vexpand(true)
            .build();

        let clamp = adw::Clamp::builder()
            .maximum_size(600)
            .tightening_threshold(400)
            .build();

        let content = GtkBox::new(Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        // Enable logging group
        let enable_group = adw::PreferencesGroup::builder()
            .title(i18n("Session Logging"))
            .description(i18n("Record terminal output to files"))
            .build();

        let enabled_switch = adw::SwitchRow::builder()
            .title(i18n("Enable Logging"))
            .subtitle(i18n("Record session output to log files"))
            .active(false)
            .build();
        enable_group.add(&enabled_switch);
        content.append(&enable_group);

        // Log settings group
        let settings_group = adw::PreferencesGroup::builder()
            .title(i18n("Log Settings"))
            .build();

        let path_entry = Entry::builder()
            .hexpand(true)
            .valign(gtk4::Align::Center)
            .placeholder_text(
                "${HOME}/.local/share/rustconn/logs/\
                 ${connection_name}_${date}.log",
            )
            .sensitive(false)
            .build();

        let path_row = adw::ActionRow::builder()
            .title(i18n("Log Path"))
            .subtitle(
                "Variables: ${connection_name}, ${protocol}, \
                 ${date}, ${time}, ${datetime}, ${HOME}",
            )
            .build();
        path_row.add_suffix(&path_entry);
        settings_group.add(&path_row);

        let timestamp_list = StringList::new(&TIMESTAMP_FORMATS);
        let timestamp_dropdown = DropDown::new(Some(timestamp_list), gtk4::Expression::NONE);
        timestamp_dropdown.set_selected(0);
        timestamp_dropdown.set_valign(gtk4::Align::Center);
        timestamp_dropdown.set_sensitive(false);

        let timestamp_row = adw::ActionRow::builder()
            .title(i18n("Timestamp Format"))
            .subtitle(i18n("Format for timestamps in log entries"))
            .build();
        timestamp_row.add_suffix(&timestamp_dropdown);
        settings_group.add(&timestamp_row);

        let size_adj = gtk4::Adjustment::new(10.0, 0.0, 1000.0, 1.0, 10.0, 0.0);
        let max_size_spin = SpinButton::builder()
            .adjustment(&size_adj)
            .climb_rate(1.0)
            .digits(0)
            .valign(gtk4::Align::Center)
            .sensitive(false)
            .build();

        let size_row = adw::ActionRow::builder()
            .title(i18n("Max Size (MB)"))
            .subtitle(i18n("Maximum log file size (0 = no limit)"))
            .build();
        size_row.add_suffix(&max_size_spin);
        settings_group.add(&size_row);

        let retention_adj = gtk4::Adjustment::new(30.0, 0.0, 365.0, 1.0, 7.0, 0.0);
        let retention_spin = SpinButton::builder()
            .adjustment(&retention_adj)
            .climb_rate(1.0)
            .digits(0)
            .valign(gtk4::Align::Center)
            .sensitive(false)
            .build();

        let retention_row = adw::ActionRow::builder()
            .title(i18n("Retention (days)"))
            .subtitle(i18n("Days to keep old log files (0 = keep forever)"))
            .build();
        retention_row.add_suffix(&retention_spin);
        settings_group.add(&retention_row);

        content.append(&settings_group);

        // === Content Options Group ===
        let content_group = adw::PreferencesGroup::builder()
            .title(i18n("Content Options"))
            .description(i18n("What to include in log files"))
            .sensitive(false)
            .build();

        let log_activity_switch = adw::SwitchRow::builder()
            .title(i18n("Log Activity"))
            .subtitle(i18n("Record connection and disconnection events"))
            .active(true)
            .sensitive(false)
            .build();
        content_group.add(&log_activity_switch);

        let log_input_switch = adw::SwitchRow::builder()
            .title(i18n("Log Input"))
            .subtitle(i18n("Record keyboard input sent to remote"))
            .active(false)
            .sensitive(false)
            .build();
        content_group.add(&log_input_switch);

        let log_output_switch = adw::SwitchRow::builder()
            .title(i18n("Log Output"))
            .subtitle(i18n("Record terminal output from remote"))
            .active(false)
            .sensitive(false)
            .build();
        content_group.add(&log_output_switch);

        let log_timestamps_switch = adw::SwitchRow::builder()
            .title(i18n("Add Timestamps"))
            .subtitle(i18n("Prepend timestamp to each log line"))
            .active(false)
            .sensitive(false)
            .build();
        content_group.add(&log_timestamps_switch);

        content.append(&content_group);

        // Wire enabled toggle
        let path_clone = path_entry.clone();
        let ts_clone = timestamp_dropdown.clone();
        let size_clone = max_size_spin.clone();
        let ret_clone = retention_spin.clone();
        let sg_clone = settings_group.clone();
        let cg_clone = content_group.clone();
        let activity_clone = log_activity_switch.clone();
        let input_clone = log_input_switch.clone();
        let output_clone = log_output_switch.clone();
        let timestamps_clone = log_timestamps_switch.clone();
        enabled_switch.connect_active_notify(move |switch| {
            let on = switch.is_active();
            path_clone.set_sensitive(on);
            ts_clone.set_sensitive(on);
            size_clone.set_sensitive(on);
            ret_clone.set_sensitive(on);
            sg_clone.set_sensitive(on);
            cg_clone.set_sensitive(on);
            activity_clone.set_sensitive(on);
            input_clone.set_sensitive(on);
            output_clone.set_sensitive(on);
            timestamps_clone.set_sensitive(on);
        });
        settings_group.set_sensitive(false);

        clamp.set_child(Some(&content));
        scrolled.set_child(Some(&clamp));

        let vbox = GtkBox::new(Orientation::Vertical, 0);
        vbox.append(&scrolled);

        let tab = Self {
            enabled_switch,
            path_entry,
            timestamp_dropdown,
            max_size_spin,
            retention_spin,
            log_activity_switch,
            log_input_switch,
            log_output_switch,
            log_timestamps_switch,
        };
        (vbox, tab)
    }

    /// Populates widgets from a `LogConfig`
    pub fn set(&self, config: Option<&LogConfig>) {
        if let Some(c) = config {
            self.enabled_switch.set_active(c.enabled);
            self.path_entry.set_text(&c.path_template);

            let idx = TIMESTAMP_FORMATS
                .iter()
                .position(|&f| f == c.timestamp_format)
                .unwrap_or(0);
            self.timestamp_dropdown.set_selected(idx as u32);
            self.max_size_spin.set_value(f64::from(c.max_size_mb));
            self.retention_spin.set_value(f64::from(c.retention_days));
            self.log_activity_switch.set_active(c.log_activity);
            self.log_input_switch.set_active(c.log_input);
            self.log_output_switch.set_active(c.log_output);
            self.log_timestamps_switch.set_active(c.log_timestamps);

            let on = c.enabled;
            self.path_entry.set_sensitive(on);
            self.timestamp_dropdown.set_sensitive(on);
            self.max_size_spin.set_sensitive(on);
            self.retention_spin.set_sensitive(on);
            self.log_activity_switch.set_sensitive(on);
            self.log_input_switch.set_sensitive(on);
            self.log_output_switch.set_sensitive(on);
            self.log_timestamps_switch.set_sensitive(on);
        } else {
            self.enabled_switch.set_active(false);
            self.path_entry.set_text("");
            self.timestamp_dropdown.set_selected(0);
            self.max_size_spin.set_value(10.0);
            self.retention_spin.set_value(30.0);
            self.log_activity_switch.set_active(true);
            self.log_input_switch.set_active(false);
            self.log_output_switch.set_active(false);
            self.log_timestamps_switch.set_active(false);

            self.path_entry.set_sensitive(false);
            self.timestamp_dropdown.set_sensitive(false);
            self.max_size_spin.set_sensitive(false);
            self.retention_spin.set_sensitive(false);
            self.log_activity_switch.set_sensitive(false);
            self.log_input_switch.set_sensitive(false);
            self.log_output_switch.set_sensitive(false);
            self.log_timestamps_switch.set_sensitive(false);
        }
    }

    /// Builds a `LogConfig` from current widget state
    #[must_use]
    pub fn build(&self) -> Option<LogConfig> {
        if !self.enabled_switch.is_active() {
            return None;
        }

        let path_template = self.path_entry.text().trim().to_string();
        let path_template = if path_template.is_empty() {
            "${HOME}/.local/share/rustconn/logs/\
             ${connection_name}_${date}.log"
                .to_string()
        } else {
            path_template
        };

        let idx = self.timestamp_dropdown.selected() as usize;
        let timestamp_format = TIMESTAMP_FORMATS
            .get(idx)
            .unwrap_or(&TIMESTAMP_FORMATS[0])
            .to_string();

        #[expect(
            clippy::cast_sign_loss,
            reason = "value is non-negative by construction in this code path"
        )]
        let max_size_mb = self.max_size_spin.value() as u32;
        #[expect(
            clippy::cast_sign_loss,
            reason = "value is non-negative by construction in this code path"
        )]
        let retention_days = self.retention_spin.value() as u32;

        Some(LogConfig {
            enabled: true,
            path_template,
            timestamp_format,
            max_size_mb,
            retention_days,
            log_activity: self.log_activity_switch.is_active(),
            log_input: self.log_input_switch.is_active(),
            log_output: self.log_output_switch.is_active(),
            log_timestamps: self.log_timestamps_switch.is_active(),
        })
    }
}
