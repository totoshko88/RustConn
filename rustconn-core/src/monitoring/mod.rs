//! Remote host monitoring for SSH sessions
//!
//! Provides agentless system metrics collection by parsing `/proc/*` and `df`
//! output from remote Linux hosts. The monitoring bar displays CPU, memory,
//! disk, and network usage below the terminal. It shows metrics only: there are
//! no thresholds, alerts or actions.
//!
//! Only SSH sessions start a collector — the GUI calls it from the SSH connect
//! and reconnect paths alone, so Telnet, Kubernetes and the other protocols
//! never show the bar.
//!
//! This module is GUI-free — it handles only data models, parsing, and the
//! shell command generation. The GTK widget lives in `rustconn/src/monitoring.rs`.

pub mod collector;
mod metrics;
mod parser;
mod settings;
pub mod ssh_exec;

pub use collector::{CollectorHandle, MetricsComputer, MetricsEvent, start_collector};
pub use metrics::{
    CpuSnapshot, DiskMetrics, LoadAverage, MemoryMetrics, NetworkMetrics, NetworkSnapshot,
    RemoteMetrics, RemoteOsType, SystemInfo,
};
pub use parser::{
    METRICS_COMMAND, MetricsParser, MonitoringError, MonitoringResult, SYSTEM_INFO_COMMAND,
};
pub use settings::{
    MonitoringConfig, MonitoringOverride, MonitoringSettings, effective_monitoring,
    effective_monitoring_interval,
};
pub use ssh_exec::{
    close_all_control_sockets, close_control_socket, close_dead_control_sockets,
    control_socket_present, ssh_control_path, ssh_exec_factory,
};
