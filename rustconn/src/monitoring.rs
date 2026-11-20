//! GTK monitoring bar widget for remote host metrics
//!
//! Displays a compact horizontal bar below the terminal showing
//! CPU, memory, disk, and network usage from the remote host.

use std::cell::{Cell, RefCell};

use gtk4::prelude::*;
use gtk4::{self, Align, Orientation};
use rustconn_core::monitoring::{MonitoringSettings, RemoteMetrics, SystemInfo};

use crate::i18n::i18n;

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
    /// Base uptime (seconds) received from SystemInfoReady
    base_uptime_secs: Cell<u64>,
    /// Instant when SystemInfoReady was received (for live uptime calculation)
    sysinfo_received_at: Cell<Option<std::time::Instant>>,
    /// Cached system info for tooltip refresh
    cached_sysinfo: RefCell<Option<SystemInfo>>,
    /// Whether the collector has stopped (stale metrics)
    collector_stopped: Cell<bool>,
    /// Status icon shown when collector stops
    status_icon: gtk4::Image,
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
            .halign(Align::Fill)
            .hexpand(true)
            .overflow(gtk4::Overflow::Hidden)
            // Height and horizontal margins are CSS-driven via `.monitoring-bar`
            // so compact mode can shrink them uniformly (see assets/style.css).
            .css_classes(["monitoring-bar"])
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
            .ellipsize(gtk4::pango::EllipsizeMode::End)
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
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();
        info_section.append(&info_icon);
        info_section.append(&info_label);
        info_section.set_visible(false);

        // Status icon — shown when collector stops (stale metrics indicator)
        let status_icon = gtk4::Image::from_icon_name("dialog-warning-symbolic");
        status_icon.set_pixel_size(14);
        status_icon.set_tooltip_text(Some(&i18n("Monitoring stopped")));
        status_icon.set_visible(false);

        container.append(&cpu_section);
        container.append(&mem_section);
        container.append(&disk_section);
        container.append(&load_section);
        container.append(&net_section);
        container.append(&status_icon);
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
            base_uptime_secs: Cell::new(0),
            sysinfo_received_at: Cell::new(None),
            cached_sysinfo: RefCell::new(None),
            collector_stopped: Cell::new(false),
            status_icon,
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
            .label(format!("{name}:"))
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

        // Disk — bar shows root, tooltip shows all mount points
        let root_disk_pct = metrics.disk.percent().clamp(0.0, 100.0);
        self.disk_bar.set_value(f64::from(root_disk_pct));
        self.disk_label.set_label(&format!("{root_disk_pct:.0}%"));

        if metrics.disks.len() > 1 {
            let mut tooltip_lines: Vec<String> = Vec::new();
            for d in &metrics.disks {
                let pct = d.percent();
                let used = format_kib(d.used_kib);
                let total = format_kib(d.total_kib);
                tooltip_lines.push(format!("{}: {used}/{total} ({pct:.0}%)", d.mount_point));
            }
            self.disk_section
                .set_tooltip_text(Some(&tooltip_lines.join("\n")));
        } else {
            self.disk_section.set_tooltip_text(None);
        }

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

        // Refresh system info tooltip with live uptime
        self.refresh_info_tooltip();

        // Show the bar once we have data
        if !self.container.is_visible() {
            self.container.set_visible(true);
        }
    }

    /// Updates the system info display (called once when info is received)
    pub fn update_system_info(&self, info: &SystemInfo) {
        // Store base uptime and timestamp for live calculation
        self.base_uptime_secs.set(info.uptime_secs);
        self.sysinfo_received_at
            .set(Some(std::time::Instant::now()));
        *self.cached_sysinfo.borrow_mut() = Some(info.clone());

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

        // Show primary private IP if available
        if let Some(primary_ip) = info.ip_addresses.first() {
            parts.push(primary_ip.clone());
        }

        let text = parts.join(" · ");
        self.info_label.set_label(&text);

        // Refresh tooltip (uses cached sysinfo + live uptime)
        self.refresh_info_tooltip();
        self.info_section.set_visible(true);
    }

    /// Refreshes the system info tooltip with live uptime calculation.
    ///
    /// Called on every metrics update to keep the uptime counter current.
    fn refresh_info_tooltip(&self) {
        let sysinfo = self.cached_sysinfo.borrow();
        let Some(info) = sysinfo.as_ref() else {
            return;
        };

        // Calculate live uptime: base + elapsed since sysinfo was received
        let live_uptime = if let Some(received_at) = self.sysinfo_received_at.get() {
            let elapsed = received_at.elapsed().as_secs();
            self.base_uptime_secs.get() + elapsed
        } else {
            self.base_uptime_secs.get()
        };
        let uptime = format_uptime(live_uptime);

        // Build tooltip with uptime, hostname, and all IP addresses
        let mut tooltip_parts: Vec<String> = Vec::new();
        tooltip_parts.push(format!("{}: {}", i18n("Uptime"), uptime));
        if !info.hostname.is_empty() {
            tooltip_parts.push(format!("{}: {}", i18n("Hostname"), info.hostname));
        }
        if !info.ip_addresses.is_empty() {
            let ipv4: Vec<&str> = info
                .ip_addresses
                .iter()
                .filter(|ip| !ip.contains(':'))
                .map(String::as_str)
                .collect();
            let ipv6: Vec<&str> = info
                .ip_addresses
                .iter()
                .filter(|ip| ip.contains(':'))
                .map(String::as_str)
                .collect();
            if !ipv4.is_empty() {
                tooltip_parts.push(format!("IPv4: {}", ipv4.join(", ")));
            }
            if !ipv6.is_empty() {
                tooltip_parts.push(format!("IPv6: {}", ipv6.join(", ")));
            }
        }
        if self.collector_stopped.get() {
            tooltip_parts.push(i18n("⚠ Monitoring stopped — metrics may be stale"));
        }
        self.info_section
            .set_tooltip_text(Some(&tooltip_parts.join("\n")));
    }

    /// Marks the monitoring bar as stopped (collector is no longer running).
    ///
    /// Shows a warning icon and dims the bar to indicate stale metrics.
    pub fn mark_stopped(&self) {
        self.collector_stopped.set(true);
        self.status_icon.set_visible(true);
        self.container.add_css_class("monitoring-stale");
        self.refresh_info_tooltip();
    }

    /// Returns whether this bar's collector has stopped (stale metrics).
    ///
    /// Used by [`MonitoringCoordinator::start_monitoring`] to tell a bar left
    /// over from a dropped connection apart from one whose collector is still
    /// running (issue #398).
    #[must_use]
    pub fn is_stopped(&self) -> bool {
        self.collector_stopped.get()
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

/// What [`MonitoringCoordinator::start_monitoring`] should do about an
/// existing bar for the session, given whether its collector has stopped.
///
/// Pulled out as a pure function so the guard's three outcomes — all of which
/// matter for issue #398 — are testable without a display: a live bar must be
/// left alone (`Skip`), a bar whose collector already stopped must be
/// replaced rather than kept dimmed forever (`Replace`), and no existing bar
/// means a fresh start (`Create`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartAction {
    /// No bar exists for this session yet — build one.
    Create,
    /// A bar with a live collector already exists — do nothing.
    Skip,
    /// A bar exists but its collector has stopped (stale metrics, e.g. after
    /// a dropped connection) — detach it and build a fresh one.
    Replace,
}

/// Decides the [`StartAction`] from whether an existing bar's collector has
/// stopped. `None` means no bar exists for the session.
fn start_action(existing_stopped: Option<bool>) -> StartAction {
    match existing_stopped {
        None => StartAction::Create,
        Some(false) => StartAction::Skip,
        Some(true) => StartAction::Replace,
    }
}

/// Manages monitoring bars and collectors for all active sessions.
///
/// Owns the mapping from session IDs to monitoring bars and collector handles.
/// Call [`Self::start_monitoring`] after an SSH session is established and
/// [`Self::stop_monitoring`] when the session closes.
pub struct MonitoringCoordinator {
    /// Active monitoring bars keyed by session ID
    bars: RefCell<HashMap<Uuid, Rc<MonitoringBar>>>,
    /// Active collector stop handles keyed by session ID
    handles: RefCell<HashMap<Uuid, rustconn_core::monitoring::CollectorHandle>>,
    /// Suspended session params — stored when monitoring is suspended for split view,
    /// so it can be resumed when the session returns to the tab view.
    suspended: RefCell<HashMap<Uuid, MonitoringParams>>,
}

/// Connection parameters needed to restart monitoring after suspend.
#[derive(Clone)]
struct MonitoringParams {
    settings: MonitoringSettings,
    host: String,
    port: u16,
    username: Option<String>,
    identity_file: Option<String>,
    password: Option<secrecy::SecretString>,
    jump_host: Option<String>,
    agent_socket: Option<String>,
}

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
            suspended: RefCell::new(HashMap::new()),
        }
    }

    /// Starts monitoring for an SSH session.
    ///
    /// Creates a [`MonitoringBar`], appends it to the terminal `container`,
    /// starts a background collector, and wires up GTK updates. If a bar
    /// already exists for `session_id` with a collector that has stopped
    /// (stale metrics from a previous, now-dead connection), it is detached
    /// and replaced rather than left in place — see [`start_action`]
    /// (issue #398). A bar whose collector is still running is left alone.
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
    /// * `jump_host` - Optional jump host chain for `-J` flag
    /// * `agent_socket` - The connection's own ssh-agent socket, if any
    #[expect(
        clippy::too_many_arguments,
        reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
    )]
    pub fn start_monitoring(
        &self,
        session_id: Uuid,
        container: &gtk4::Box,
        settings: &MonitoringSettings,
        host: &str,
        port: u16,
        username: Option<&str>,
        identity_file: Option<&str>,
        password: Option<secrecy::SecretString>,
        jump_host: Option<&str>,
        agent_socket: Option<&str>,
    ) {
        // Don't start if monitoring is disabled
        if !settings.enabled {
            return;
        }

        // A bar with a live collector is left alone; a bar whose collector
        // already stopped (stale, e.g. after a dropped connection — #398) is
        // replaced rather than kept forever. The borrow ends before
        // `detach_bar` runs below so it does not nest into `bars.borrow_mut()`.
        let action = start_action(self.bars.borrow().get(&session_id).map(|b| b.is_stopped()));
        match action {
            StartAction::Skip => return,
            StartAction::Replace => {
                tracing::debug!(%session_id, "Replacing stale monitoring bar");
                self.detach_bar(session_id);
            }
            StartAction::Create => {}
        }

        let bar = Rc::new(MonitoringBar::new());
        bar.apply_settings(settings);
        container.append(bar.widget());

        // Clone password before it's consumed by the exec factory
        let password_clone = password.clone();

        // Start the collector with SSH exec
        let exec_fn = rustconn_core::monitoring::ssh_exec_factory(
            host.to_string(),
            port,
            username.map(String::from),
            identity_file.map(String::from),
            password,
            jump_host.map(String::from),
            agent_socket.map(String::from),
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
                        let bar_ref = bar_clone.clone();
                        gtk4::glib::idle_add_local_once(move || {
                            bar_ref.mark_stopped();
                        });
                        break;
                    }
                }
            }
        });

        self.bars.borrow_mut().insert(session_id, bar);
        self.handles.borrow_mut().insert(session_id, handle);

        // Store connection params for suspend/resume
        self.suspended.borrow_mut().remove(&session_id);
        self.store_params(
            session_id,
            settings,
            host,
            port,
            username,
            identity_file,
            password_clone,
            jump_host,
            agent_socket,
        );

        tracing::info!(
            session_id = %session_id,
            host = %host,
            port = %port,
            "Started remote monitoring"
        );
    }

    /// Stops the collector (fire-and-forget) and removes the bar widget from
    /// its parent for a session, without touching `suspended`.
    ///
    /// Shared by [`Self::stop_monitoring`], [`Self::suspend_monitoring`] and
    /// the `Replace` branch of [`Self::start_monitoring`], so a stale bar is
    /// always torn down the same way instead of being patched with CSS at
    /// one call site (issue #398).
    ///
    /// Returns `true` if a bar actually existed for `session_id` and was
    /// removed. Callers that need to know whether the session had a bar
    /// before the call — [`Self::reset_for_reconnect`] — use this to decide
    /// whether to restart monitoring afterwards.
    fn detach_bar(&self, session_id: Uuid) -> bool {
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
            true
        } else {
            false
        }
    }

    /// Stops monitoring for a session and removes the bar widget.
    pub fn stop_monitoring(&self, session_id: Uuid) {
        // Clean up stored params (no longer needed)
        self.suspended.borrow_mut().remove(&session_id);
        self.detach_bar(session_id);
    }

    /// Discards the previous connection's monitoring bar and collector ahead
    /// of a reconnect (issue #398).
    ///
    /// A reconnect starts a new SSH connection lifecycle, so any bar left
    /// over from the previous one — whether already dimmed as stale or still
    /// running with a collector that has not noticed the drop yet — must not
    /// survive it. `suspended` (the split-view resume params) is left alone:
    /// `start_monitoring`'s own `store_params` call overwrites it once the
    /// reconnect's collector starts, and clearing it here would only risk
    /// losing those params if the reconnect attempt fails before that point.
    ///
    /// Returns `true` if a bar actually existed and was torn down — the
    /// caller uses this to decide how to resolve the container to restart
    /// monitoring into once the reconnect establishes. A session whose
    /// monitoring was suspended for split view (no bar in `bars`, only in
    /// `suspended`) returns `false`: it must not have monitoring restarted
    /// into its split pane, since monitoring there is suspended by design
    /// until the session leaves the split (see `resume_monitoring`).
    #[must_use]
    pub fn reset_for_reconnect(&self, session_id: Uuid) -> bool {
        self.detach_bar(session_id)
    }

    /// Stops all active monitoring sessions (e.g. on app shutdown)
    pub fn stop_all(&self) {
        let session_ids: Vec<Uuid> = self.handles.borrow().keys().copied().collect();
        for session_id in session_ids {
            self.stop_monitoring(session_id);
        }
    }

    /// Suspends monitoring for a session (e.g. when entering split view).
    ///
    /// Stops the collector and removes the bar widget, but keeps the
    /// connection parameters so monitoring can be resumed later.
    pub fn suspend_monitoring(&self, session_id: Uuid) {
        if !self.bars.borrow().contains_key(&session_id) {
            return;
        }

        self.detach_bar(session_id);

        tracing::info!(
            session_id = %session_id,
            "Suspended monitoring for split view"
        );
    }

    /// Resumes monitoring for a session (e.g. when returning from split view).
    ///
    /// Restarts the collector using stored connection parameters and attaches
    /// a new bar to the given container.
    pub fn resume_monitoring(&self, session_id: Uuid, container: &gtk4::Box) {
        // Don't resume if already active
        if self.bars.borrow().contains_key(&session_id) {
            return;
        }

        let params = match self.suspended.borrow().get(&session_id).cloned() {
            Some(p) => p,
            None => return,
        };

        tracing::info!(
            session_id = %session_id,
            host = %params.host,
            "Resuming monitoring after split view"
        );

        self.start_monitoring(
            session_id,
            container,
            &params.settings,
            &params.host,
            params.port,
            params.username.as_deref(),
            params.identity_file.as_deref(),
            params.password,
            params.jump_host.as_deref(),
            params.agent_socket.as_deref(),
        );
    }

    /// Returns whether a session has suspended monitoring params.
    #[must_use]
    pub fn is_suspended(&self, session_id: Uuid) -> bool {
        self.suspended.borrow().contains_key(&session_id)
    }

    /// Stores connection parameters for later suspend/resume.
    #[expect(
        clippy::too_many_arguments,
        reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
    )]
    fn store_params(
        &self,
        session_id: Uuid,
        settings: &MonitoringSettings,
        host: &str,
        port: u16,
        username: Option<&str>,
        identity_file: Option<&str>,
        password: Option<secrecy::SecretString>,
        jump_host: Option<&str>,
        agent_socket: Option<&str>,
    ) {
        self.suspended.borrow_mut().insert(
            session_id,
            MonitoringParams {
                settings: settings.clone(),
                host: host.to_string(),
                port,
                username: username.map(String::from),
                identity_file: identity_file.map(String::from),
                password,
                jump_host: jump_host.map(String::from),
                agent_socket: agent_socket.map(String::from),
            },
        );
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

#[cfg(test)]
mod tests {
    use super::{StartAction, start_action};

    /// No bar exists yet — a fresh collector and bar should be created.
    #[test]
    fn start_action_creates_when_no_bar() {
        assert_eq!(start_action(None), StartAction::Create);
    }

    /// A bar with a live collector must be left alone: starting monitoring
    /// again (e.g. a repeated `connect_contents_changed` or a resume while
    /// already active) must not create a second bar.
    #[test]
    fn start_action_skips_live_bar() {
        assert_eq!(start_action(Some(false)), StartAction::Skip);
    }

    /// Regression for issue #398: a bar whose collector already stopped
    /// (stale metrics, dimmed with `monitoring-stale`) must be replaced, not
    /// kept — otherwise reconnect leaves the washed-out bar in place forever.
    #[test]
    fn start_action_replaces_stopped_bar() {
        assert_eq!(start_action(Some(true)), StartAction::Replace);
    }

    // `detach_bar`/`reset_for_reconnect` act on `bars: HashMap<_, Rc<MonitoringBar>>`,
    // and `MonitoringBar::new()` builds real `gtk4::Box`/`LevelBar`/`Image`
    // widgets — there is no way to populate that map, even just to check
    // removal and the returned `bool`, without `gtk4::init()` succeeding
    // first. The two tests below need a display and their own process (GTK
    // is not safe to initialise more than once per process, and `cargo test`
    // runs targets in parallel threads of one process), so they follow the
    // same `#[ignore]` + graceful-skip convention as the other GTK-backed
    // tests in this crate (e.g. `terminal::detach::tests`) rather than
    // running in the headless default suite. On-device coverage for the
    // widget *placement* (tab/detached/split) stays in
    // `.agents/tasks/fix-398-392/plan-398.md`.

    use uuid::Uuid;

    use super::MonitoringCoordinator;
    use rustconn_core::monitoring::MonitoringSettings;

    /// `detach_bar` removes the session from both `bars` and `handles`, and
    /// reports via its return value whether a bar actually existed —
    /// idempotently on a second call.
    #[test]
    #[ignore = "initialises GTK: needs a display and its own process; run alone with `cargo test -p rustconn --bin rustconn -- --ignored --exact monitoring::tests::detach_bar_removes_from_both_maps_and_is_idempotent`"]
    fn detach_bar_removes_from_both_maps_and_is_idempotent() {
        if gtk4::init().is_err() {
            // No display in this environment — on-device only (see module doc above).
            return;
        }

        let coordinator = MonitoringCoordinator::new();
        let session_id = Uuid::new_v4();
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        let settings = MonitoringSettings::default();

        // `start_collector` calls `tokio::spawn`, which needs an ambient
        // runtime context; `detach_bar`'s fire-and-forget stop goes through
        // `spawn_async` (the GLib main context), which needs no running loop
        // just to queue the future.
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            coordinator.start_monitoring(
                session_id,
                &container,
                &settings,
                "example.invalid",
                22,
                None,
                None,
                None,
                None,
                None,
            );
        });

        assert!(coordinator.bars.borrow().contains_key(&session_id));
        assert!(coordinator.handles.borrow().contains_key(&session_id));

        assert!(
            coordinator.detach_bar(session_id),
            "a bar existed, so detach_bar must report it was removed"
        );
        assert!(!coordinator.bars.borrow().contains_key(&session_id));
        assert!(!coordinator.handles.borrow().contains_key(&session_id));

        assert!(
            !coordinator.detach_bar(session_id),
            "second call on an already-detached session must be a no-op reported as such"
        );
    }

    /// `reset_for_reconnect` is a thin wrapper over `detach_bar`: it must
    /// return `false` with no bar present, and `true` after tearing one
    /// down — the signal `reconnect_ssh_in_place` uses to resolve where the
    /// restarted bar is allowed to go (issue #398, pass 3).
    #[test]
    #[ignore = "initialises GTK: needs a display and its own process; run alone with `cargo test -p rustconn --bin rustconn -- --ignored --exact monitoring::tests::reset_for_reconnect_reports_whether_a_bar_existed`"]
    fn reset_for_reconnect_reports_whether_a_bar_existed() {
        if gtk4::init().is_err() {
            return;
        }

        let coordinator = MonitoringCoordinator::new();
        let session_id = Uuid::new_v4();

        // No bar was ever started for this session (e.g. monitoring
        // suspended for split view, or disabled) — nothing to reset.
        assert!(!coordinator.reset_for_reconnect(session_id));

        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        let settings = MonitoringSettings::default();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            coordinator.start_monitoring(
                session_id,
                &container,
                &settings,
                "example.invalid",
                22,
                None,
                None,
                None,
                None,
                None,
            );
        });
        assert!(coordinator.bars.borrow().contains_key(&session_id));

        assert!(coordinator.reset_for_reconnect(session_id));
        assert!(!coordinator.bars.borrow().contains_key(&session_id));
        assert!(!coordinator.handles.borrow().contains_key(&session_id));
    }
}
