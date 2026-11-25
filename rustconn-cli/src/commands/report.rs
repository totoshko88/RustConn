//! Report command — collect one read-only device report across SSH targets.
//!
//! This is the CDP-to-CSV, ARP-dump and inventory-report family of the
//! SecureCRT tool collections, restated over RustConn's connection database.
//! The selection and ssh argv come from [`super::exec`]; the difference is the
//! output: instead of one log file per host, each host's stdout is parsed into
//! rows with a leading `connection` column and all hosts are printed as one
//! table, JSON array or CSV document.
//!
//! Every report runs exactly one remote command, so a device is only ever asked
//! for a `show` — nothing here configures a device.

use std::path::Path;
use std::process::Stdio;

use rustconn_core::models::Connection;

use crate::cli::{OutputFormat, ReportKind};
use crate::error::CliError;
use crate::format::escape_csv_field;
use crate::util::create_config_manager;

use super::exec::select_ssh_jobs;

/// Runs the report for `kind` on every selected SSH target and prints the rows.
///
/// # Errors
///
/// Returns [`CliError::Config`] when the configuration cannot be loaded or a
/// selection fails, and [`CliError::Connection`] when at least one host's
/// command failed. A failed host is reported on stderr and skipped, so the rows
/// of the remaining hosts are still printed.
pub(super) fn cmd_report(
    config_path: Option<&Path>,
    kind: ReportKind,
    names: &[String],
    group: Option<&str>,
    tag: Option<&str>,
    format: Option<OutputFormat>,
    quiet: bool,
) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;
    let connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;
    let groups = config_manager
        .load_groups()
        .map_err(|e| CliError::Config(format!("Failed to load groups: {e}")))?;

    let command = vec![kind.command().to_string()];
    let jobs = select_ssh_jobs(&connections, &groups, names, group, tag, &command)?;

    let (rows, failed) = collect(kind, &jobs, quiet);
    print_rows(kind.headers(), &rows, OutputFormat::effective(format))?;

    if failed > 0 {
        return Err(CliError::Connection(format!(
            "{failed} of {} `{}` command(s) failed.",
            jobs.len(),
            kind.command()
        )));
    }
    Ok(())
}

/// Runs the report command on every target and parses each successful stdout.
///
/// A host whose command fails is named on stderr and contributes no rows; it
/// never stops the remaining hosts. Returns the rows and the number of failed
/// hosts.
fn collect(
    kind: ReportKind,
    jobs: &[(&Connection, Vec<String>)],
    quiet: bool,
) -> (Vec<Vec<String>>, usize) {
    let mut rows = Vec::new();
    let mut failed = 0;
    for (conn, args) in jobs {
        // build_ssh_exec_args always yields at least `ssh -n <dest> <command>`.
        // No shell is involved and the report command is a fixed string per kind.
        let (program, rest) = (args[0].as_str(), &args[1..]);
        match std::process::Command::new(program)
            .args(rest)
            .stdin(Stdio::null())
            .output()
        {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let width = kind.headers().len();
                rows.extend(kind.parse(&stdout).into_iter().map(|row| {
                    let mut row = normalize_row(row, width - 1);
                    row.insert(0, conn.name.clone());
                    row
                }));
            }
            Ok(output) => {
                failed += 1;
                warn_failed(
                    quiet,
                    &conn.name,
                    &format!("`{}` failed ({})", kind.command(), output.status),
                );
            }
            Err(e) => {
                failed += 1;
                warn_failed(quiet, &conn.name, &format!("ssh could not start: {e}"));
            }
        }
    }
    (rows, failed)
}

/// Names a failed host on stderr. `--quiet` shortens the line but never hides
/// which hosts are missing from the data.
fn warn_failed(quiet: bool, host: &str, reason: &str) {
    if quiet {
        eprintln!("failed: {host}");
    } else {
        eprintln!("warning: {host}: {reason} — host skipped");
    }
}

/// Pads or truncates a parser row so every row matches the header width.
///
/// A parser that drifts cannot panic the whole report by indexing past the
/// end of a row in `print_rows`; a short row simply gets empty cells.
fn normalize_row(mut row: Vec<String>, width: usize) -> Vec<String> {
    row.resize(width, String::new());
    row.truncate(width);
    row
}

/// Prints the aggregated rows in the resolved format.
///
/// The `connection` column is always first and every row has exactly as many
/// fields as `headers`.
fn print_rows(
    headers: &[&str],
    rows: &[Vec<String>],
    format: OutputFormat,
) -> Result<(), CliError> {
    match format {
        OutputFormat::Table => {
            let widths: Vec<usize> = headers
                .iter()
                .enumerate()
                .map(|(i, header)| {
                    rows.iter()
                        .map(|row| row[i].len())
                        .chain([header.len()])
                        .max()
                        .unwrap_or(0)
                })
                .collect();
            let line = |cells: &[String]| {
                cells
                    .iter()
                    .zip(&widths)
                    .map(|(cell, width)| format!("{cell:<width$}"))
                    .collect::<Vec<_>>()
                    .join("  ")
                    .trim_end()
                    .to_string()
            };
            let header_cells = headers.iter().map(|h| (*h).to_string()).collect::<Vec<_>>();
            println!("{}", line(&header_cells));
            println!(
                "{}",
                widths
                    .iter()
                    .map(|w| "-".repeat(*w))
                    .collect::<Vec<_>>()
                    .join("  ")
            );
            for row in rows {
                println!("{}", line(row));
            }
        }
        OutputFormat::Json => {
            let objects = rows
                .iter()
                .map(|row| {
                    headers
                        .iter()
                        .zip(row)
                        .map(|(header, cell)| {
                            (
                                (*header).to_string(),
                                serde_json::Value::String(cell.clone()),
                            )
                        })
                        .collect::<serde_json::Map<_, _>>()
                })
                .collect::<Vec<_>>();
            let json = serde_json::to_string_pretty(&objects)
                .map_err(|e| CliError::Config(format!("Failed to serialize report: {e}")))?;
            println!("{json}");
        }
        OutputFormat::Csv => {
            println!("{}", headers.join(","));
            for row in rows {
                let fields = row
                    .iter()
                    .map(|field| escape_csv_field(field))
                    .collect::<Vec<_>>();
                println!("{}", fields.join(","));
            }
        }
    }
    Ok(())
}

impl ReportKind {
    /// The single read-only command the report runs on each host.
    fn command(self) -> &'static str {
        match self {
            Self::Inventory | Self::DocumentDevice => "show version",
            Self::Cdp => "show cdp neighbors detail",
            Self::Arp | Self::MergedArp => "show ip arp",
            Self::Mac | Self::MacsByVlan => "show mac address-table",
            Self::Vlan => "show vlan",
            Self::InterfaceStats => "show interfaces",
            Self::NextHop => "show ip route",
            Self::EigrpTopology | Self::EigrpSummary => "show ip eigrp topology",
            Self::Switchport => "show interface status",
            Self::AireosApDetail => "show ap summary",
            Self::AireosApSummary => "show ap summary",
            Self::AireosAuthList => "show auth-list",
            Self::AireosInterfaceDetail => "show interface summary",
            Self::AireosMobilityGroup => "show mobility summary",
            Self::AireosWlanDetail => "show wlan summary",
            Self::CiscoBgp => "show ip bgp summary",
            Self::HuaweiBgp => "display bgp peer",
            Self::JuniperBgp => "show bgp summary",
        }
    }

    /// The output columns, `connection` first.
    fn headers(self) -> &'static [&'static str] {
        match self {
            Self::Inventory => &["connection", "version", "model", "serial", "uptime"],
            Self::Cdp => &[
                "connection",
                "device_id",
                "local_interface",
                "remote_port",
                "platform",
                "ip",
            ],
            Self::Arp => &[
                "connection",
                "protocol",
                "address",
                "age",
                "hardware_addr",
                "type",
                "interface",
            ],
            Self::Mac => &["connection", "vlan", "mac", "type", "ports"],
            Self::Vlan => &["connection", "vlan", "name", "status", "ports"],
            Self::InterfaceStats => &[
                "connection",
                "interface",
                "status",
                "input_rate",
                "output_rate",
                "input_errors",
                "crc",
                "output_errors",
            ],
            Self::NextHop => &["connection", "next_hop", "routes", "protocols"],
            Self::EigrpTopology => &[
                "connection",
                "network",
                "successors",
                "fd",
                "via",
                "interface",
            ],
            Self::EigrpSummary => &["connection", "successor", "networks"],
            Self::Switchport => &["connection", "port", "name", "status", "vlan"],
            Self::MergedArp => Self::Arp.headers(),
            Self::MacsByVlan => &["connection", "vlan", "mac", "type", "ports"],
            Self::DocumentDevice => Self::Inventory.headers(),
            Self::AireosApDetail => &[
                "connection",
                "ap_name",
                "slots",
                "model",
                "ethernet_mac",
                "location",
            ],
            Self::AireosApSummary => &[
                "connection",
                "ap_name",
                "slots",
                "model",
                "ethernet_mac",
                "location",
            ],
            Self::AireosAuthList => &["connection", "setting", "value"],
            Self::AireosInterfaceDetail => &[
                "connection",
                "interface",
                "port",
                "vlan",
                "ip_address",
                "type",
            ],
            Self::AireosMobilityGroup => &[
                "connection",
                "mac_address",
                "ip_address",
                "group_name",
                "multicast_ip",
                "status",
            ],
            Self::AireosWlanDetail => &[
                "connection",
                "wlan_id",
                "profile_name",
                "ssid",
                "status",
                "interface",
            ],
            Self::CiscoBgp | Self::HuaweiBgp | Self::JuniperBgp => {
                &["connection", "neighbor", "remote_as", "uptime", "state"]
            }
        }
    }

    /// Parses one host's stdout into rows without the `connection` column.
    ///
    /// Text that does not look like an entry is ignored rather than guessed
    /// at: a device that answers with an error or a different output format
    /// contributes no rows, never a half-parsed one.
    fn parse(self, output: &str) -> Vec<Vec<String>> {
        match self {
            Self::Inventory | Self::DocumentDevice => parse_inventory(output),
            Self::Cdp => parse_cdp(output),
            Self::Arp | Self::MergedArp => parse_arp(output),
            Self::Mac | Self::MacsByVlan => parse_mac(output),
            Self::Vlan => parse_vlan(output),
            Self::InterfaceStats => parse_interface_stats(output),
            Self::NextHop => parse_next_hop(output),
            Self::EigrpTopology => parse_eigrp_topology(output),
            Self::EigrpSummary => parse_eigrp_summary(output),
            Self::Switchport => parse_switchport(output),
            Self::AireosApDetail | Self::AireosApSummary => parse_ap_summary(output),
            Self::AireosAuthList => parse_aireos_kv(output),
            Self::AireosInterfaceDetail => parse_aireos_interfaces(output),
            Self::AireosMobilityGroup => parse_mobility(output),
            Self::AireosWlanDetail => parse_wlan_summary(output),
            Self::CiscoBgp | Self::HuaweiBgp | Self::JuniperBgp => parse_bgp(output),
        }
    }
}

/// `show version` → one row per host: version, model, serial, uptime.
///
/// The hostname is not in the output (it lives in the prompt), so the
/// connection name is the host identifier.
fn parse_inventory(output: &str) -> Vec<Vec<String>> {
    let (mut version, mut model, mut serial, mut uptime) =
        (String::new(), String::new(), String::new(), String::new());

    for line in output.lines() {
        if version.is_empty()
            && let Some((_, rest)) = line.split_once(", Version ")
        {
            version = rest.split(',').next().unwrap_or(rest).trim().to_string();
        }

        if model.is_empty() {
            if let Some((key, value)) = line.split_once(':')
                && key.trim().eq_ignore_ascii_case("model number")
            {
                model = value.trim().to_string();
            } else if line.starts_with("cisco ") && line.contains(" processor") {
                model = line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or_default()
                    .to_string();
            }
        }

        if serial.is_empty() {
            for prefix in ["Processor board ID", "System serial number"] {
                if let Some(rest) = line.trim_start().strip_prefix(prefix) {
                    serial = rest.trim_start_matches([' ', ':']).trim().to_string();
                    break;
                }
            }
        }

        if uptime.is_empty()
            && let Some((_, rest)) = line.split_once(" uptime is ")
        {
            uptime = rest.trim().to_string();
        }
    }

    if version.is_empty() && model.is_empty() && serial.is_empty() && uptime.is_empty() {
        return Vec::new();
    }
    vec![vec![version, model, serial, uptime]]
}

/// `show cdp neighbors detail` → one row per neighbour.
fn parse_cdp(output: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    // [device_id, local_interface, remote_port, platform, ip]
    let mut current: Option<Vec<String>> = None;

    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(device) = trimmed.strip_prefix("Device ID:") {
            if let Some(row) = current.take() {
                rows.push(row);
            }
            current = Some(vec![
                device.trim().to_string(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
            ]);
            continue;
        }
        let Some(row) = current.as_mut() else {
            continue;
        };

        if let Some(rest) = trimmed.strip_prefix("IP address:") {
            if row[4].is_empty() {
                row[4] = rest.trim().to_string();
            }
        } else if let Some(rest) = trimmed.strip_prefix("Platform:") {
            row[3] = rest.split(',').next().unwrap_or(rest).trim().to_string();
        } else if let Some(rest) = trimmed.strip_prefix("Interface:") {
            let mut parts = rest.split(',');
            row[1] = parts.next().unwrap_or_default().trim().to_string();
            for part in parts {
                let part = part.trim();
                if let Some(port) = part
                    .strip_prefix("Port ID (outgoing port):")
                    .or_else(|| part.strip_prefix("Port ID:"))
                {
                    row[2] = port.trim().to_string();
                }
            }
        }
    }

    if let Some(row) = current {
        rows.push(row);
    }
    rows
}

/// `show ip arp` → one row per ARP entry.
fn parse_arp(output: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut in_table = false;

    for line in output.lines() {
        let trimmed = line.trim();
        if !in_table {
            if trimmed.starts_with("Protocol") && trimmed.contains("Address") {
                in_table = true;
            }
            continue;
        }
        // protocol address age hardware_addr type interface
        let fields = trimmed.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 6 {
            continue;
        }
        rows.push(vec![
            fields[0].to_string(),
            fields[1].to_string(),
            fields[2].to_string(),
            fields[3].to_string(),
            fields[4].to_string(),
            fields[5..].join(" "),
        ]);
    }
    rows
}

/// `show mac address-table` → one row per learned MAC entry.
fn parse_mac(output: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut in_table = false;

    for line in output.lines() {
        let trimmed = line.trim();
        if !in_table {
            if trimmed.starts_with("Vlan") && trimmed.contains("Mac Address") {
                in_table = true;
            }
            continue;
        }
        // vlan mac type ports…
        let fields = trimmed.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 4 {
            continue;
        }
        let vlan_ok =
            fields[0].eq_ignore_ascii_case("All") || fields[0].chars().all(|c| c.is_ascii_digit());
        let type_ok = fields[2].chars().all(|c| c.is_ascii_alphabetic());
        if !vlan_ok || !type_ok {
            continue;
        }
        rows.push(vec![
            fields[0].to_string(),
            fields[1].to_string(),
            fields[2].to_string(),
            fields[3..].join(" "),
        ]);
    }
    rows
}

/// `show vlan` → one row per VLAN, with wrapped port lists re-joined.
fn parse_vlan(output: &str) -> Vec<Vec<String>> {
    const STATUSES: [&str; 5] = ["active", "suspend", "act/unsup", "act/lshut", "suspended"];
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut in_table = false;

    for line in output.lines() {
        let trimmed = line.trim_end();
        if !in_table {
            if trimmed.starts_with("VLAN Name") {
                in_table = true;
            }
            continue;
        }
        if trimmed.trim().is_empty() || trimmed.starts_with("VLAN Type") {
            break;
        }
        if trimmed.starts_with("----") {
            continue;
        }

        if !trimmed.starts_with(|c: char| c.is_ascii_digit()) {
            // The ports column wraps; the continuation belongs to the VLAN
            // above it.
            if let Some(last) = rows.last_mut() {
                let extra = trimmed.trim();
                if !extra.is_empty() {
                    if !last[3].is_empty() && !last[3].ends_with(',') {
                        last[3].push(',');
                    }
                    if !last[3].is_empty() {
                        last[3].push(' ');
                    }
                    last[3].push_str(extra);
                }
            }
            continue;
        }

        // vlan name… status ports…
        let fields = trimmed.split_whitespace().collect::<Vec<_>>();
        let Some(status_at) = fields.iter().position(|f| STATUSES.contains(f)) else {
            continue;
        };
        rows.push(vec![
            fields[0].to_string(),
            fields[1..status_at].join(" "),
            fields[status_at].to_string(),
            fields[status_at + 1..].join(" "),
        ]);
    }
    rows
}

/// BGP peer parser shared by IOS, VRP and Junos summary tables.
///
/// All three print one peer per line, starting with an IP address. Cisco and
/// Huawei put a one-digit version before the AS; Junos does not. A line that
/// does not start with an IPv4-like token is not a peer and is ignored.
fn parse_bgp(output: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    for line in output.lines() {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 4 {
            continue;
        }
        let neighbor = fields[0];
        if !neighbor.chars().all(|c| c.is_ascii_digit() || c == '.') || !neighbor.contains('.') {
            continue;
        }
        let numeric = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
        let as_index = if numeric(fields[1]) && fields[1].len() <= 2 {
            2
        } else {
            1
        };
        if !fields.get(as_index).is_some_and(|s| numeric(s)) {
            continue;
        }
        // All three vendors put Up/Down immediately before the state column;
        // it may be a duration, a day count, or the literal `never`.
        let uptime = fields
            .get(fields.len().saturating_sub(2))
            .map_or("", |s| *s);
        let state = fields.last().map_or(String::new(), ToString::to_string);
        rows.push(vec![
            neighbor.to_string(),
            fields[as_index].to_string(),
            uptime.to_string(),
            state,
        ]);
    }
    rows
}

/// `show ap summary` → one row per AP, starting after the table header.
///
/// AireOS prints key/value lines first (`Number of APs...`). Only the table
/// after `AP Name` is data; with no APs joined the report is empty.
fn parse_ap_summary(output: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut in_table = false;
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.contains("AP Name") {
            in_table = true;
            continue;
        }
        if !in_table || trimmed.is_empty() || trimmed.starts_with("----") {
            continue;
        }
        let fields = trimmed.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 5 {
            continue;
        }
        rows.push(fields.into_iter().take(5).map(str::to_string).collect());
    }
    rows
}

/// AireOS `show auth-list` → one row per dotted key/value setting.
fn parse_aireos_kv(output: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    for line in output.lines() {
        let trimmed = line.trim();
        if !trimmed.contains("..") {
            continue;
        }
        // AireOS pads with dots instead of a colon: `Setting ...... value`.
        let Some(idx) = trimmed.rfind("..") else {
            continue;
        };
        let (key, value) = trimmed.split_at(idx);
        rows.push(vec![
            key.trim().trim_end_matches('.').trim().to_string(),
            value.trim_start_matches('.').trim().to_string(),
        ]);
    }
    rows
}

/// AireOS `show interface summary` → management/service/virtual interfaces.
fn parse_aireos_interfaces(output: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut in_table = false;
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Interface Name") {
            in_table = true;
            continue;
        }
        if !in_table || trimmed.is_empty() || trimmed.starts_with("----") {
            continue;
        }
        let fields = trimmed.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 5 {
            continue;
        }
        rows.push(fields.into_iter().take(5).map(str::to_string).collect());
    }
    rows
}

/// AireOS `show mobility summary` → one row per mobility-group member.
fn parse_mobility(output: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut in_table = false;
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Controllers configured") {
            in_table = true;
            continue;
        }
        if !in_table
            || trimmed.is_empty()
            || trimmed.starts_with("MAC Address")
            || trimmed.starts_with("----")
        {
            continue;
        }
        let fields = trimmed.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 5 || !fields[0].contains(':') {
            continue;
        }
        rows.push(fields.into_iter().take(5).map(str::to_string).collect());
    }
    rows
}

/// AireOS `show wlan summary` → one row per WLAN profile.
fn parse_wlan_summary(output: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut in_table = false;
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("WLAN ID") {
            in_table = true;
            continue;
        }
        if !in_table || trimmed.is_empty() || trimmed.starts_with("----") {
            continue;
        }
        let fields = trimmed.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 4 || !fields[0].chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let status_at = fields
            .iter()
            .position(|f| *f == "Enabled" || *f == "Disabled")
            .unwrap_or(fields.len() - 1);
        let names = fields[1..status_at].join(" ");
        let (profile, ssid) = names.split_once(" / ").unwrap_or((&names, ""));
        rows.push(vec![
            fields[0].to_string(),
            profile.trim().to_string(),
            ssid.trim().to_string(),
            fields
                .get(status_at)
                .map_or(String::new(), ToString::to_string),
            fields.last().map_or(String::new(), ToString::to_string),
        ]);
    }
    rows
}

/// `show interfaces` → one row per interface with rate and error counters.
fn parse_interface_stats(output: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut current: Option<[String; 7]> = None;
    for line in output.lines() {
        let trimmed = line.trim();
        let cleaned = trimmed
            .split_whitespace()
            .map(|w| w.trim_end_matches(',').to_string())
            .collect::<Vec<_>>();
        let words = cleaned.iter().map(String::as_str).collect::<Vec<_>>();
        if words.len() > 3 && words[1] == "is" {
            if let Some(row) = current.take() {
                rows.push(row.into_iter().collect());
            }
            let status = words[2..]
                .iter()
                .take_while(|w| **w != "line")
                .map(|w| w.trim_end_matches(','))
                .collect::<Vec<_>>()
                .join(" ");
            current = Some([
                words[0].to_string(),
                status,
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
            ]);
            continue;
        }
        let Some(row) = current.as_mut() else {
            continue;
        };
        if words.contains(&"input") && words.contains(&"rate") {
            if let Some(v) = words
                .iter()
                .position(|w| *w == "rate")
                .and_then(|i| words.get(i + 1))
            {
                row[2] = v.trim_end_matches(',').to_string();
            }
        } else if words.contains(&"output") && words.contains(&"rate") {
            if let Some(v) = words
                .iter()
                .position(|w| *w == "rate")
                .and_then(|i| words.get(i + 1))
            {
                row[3] = v.trim_end_matches(',').to_string();
            }
        } else if words.contains(&"input") && words.contains(&"errors") {
            if let Some(v) = words.first() {
                row[4] = v.trim_end_matches(',').to_string();
            }
            if let Some(i) = words
                .iter()
                .position(|w| *w == "CRC,")
                .or_else(|| words.iter().position(|w| *w == "CRC"))
            {
                row[5] = words
                    .get(i.wrapping_sub(1))
                    .map_or(String::new(), |v| v.trim_end_matches(',').to_string());
            }
        } else if words.contains(&"output")
            && words.contains(&"errors")
            && let Some(i) = words.iter().position(|w| *w == "errors")
        {
            row[6] = words
                .get(i.wrapping_sub(1))
                .map_or(String::new(), |v| v.trim_end_matches(',').to_string());
        }
    }
    if let Some(row) = current {
        rows.push(row.into_iter().collect());
    }
    rows
}

/// Aggregates `show ip route` next-hops: one row per distinct next hop with
/// the route count and the protocols that use it.
fn parse_next_hop(output: &str) -> Vec<Vec<String>> {
    use std::collections::{BTreeMap, BTreeSet};

    let mut summary: BTreeMap<String, (usize, BTreeSet<String>)> = BTreeMap::new();
    for line in output.lines() {
        let words = line.split_whitespace().collect::<Vec<_>>();
        let Some(via) = words.iter().position(|w| *w == "via") else {
            continue;
        };
        let Some(next_hop) = words.get(via + 1) else {
            continue;
        };
        let next_hop = next_hop.trim_end_matches(',');
        if !next_hop.chars().all(|c| c.is_ascii_digit() || c == '.') {
            continue;
        }
        let protocol = words
            .first()
            .map(|p| p.trim_end_matches('*').to_string())
            .filter(|p| p.chars().all(|c| c.is_ascii_alphanumeric()))
            .unwrap_or_else(|| "unknown".to_string());
        let entry = summary.entry((*next_hop).to_string()).or_default();
        entry.0 += 1;
        entry.1.insert(protocol);
    }
    summary
        .into_iter()
        .map(|(next_hop, (count, protocols))| {
            vec![
                next_hop,
                count.to_string(),
                protocols.into_iter().collect::<Vec<_>>().join(","),
            ]
        })
        .collect()
}

/// `show ip eigrp topology` → one row per network/successor pair.
fn parse_eigrp_topology(output: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut pending: Option<(String, String, String)> = None;
    for line in output.lines() {
        let trimmed = line.trim();
        let words = trimmed.split_whitespace().collect::<Vec<_>>();
        let status = words.first().copied().unwrap_or("");
        if (status == "P" || status == "D")
            && words.len() > 4
            && words
                .iter()
                .any(|w| w.trim_end_matches(',') == "successors")
        {
            let network = words
                .get(1)
                .unwrap_or(&"")
                .trim_end_matches(',')
                .to_string();
            let successors = words
                .iter()
                .position(|w| w.trim_end_matches(',') == "successors")
                .and_then(|i| words.get(i.wrapping_sub(1)))
                .map_or(String::new(), ToString::to_string);
            let fd = words
                .iter()
                .position(|w| *w == "is")
                .and_then(|i| words.get(i + 1))
                .map_or(String::new(), ToString::to_string);
            pending = Some((network, successors, fd));
            continue;
        }
        if let Some((network, successors, fd)) = pending.take() {
            if !status.starts_with("via") {
                continue;
            }
            let via = words.get(1).map_or(String::new(), ToString::to_string);
            let interface = words.last().map_or(String::new(), ToString::to_string);
            rows.push(vec![network, successors, fd, via, interface]);
        }
    }
    rows
}

/// `show ip eigrp topology` → one row per successor with its network count.
fn parse_eigrp_summary(output: &str) -> Vec<Vec<String>> {
    use std::collections::BTreeMap;
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for row in parse_eigrp_topology(output) {
        if let Some(via) = row.get(3) {
            *counts.entry(via.clone()).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .map(|(via, count)| vec![via, count.to_string()])
        .collect()
}

/// `show interface status` → one row per physical port.
fn parse_switchport(output: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("----") || trimmed.starts_with("Port ") {
            continue;
        }
        let fields = trimmed.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 4 {
            continue;
        }
        rows.push(vec![
            fields[0].to_string(),
            fields[1].to_string(),
            fields[2].to_string(),
            fields[3].to_string(),
        ]);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    const VERSION: &str = "\
Cisco IOS XE Software, Version 17.03.04
Cisco IOS Software [Amsterdam], Catalyst L3 Switch Software (CAT9K_IOSXE), Version 17.3.4, RELEASE SOFTWARE (fc7)
Copyright (c) 1986-2020 by Cisco Systems, Inc.

ROM: IOS-XE ROMMON

SW-CORE-01 uptime is 2 weeks, 3 days, 4 hours, 5 minutes
Uptime for this control processor is 2 weeks, 3 days, 4 hours, 7 minutes
System returned to ROM by reload
System image file is \"flash:packages.conf\"

cisco C9300-48P (X86) processor (revision V02) with 931584K/6147K bytes of memory.
Processor board ID FDO2345ABCD
2 Virtual Ethernet interfaces
";

    #[test]
    fn inventory_reads_version_model_serial_and_uptime() {
        let rows = parse_inventory(VERSION);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0],
            vec![
                "17.03.04",
                "C9300-48P",
                "FDO2345ABCD",
                "2 weeks, 3 days, 4 hours, 5 minutes",
            ]
        );
    }

    #[test]
    fn inventory_reads_ios_classic_and_model_number() {
        let output = "\
Cisco IOS Software, C2960 Software (C2960-LANBASEK9-M), Version 15.0(2)SE11, RELEASE SOFTWARE (fc3)
Model number                    : WS-C2960-24TT-L
System serial number            : FOC1234ABCD
Switch uptime is 1 year, 2 weeks
";
        let rows = parse_inventory(output);
        assert_eq!(
            rows[0],
            vec![
                "15.0(2)SE11",
                "WS-C2960-24TT-L",
                "FOC1234ABCD",
                "1 year, 2 weeks",
            ]
        );
    }

    #[test]
    fn inventory_ignores_unrelated_output() {
        assert!(parse_inventory("% Invalid input detected\n").is_empty());
    }

    #[test]
    fn cdp_reads_neighbour_blocks() {
        let output = "\
-------------------------
Device ID: SW2.example.com
Entry address(es):
  IP address: 10.0.0.2
Platform: cisco WS-C2960-24TT-L,  Capabilities: Switch IGMP
Interface: GigabitEthernet1/0/1,  Port ID (outgoing port): GigabitEthernet0/2
Holdtime : 145 sec

Version :
Cisco IOS Software, C2960 Software

advertisement version: 2
Management address(es):
  IP address: 10.0.0.2

-------------------------
Device ID: R1
Entry address(es):
  IP address: 10.0.0.1
Platform: cisco ISR4331/K9,  Capabilities: Router
Interface: GigabitEthernet1/0/2,  Port ID (outgoing port): GigabitEthernet0/0/0

Total cdp entries displayed : 2
";
        let rows = parse_cdp(output);
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0],
            vec![
                "SW2.example.com",
                "GigabitEthernet1/0/1",
                "GigabitEthernet0/2",
                "cisco WS-C2960-24TT-L",
                "10.0.0.2",
            ]
        );
        assert_eq!(rows[1][0], "R1");
        assert_eq!(rows[1][3], "cisco ISR4331/K9");
    }

    #[test]
    fn arp_skips_the_header_and_splits_the_interface() {
        let output = "\
Protocol  Address          Age (min)  Hardware Addr   Type   Interface
Internet  10.0.0.1                -   0011.2233.4455  ARPA   GigabitEthernet0/0
Internet  10.0.0.2                5   0011.2233.4456  ARPA   GigabitEthernet0/0.100
Internet  10.0.0.3                2   Incomplete      ARPA   Vlan10
";
        let rows = parse_arp(output);
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows[1],
            vec![
                "Internet",
                "10.0.0.2",
                "5",
                "0011.2233.4456",
                "ARPA",
                "GigabitEthernet0/0.100",
            ]
        );
        assert_eq!(rows[2][3], "Incomplete");
    }

    #[test]
    fn mac_skips_titles_and_keeps_multiple_ports() {
        let output = "\
          Mac Address Table
-------------------------------------------

Vlan    Mac Address       Type        Ports
----    -----------       ----        -----
 All    0100.0ccc.cccc    STATIC      CPU
   1    0011.2233.4455    DYNAMIC     Gi1/0/1, Gi1/0/2
";
        let rows = parse_mac(output);
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[1],
            vec!["1", "0011.2233.4455", "DYNAMIC", "Gi1/0/1, Gi1/0/2"]
        );
    }

    #[test]
    fn vlan_reads_rows_and_rejoins_wrapped_ports() {
        let output = "\
VLAN Name                             Status    Ports
---- -------------------------------- --------- -------------------------------
1    default                          active    Gi1/0/1, Gi1/0/2,
                                                 Gi1/0/3
10   VLAN0010                         active
1002 fddi-default                     act/unsup

VLAN Type  SAID       MTU   Parent RingNo BridgeNo Stp  BrdgMode Trans1 Trans2
---- ----- ---------- ----- ------ ------ -------- ---- -------- ------ ------
1    enet  100001     1500  -      -      -        -    -        0      0
";
        let rows = parse_vlan(output);
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows[0],
            vec!["1", "default", "active", "Gi1/0/1, Gi1/0/2, Gi1/0/3",]
        );
        assert_eq!(rows[1], vec!["10", "VLAN0010", "active", ""]);
        assert_eq!(rows[2], vec!["1002", "fddi-default", "act/unsup", ""]);
    }

    #[test]
    fn csv_output_quotes_commas() {
        assert_eq!(escape_csv_field("Gi1/0/1, Gi1/0/2"), "\"Gi1/0/1, Gi1/0/2\"");
    }

    const INTERFACES: &str = "\
GigabitEthernet0/1 is up, line protocol is up
  5 minute input rate 12000 bits/sec, 15 packets/sec
  5 minute output rate 8000 bits/sec, 9 packets/sec
     0 input errors, 0 CRC, 0 frame, 0 overrun
     2 output errors, 0 collisions, 1 interface resets
GigabitEthernet0/2 is administratively down, line protocol is down
     5 input errors, 3 CRC, 0 frame, 0 overrun
";

    const ROUTES: &str = "\
Codes: O - OSPF, D - EIGRP
O     10.0.0.0/24 [110/2] via 192.0.2.1, 00:01:00, GigabitEthernet0/1
O     10.0.1.0/24 [110/2] via 192.0.2.1, 00:01:00, GigabitEthernet0/1
D EX  10.9.0.0/16 [170/3] via 192.0.2.9, 00:02:00, GigabitEthernet0/2
C     192.0.2.0/24 is directly connected, GigabitEthernet0/1
";

    const EIGRP: &str = "\
P 10.0.0.0/24, 1 successors, FD is 2816
        via 192.0.2.1 (2816/256), GigabitEthernet0/1
P 10.0.1.0/24, 1 successors, FD is 2816
        via 192.0.2.1 (2816/256), GigabitEthernet0/1
P 10.9.0.0/16, 1 successors, FD is 3072
        via 192.0.2.9 (3072/512), GigabitEthernet0/2
";

    // Captured from the licensed vWLC 8.7.102 over the QEMU serial console.
    const REAL_AP_SUMMARY: &str = "\
Number of APs.................................... 0

Global AP User Name.............................. Not Configured
";

    const REAL_AUTH_LIST: &str = "\
Authorize MIC APs against Auth-list or AAA ...... disabled
Authorize LSC APs against Auth-List ............. disabled
APs Allowed to Join
  AP with Manufacturing Installed Certificate.... yes
";

    const REAL_INTERFACES: &str = "\
 Number of Interfaces.......................... 3

Interface Name                   Port Vlan Id  IP Address      Type    Ap Mgr Guest
-------------------------------- ---- -------- --------------- ------- ------ -----
management                       1    untagged 10.0.2.16       Static  Yes    N/A
service-port                     N/A  N/A      0.0.0.0         DHCP    No     N/A
virtual                          N/A  N/A      10.0.3.1        Static  No     N/A
";

    const REAL_MOBILITY: &str = "\
Mobility Protocol Port........................... 16666
Default Mobility Domain.......................... lab

Controllers configured in the Mobility Group
 MAC Address        IP Address                                       Group Name                        Multicast IP                                     Status
 02:00:00:00:00:02  10.0.2.16                                        lab                               0.0.0.0                                          Up
";

    const REAL_WLANS: &str = "\
Number of WLANs.................................. 1

WLAN ID  WLAN Profile Name / SSID                                                  Status    Interface Name
-------  -------------------------------------                                    --------  --------------------
1        lab / lab                                                                Enabled   management
";

    #[test]
    fn real_aireos_output_parses_into_documented_rows() {
        assert!(parse_ap_summary(REAL_AP_SUMMARY).is_empty());
        assert_eq!(
            parse_aireos_kv(REAL_AUTH_LIST)[0],
            vec!["Authorize MIC APs against Auth-list or AAA", "disabled"]
        );
        let interfaces = parse_aireos_interfaces(REAL_INTERFACES);
        assert_eq!(
            interfaces[0],
            vec!["management", "1", "untagged", "10.0.2.16", "Static"]
        );
        let mobility = parse_mobility(REAL_MOBILITY);
        assert_eq!(
            mobility[0],
            vec!["02:00:00:00:00:02", "10.0.2.16", "lab", "0.0.0.0", "Up"]
        );
        let wlans = parse_wlan_summary(REAL_WLANS);
        assert_eq!(wlans[0], vec!["1", "lab", "lab", "Enabled", "management"]);
    }

    const AIREOS_APS: &str = "\
AP Name            Slots  AP Model   Ethernet MAC    Location
AP-1               2      AIR-AP3802 00:11:22:33:44:55  Floor1
AP-2               2      AIR-AP2802 00:11:22:33:44:66  Floor2
";

    #[test]
    fn interface_stats_reads_rates_and_errors() {
        let rows = parse_interface_stats(INTERFACES);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][0], "GigabitEthernet0/1");
        assert_eq!(rows[0][1], "up");
        assert_eq!(rows[0][2], "12000");
        assert_eq!(rows[0][4], "0");
        assert_eq!(rows[0][5], "0");
        assert_eq!(rows[1][1], "administratively down");
    }

    #[test]
    fn next_hop_aggregates_routes_and_protocols() {
        let rows = parse_next_hop(ROUTES);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], vec!["192.0.2.1", "2", "O"]);
        assert_eq!(rows[1], vec!["192.0.2.9", "1", "D"]);
    }

    #[test]
    fn eigrp_topology_and_summary_share_the_same_source() {
        let rows = parse_eigrp_topology(EIGRP);
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows[0],
            vec![
                "10.0.0.0/24",
                "1",
                "2816",
                "192.0.2.1",
                "GigabitEthernet0/1"
            ]
        );
        let summary = parse_eigrp_summary(EIGRP);
        assert_eq!(
            summary,
            vec![vec!["192.0.2.1", "2"], vec!["192.0.2.9", "1"]]
        );
    }

    #[test]
    fn aireos_ap_table_ignores_headers_and_rules() {
        let rows = parse_ap_summary(AIREOS_APS);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][0], "AP-1");
        assert_eq!(rows[1][4], "Floor2");
    }

    const CISCO_BGP: &str = "\
Neighbor        V    AS MsgRcvd MsgSent   TblVer  InQ OutQ Up/Down  State/PfxRcd
10.0.0.2        4  65002     100     100       10    0    0 00:10:00        2
10.0.0.3        4  65003      50      50       10    0    0 00:05:00        1
";
    // Captured from the Cisco 3745 / IOS 12.4(25d) lab router over console.
    const REAL_CISCO_BGP: &str = "\
BGP router identifier 10.0.0.1, local AS number 65001
BGP table version is 1, main routing table version 1

Neighbor        V    AS MsgRcvd MsgSent   TblVer  InQ OutQ Up/Down  State/PfxRcd
10.0.0.2        4 65002       0       0        0    0    0 never    Idle
";

    const HUAWEI_BGP: &str = "\
BGP local router ID : 1.1.1.1
Local AS number : 65001
Peer        V          AS  MsgRcvd  MsgSent  OutQ  Up/Down       State
2.2.2.2     4       65002      100      100     0  00:10:00        Established
";
    const JUNIPER_BGP: &str = "\
Groups: 1 Peers: 1 Down peers: 0
Peer             AS  InPkt  OutPkt  OutQ  Flaps Last Up/Dwn State
10.0.0.2      65002     100     100     0      0    00:10:00 1/1/1/0
";

    #[test]
    fn bgp_parsers_handle_cisco_huawei_and_junos() {
        let cisco = parse_bgp(CISCO_BGP);
        assert_eq!(cisco[0], vec!["10.0.0.2", "65002", "00:10:00", "2"]);
        let huawei = parse_bgp(HUAWEI_BGP);
        assert_eq!(
            huawei[0],
            vec!["2.2.2.2", "65002", "00:10:00", "Established"]
        );
        let juniper = parse_bgp(JUNIPER_BGP);
        assert_eq!(juniper[0], vec!["10.0.0.2", "65002", "00:10:00", "1/1/1/0"]);
        let real = parse_bgp(REAL_CISCO_BGP);
        assert_eq!(real[0], vec!["10.0.0.2", "65002", "never", "Idle"]);
    }

    #[test]
    fn every_new_parser_matches_its_header_width() {
        let cases = [
            (ReportKind::InterfaceStats, INTERFACES),
            (ReportKind::NextHop, ROUTES),
            (ReportKind::EigrpTopology, EIGRP),
            (ReportKind::EigrpSummary, EIGRP),
            (
                ReportKind::Switchport,
                "Port Name Status Vlan\n---- ---- ------ ----\nGi1/0/1 desk1 up 10\n",
            ),
            (ReportKind::AireosApDetail, REAL_AP_SUMMARY),
            (ReportKind::AireosApSummary, REAL_AP_SUMMARY),
            (ReportKind::AireosAuthList, REAL_AUTH_LIST),
            (ReportKind::AireosInterfaceDetail, REAL_INTERFACES),
            (ReportKind::AireosMobilityGroup, REAL_MOBILITY),
            (ReportKind::AireosWlanDetail, REAL_WLANS),
            (ReportKind::CiscoBgp, CISCO_BGP),
            (ReportKind::HuaweiBgp, HUAWEI_BGP),
            (ReportKind::JuniperBgp, JUNIPER_BGP),
        ];
        for (kind, output) in cases {
            let width = kind.headers().len() - 1;
            for row in kind.parse(output) {
                assert_eq!(row.len(), width, "{kind:?}");
            }
        }
    }
}
