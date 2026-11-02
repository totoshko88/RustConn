//! Metrics collector that runs periodic polling via a shell command
//!
//! The collector sends [`super::parser::METRICS_COMMAND`] through a callback, parses the
//! output, computes deltas for CPU and network, and emits [`RemoteMetrics`].

use std::time::Duration;

use chrono::Utc;
use tokio::sync::mpsc;

use super::metrics::{
    CpuSnapshot, DiskMetrics, NetworkMetrics, NetworkSnapshot, RemoteMetrics, SystemInfo,
};
use super::parser::{MetricsParser, MonitoringError, ParsedMetrics};
use super::settings::MonitoringSettings;

/// Maximum consecutive errors before the collector gives up
const MAX_CONSECUTIVE_ERRORS: u32 = 3;

/// Events emitted by the metrics collector
#[derive(Debug, Clone)]
pub enum MetricsEvent {
    /// New metrics snapshot available
    Update(RemoteMetrics),
    /// Static system information collected once at start
    SystemInfoReady(SystemInfo),
    /// Collector encountered a parse error (non-fatal, will retry)
    ParseError(String),
    /// Collector stopped
    Stopped,
}

/// Handle to control a running collector
#[derive(Debug)]
pub struct CollectorHandle {
    /// Send to stop the collector
    stop_tx: mpsc::Sender<()>,
}

impl CollectorHandle {
    /// Signals the collector to stop
    pub async fn stop(&self) {
        let _ = self.stop_tx.send(()).await;
    }
}

/// Computes delta-based metrics from two consecutive parsed snapshots
pub struct MetricsComputer {
    cpu: Option<CpuSnapshot>,
    net: Option<NetworkSnapshot>,
    sampled_at: Option<std::time::Instant>,
}

impl MetricsComputer {
    /// Creates a new computer with no previous state
    #[must_use]
    pub const fn new() -> Self {
        Self {
            cpu: None,
            net: None,
            sampled_at: None,
        }
    }

    /// Computes [`RemoteMetrics`] from a parsed snapshot
    ///
    /// The first call returns 0% CPU and 0 B/s network (no delta yet).
    #[must_use]
    pub fn compute(&mut self, parsed: &ParsedMetrics) -> RemoteMetrics {
        let now = std::time::Instant::now();

        // CPU: a source that reports a percentage directly (RouterOS) uses it
        // as-is; otherwise it is the delta between two `/proc/stat` snapshots.
        let cpu_percent = parsed.cpu_percent_direct.unwrap_or_else(|| {
            self.cpu
                .as_ref()
                .map_or(0.0, |prev| parsed.cpu.cpu_percent_since(prev))
        });

        // Network: bytes/sec from delta
        let network =
            if let (Some(prev_net), Some(prev_time)) = (self.net.as_ref(), self.sampled_at) {
                let elapsed = now.duration_since(prev_time).as_secs_f64();
                if elapsed > 0.0 {
                    let rx_delta = parsed.network.rx_bytes.saturating_sub(prev_net.rx_bytes);
                    let tx_delta = parsed.network.tx_bytes.saturating_sub(prev_net.tx_bytes);
                    NetworkMetrics {
                        rx_bytes_per_sec: rx_delta as f64 / elapsed,
                        tx_bytes_per_sec: tx_delta as f64 / elapsed,
                    }
                } else {
                    NetworkMetrics {
                        rx_bytes_per_sec: 0.0,
                        tx_bytes_per_sec: 0.0,
                    }
                }
            } else {
                NetworkMetrics {
                    rx_bytes_per_sec: 0.0,
                    tx_bytes_per_sec: 0.0,
                }
            };

        // Store current as previous for next delta
        self.cpu = Some(parsed.cpu);
        self.net = Some(parsed.network);
        self.sampled_at = Some(now);

        RemoteMetrics {
            cpu_percent,
            memory: parsed.memory,
            disk: parsed
                .disks
                .first()
                .cloned()
                .unwrap_or_else(|| DiskMetrics {
                    total_kib: 0,
                    used_kib: 0,
                    available_kib: 0,
                    mount_point: String::from("/"),
                }),
            disks: parsed.disks.clone(),
            network,
            timestamp: Utc::now(),
            os_type: parsed.os_type,
            load_average: parsed.load_average,
        }
    }

    /// Resets the computer state (e.g. on reconnect)
    pub fn reset(&mut self) {
        self.cpu = None;
        self.net = None;
        self.sampled_at = None;
    }
}

impl Default for MetricsComputer {
    fn default() -> Self {
        Self::new()
    }
}

/// Starts a metrics collection loop.
///
/// The `exec_command` callback sends the shell command to the remote host
/// and returns the output. This abstraction allows the collector to work
/// with any transport (SSH channel, exec, etc.).
///
/// Returns a handle to stop the collector and a receiver for events.
#[expect(
    clippy::needless_pass_by_value,
    clippy::too_many_lines,
    reason = "settings is consumed by value to move into the spawned loop; the long body is a single sequential collection loop"
)]
pub fn start_collector<F, Fut>(
    settings: MonitoringSettings,
    exec_command: F,
) -> (CollectorHandle, mpsc::Receiver<MetricsEvent>)
where
    F: Fn(String) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<String, String>> + Send,
{
    let (stop_tx, mut stop_rx) = mpsc::channel::<()>(1);
    let (event_tx, event_rx) = mpsc::channel::<MetricsEvent>(8);

    let interval = Duration::from_secs(u64::from(settings.effective_interval_secs()));
    let command = super::parser::METRICS_COMMAND.to_string();
    let sysinfo_command = super::parser::SYSTEM_INFO_COMMAND.to_string();

    let ros_command = super::parser::ROUTEROS_METRICS_COMMAND.to_string();
    let ros_sysinfo_command = super::parser::ROUTEROS_SYSINFO_COMMAND.to_string();

    tokio::spawn(async move {
        let mut computer = MetricsComputer::new();
        let mut ticker = tokio::time::interval(interval);
        let mut sysinfo_fetched = false;
        let mut consecutive_errors: u32 = 0;
        // Default to Linux; flipped to RouterOS the first time the Linux metrics
        // command comes back as "bad command name" (RouterOS' own console). Once
        // detected it sticks for the session, so the probe cost is paid once.
        let mut remote_os = super::metrics::RemoteOsType::Linux;

        loop {
            tokio::select! {
                _ = stop_rx.recv() => {
                    let _ = event_tx.send(MetricsEvent::Stopped).await;
                    break;
                }
                _ = ticker.tick() => {
                    // Fetch static system info once on first tick, using whichever
                    // command set matches the detected OS. On the first tick the OS
                    // may still be Linux-by-default; if detection later flips it to
                    // RouterOS, sysinfo is re-fetched with the RouterOS command.
                    if !sysinfo_fetched && remote_os == super::metrics::RemoteOsType::Linux {
                        sysinfo_fetched = true;
                        if let Ok(output) = exec_command(sysinfo_command.clone()).await
                            && let Ok(info) = MetricsParser::parse_system_info(&output)
                        {
                            let _ = event_tx
                                .send(MetricsEvent::SystemInfoReady(info))
                                .await;
                        }
                    }

                    let metrics_cmd = if remote_os == super::metrics::RemoteOsType::RouterOs {
                        ros_command.clone()
                    } else {
                        command.clone()
                    };
                    match exec_command(metrics_cmd).await {
                        Ok(output) => {
                            // RouterOS detection: the Linux command returns
                            // "bad command name". Switch the command set, fetch
                            // RouterOS sysinfo, and parse this tick as RouterOS.
                            if remote_os == super::metrics::RemoteOsType::Linux
                                && super::parser::looks_like_routeros(&output)
                            {
                                remote_os = super::metrics::RemoteOsType::RouterOs;
                                tracing::info!("Remote monitoring: detected RouterOS, switching command set");
                                if let Ok(si) = exec_command(ros_sysinfo_command.clone()).await
                                    && let Ok(info) = MetricsParser::parse_system_info_routeros(&si)
                                {
                                    sysinfo_fetched = true;
                                    let _ = event_tx.send(MetricsEvent::SystemInfoReady(info)).await;
                                }
                            }
                            let parse_result = if remote_os == super::metrics::RemoteOsType::RouterOs {
                                // Re-fetch via the RouterOS command when we only
                                // just switched (this tick's output is the Linux
                                // probe's error), otherwise parse it directly.
                                if super::parser::looks_like_routeros(&output) {
                                    match exec_command(ros_command.clone()).await {
                                        Ok(ros_out) => MetricsParser::parse_routeros(&ros_out),
                                        Err(e) => Err(MonitoringError::ParseError(e)),
                                    }
                                } else {
                                    MetricsParser::parse_routeros(&output)
                                }
                            } else {
                                MetricsParser::parse(&output)
                            };
                            match parse_result {
                                Ok(parsed) => {
                                    consecutive_errors = 0;
                                    let metrics = computer.compute(&parsed);
                                    if event_tx
                                        .send(MetricsEvent::Update(metrics))
                                        .await
                                        .is_err()
                                    {
                                        break; // receiver dropped
                                    }
                                }
                                Err(MonitoringError::ParseError(msg)) => {
                                    consecutive_errors += 1;
                                    if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                                        tracing::warn!(
                                            errors = consecutive_errors,
                                            "Monitoring stopped after \
                                             {MAX_CONSECUTIVE_ERRORS} consecutive \
                                             parse errors"
                                        );
                                        let _ = event_tx
                                            .send(MetricsEvent::Stopped)
                                            .await;
                                        break;
                                    }
                                    let _ = event_tx
                                        .send(MetricsEvent::ParseError(msg))
                                        .await;
                                }
                                Err(MonitoringError::UnsupportedOs) => {
                                    tracing::warn!(
                                        "Remote host OS not supported \
                                         for monitoring, stopping"
                                    );
                                    let _ = event_tx
                                        .send(MetricsEvent::Stopped)
                                        .await;
                                    break;
                                }
                            }
                        }
                        Err(err) => {
                            consecutive_errors += 1;
                            tracing::debug!(
                                error = %err,
                                attempt = consecutive_errors,
                                "Monitoring command execution failed"
                            );
                            if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                                tracing::warn!(
                                    errors = consecutive_errors,
                                    last_error = %err,
                                    "Monitoring stopped after \
                                     {MAX_CONSECUTIVE_ERRORS} consecutive \
                                     execution errors"
                                );
                                let _ = event_tx
                                    .send(MetricsEvent::Stopped)
                                    .await;
                                break;
                            }
                            let _ = event_tx
                                .send(MetricsEvent::ParseError(err))
                                .await;
                        }
                    }
                }
            }
        }
    });

    (CollectorHandle { stop_tx }, event_rx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitoring::metrics::{
        CpuSnapshot, DiskMetrics, LoadAverage, MemoryMetrics, NetworkSnapshot, RemoteOsType,
    };
    use crate::monitoring::parser::ParsedMetrics;

    #[test]
    fn test_first_compute_returns_zero_deltas() {
        let mut computer = MetricsComputer::new();
        let parsed = ParsedMetrics {
            cpu: CpuSnapshot {
                user: 1000,
                nice: 0,
                system: 500,
                idle: 8000,
                iowait: 500,
                irq: 0,
                softirq: 0,
                steal: 0,
            },
            memory: MemoryMetrics {
                total_kib: 16_000_000,
                used_kib: 8_000_000,
                available_kib: 8_000_000,
                swap_total_kib: 0,
                swap_used_kib: 0,
            },
            network: NetworkSnapshot {
                rx_bytes: 1_000_000,
                tx_bytes: 500_000,
            },
            disks: vec![DiskMetrics {
                total_kib: 100_000_000,
                used_kib: 50_000_000,
                available_kib: 50_000_000,
                mount_point: String::from("/"),
            }],
            load_average: LoadAverage::default(),
            os_type: RemoteOsType::Linux,
            cpu_percent_direct: None,
        };

        let metrics = computer.compute(&parsed);
        assert!((metrics.cpu_percent - 0.0).abs() < f32::EPSILON);
        assert!((metrics.network.rx_bytes_per_sec - 0.0).abs() < f64::EPSILON);
        assert!((metrics.network.tx_bytes_per_sec - 0.0).abs() < f64::EPSILON);
        // Memory and disk are absolute, not delta
        assert!((metrics.memory.percent() - 50.0).abs() < 0.1);
        assert!((metrics.disk.percent() - 50.0).abs() < 0.1);
        assert_eq!(metrics.disks.len(), 1);
        assert_eq!(metrics.disks[0].mount_point, "/");
    }

    #[test]
    fn test_second_compute_returns_deltas() {
        let mut computer = MetricsComputer::new();

        let parsed1 = ParsedMetrics {
            cpu: CpuSnapshot {
                user: 100,
                nice: 0,
                system: 50,
                idle: 800,
                iowait: 50,
                irq: 0,
                softirq: 0,
                steal: 0,
            },
            memory: MemoryMetrics {
                total_kib: 16_000_000,
                used_kib: 8_000_000,
                available_kib: 8_000_000,
                swap_total_kib: 0,
                swap_used_kib: 0,
            },
            network: NetworkSnapshot {
                rx_bytes: 1_000_000,
                tx_bytes: 500_000,
            },
            disks: vec![DiskMetrics {
                total_kib: 100_000,
                used_kib: 50_000,
                available_kib: 50_000,
                mount_point: String::from("/"),
            }],
            load_average: LoadAverage::default(),
            os_type: RemoteOsType::Linux,
            cpu_percent_direct: None,
        };
        let _ = computer.compute(&parsed1);

        // Small sleep to get non-zero elapsed time
        std::thread::sleep(std::time::Duration::from_millis(50));

        let parsed2 = ParsedMetrics {
            cpu: CpuSnapshot {
                user: 200,
                nice: 0,
                system: 100,
                idle: 1600,
                iowait: 100,
                irq: 0,
                softirq: 0,
                steal: 0,
            },
            memory: MemoryMetrics {
                total_kib: 16_000_000,
                used_kib: 12_000_000,
                available_kib: 4_000_000,
                swap_total_kib: 0,
                swap_used_kib: 0,
            },
            network: NetworkSnapshot {
                rx_bytes: 2_000_000,
                tx_bytes: 1_000_000,
            },
            disks: vec![DiskMetrics {
                total_kib: 100_000,
                used_kib: 75_000,
                available_kib: 25_000,
                mount_point: String::from("/"),
            }],
            load_average: LoadAverage::default(),
            os_type: RemoteOsType::Linux,
            cpu_percent_direct: None,
        };
        let metrics = computer.compute(&parsed2);

        // CPU: total delta=1000, idle delta=850, busy=150 → 15%
        assert!((metrics.cpu_percent - 15.0).abs() < 0.1);
        // Network: should have positive rates
        assert!(metrics.network.rx_bytes_per_sec > 0.0);
        assert!(metrics.network.tx_bytes_per_sec > 0.0);
        // Memory: 75%
        assert!((metrics.memory.percent() - 75.0).abs() < 0.1);
    }

    #[test]
    fn test_reset_clears_state() {
        let mut computer = MetricsComputer::new();
        let parsed = ParsedMetrics {
            cpu: CpuSnapshot {
                user: 100,
                ..CpuSnapshot::default()
            },
            memory: MemoryMetrics {
                total_kib: 1000,
                used_kib: 500,
                available_kib: 500,
                swap_total_kib: 0,
                swap_used_kib: 0,
            },
            network: NetworkSnapshot {
                rx_bytes: 1000,
                tx_bytes: 500,
            },
            disks: vec![DiskMetrics {
                total_kib: 1000,
                used_kib: 500,
                available_kib: 500,
                mount_point: String::from("/"),
            }],
            load_average: LoadAverage::default(),
            os_type: RemoteOsType::Linux,
            cpu_percent_direct: None,
        };
        let _ = computer.compute(&parsed);
        assert!(computer.cpu.is_some());

        computer.reset();
        assert!(computer.cpu.is_none());
        assert!(computer.net.is_none());
        assert!(computer.sampled_at.is_none());
    }
}
