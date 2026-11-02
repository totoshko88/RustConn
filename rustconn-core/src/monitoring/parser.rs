//! Parser for remote host metrics output
//!
//! Parses the combined output of the monitoring shell command that reads
//! `/proc/stat`, `/proc/meminfo`, `/proc/net/dev`, and `df -Pk`.

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
    "df -Pk 2>/dev/null | tail -n +2;",
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
    "cat /etc/os-release 2>/dev/null; cat /etc/openwrt_release 2>/dev/null;",
    "echo '---RUSTCONN_UPTIME---';",
    "cat /proc/uptime;",
    "echo '---RUSTCONN_HWINFO---';",
    "grep '^MemTotal:' /proc/meminfo;",
    "echo 'CPUTHREADS='$(grep -c '^processor' /proc/cpuinfo);",
    "echo 'CPUCORES='$(grep '^cpu cores' /proc/cpuinfo | head -1 | awk '{print $NF}');",
    "echo 'ARCH='$(uname -m);",
    "echo '---RUSTCONN_NETWORK_ID---';",
    "hostname -f 2>/dev/null || hostname;",
    "echo '---RUSTCONN_IPS---';",
    "hostname -I 2>/dev/null || hostname -i 2>/dev/null;",
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
    /// Disk metrics for all mounted real filesystems
    pub disks: Vec<DiskMetrics>,
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
        let disks = Self::parse_df(output).unwrap_or_default();

        Ok(ParsedMetrics {
            cpu,
            memory,
            network,
            disks,
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

    /// Parses `df -Pk` output for all real filesystem metrics.
    ///
    /// Format per line: `Filesystem  1024-blocks  Used  Available  Capacity  Mounted`
    ///
    /// BusyBox `df` (OpenWrt, EdgeOS) has no `-x`, so pseudo-filesystems are
    /// filtered here instead of in the command. Snap loop devices (`/snap/`)
    /// are filtered out too. `/rom` is OpenWrt's read-only squashfs; the
    /// writable space is `/` (overlayfs) and `/overlay`.
    fn parse_df(output: &str) -> MonitoringResult<Vec<DiskMetrics>> {
        let section = Self::section(output, "---RUSTCONN_DF---", "---RUSTCONN_END---")
            .ok_or_else(|| MonitoringError::ParseError("Missing df section".into()))?;

        let mut disks = Vec::new();

        for line in section.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 6 {
                continue;
            }

            let mount_point = parts[5];

            if matches!(mount_point, "/dev" | "/proc" | "/sys" | "/run" | "/rom")
                || mount_point.starts_with("/dev/")
                || mount_point.starts_with("/proc/")
                || mount_point.starts_with("/sys/")
                || mount_point == "/run/lock"
                || mount_point.starts_with("/run/user/")
            {
                continue;
            }

            // Skip snap mounts (loop devices mounted at /snap/*)
            if mount_point.starts_with("/snap/") || mount_point.starts_with("/var/snap/") {
                continue;
            }

            let total_kib = parts[1].parse::<u64>().unwrap_or(0);
            let used_kib = parts[2].parse::<u64>().unwrap_or(0);
            let available_kib = parts[3].parse::<u64>().unwrap_or(0);

            // Skip zero-size filesystems
            if total_kib == 0 {
                continue;
            }

            disks.push(DiskMetrics {
                total_kib,
                used_kib,
                available_kib,
                mount_point: mount_point.to_string(),
            });
        }

        // Sort: root `/` first, then alphabetically by mount point
        disks.sort_by(|a, b| {
            if a.mount_point == "/" {
                std::cmp::Ordering::Less
            } else if b.mount_point == "/" {
                std::cmp::Ordering::Greater
            } else {
                a.mount_point.cmp(&b.mount_point)
            }
        });

        Ok(disks)
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

        let hostname = Self::section(output, "---RUSTCONN_NETWORK_ID---", "---RUSTCONN_IPS---")
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        let ip_addresses =
            Self::section(output, "---RUSTCONN_IPS---", "---RUSTCONN_SYSINFO_END---")
                .map(|s| s.split_whitespace().map(String::from).collect::<Vec<_>>())
                .unwrap_or_default();

        Ok(SystemInfo {
            kernel_version,
            distro_name,
            uptime_secs,
            total_ram_kib,
            cpu_cores,
            cpu_threads,
            arch,
            hostname,
            ip_addresses,
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
    /// to `DISTRIB_DESCRIPTION` from `/etc/openwrt_release`, then to
    /// `NAME` + `VERSION` if neither is present.
    fn extract_pretty_name(os_release: &str) -> Option<String> {
        let mut pretty_name = None;
        let mut name = None;
        let mut version = None;

        for line in os_release.lines() {
            if let Some(val) = line.strip_prefix("PRETTY_NAME=") {
                pretty_name = Some(val.trim_matches('"').to_string());
            } else if let Some(val) = line.strip_prefix("DISTRIB_DESCRIPTION=") {
                pretty_name
                    .get_or_insert_with(|| val.trim_matches(|c| c == '\'' || c == '"').to_string());
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
        assert_eq!(result.disks.len(), 1);
        assert_eq!(result.disks[0].total_kib, 102_400_000);
        assert_eq!(result.disks[0].used_kib, 51_200_000);
        assert_eq!(result.disks[0].mount_point, "/");
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
            mount_point: String::from("/"),
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
        // df failed gracefully — no disk metrics
        assert!(result.disks.is_empty());
    }

    #[test]
    fn test_parse_multiple_mount_points() {
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
/dev/sda1     102400000 51200000 46080000  53% /
/dev/sdb1     204800000 10240000 184320000   5% /home
/dev/sdc1      51200000 40960000  5120000  89% /var
/dev/loop0       131072   131072        0 100% /snap/core/12345
---RUSTCONN_END---
";
        let result = MetricsParser::parse(output).unwrap();
        // 3 real mounts (snap filtered out)
        assert_eq!(result.disks.len(), 3);
        // Root comes first
        assert_eq!(result.disks[0].mount_point, "/");
        assert_eq!(result.disks[0].total_kib, 102_400_000);
        // Then alphabetical
        assert_eq!(result.disks[1].mount_point, "/home");
        assert_eq!(result.disks[1].total_kib, 204_800_000);
        assert_eq!(result.disks[2].mount_point, "/var");
        assert_eq!(result.disks[2].used_kib, 40_960_000);
    }

    #[test]
    fn test_parse_openwrt_busybox_df() {
        let output = "\
---RUSTCONN_PROC_STAT---
cpu  100 0 50 800 50 0 0 0 0 0
---RUSTCONN_MEMINFO---
MemTotal:         126708 kB
MemAvailable:      80000 kB
---RUSTCONN_LOADAVG---
0.00 0.00 0.00 1/100 1234
---RUSTCONN_NET_DEV---
  br-lan: 1000 10 0 0 0 0 0 0 500 8 0 0 0 0 0 0
---RUSTCONN_DF---
/dev/root                  5632    5632         0     100% /rom
tmpfs                    126708      92    126616       0% /tmp
/dev/mtdblock5            14608    1240     13368       8% /overlay
overlayfs:/overlay        14608    1240     13368       8% /
---RUSTCONN_END---
";
        let result = MetricsParser::parse(output).unwrap();

        // The writable overlay is the root filesystem, and it comes first.
        assert_eq!(result.disks[0].mount_point, "/");
        assert_eq!(result.disks[0].total_kib, 14_608);
        assert_eq!(result.disks[0].available_kib, 13_368);
        // The read-only squashfs is not a free-space metric.
        assert!(result.disks.iter().all(|d| d.mount_point != "/rom"));
        assert!(result.disks.iter().any(|d| d.mount_point == "/overlay"));
    }

    #[test]
    fn test_parse_system_info_openwrt_release() {
        let output = "\
---RUSTCONN_UNAME---
5.15.137
---RUSTCONN_OSRELEASE---
DISTRIB_ID='OpenWrt'
DISTRIB_RELEASE='23.05.3'
DISTRIB_DESCRIPTION='OpenWrt 23.05.3 r23809-234f1a2efa'
---RUSTCONN_UPTIME---
12345.67 23456.78
---RUSTCONN_HWINFO---
MemTotal:         126708 kB
CPUTHREADS=2
CPUCORES=
ARCH=mips
---RUSTCONN_NETWORK_ID---
OpenWrt.lan
---RUSTCONN_IPS---
192.168.1.1
---RUSTCONN_SYSINFO_END---
";
        let info = MetricsParser::parse_system_info(output).unwrap();
        assert_eq!(info.distro_name, "OpenWrt 23.05.3 r23809-234f1a2efa");
        assert_eq!(info.cpu_cores, 2); // falls back to threads
        assert_eq!(info.cpu_threads, 2);
        assert_eq!(info.arch, "mips");
        assert_eq!(info.hostname, "OpenWrt.lan");
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
---RUSTCONN_NETWORK_ID---
server01.example.com
---RUSTCONN_IPS---
10.0.1.5 192.168.1.100 fd12::1
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
        assert_eq!(info.hostname, "server01.example.com");
        assert_eq!(
            info.ip_addresses,
            vec!["10.0.1.5", "192.168.1.100", "fd12::1"]
        );
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
---RUSTCONN_NETWORK_ID---
archbox
---RUSTCONN_IPS---
172.16.0.10
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
        assert_eq!(info.hostname, "archbox");
        assert_eq!(info.ip_addresses, vec!["172.16.0.10"]);
    }

    #[test]
    fn test_parse_system_info_no_network_sections() {
        // Legacy output without network sections — should gracefully default
        let output = "\
---RUSTCONN_UNAME---
6.1.0
---RUSTCONN_OSRELEASE---
PRETTY_NAME=\"Debian 12\"
---RUSTCONN_UPTIME---
600.00 1200.00
---RUSTCONN_HWINFO---
MemTotal:        4096000 kB
CPUTHREADS=2
CPUCORES=2
ARCH=x86_64
---RUSTCONN_SYSINFO_END---
";
        let info = MetricsParser::parse_system_info(output).unwrap();
        assert_eq!(info.kernel_version, "6.1.0");
        assert_eq!(info.hostname, "");
        assert!(info.ip_addresses.is_empty());
    }
}
