//! Parser for remote host metrics output
//!
//! Parses the combined output of the monitoring shell command that reads
//! `/proc/stat`, `/proc/meminfo`, `/proc/net/dev`, and `df -Pk /`.

use super::metrics::{
    CpuSnapshot, DiskMetrics, LoadAverage, MemoryMetrics, NetworkSnapshot, RemoteOsType, SystemInfo,
};

/// Errors that can occur during metrics parsing
#[derive(Debug, thiserror::Error)]
pub enum MonitoringError {
    /// The remote output could not be parsed
    #[error("Failed to parse monitoring output: {0}")]
    ParseError(String),
    /// The remote host OS is not supported
    #[error("Unsupported remote OS")]
    UnsupportedOs,
}

/// Result type for monitoring operations
pub type MonitoringResult<T> = Result<T, MonitoringError>;

/// Shell command that collects all metrics in a single invocation.
///
/// The output is delimited by marker lines so the parser can split sections
/// reliably even if individual commands produce unexpected output.
pub const METRICS_COMMAND: &str = concat!(
    "echo '---RUSTCONN_PROC_STAT---';",
    "head -1 /proc/stat;",
    "echo '---RUSTCONN_MEMINFO---';",
    "grep -E '^(MemTotal|MemAvailable|SwapTotal|SwapFree):' /proc/meminfo;",
    "echo '---RUSTCONN_LOADAVG---';",
    "cat /proc/loadavg;",
    "echo '---RUSTCONN_NET_DEV---';",
    "tail -n +3 /proc/net/dev;",
    "echo '---RUSTCONN_DF---';",
    "df -Pk / 2>/dev/null | tail -1;",
    "echo '---RUSTCONN_END---'",
);

/// Shell command that collects static system information (run once).
///
/// Fetches kernel version, distribution name, uptime, total RAM,
/// CPU cores/threads, and architecture.
pub const SYSTEM_INFO_COMMAND: &str = concat!(
    "echo '---RUSTCONN_UNAME---';",
    "uname -r;",
    "echo '---RUSTCONN_OSRELEASE---';",
    "cat /etc/os-release 2>/dev/null;",
    "echo '---RUSTCONN_UPTIME---';",
    "cat /proc/uptime;",
    "echo '---RUSTCONN_HWINFO---';",
    "grep '^MemTotal:' /proc/meminfo;",
    "echo 'CPUTHREADS='$(grep -c '^processor' /proc/cpuinfo);",
    "echo 'CPUCORES='$(grep '^cpu cores' /proc/cpuinfo | head -1 | awk '{print $NF}');",
    "echo 'ARCH='$(uname -m);",
    "echo '---RUSTCONN_SYSINFO_END---'",
);

/// Parsed raw output from a single metrics collection
#[derive(Debug, Clone)]
pub struct ParsedMetrics {
    /// CPU counters (for delta calculation)
    pub cpu: CpuSnapshot,
    /// Memory metrics (absolute, no delta needed)
    pub memory: MemoryMetrics,
    /// Network counters (for delta calculation)
    pub network: NetworkSnapshot,
    /// Disk metrics (absolute, no delta needed)
    pub disk: DiskMetrics,
    /// Load average from `/proc/loadavg`
    pub load_average: LoadAverage,
    /// Detected OS type
    pub os_type: RemoteOsType,
}

/// Stateless parser for remote metrics output
pub struct MetricsParser;

impl MetricsParser {
    /// Parses the combined output of [`METRICS_COMMAND`].
    ///
    /// # Errors
    ///
    /// Returns [`MonitoringError::ParseError`] if any section is missing
    /// or contains unparseable data.
    pub fn parse(output: &str) -> MonitoringResult<ParsedMetrics> {
        let cpu = Self::parse_proc_stat(output)?;
        let memory = Self::parse_meminfo(output)?;
        let load_average = Self::parse_loadavg(output).unwrap_or_default();
        let network = Self::parse_net_dev(output)?;
        let disk = Self::parse_df(output).unwrap_or(DiskMetrics {
            total_kib: 0,
            used_kib: 0,
            available_kib: 0,
        });

        Ok(ParsedMetrics {
            cpu,
            memory,
            network,
            disk,
            load_average,
            os_type: RemoteOsType::Linux,
        })
    }

    /// Extracts text between two marker lines
    fn section<'a>(output: &'a str, start: &str, end: &str) -> Option<&'a str> {
        let start_idx = output.find(start).map(|i| i + start.len())?;
        let end_idx = output[start_idx..].find(end).map(|i| start_idx + i)?;
        Some(output[start_idx..end_idx].trim())
    }

    /// Parses the first line of `/proc/stat` into a [`CpuSnapshot`].
    ///
    /// Format: `cpu  user nice system idle iowait irq softirq steal ...`
    fn parse_proc_stat(output: &str) -> MonitoringResult<CpuSnapshot> {
        let section =
            Self::section(output, "---RUSTCONN_PROC_STAT---", "---RUSTCONN_MEMINFO---")
                .ok_or_else(|| MonitoringError::ParseError("Missing /proc/stat section".into()))?;

        let line = section
            .lines()
            .find(|l| l.starts_with("cpu "))
            .ok_or_else(|| {
                MonitoringError::ParseError("No aggregate cpu line in /proc/stat".into())
            })?;

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 9 {
            return Err(MonitoringError::ParseError(
                "Too few fields in /proc/stat cpu line".into(),
            ));
        }

        let p = |i: usize| -> u64 { parts[i].parse().unwrap_or(0) };

        Ok(CpuSnapshot {
            user: p(1),
            nice: p(2),
            system: p(3),
            idle: p(4),
            iowait: p(5),
            irq: p(6),
            softirq: p(7),
            steal: p(8),
        })
    }

    /// Parses `MemTotal`, `MemAvailable`, `SwapTotal`, `SwapFree` from `/proc/meminfo`.
    fn parse_meminfo(output: &str) -> MonitoringResult<MemoryMetrics> {
        let section = Self::section(output, "---RUSTCONN_MEMINFO---", "---RUSTCONN_LOADAVG---")
            .ok_or_else(|| MonitoringError::ParseError("Missing /proc/meminfo section".into()))?;

        let mut total_kib: u64 = 0;
        let mut available_kib: u64 = 0;
        let mut swap_total_kib: u64 = 0;
        let mut swap_free_kib: u64 = 0;

        for line in section.lines() {
            if let Some(rest) = line.strip_prefix("MemTotal:") {
                total_kib = Self::parse_kib_value(rest);
            } else if let Some(rest) = line.strip_prefix("MemAvailable:") {
                available_kib = Self::parse_kib_value(rest);
            } else if let Some(rest) = line.strip_prefix("SwapTotal:") {
                swap_total_kib = Self::parse_kib_value(rest);
            } else if let Some(rest) = line.strip_prefix("SwapFree:") {
                swap_free_kib = Self::parse_kib_value(rest);
            }
        }

        if total_kib == 0 {
            return Err(MonitoringError::ParseError(
                "MemTotal not found in /proc/meminfo".into(),
            ));
        }

        Ok(MemoryMetrics {
            total_kib,
            used_kib: total_kib.saturating_sub(available_kib),
            available_kib,
            swap_total_kib,
            swap_used_kib: swap_total_kib.saturating_sub(swap_free_kib),
        })
    }

    /// Parses a value like `  16384000 kB` into KiB
    fn parse_kib_value(s: &str) -> u64 {
        s.split_whitespace()
            .next()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    }

    /// Parses `/proc/net/dev` and sums rx/tx bytes across all non-lo interfaces.
    fn parse_net_dev(output: &str) -> MonitoringResult<NetworkSnapshot> {
        let section = Self::section(output, "---RUSTCONN_NET_DEV---", "---RUSTCONN_DF---")
            .ok_or_else(|| MonitoringError::ParseError("Missing /proc/net/dev section".into()))?;

        let mut rx_bytes: u64 = 0;
        let mut tx_bytes: u64 = 0;

        for line in section.lines() {
            let line = line.trim();
            // Skip loopback
            if line.starts_with("lo:") {
                continue;
            }
            // Format: iface: rx_bytes rx_packets ... tx_bytes tx_packets ...
            if let Some((_iface, stats)) = line.split_once(':') {
                let parts: Vec<&str> = stats.split_whitespace().collect();
                if parts.len() >= 9 {
                    rx_bytes += parts[0].parse::<u64>().unwrap_or(0);
                    tx_bytes += parts[8].parse::<u64>().unwrap_or(0);
                }
            }
        }

        Ok(NetworkSnapshot { rx_bytes, tx_bytes })
    }

    /// Parses `df -Pk /` output for root filesystem metrics.
    ///
    /// Format: `Filesystem  1024-blocks  Used  Available  Capacity  Mounted`
    fn parse_df(output: &str) -> MonitoringResult<DiskMetrics> {
        let section = Self::section(output, "---RUSTCONN_DF---", "---RUSTCONN_END---")
            .ok_or_else(|| MonitoringError::ParseError("Missing df section".into()))?;

        let line = section
            .lines()
            .next()
            .ok_or_else(|| MonitoringError::ParseError("Empty df output".into()))?;

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 4 {
            return Err(MonitoringError::ParseError(
                "Too few fields in df output".into(),
            ));
        }

        let total_kib = parts[1].parse::<u64>().unwrap_or(0);
        let used_kib = parts[2].parse::<u64>().unwrap_or(0);
        let available_kib = parts[3].parse::<u64>().unwrap_or(0);

        Ok(DiskMetrics {
            total_kib,
            used_kib,
            available_kib,
        })
    }

    /// Parses `/proc/loadavg` for load averages and process counts.
    ///
    /// Format: `0.52 0.34 0.28 2/1234 56789`
    fn parse_loadavg(output: &str) -> MonitoringResult<LoadAverage> {
        let section = Self::section(output, "---RUSTCONN_LOADAVG---", "---RUSTCONN_NET_DEV---")
            .ok_or_else(|| MonitoringError::ParseError("Missing /proc/loadavg section".into()))?;

        let line = section.lines().next().unwrap_or("");
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 4 {
            return Err(MonitoringError::ParseError(
                "Too few fields in /proc/loadavg".into(),
            ));
        }

        let (running_procs, total_procs) = parts[3]
            .split_once('/')
            .map(|(r, t)| (r.parse().unwrap_or(0), t.parse().unwrap_or(0)))
            .unwrap_or((0, 0));

        Ok(LoadAverage {
            one: parts[0].parse().unwrap_or(0.0),
            five: parts[1].parse().unwrap_or(0.0),
            fifteen: parts[2].parse().unwrap_or(0.0),
            running_procs,
            total_procs,
        })
    }

    /// Parses the output of [`SYSTEM_INFO_COMMAND`] into [`SystemInfo`].
    ///
    /// # Errors
    ///
    /// Returns [`MonitoringError::ParseError`] if the output is missing
    /// required sections.
    pub fn parse_system_info(output: &str) -> MonitoringResult<SystemInfo> {
        let kernel_version =
            Self::section(output, "---RUSTCONN_UNAME---", "---RUSTCONN_OSRELEASE---")
                .map(|s| s.trim().to_string())
                .unwrap_or_default();

        let distro_name =
            Self::section(output, "---RUSTCONN_OSRELEASE---", "---RUSTCONN_UPTIME---")
                .and_then(Self::extract_pretty_name)
                .unwrap_or_default();

        let uptime_secs = Self::section(output, "---RUSTCONN_UPTIME---", "---RUSTCONN_HWINFO---")
            .and_then(|s| {
                s.split_whitespace()
                    .next()
                    .and_then(|v| v.parse::<f64>().ok())
            })
            .map(|v| v as u64)
            .unwrap_or(0);

        let hwinfo = Self::section(
            output,
            "---RUSTCONN_HWINFO---",
            "---RUSTCONN_SYSINFO_END---",
        )
        .unwrap_or("");

        let total_ram_kib = hwinfo
            .lines()
            .find(|l| l.starts_with("MemTotal:"))
            .map(|l| Self::parse_kib_value(l.trim_start_matches("MemTotal:")))
            .unwrap_or(0);

        let cpu_threads = Self::extract_hwinfo_u16(hwinfo, "CPUTHREADS=");
        let cpu_cores = Self::extract_hwinfo_u16(hwinfo, "CPUCORES=");
        // Fallback: if cpu cores not reported (single-core or missing), use threads
        let cpu_cores = if cpu_cores == 0 {
            cpu_threads
        } else {
            cpu_cores
        };

        let arch = hwinfo
            .lines()
            .find_map(|l| l.strip_prefix("ARCH="))
            .unwrap_or("")
            .trim()
            .to_string();

        Ok(SystemInfo {
            kernel_version,
            distro_name,
            uptime_secs,
            total_ram_kib,
            cpu_cores,
            cpu_threads,
            arch,
        })
    }

    /// Extracts a `u16` value from a `KEY=value` line in hardware info output
    fn extract_hwinfo_u16(hwinfo: &str, prefix: &str) -> u16 {
        hwinfo
            .lines()
            .find_map(|l| l.strip_prefix(prefix))
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0)
    }

    /// Extracts `PRETTY_NAME` from `/etc/os-release` content, falling back
    /// to `NAME` + `VERSION` if `PRETTY_NAME` is absent.
    fn extract_pretty_name(os_release: &str) -> Option<String> {
        let mut pretty_name = None;
        let mut name = None;
        let mut version = None;

        for line in os_release.lines() {
            if let Some(val) = line.strip_prefix("PRETTY_NAME=") {
                pretty_name = Some(val.trim_matches('"').to_string());
            } else if let Some(val) = line.strip_prefix("NAME=") {
                name = Some(val.trim_matches('"').to_string());
            } else if let Some(val) = line.strip_prefix("VERSION=") {
                version = Some(val.trim_matches('"').to_string());
            }
        }

        pretty_name
            .or_else(|| name.map(|n| version.map_or_else(|| n.clone(), |v| format!("{n} {v}"))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_OUTPUT: &str = "\
---RUSTCONN_PROC_STAT---
cpu  10132153 290696 3084719 46828483 16683 0 25195 0 0 0
---RUSTCONN_MEMINFO---
MemTotal:       16384000 kB
MemAvailable:    8192000 kB
SwapTotal:       4096000 kB
SwapFree:        3072000 kB
---RUSTCONN_LOADAVG---
0.52 0.34 0.28 3/1234 56789
---RUSTCONN_NET_DEV---
  eth0: 1000000    1000    0    0    0     0          0         0  500000    800    0    0    0     0       0          0
    lo:  200000     500    0    0    0     0          0         0  200000    500    0    0    0     0       0          0
---RUSTCONN_DF---
/dev/sda1     102400000 51200000 46080000  53% /
---RUSTCONN_END---
";

    #[test]
    fn test_parse_full_output() {
        let result = MetricsParser::parse(SAMPLE_OUTPUT).unwrap();

        assert_eq!(result.cpu.user, 10_132_153);
        assert_eq!(result.cpu.idle, 46_828_483);
        assert_eq!(result.memory.total_kib, 16_384_000);
        assert_eq!(result.memory.available_kib, 8_192_000);
        assert_eq!(result.memory.used_kib, 8_192_000);
        assert_eq!(result.memory.swap_total_kib, 4_096_000);
        assert_eq!(result.memory.swap_used_kib, 1_024_000);
        assert_eq!(result.network.rx_bytes, 1_000_000); // eth0 only, not lo
        assert_eq!(result.network.tx_bytes, 500_000);
        assert_eq!(result.disk.total_kib, 102_400_000);
        assert_eq!(result.disk.used_kib, 51_200_000);
        assert_eq!(result.os_type, RemoteOsType::Linux);
        assert!((result.load_average.one - 0.52).abs() < 0.01);
        assert!((result.load_average.five - 0.34).abs() < 0.01);
        assert!((result.load_average.fifteen - 0.28).abs() < 0.01);
        assert_eq!(result.load_average.running_procs, 3);
        assert_eq!(result.load_average.total_procs, 1234);
    }

    #[test]
    fn test_cpu_percent_calculation() {
        let prev = CpuSnapshot {
            user: 100,
            nice: 0,
            system: 50,
            idle: 800,
            iowait: 50,
            irq: 0,
            softirq: 0,
            steal: 0,
        };
        let curr = CpuSnapshot {
            user: 200,
            nice: 0,
            system: 100,
            idle: 1600,
            iowait: 100,
            irq: 0,
            softirq: 0,
            steal: 0,
        };
        // total delta = 1000, idle delta = 850, busy = 150
        let pct = curr.cpu_percent_since(&prev);
        assert!((pct - 15.0).abs() < 0.1);
    }

    #[test]
    fn test_memory_percent() {
        let mem = MemoryMetrics {
            total_kib: 16_000_000,
            used_kib: 8_000_000,
            available_kib: 8_000_000,
            swap_total_kib: 4_000_000,
            swap_used_kib: 1_000_000,
        };
        assert!((mem.percent() - 50.0).abs() < 0.1);
        assert!((mem.swap_percent() - 25.0).abs() < 0.1);
    }

    #[test]
    fn test_disk_percent() {
        let disk = DiskMetrics {
            total_kib: 100_000,
            used_kib: 75_000,
            available_kib: 25_000,
        };
        assert!((disk.percent() - 75.0).abs() < 0.1);
    }

    #[test]
    fn test_parse_missing_section() {
        let result = MetricsParser::parse("garbage output");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_graceful_df_failure() {
        let output = "\
---RUSTCONN_PROC_STAT---
cpu  100 0 50 800 50 0 0 0 0 0
---RUSTCONN_MEMINFO---
MemTotal:       16384000 kB
MemAvailable:    8192000 kB
---RUSTCONN_LOADAVG---
0.00 0.00 0.00 1/100 1234
---RUSTCONN_NET_DEV---
  eth0: 1000 10 0 0 0 0 0 0 500 8 0 0 0 0 0 0
---RUSTCONN_DF---
---RUSTCONN_END---
";
        let result = MetricsParser::parse(output).unwrap();
        // df failed gracefully — disk metrics are zero
        assert_eq!(result.disk.total_kib, 0);
    }

    #[test]
    fn test_parse_system_info() {
        let output = "\
---RUSTCONN_UNAME---
6.8.0-45-generic
---RUSTCONN_OSRELEASE---
NAME=\"Ubuntu\"
VERSION=\"24.04.1 LTS (Noble Numbat)\"
PRETTY_NAME=\"Ubuntu 24.04.1 LTS\"
ID=ubuntu
---RUSTCONN_UPTIME---
123456.78 234567.89
---RUSTCONN_HWINFO---
MemTotal:       16384000 kB
CPUTHREADS=16
CPUCORES=8
ARCH=x86_64
---RUSTCONN_SYSINFO_END---
";
        let info = MetricsParser::parse_system_info(output).unwrap();
        assert_eq!(info.kernel_version, "6.8.0-45-generic");
        assert_eq!(info.distro_name, "Ubuntu 24.04.1 LTS");
        assert_eq!(info.uptime_secs, 123_456);
        assert_eq!(info.total_ram_kib, 16_384_000);
        assert_eq!(info.cpu_cores, 8);
        assert_eq!(info.cpu_threads, 16);
        assert_eq!(info.arch, "x86_64");
    }

    #[test]
    fn test_parse_system_info_fallback_name() {
        let output = "\
---RUSTCONN_UNAME---
5.15.0-generic
---RUSTCONN_OSRELEASE---
NAME=\"Arch Linux\"
ID=arch
---RUSTCONN_UPTIME---
3600.00 7200.00
---RUSTCONN_HWINFO---
MemTotal:        8192000 kB
CPUTHREADS=4
CPUCORES=
ARCH=aarch64
---RUSTCONN_SYSINFO_END---
";
        let info = MetricsParser::parse_system_info(output).unwrap();
        assert_eq!(info.distro_name, "Arch Linux");
        assert_eq!(info.uptime_secs, 3600);
        assert_eq!(info.total_ram_kib, 8_192_000);
        // cpu_cores falls back to cpu_threads when empty
        assert_eq!(info.cpu_cores, 4);
        assert_eq!(info.cpu_threads, 4);
        assert_eq!(info.arch, "aarch64");
    }
}
