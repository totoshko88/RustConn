//! Data models for remote host metrics
//!
//! All types are GUI-free and serializable for potential future use
//! (e.g. metrics history, export).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Detected remote operating system type
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemoteOsType {
    /// Linux (reads `/proc/*`)
    Linux,
    /// Unknown or unsupported OS
    #[default]
    Unknown,
}

/// A snapshot of remote host metrics at a point in time
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteMetrics {
    /// CPU usage as a percentage (0.0–100.0)
    pub cpu_percent: f32,
    /// Memory metrics
    pub memory: MemoryMetrics,
    /// Root filesystem disk metrics
    pub disk: DiskMetrics,
    /// Network throughput metrics
    pub network: NetworkMetrics,
    /// When these metrics were collected
    pub timestamp: DateTime<Utc>,
    /// Detected OS type
    pub os_type: RemoteOsType,
    /// Load average (1, 5, 15 min)
    pub load_average: LoadAverage,
}

/// Memory usage metrics in kibibytes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryMetrics {
    /// Total physical memory (KiB)
    pub total_kib: u64,
    /// Used memory (KiB) — total minus available
    pub used_kib: u64,
    /// Available memory (KiB) — includes reclaimable caches
    pub available_kib: u64,
    /// Total swap space (KiB)
    pub swap_total_kib: u64,
    /// Used swap space (KiB)
    pub swap_used_kib: u64,
}

impl MemoryMetrics {
    /// Returns memory usage as a percentage (0.0–100.0)
    #[must_use]
    pub fn percent(&self) -> f32 {
        if self.total_kib == 0 {
            return 0.0;
        }
        (self.used_kib as f32 / self.total_kib as f32) * 100.0
    }

    /// Returns swap usage as a percentage (0.0–100.0), or 0 if no swap
    #[must_use]
    pub fn swap_percent(&self) -> f32 {
        if self.swap_total_kib == 0 {
            return 0.0;
        }
        (self.swap_used_kib as f32 / self.swap_total_kib as f32) * 100.0
    }
}

/// Disk usage metrics for the root filesystem
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskMetrics {
    /// Total disk space (KiB)
    pub total_kib: u64,
    /// Used disk space (KiB)
    pub used_kib: u64,
    /// Available disk space (KiB)
    pub available_kib: u64,
}

impl DiskMetrics {
    /// Returns disk usage as a percentage (0.0–100.0)
    #[must_use]
    pub fn percent(&self) -> f32 {
        if self.total_kib == 0 {
            return 0.0;
        }
        (self.used_kib as f32 / self.total_kib as f32) * 100.0
    }
}

/// Network throughput metrics (bytes per second)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NetworkMetrics {
    /// Receive rate in bytes per second
    pub rx_bytes_per_sec: f64,
    /// Transmit rate in bytes per second
    pub tx_bytes_per_sec: f64,
}

/// Raw CPU counters from `/proc/stat` for delta calculation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CpuSnapshot {
    /// Total user time (jiffies)
    pub user: u64,
    /// Total nice time (jiffies)
    pub nice: u64,
    /// Total system time (jiffies)
    pub system: u64,
    /// Total idle time (jiffies)
    pub idle: u64,
    /// Total iowait time (jiffies)
    pub iowait: u64,
    /// Total irq time (jiffies)
    pub irq: u64,
    /// Total softirq time (jiffies)
    pub softirq: u64,
    /// Total steal time (jiffies)
    pub steal: u64,
}

impl CpuSnapshot {
    /// Total jiffies across all states
    #[must_use]
    pub fn total(&self) -> u64 {
        self.user
            + self.nice
            + self.system
            + self.idle
            + self.iowait
            + self.irq
            + self.softirq
            + self.steal
    }

    /// Total idle jiffies (idle + iowait)
    #[must_use]
    pub fn idle_total(&self) -> u64 {
        self.idle + self.iowait
    }

    /// Calculates CPU usage percentage between two snapshots
    #[must_use]
    pub fn cpu_percent_since(&self, prev: &Self) -> f32 {
        let total_delta = self.total().saturating_sub(prev.total());
        if total_delta == 0 {
            return 0.0;
        }
        let idle_delta = self.idle_total().saturating_sub(prev.idle_total());
        let busy_delta = total_delta.saturating_sub(idle_delta);
        (busy_delta as f32 / total_delta as f32) * 100.0
    }
}

/// Raw network counters from `/proc/net/dev` for delta calculation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NetworkSnapshot {
    /// Total received bytes
    pub rx_bytes: u64,
    /// Total transmitted bytes
    pub tx_bytes: u64,
}

/// Load average values from `/proc/loadavg`
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LoadAverage {
    /// 1-minute load average
    pub one: f32,
    /// 5-minute load average
    pub five: f32,
    /// 15-minute load average
    pub fifteen: f32,
    /// Number of currently running processes
    pub running_procs: u32,
    /// Total number of processes
    pub total_procs: u32,
}

/// Static system information collected once at monitoring start.
///
/// These values don't change between polling intervals, so they are
/// fetched once and cached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemInfo {
    /// Kernel version (e.g. "6.8.0-45-generic")
    pub kernel_version: String,
    /// Distribution name (e.g. "Ubuntu 24.04.1 LTS")
    pub distro_name: String,
    /// System uptime in seconds
    pub uptime_secs: u64,
    /// Total physical RAM in KiB
    pub total_ram_kib: u64,
    /// Number of physical CPU cores
    pub cpu_cores: u16,
    /// Number of logical CPU threads (hyperthreading)
    pub cpu_threads: u16,
    /// CPU architecture (e.g. "x86_64", "aarch64")
    pub arch: String,
}
