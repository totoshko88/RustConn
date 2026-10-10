//! Monitoring settings tab using libadwaita components

use adw::prelude::*;
use gtk4::StringList;
use gtk4::prelude::*;
use libadwaita as adw;
use rustconn_core::activity_monitor::ActivityMonitorDefaults;
use rustconn_core::monitoring::MonitoringSettings;

use crate::i18n::i18n;

/// Holds all monitoring settings page widgets
#[derive(Clone)]
pub struct MonitoringPageWidgets {
    /// The preferences page
    pub page: adw::PreferencesPage,
    /// Global enable switch row
    pub enabled_row: adw::SwitchRow,
    /// Polling interval spin row
    pub interval_row: adw::SpinRow,
    /// Opens the "Reset Per-Connection Overrides" confirmation (issue #352).
    ///
    /// Left unwired here: the count and the reset need the connection list,
    /// which this page never sees. `SettingsDialog::connect_monitoring_override_reset`
    /// attaches the handler.
    pub reset_overrides_button: gtk4::Button,
    /// Show CPU usage
    pub show_cpu: adw::SwitchRow,
    /// Show memory usage
    pub show_memory: adw::SwitchRow,
    /// Show disk usage
    pub show_disk: adw::SwitchRow,
    /// Show network throughput
    pub show_network: adw::SwitchRow,
    /// Show load average
    pub show_load: adw::SwitchRow,
    /// Show system info (distro, kernel, uptime)
    pub show_system_info: adw::SwitchRow,
    /// Activity monitor default mode combo
    pub activity_mode_combo: adw::ComboRow,
    /// Activity monitor default quiet period spin
    pub activity_quiet_period_spin: adw::SpinRow,
    /// Activity monitor default silence timeout spin
    pub activity_silence_timeout_spin: adw::SpinRow,
}

impl MonitoringPageWidgets {
    /// Creates the monitoring settings page using `AdwPreferencesPage`
    #[must_use]
    pub fn new() -> Self {
        let page = adw::PreferencesPage::builder()
            .title(i18n("Monitoring"))
            .icon_name("power-profile-performance-symbolic")
            .build();

        // === General Group ===
        let general_group = adw::PreferencesGroup::builder()
            .title(i18n("General"))
            .description(i18n("Remote host metrics collection"))
            .build();

        let enabled_row = adw::SwitchRow::builder()
            .title(i18n("Enable monitoring"))
            .subtitle(i18n(
                "Global switch for all connections (per-connection: Advanced tab in connection settings)",
            ))
            .build();
        general_group.add(&enabled_row);

        let interval_row = adw::SpinRow::builder()
            .title(i18n("Polling interval"))
            .subtitle(i18n("Seconds between metric updates"))
            .adjustment(&gtk4::Adjustment::new(3.0, 1.0, 60.0, 1.0, 5.0, 0.0))
            .sensitive(false)
            .build();
        general_group.add(&interval_row);

        // A connection saved by the editor before 0.22.13 stored an explicit
        // "on" and stopped following the switch above (issue #352). Nothing can
        // tell those apart from a deliberate choice, so the way back is this
        // explicit bulk action rather than a migration.
        let reset_overrides_row = adw::ActionRow::builder()
            .title(i18n("Reset Per-Connection Overrides"))
            .subtitle(i18n(
                "Make every connection follow the global switch. Polling intervals set per connection are kept.",
            ))
            .build();
        let reset_overrides_button = gtk4::Button::builder()
            .label(i18n("Reset…"))
            .valign(gtk4::Align::Center)
            .build();
        reset_overrides_row.add_suffix(&reset_overrides_button);
        reset_overrides_row.set_activatable_widget(Some(&reset_overrides_button));
        general_group.add(&reset_overrides_row);

        page.add(&general_group);

        // === Metrics Group ===
        let metrics_group = adw::PreferencesGroup::builder()
            .title(i18n("Visible Metrics"))
            .description(i18n("Select which metrics to display"))
            .build();

        let show_cpu = adw::SwitchRow::builder()
            .title(i18n("CPU usage"))
            .active(true)
            .sensitive(false)
            .build();
        metrics_group.add(&show_cpu);

        let show_memory = adw::SwitchRow::builder()
            .title(i18n("Memory usage"))
            .active(true)
            .sensitive(false)
            .build();
        metrics_group.add(&show_memory);

        let show_disk = adw::SwitchRow::builder()
            .title(i18n("Disk usage"))
            .active(true)
            .sensitive(false)
            .build();
        metrics_group.add(&show_disk);

        let show_network = adw::SwitchRow::builder()
            .title(i18n("Network throughput"))
            .active(true)
            .sensitive(false)
            .build();
        metrics_group.add(&show_network);

        let show_load = adw::SwitchRow::builder()
            .title(i18n("Load average"))
            .active(true)
            .sensitive(false)
            .build();
        metrics_group.add(&show_load);

        let show_system_info = adw::SwitchRow::builder()
            .title(i18n("System information"))
            .subtitle(i18n("Distribution, kernel version, uptime"))
            .active(true)
            .sensitive(false)
            .build();
        metrics_group.add(&show_system_info);

        page.add(&metrics_group);

        // Connect switch to enable/disable other controls
        let interval_clone = interval_row.clone();
        let cpu_clone = show_cpu.clone();
        let mem_clone = show_memory.clone();
        let disk_clone = show_disk.clone();
        let net_clone = show_network.clone();
        let load_clone = show_load.clone();
        let sysinfo_clone = show_system_info.clone();
        enabled_row.connect_active_notify(move |row| {
            let state = row.is_active();
            interval_clone.set_sensitive(state);
            cpu_clone.set_sensitive(state);
            mem_clone.set_sensitive(state);
            disk_clone.set_sensitive(state);
            net_clone.set_sensitive(state);
            load_clone.set_sensitive(state);
            sysinfo_clone.set_sensitive(state);
        });

        // === Activity Monitor Group ===
        let activity_group = adw::PreferencesGroup::builder()
            .title(i18n("Activity Monitor"))
            .description(i18n(
                "Default settings for terminal activity and silence detection",
            ))
            .build();

        let mode_labels = crate::monitor_mode::labels();
        let mode_items =
            StringList::new(&mode_labels.iter().map(String::as_str).collect::<Vec<_>>());
        let activity_mode_combo = adw::ComboRow::builder()
            .title(i18n("Default Mode"))
            .subtitle(i18n("Monitoring mode applied to new connections"))
            .model(&mode_items)
            .selected(0)
            .build();
        activity_group.add(&activity_mode_combo);

        let quiet_period_adj = gtk4::Adjustment::new(10.0, 1.0, 300.0, 1.0, 10.0, 0.0);
        let activity_quiet_period_spin = adw::SpinRow::builder()
            .title(i18n("Default Quiet Period"))
            .subtitle(i18n("Seconds of silence before activity notification"))
            .adjustment(&quiet_period_adj)
            .build();
        activity_group.add(&activity_quiet_period_spin);

        let silence_timeout_adj = gtk4::Adjustment::new(30.0, 1.0, 600.0, 1.0, 10.0, 0.0);
        let activity_silence_timeout_spin = adw::SpinRow::builder()
            .title(i18n("Default Silence Timeout"))
            .subtitle(i18n("Seconds of no output before silence notification"))
            .adjustment(&silence_timeout_adj)
            .build();
        activity_group.add(&activity_silence_timeout_spin);

        page.add(&activity_group);

        Self {
            page,
            enabled_row,
            interval_row,
            reset_overrides_button,
            show_cpu,
            show_memory,
            show_disk,
            show_network,
            show_load,
            show_system_info,
            activity_mode_combo,
            activity_quiet_period_spin,
            activity_silence_timeout_spin,
        }
    }

    /// Loads monitoring settings into UI controls
    pub fn load(&self, settings: &MonitoringSettings) {
        self.enabled_row.set_active(settings.enabled);
        self.interval_row
            .set_value(f64::from(settings.effective_interval_secs()));
        self.show_cpu.set_active(settings.show_cpu);
        self.show_memory.set_active(settings.show_memory);
        self.show_disk.set_active(settings.show_disk);
        self.show_network.set_active(settings.show_network);
        self.show_load.set_active(settings.show_load);
        self.show_system_info.set_active(settings.show_system_info);

        // Update sensitivity based on enabled state
        let enabled = settings.enabled;
        self.interval_row.set_sensitive(enabled);
        self.show_cpu.set_sensitive(enabled);
        self.show_memory.set_sensitive(enabled);
        self.show_disk.set_sensitive(enabled);
        self.show_network.set_sensitive(enabled);
        self.show_load.set_sensitive(enabled);
        self.show_system_info.set_sensitive(enabled);
    }

    /// Collects monitoring settings from UI controls
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "value range fits the target type and is non-negative by construction in this code path"
    )]
    #[must_use]
    pub fn collect(&self) -> MonitoringSettings {
        MonitoringSettings {
            enabled: self.enabled_row.is_active(),
            interval_secs: self.interval_row.value() as u8,
            show_cpu: self.show_cpu.is_active(),
            show_memory: self.show_memory.is_active(),
            show_disk: self.show_disk.is_active(),
            show_network: self.show_network.is_active(),
            show_load: self.show_load.is_active(),
            show_system_info: self.show_system_info.is_active(),
        }
    }

    /// Loads activity monitor defaults into UI controls
    pub fn load_activity_monitor(&self, defaults: &ActivityMonitorDefaults) {
        self.activity_mode_combo
            .set_selected(crate::monitor_mode::index_of(defaults.mode));
        self.activity_quiet_period_spin
            .set_value(f64::from(defaults.effective_quiet_period()));
        self.activity_silence_timeout_spin
            .set_value(f64::from(defaults.effective_silence_timeout()));
    }

    /// Collects activity monitor defaults from UI controls
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "value range fits the target type and is non-negative by construction in this code path"
    )]
    #[must_use]
    pub fn collect_activity_monitor(&self) -> ActivityMonitorDefaults {
        ActivityMonitorDefaults {
            mode: crate::monitor_mode::from_index(self.activity_mode_combo.selected()),
            quiet_period_secs: self.activity_quiet_period_spin.value() as u32,
            silence_timeout_secs: self.activity_silence_timeout_spin.value() as u32,
        }
    }
}

impl Default for MonitoringPageWidgets {
    fn default() -> Self {
        Self::new()
    }
}
