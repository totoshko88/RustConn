//! Monitoring settings tab using libadwaita components

use adw::prelude::*;
use gtk4::CheckButton;
use gtk4::StringList;
use gtk4::prelude::*;
use libadwaita as adw;
use rustconn_core::activity_monitor::{ActivityMonitorDefaults, MonitorMode};
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
    /// Show CPU usage
    pub show_cpu: CheckButton,
    /// Show memory usage
    pub show_memory: CheckButton,
    /// Show disk usage
    pub show_disk: CheckButton,
    /// Show network throughput
    pub show_network: CheckButton,
    /// Show load average
    pub show_load: CheckButton,
    /// Show system info (distro, kernel, uptime)
    pub show_system_info: CheckButton,
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
            .icon_name("preferences-system-symbolic")
            .build();

        // === General Group ===
        let general_group = adw::PreferencesGroup::builder()
            .title(i18n("General"))
            .description(i18n("Remote host metrics collection"))
            .build();

        let enabled_row = adw::SwitchRow::builder()
            .title(i18n("Enable monitoring"))
            .subtitle(i18n("Show CPU, memory, disk, and network for SSH sessions"))
            .build();
        general_group.add(&enabled_row);

        let interval_row = adw::SpinRow::builder()
            .title(i18n("Polling interval"))
            .subtitle(i18n("Seconds between metric updates"))
            .adjustment(&gtk4::Adjustment::new(3.0, 1.0, 60.0, 1.0, 5.0, 0.0))
            .sensitive(false)
            .build();
        general_group.add(&interval_row);

        page.add(&general_group);

        // === Metrics Group ===
        let metrics_group = adw::PreferencesGroup::builder()
            .title(i18n("Visible Metrics"))
            .description(i18n("Select which metrics to display"))
            .build();

        let show_cpu = CheckButton::builder()
            .active(true)
            .valign(gtk4::Align::Center)
            .sensitive(false)
            .build();
        let cpu_row = adw::ActionRow::builder()
            .title(i18n("CPU usage"))
            .activatable_widget(&show_cpu)
            .build();
        cpu_row.add_prefix(&show_cpu);
        metrics_group.add(&cpu_row);

        let show_memory = CheckButton::builder()
            .active(true)
            .valign(gtk4::Align::Center)
            .sensitive(false)
            .build();
        let memory_row = adw::ActionRow::builder()
            .title(i18n("Memory usage"))
            .activatable_widget(&show_memory)
            .build();
        memory_row.add_prefix(&show_memory);
        metrics_group.add(&memory_row);

        let show_disk = CheckButton::builder()
            .active(true)
            .valign(gtk4::Align::Center)
            .sensitive(false)
            .build();
        let disk_row = adw::ActionRow::builder()
            .title(i18n("Disk usage"))
            .activatable_widget(&show_disk)
            .build();
        disk_row.add_prefix(&show_disk);
        metrics_group.add(&disk_row);

        let show_network = CheckButton::builder()
            .active(true)
            .valign(gtk4::Align::Center)
            .sensitive(false)
            .build();
        let network_row = adw::ActionRow::builder()
            .title(i18n("Network throughput"))
            .activatable_widget(&show_network)
            .build();
        network_row.add_prefix(&show_network);
        metrics_group.add(&network_row);

        let show_load = CheckButton::builder()
            .active(true)
            .valign(gtk4::Align::Center)
            .sensitive(false)
            .build();
        let load_row = adw::ActionRow::builder()
            .title(i18n("Load average"))
            .activatable_widget(&show_load)
            .build();
        load_row.add_prefix(&show_load);
        metrics_group.add(&load_row);

        let show_system_info = CheckButton::builder()
            .active(true)
            .valign(gtk4::Align::Center)
            .sensitive(false)
            .build();
        let system_info_row = adw::ActionRow::builder()
            .title(i18n("System info"))
            .subtitle(i18n("Distribution, kernel version, uptime"))
            .activatable_widget(&show_system_info)
            .build();
        system_info_row.add_prefix(&show_system_info);
        metrics_group.add(&system_info_row);

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

        let mode_items = StringList::new(&[&i18n("Off"), &i18n("Activity"), &i18n("Silence")]);
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
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
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
        let mode_idx = match defaults.mode {
            MonitorMode::Off => 0,
            MonitorMode::Activity => 1,
            MonitorMode::Silence => 2,
        };
        self.activity_mode_combo.set_selected(mode_idx);
        self.activity_quiet_period_spin
            .set_value(f64::from(defaults.effective_quiet_period()));
        self.activity_silence_timeout_spin
            .set_value(f64::from(defaults.effective_silence_timeout()));
    }

    /// Collects activity monitor defaults from UI controls
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    #[must_use]
    pub fn collect_activity_monitor(&self) -> ActivityMonitorDefaults {
        let mode = match self.activity_mode_combo.selected() {
            1 => MonitorMode::Activity,
            2 => MonitorMode::Silence,
            _ => MonitorMode::Off,
        };
        ActivityMonitorDefaults {
            mode,
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
