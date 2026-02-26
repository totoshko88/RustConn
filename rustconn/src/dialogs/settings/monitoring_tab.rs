//! Monitoring settings tab using libadwaita components

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{CheckButton, Switch};
use libadwaita as adw;
use rustconn_core::monitoring::MonitoringSettings;

use crate::i18n::i18n;

/// Holds all monitoring settings page widgets
#[derive(Clone)]
pub struct MonitoringPageWidgets {
    /// The preferences page
    pub page: adw::PreferencesPage,
    /// Global enable switch
    pub enabled_switch: Switch,
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
}

impl MonitoringPageWidgets {
    /// Creates the monitoring settings page using `AdwPreferencesPage`
    #[must_use]
    pub fn new() -> Self {
        let page = adw::PreferencesPage::builder()
            .title(i18n("Monitoring"))
            .icon_name("utilities-system-monitor-symbolic")
            .build();

        // === General Group ===
        let general_group = adw::PreferencesGroup::builder()
            .title(i18n("General"))
            .description(i18n("Remote host metrics collection"))
            .build();

        let enabled_switch = Switch::builder().valign(gtk4::Align::Center).build();
        let enable_row = adw::ActionRow::builder()
            .title(i18n("Enable monitoring"))
            .subtitle(i18n("Show CPU, memory, disk, and network for SSH sessions"))
            .build();
        enable_row.add_suffix(&enabled_switch);
        enable_row.set_activatable_widget(Some(&enabled_switch));
        general_group.add(&enable_row);

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
        enabled_switch.connect_state_set(move |_, state| {
            interval_clone.set_sensitive(state);
            cpu_clone.set_sensitive(state);
            mem_clone.set_sensitive(state);
            disk_clone.set_sensitive(state);
            net_clone.set_sensitive(state);
            load_clone.set_sensitive(state);
            sysinfo_clone.set_sensitive(state);
            gtk4::glib::Propagation::Proceed
        });

        Self {
            page,
            enabled_switch,
            interval_row,
            show_cpu,
            show_memory,
            show_disk,
            show_network,
            show_load,
            show_system_info,
        }
    }

    /// Loads monitoring settings into UI controls
    pub fn load(&self, settings: &MonitoringSettings) {
        self.enabled_switch.set_active(settings.enabled);
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
            enabled: self.enabled_switch.is_active(),
            interval_secs: self.interval_row.value() as u8,
            show_cpu: self.show_cpu.is_active(),
            show_memory: self.show_memory.is_active(),
            show_disk: self.show_disk.is_active(),
            show_network: self.show_network.is_active(),
            show_load: self.show_load.is_active(),
            show_system_info: self.show_system_info.is_active(),
        }
    }
}

impl Default for MonitoringPageWidgets {
    fn default() -> Self {
        Self::new()
    }
}
