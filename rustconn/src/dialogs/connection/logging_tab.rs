//! Logging tab for the connection dialog
//!
//! Contains the `LoggingTab` struct that owns all logging-related widgets
//! and provides `set`/`build` methods for `LogConfig`.

use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, CheckButton, DropDown, Entry, Orientation, ScrolledWindow, SpinButton,
    StringList,
};
use libadwaita as adw;
use libadwaita::prelude::*;
use rustconn_core::session::LogConfig;

/// Timestamp format options matching the dropdown order
const TIMESTAMP_FORMATS: [&str; 5] = [
    "%Y-%m-%d %H:%M:%S",
    "%H:%M:%S",
    "%Y-%m-%d %H:%M:%S%.3f",
    "[%Y-%m-%d %H:%M:%S]",
    "%d/%m/%Y %H:%M:%S",
];

/// Logging tab widget group
#[allow(dead_code)] // Fields kept for GTK widget lifecycle
pub struct LoggingTab {
    pub enabled_check: CheckButton,
    pub path_entry: Entry,
    pub timestamp_dropdown: DropDown,
    pub max_size_spin: SpinButton,
    pub retention_spin: SpinButton,
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
            .title("Session Logging")
            .description("Record terminal output to files")
            .build();

        let enabled_check = CheckButton::builder().valign(gtk4::Align::Center).build();

        let enable_row = adw::ActionRow::builder()
            .title("Enable Logging")
            .subtitle("Record session output to log files")
            .activatable_widget(&enabled_check)
            .build();
        enable_row.add_suffix(&enabled_check);
        enable_group.add(&enable_row);
        content.append(&enable_group);

        // Log settings group
        let settings_group = adw::PreferencesGroup::builder()
            .title("Log Settings")
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
            .title("Log Path")
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
            .title("Timestamp Format")
            .subtitle("Format for timestamps in log entries")
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
            .title("Max Size (MB)")
            .subtitle("Maximum log file size (0 = no limit)")
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
            .title("Retention (days)")
            .subtitle("Days to keep old log files (0 = keep forever)")
            .build();
        retention_row.add_suffix(&retention_spin);
        settings_group.add(&retention_row);

        content.append(&settings_group);

        // Wire enabled toggle
        let path_clone = path_entry.clone();
        let ts_clone = timestamp_dropdown.clone();
        let size_clone = max_size_spin.clone();
        let ret_clone = retention_spin.clone();
        let sg_clone = settings_group.clone();
        enabled_check.connect_toggled(move |check| {
            let on = check.is_active();
            path_clone.set_sensitive(on);
            ts_clone.set_sensitive(on);
            size_clone.set_sensitive(on);
            ret_clone.set_sensitive(on);
            sg_clone.set_sensitive(on);
        });
        settings_group.set_sensitive(false);

        clamp.set_child(Some(&content));
        scrolled.set_child(Some(&clamp));

        let vbox = GtkBox::new(Orientation::Vertical, 0);
        vbox.append(&scrolled);

        let tab = Self {
            enabled_check,
            path_entry,
            timestamp_dropdown,
            max_size_spin,
            retention_spin,
        };
        (vbox, tab)
    }

    /// Populates widgets from a `LogConfig`
    pub fn set(&self, config: Option<&LogConfig>) {
        if let Some(c) = config {
            self.enabled_check.set_active(c.enabled);
            self.path_entry.set_text(&c.path_template);

            let idx = TIMESTAMP_FORMATS
                .iter()
                .position(|&f| f == c.timestamp_format)
                .unwrap_or(0);
            self.timestamp_dropdown.set_selected(idx as u32);
            self.max_size_spin.set_value(f64::from(c.max_size_mb));
            self.retention_spin.set_value(f64::from(c.retention_days));

            let on = c.enabled;
            self.path_entry.set_sensitive(on);
            self.timestamp_dropdown.set_sensitive(on);
            self.max_size_spin.set_sensitive(on);
            self.retention_spin.set_sensitive(on);
        } else {
            self.enabled_check.set_active(false);
            self.path_entry.set_text("");
            self.timestamp_dropdown.set_selected(0);
            self.max_size_spin.set_value(10.0);
            self.retention_spin.set_value(30.0);

            self.path_entry.set_sensitive(false);
            self.timestamp_dropdown.set_sensitive(false);
            self.max_size_spin.set_sensitive(false);
            self.retention_spin.set_sensitive(false);
        }
    }

    /// Builds a `LogConfig` from current widget state
    #[must_use]
    pub fn build(&self) -> Option<LogConfig> {
        if !self.enabled_check.is_active() {
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

        #[allow(clippy::cast_sign_loss)]
        let max_size_mb = self.max_size_spin.value() as u32;
        #[allow(clippy::cast_sign_loss)]
        let retention_days = self.retention_spin.value() as u32;

        Some(LogConfig {
            enabled: true,
            path_template,
            timestamp_format,
            max_size_mb,
            retention_days,
            log_activity: true,
            log_input: false,
            log_output: false,
            log_timestamps: false,
        })
    }
}
