//! Remote host monitoring for SSH/Telnet/Kubernetes sessions
//!
//! Provides agentless system metrics collection by parsing `/proc/*` and `df`
//! output from remote Linux hosts. The monitoring bar displays CPU, memory,
//! disk, and network usage below the terminal.
//!
//! This module is GUI-free — it handles only data models, parsing, and the
//! shell command generation. The GTK widget lives in `rustconn/src/monitoring/`.

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
pub use settings::{MonitoringConfig, MonitoringSettings};
pub use ssh_exec::ssh_exec_factory;
