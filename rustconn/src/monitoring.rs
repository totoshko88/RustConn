//! GTK monitoring bar widget for remote host metrics
//!
//! Displays a compact horizontal bar below the terminal showing
//! CPU, memory, disk, and network usage from the remote host.

use gtk4::prelude::*;
use gtk4::{self, Align, Orientation};
use rustconn_core::monitoring::{MonitoringSettings, RemoteMetrics, SystemInfo};

use crate::i18n::i18n;

/// Height of the monitoring bar in pixels
const BAR_HEIGHT: i32 = 28;

/// A compact monitoring bar widget showing remote host metrics.
///
/// Layout: `[CPU: ██░░ 45%] [RAM: ██░░ 62%] [Disk: ██░░ 78%] [↓ 1.2 MB/s ↑ 0.3 MB/s]`
pub struct MonitoringBar {
    /// Root container
    container: gtk4::Box,
    /// CPU level bar
    cpu_bar: gtk4::LevelBar,
    /// CPU percentage label
    cpu_label: gtk4::Label,
    /// Memory level bar
    mem_bar: gtk4::LevelBar,
    /// Memory percentage label
    mem_label: gtk4::Label,
    /// Disk level bar
    disk_bar: gtk4::LevelBar,
    /// Disk percentage label
    disk_label: gtk4::Label,
    /// Network label (rx/tx rates)
    net_label: gtk4::Label,
    /// Load average label
    load_label: gtk4::Label,
    /// System info label (distro, kernel, uptime)
    info_label: gtk4::Label,
    /// CPU section box
    cpu_section: gtk4::Box,
    /// Memory section box
    mem_section: gtk4::Box,
    /// Disk section box
    disk_section: gtk4::Box,
    /// Network section box
    net_section: gtk4::Box,
    /// Load average section box
    load_section: gtk4::Box,
    /// System info section box
    info_section: gtk4::Box,
}

impl Default for MonitoringBar {
    fn default() -> Self {
        Self::new()
    }
}

impl MonitoringBar {
    /// Creates a new monitoring bar widget
    #[must_use]
    pub fn new() -> Self {
        let container = gtk4::Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(12)
            .height_request(BAR_HEIGHT)
            .halign(Align::Fill)
            .hexpand(true)
            .css_classes(["monitoring-bar"])
            .margin_start(6)
            .margin_end(6)
            .build();

        let (cpu_section, cpu_bar, cpu_label) =
            Self::create_metric_section(&i18n("CPU"), "preferences-system-symbolic");
        let (mem_section, mem_bar, mem_label) =
            Self::create_metric_section(&i18n("RAM"), "drive-harddisk-symbolic");
        let (disk_section, disk_bar, disk_label) =
            Self::create_metric_section(&i18n("Disk"), "drive-harddisk-symbolic");

        let net_section = gtk4::Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(4)
            .build();
        let net_icon = gtk4::Image::from_icon_name("network-transmit-receive-symbolic");
        net_icon.set_pixel_size(14);
        let net_label = gtk4::Label::builder()
            .label("↓ — ↑ —")
            .css_classes(["caption", "monitoring-net"])
            .build();
        net_section.append(&net_icon);
        net_section.append(&net_label);

        // Load average section
        let load_section = gtk4::Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(4)
            .build();
        let load_icon = gtk4::Image::from_icon_name("system-run-symbolic");
        load_icon.set_pixel_size(14);
        let load_label = gtk4::Label::builder()
            .label("—")
            .css_classes(["caption", "monitoring-load"])
            .build();
        load_section.append(&load_icon);
        load_section.append(&load_label);

        // System info section (right-aligned)
        let info_section = gtk4::Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(4)
            .halign(Align::End)
            .hexpand(true)
            .build();
        let info_icon = gtk4::Image::from_icon_name("computer-symbolic");
        info_icon.set_pixel_size(14);
        let info_label = gtk4::Label::builder()
            .label("")
            .css_classes(["caption", "monitoring-info"])
            .build();
        info_section.append(&info_icon);
        info_section.append(&info_label);
        info_section.set_visible(false);

        container.append(&cpu_section);
        container.append(&mem_section);
        container.append(&disk_section);
        container.append(&load_section);
        container.append(&net_section);
        container.append(&info_section);

        // Initially hidden until first metrics arrive
        container.set_visible(false);

        Self {
            container,
            cpu_bar,
            cpu_label,
            mem_bar,
            mem_label,
            disk_bar,
            disk_label,
            net_label,
            load_label,
            info_label,
            cpu_section,
            mem_section,
            disk_section,
            net_section,
            load_section,
            info_section,
        }
    }

    /// Creates a metric section: `[icon label: ████░░ XX%]`
    fn create_metric_section(
        name: &str,
        icon_name: &str,
    ) -> (gtk4::Box, gtk4::LevelBar, gtk4::Label) {
        let section = gtk4::Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(4)
            .build();

        let icon = gtk4::Image::from_icon_name(icon_name);
        icon.set_pixel_size(14);

        let label = gtk4::Label::builder()
            .label(&format!("{name}:"))
            .css_classes(["caption", "monitoring-label"])
            .build();

        let bar = gtk4::LevelBar::builder()
            .min_value(0.0)
            .max_value(100.0)
            .value(0.0)
            .hexpand(false)
            .width_request(60)
            .valign(Align::Center)
            .build();
        bar.add_css_class("monitoring-level");

        let pct_label = gtk4::Label::builder()
            .label("—")
            .width_chars(4)
            .css_classes(["caption", "monitoring-pct"])
            .build();

        section.append(&icon);
        section.append(&label);
        section.append(&bar);
        section.append(&pct_label);

        (section, bar, pct_label)
    }

    /// Updates the bar with new metrics
    pub fn update(&self, metrics: &RemoteMetrics) {
        // CPU — tooltip shows load average
        let cpu = metrics.cpu_percent.clamp(0.0, 100.0);
        self.cpu_bar.set_value(f64::from(cpu));
        self.cpu_label.set_label(&format!("{cpu:.0}%"));

        // Memory — tooltip shows swap if present
        let mem = metrics.memory.percent().clamp(0.0, 100.0);
        self.mem_bar.set_value(f64::from(mem));
        self.mem_label.set_label(&format!("{mem:.0}%"));
        if metrics.memory.swap_total_kib > 0 {
            let swap_pct = metrics.memory.swap_percent();
            let swap_used = format_kib(metrics.memory.swap_used_kib);
            let swap_total = format_kib(metrics.memory.swap_total_kib);
            self.mem_section.set_tooltip_text(Some(&format!(
                "Swap: {swap_used}/{swap_total} ({swap_pct:.0}%)"
            )));
        }

        // Disk
        let disk = metrics.disk.percent().clamp(0.0, 100.0);
        self.disk_bar.set_value(f64::from(disk));
        self.disk_label.set_label(&format!("{disk:.0}%"));

        // Network
        let rx = format_throughput(metrics.network.rx_bytes_per_sec);
        let tx = format_throughput(metrics.network.tx_bytes_per_sec);
        self.net_label.set_label(&format!("↓ {rx} ↑ {tx}"));

        // Load average
        let la = &metrics.load_average;
        self.load_label
            .set_label(&format!("{:.2} {:.2} {:.2}", la.one, la.five, la.fifteen));
        self.load_section.set_tooltip_text(Some(&format!(
            "{}: {}/{}",
            i18n("Processes"),
            la.running_procs,
            la.total_procs
        )));

        // Show the bar once we have data
        if !self.container.is_visible() {
            self.container.set_visible(true);
        }
    }

    /// Updates the system info display (called once when info is received)
    pub fn update_system_info(&self, info: &SystemInfo) {
        let uptime = format_uptime(info.uptime_secs);

        // Build base: "Ubuntu 24.04 (6.8.0)" or just kernel/distro
        let mut parts: Vec<String> = Vec::new();

        let base = if info.distro_name.is_empty() {
            info.kernel_version.clone()
        } else if info.kernel_version.is_empty() {
            info.distro_name.clone()
        } else {
            format!("{} ({})", info.distro_name, info.kernel_version)
        };
        if !base.is_empty() {
            parts.push(base);
        }

        if !info.arch.is_empty() {
            parts.push(info.arch.clone());
        }

        if info.total_ram_kib > 0 {
            parts.push(format_kib(info.total_ram_kib));
        }

        if info.cpu_threads > 0 {
            if info.cpu_cores > 0 && info.cpu_cores != info.cpu_threads {
                parts.push(format!("{}C/{}T", info.cpu_cores, info.cpu_threads));
            } else {
                parts.push(format!("{}C", info.cpu_threads));
            }
        }

        let text = parts.join(" · ");
        self.info_label.set_label(&text);
        self.info_section
            .set_tooltip_text(Some(&format!("{}: {}", i18n("Uptime"), uptime)));
        self.info_section.set_visible(true);
    }

    /// Applies visibility settings from global monitoring config
    pub fn apply_settings(&self, settings: &MonitoringSettings) {
        self.cpu_section.set_visible(settings.show_cpu);
        self.mem_section.set_visible(settings.show_memory);
        self.disk_section.set_visible(settings.show_disk);
        self.net_section.set_visible(settings.show_network);
        self.load_section.set_visible(settings.show_load);
        // info_section visibility depends on both the setting and whether
        // system info has been received (non-empty label means data arrived)
        let has_info = !self.info_label.label().is_empty();
        self.info_section
            .set_visible(settings.show_system_info && has_info);
    }

    /// Returns the root widget for embedding in a container
    #[must_use]
    pub fn widget(&self) -> &gtk4::Box {
        &self.container
    }

    /// Shows the monitoring bar
    pub fn show(&self) {
        self.container.set_visible(true);
    }

    /// Hides the monitoring bar
    pub fn hide(&self) {
        self.container.set_visible(false);
    }

    /// Returns whether the bar is currently visible
    #[must_use]
    pub fn is_visible(&self) -> bool {
        self.container.is_visible()
    }
}

/// Formats bytes/sec into a human-readable throughput string
fn format_throughput(bytes_per_sec: f64) -> String {
    if bytes_per_sec < 1024.0 {
        format!("{bytes_per_sec:.0} B/s")
    } else if bytes_per_sec < 1_048_576.0 {
        format!("{:.1} KB/s", bytes_per_sec / 1024.0)
    } else if bytes_per_sec < 1_073_741_824.0 {
        format!("{:.1} MB/s", bytes_per_sec / 1_048_576.0)
    } else {
        format!("{:.2} GB/s", bytes_per_sec / 1_073_741_824.0)
    }
}

/// Formats KiB into a human-readable size string
fn format_kib(kib: u64) -> String {
    if kib < 1024 {
        format!("{kib} KiB")
    } else if kib < 1_048_576 {
        format!("{:.1} MiB", kib as f64 / 1024.0)
    } else {
        format!("{:.1} GiB", kib as f64 / 1_048_576.0)
    }
}

/// Formats seconds into a human-readable uptime string (e.g. "3d 5h 12m")
fn format_uptime(secs: u64) -> String {
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let mins = (secs % 3600) / 60;
    if days > 0 {
        format!("{days}d {hours}h {mins}m")
    } else if hours > 0 {
        format!("{hours}h {mins}m")
    } else {
        format!("{mins}m")
    }
}

/// Manages monitoring bars and collectors for all active sessions.
///
/// Owns the mapping from session IDs to monitoring bars and collector handles.
/// Call [`start_monitoring`] after an SSH session is established and
/// [`stop_monitoring`] when the session closes.
pub struct MonitoringCoordinator {
    /// Active monitoring bars keyed by session ID
    bars: RefCell<HashMap<Uuid, Rc<MonitoringBar>>>,
    /// Active collector stop handles keyed by session ID
    handles: RefCell<HashMap<Uuid, rustconn_core::monitoring::CollectorHandle>>,
}

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use uuid::Uuid;

impl MonitoringCoordinator {
    /// Creates a new coordinator with no active sessions
    #[must_use]
    pub fn new() -> Self {
        Self {
            bars: RefCell::new(HashMap::new()),
            handles: RefCell::new(HashMap::new()),
        }
    }

    /// Starts monitoring for an SSH session.
    ///
    /// Creates a [`MonitoringBar`], appends it to the terminal `container`,
    /// starts a background collector, and wires up GTK updates.
    ///
    /// # Arguments
    /// * `session_id` - Unique session identifier
    /// * `container` - The vertical `gtk4::Box` that holds the terminal
    /// * `settings` - Global monitoring settings
    /// * `host` - Remote hostname
    /// * `port` - SSH port
    /// * `username` - Optional SSH username
    /// * `identity_file` - Optional SSH key path
    /// * `password` - Optional password for sshpass authentication
    #[allow(clippy::too_many_arguments)]
    pub fn start_monitoring(
        &self,
        session_id: Uuid,
        container: &gtk4::Box,
        settings: &MonitoringSettings,
        host: &str,
        port: u16,
        username: Option<&str>,
        identity_file: Option<&str>,
        password: Option<&str>,
    ) {
        // Don't start if monitoring is disabled
        if !settings.enabled {
            return;
        }

        // Don't double-start
        if self.bars.borrow().contains_key(&session_id) {
            return;
        }

        let bar = Rc::new(MonitoringBar::new());
        bar.apply_settings(settings);
        container.append(bar.widget());

        // Start the collector with SSH exec
        let exec_fn = rustconn_core::monitoring::ssh_exec_factory(
            host.to_string(),
            port,
            username.map(String::from),
            identity_file.map(String::from),
            password.map(String::from),
        );

        let (handle, mut rx) =
            rustconn_core::monitoring::start_collector(settings.clone(), exec_fn);

        // Wire up GTK updates from collector events
        let bar_clone = Rc::clone(&bar);
        crate::async_utils::spawn_async(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    rustconn_core::monitoring::MetricsEvent::Update(metrics) => {
                        let bar_ref = bar_clone.clone();
                        gtk4::glib::idle_add_local_once(move || {
                            bar_ref.update(&metrics);
                        });
                    }
                    rustconn_core::monitoring::MetricsEvent::SystemInfoReady(info) => {
                        let bar_ref = bar_clone.clone();
                        gtk4::glib::idle_add_local_once(move || {
                            bar_ref.update_system_info(&info);
                        });
                    }
                    rustconn_core::monitoring::MetricsEvent::ParseError(msg) => {
                        tracing::debug!(
                            session_id = %session_id,
                            error = %msg,
                            "Monitoring parse error"
                        );
                    }
                    rustconn_core::monitoring::MetricsEvent::Stopped => {
                        tracing::info!(
                            session_id = %session_id,
                            "Monitoring collector stopped"
                        );
                        break;
                    }
                }
            }
        });

        self.bars.borrow_mut().insert(session_id, bar);
        self.handles.borrow_mut().insert(session_id, handle);

        tracing::info!(
            session_id = %session_id,
            host = %host,
            port = %port,
            "Started remote monitoring"
        );
    }

    /// Stops monitoring for a session and removes the bar widget.
    pub fn stop_monitoring(&self, session_id: Uuid) {
        if let Some(handle) = self.handles.borrow_mut().remove(&session_id) {
            // Fire-and-forget stop signal
            crate::async_utils::spawn_async(async move {
                handle.stop().await;
            });
        }

        if let Some(bar) = self.bars.borrow_mut().remove(&session_id) {
            // Remove widget from parent
            if let Some(parent) = bar.widget().parent()
                && let Some(parent_box) = parent.downcast_ref::<gtk4::Box>()
            {
                parent_box.remove(bar.widget());
            }
        }
    }

    /// Stops all active monitoring sessions (e.g. on app shutdown)
    pub fn stop_all(&self) {
        let session_ids: Vec<Uuid> = self.handles.borrow().keys().copied().collect();
        for session_id in session_ids {
            self.stop_monitoring(session_id);
        }
    }

    /// Returns the monitoring bar for a session, if active
    #[must_use]
    pub fn get_bar(&self, session_id: Uuid) -> Option<Rc<MonitoringBar>> {
        self.bars.borrow().get(&session_id).cloned()
    }

    /// Updates settings on all active monitoring bars
    pub fn apply_settings_to_all(&self, settings: &MonitoringSettings) {
        for bar in self.bars.borrow().values() {
            bar.apply_settings(settings);
        }
    }
}

impl Default for MonitoringCoordinator {
    fn default() -> Self {
        Self::new()
    }
}
