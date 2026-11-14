//! Quick network scan: find the hosts of an IPv4 range that answer on the ports
//! RustConn can connect to.
//!
//! The scan is a plain TCP connect probe, so it needs no privilege and no
//! external tool. A host counts as *up* when any probed port either accepts the
//! connection or actively refuses it — a refusal is an answer from the host's
//! TCP stack, which is all "is something at this address" needs. A host behind
//! a firewall that drops every probed port stays invisible; that is the price
//! of not sending ICMP, which would need a raw socket.
//!
//! The target is written the way an administrator writes it:
//!
//! - a network with a prefix length — `192.168.1.0/24`
//! - a network with a dotted mask — `192.168.1.0/255.255.255.0`
//! - a first and a last address — `192.168.1.10-192.168.1.50`
//! - a first address and a last octet — `192.168.1.10-50`
//! - a single address — `192.168.1.10`

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::StreamExt;
use thiserror::Error;
use tokio::net::TcpStream;

/// Most addresses one scan may cover: a `/16`.
///
/// ponytail: a hard ceiling rather than a streamed, resumable scan. Anything
/// larger takes minutes even at full concurrency and is better split by the
/// user; lift it together with a paged result list if a `/8` is ever asked for.
pub const MAX_SCAN_HOSTS: u32 = 65_536;

/// How long one port gets to answer before it counts as filtered.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_millis(500);

/// How many addresses are probed at the same time.
///
/// Every address opens one socket per port, so this times the port count is the
/// number of descriptors a scan holds at its peak: 384 with the default ports,
/// which leaves room under the usual soft limit of 1024 for the sessions the
/// application already has open.
pub const DEFAULT_CONCURRENCY: usize = 64;

/// A port the scan probes, with the protocol RustConn would open on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanPort {
    /// TCP port number.
    pub port: u16,
    /// Short service name, shown next to the port.
    pub service: &'static str,
    /// The RustConn protocol identifier a connection to this port uses, when
    /// there is one (`ssh`, `rdp`, `vnc`, `telnet`).
    pub protocol: Option<&'static str>,
}

/// The ports probed by default: every protocol RustConn connects to directly,
/// plus the two web ports that tell a device with a management page apart.
pub const DEFAULT_PORTS: [ScanPort; 6] = [
    ScanPort {
        port: 22,
        service: "SSH",
        protocol: Some("ssh"),
    },
    ScanPort {
        port: 3389,
        service: "RDP",
        protocol: Some("rdp"),
    },
    ScanPort {
        port: 5900,
        service: "VNC",
        protocol: Some("vnc"),
    },
    ScanPort {
        port: 23,
        service: "Telnet",
        protocol: Some("telnet"),
    },
    ScanPort {
        port: 80,
        service: "HTTP",
        protocol: None,
    },
    ScanPort {
        port: 443,
        service: "HTTPS",
        protocol: None,
    },
];

/// Returns the default-port entry for `port`, when it is one of them.
#[must_use]
pub fn known_port(port: u16) -> Option<ScanPort> {
    DEFAULT_PORTS.iter().copied().find(|p| p.port == port)
}

/// Why a scan target could not be understood.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum NetworkScanError {
    /// Nothing was entered.
    #[error("no address range given")]
    Empty,
    /// A part that should be an IPv4 address is not one.
    #[error("'{0}' is not an IPv4 address")]
    InvalidAddress(String),
    /// The part after `/` is neither a prefix length nor a netmask.
    #[error("'{0}' is not a prefix length (0-32) or a netmask")]
    InvalidMask(String),
    /// The last address comes before the first.
    #[error("the last address {end} is before the first address {start}")]
    ReversedRange {
        /// First address of the range.
        start: Ipv4Addr,
        /// Last address of the range.
        end: Ipv4Addr,
    },
    /// The range covers more than [`MAX_SCAN_HOSTS`] addresses.
    #[error("the range covers {count} addresses, more than the {max} a scan allows")]
    TooLarge {
        /// Addresses in the requested range.
        count: u64,
        /// The ceiling, [`MAX_SCAN_HOSTS`].
        max: u32,
    },
}

/// An inclusive range of IPv4 addresses to scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanTarget {
    first: u32,
    last: u32,
}

impl ScanTarget {
    /// Parses a target in any of the forms listed in the module documentation.
    ///
    /// For a network of more than two addresses the network address and the
    /// broadcast address are left out, since neither is a host.
    ///
    /// # Errors
    ///
    /// Returns a [`NetworkScanError`] naming the part that is wrong, or
    /// [`NetworkScanError::TooLarge`] when the range exceeds
    /// [`MAX_SCAN_HOSTS`].
    pub fn parse(input: &str) -> Result<Self, NetworkScanError> {
        let input = input.trim();
        if input.is_empty() {
            return Err(NetworkScanError::Empty);
        }

        if let Some((address, mask)) = input.split_once('/') {
            let address = parse_address(address)?;
            let prefix = parse_prefix(mask)?;
            return Self::from_network(address, prefix);
        }

        if let Some((start, end)) = input.split_once('-') {
            let start = parse_address(start)?;
            let end = end.trim();
            // `192.168.1.10-50`: the last octet alone.
            let end = match end.parse::<u8>() {
                Ok(octet) => {
                    let [a, b, c, _] = start.octets();
                    Ipv4Addr::new(a, b, c, octet)
                }
                Err(_) => parse_address(end)?,
            };
            return Self::from_range(start, end);
        }

        let address = parse_address(input)?;
        Self::from_range(address, address)
    }

    /// Builds a target from a first and a last address, both included.
    ///
    /// # Errors
    ///
    /// Returns [`NetworkScanError::ReversedRange`] when `end` is before `start`
    /// and [`NetworkScanError::TooLarge`] when the range exceeds
    /// [`MAX_SCAN_HOSTS`].
    pub fn from_range(start: Ipv4Addr, end: Ipv4Addr) -> Result<Self, NetworkScanError> {
        let (first, last) = (u32::from(start), u32::from(end));
        if last < first {
            return Err(NetworkScanError::ReversedRange { start, end });
        }
        Self::checked(first, last)
    }

    /// Builds a target from any address of a network and its prefix length.
    fn from_network(address: Ipv4Addr, prefix: u8) -> Result<Self, NetworkScanError> {
        let mask = if prefix == 0 {
            0
        } else {
            u32::MAX << (32 - u32::from(prefix))
        };
        let network = u32::from(address) & mask;
        let broadcast = network | !mask;
        // A /31 is a point-to-point link and a /32 a single host: every address
        // in them is a host, so nothing is trimmed.
        let (first, last) = if prefix >= 31 {
            (network, broadcast)
        } else {
            (network + 1, broadcast - 1)
        };
        Self::checked(first, last)
    }

    /// Applies the size ceiling.
    fn checked(first: u32, last: u32) -> Result<Self, NetworkScanError> {
        let count = u64::from(last - first) + 1;
        if count > u64::from(MAX_SCAN_HOSTS) {
            return Err(NetworkScanError::TooLarge {
                count,
                max: MAX_SCAN_HOSTS,
            });
        }
        Ok(Self { first, last })
    }

    /// First address scanned.
    #[must_use]
    pub const fn first(&self) -> Ipv4Addr {
        Ipv4Addr::from_bits(self.first)
    }

    /// Last address scanned.
    #[must_use]
    pub const fn last(&self) -> Ipv4Addr {
        Ipv4Addr::from_bits(self.last)
    }

    /// Number of addresses in the range; never zero.
    #[must_use]
    pub const fn host_count(&self) -> u32 {
        self.last - self.first + 1
    }

    /// The addresses of the range, in ascending order.
    pub fn hosts(&self) -> impl Iterator<Item = Ipv4Addr> + use<> {
        (self.first..=self.last).map(Ipv4Addr::from_bits)
    }
}

/// Parses one IPv4 address, reporting the offending text on failure.
fn parse_address(text: &str) -> Result<Ipv4Addr, NetworkScanError> {
    let text = text.trim();
    text.parse()
        .map_err(|_| NetworkScanError::InvalidAddress(text.to_string()))
}

/// Parses what follows the `/`: a prefix length or a dotted netmask.
fn parse_prefix(text: &str) -> Result<u8, NetworkScanError> {
    let text = text.trim();
    let invalid = || NetworkScanError::InvalidMask(text.to_string());

    if let Ok(prefix) = text.parse::<u8>() {
        return if prefix <= 32 {
            Ok(prefix)
        } else {
            Err(invalid())
        };
    }

    let mask = u32::from(text.parse::<Ipv4Addr>().map_err(|_| invalid())?);
    // A netmask is a run of ones followed by a run of zeros and nothing else.
    let prefix = mask.leading_ones();
    if mask.checked_shl(prefix).unwrap_or(0) != 0 {
        return Err(invalid());
    }
    u8::try_from(prefix).map_err(|_| invalid())
}

/// What the scan found at one address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostResult {
    /// The address probed.
    pub address: Ipv4Addr,
    /// The probed ports that accepted a connection, in the order they were asked.
    pub open_ports: Vec<u16>,
}

impl HostResult {
    /// The protocol RustConn would most naturally use for this host: the first
    /// open port, in probe order, that maps to one.
    #[must_use]
    pub fn suggested_port(&self) -> Option<ScanPort> {
        self.open_ports
            .iter()
            .filter_map(|&port| known_port(port))
            .find(|port| port.protocol.is_some())
    }
}

/// The tuning knobs of a scan.
#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// Ports probed on every address, in order of preference.
    pub ports: Vec<u16>,
    /// How long one port gets to answer.
    pub timeout: Duration,
    /// How many addresses are probed at the same time; at least one.
    pub concurrency: usize,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            ports: DEFAULT_PORTS.iter().map(|p| p.port).collect(),
            timeout: DEFAULT_TIMEOUT,
            concurrency: DEFAULT_CONCURRENCY,
        }
    }
}

/// One step of a running scan, handed to the progress callback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanProgress {
    /// Addresses finished so far, this one included.
    pub done: u32,
    /// Addresses in the whole scan.
    pub total: u32,
    /// The host found at this address, when it answered.
    pub host: Option<HostResult>,
}

/// How one port answered.
enum PortState {
    Open,
    /// Actively refused: the host is there, the port is closed.
    Closed,
    /// No answer in time, or no route.
    Silent,
}

/// Probes one port of one address.
async fn probe_port(address: Ipv4Addr, port: u16, timeout: Duration) -> PortState {
    let socket = SocketAddr::new(IpAddr::V4(address), port);
    match tokio::time::timeout(timeout, TcpStream::connect(socket)).await {
        Ok(Ok(_stream)) => PortState::Open,
        Ok(Err(error)) if error.kind() == std::io::ErrorKind::ConnectionRefused => {
            PortState::Closed
        }
        Ok(Err(_)) | Err(_) => PortState::Silent,
    }
}

/// Probes every port of one address at once; `None` when nothing answered.
async fn probe_host(address: Ipv4Addr, ports: &[u16], timeout: Duration) -> Option<HostResult> {
    let states =
        futures::future::join_all(ports.iter().map(|&port| probe_port(address, port, timeout)))
            .await;

    let mut answered = false;
    let mut open_ports = Vec::new();
    for (&port, state) in ports.iter().zip(states) {
        match state {
            PortState::Open => {
                answered = true;
                open_ports.push(port);
            }
            PortState::Closed => answered = true,
            PortState::Silent => {}
        }
    }
    answered.then_some(HostResult {
        address,
        open_ports,
    })
}

/// Scans `target`, calling `on_progress` once per address as it finishes.
///
/// Addresses finish out of order — a host that answers is done long before one
/// that times out — so the returned list is sorted by address before it is
/// handed back. Setting `cancel` stops the scan at the next finished address;
/// the hosts found so far are still returned.
///
/// Must be awaited on a tokio runtime.
pub async fn scan<F>(
    target: &ScanTarget,
    options: &ScanOptions,
    cancel: &AtomicBool,
    mut on_progress: F,
) -> Vec<HostResult>
where
    F: FnMut(ScanProgress),
{
    let total = target.host_count();
    let ports = options.ports.as_slice();
    let timeout = options.timeout;

    let mut pending = futures::stream::iter(target.hosts())
        .map(|address| probe_host(address, ports, timeout))
        .buffer_unordered(options.concurrency.max(1));

    let mut found = Vec::new();
    let mut done = 0;
    while let Some(host) = pending.next().await {
        if cancel.load(Ordering::Relaxed) {
            tracing::debug!(done, total, "network scan cancelled");
            break;
        }
        done += 1;
        on_progress(ScanProgress {
            done,
            total,
            host: host.clone(),
        });
        found.extend(host);
    }

    found.sort_by_key(|host| host.address);
    tracing::debug!(
        first = %target.first(),
        last = %target.last(),
        scanned = done,
        found = found.len(),
        "network scan finished"
    );
    found
}

/// Runs [`scan`] to completion on a runtime of its own, blocking the caller.
///
/// For callers without a tokio runtime at hand — a worker thread of the GUI, or
/// the command line. Never call it on a thread that must stay responsive.
///
/// # Errors
///
/// Returns the I/O error of a runtime that could not be created.
pub fn scan_blocking<F>(
    target: &ScanTarget,
    options: &ScanOptions,
    cancel: &AtomicBool,
    on_progress: F,
) -> std::io::Result<Vec<HostResult>>
where
    F: FnMut(ScanProgress),
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()?;
    Ok(runtime.block_on(scan(target, options, cancel, on_progress)))
}

/// Guesses the network this machine is on, as a `/24` in CIDR notation.
///
/// Used to pre-fill the scan target. It asks the kernel which local address
/// would be used to reach a public one — connecting a UDP socket picks a route
/// and sends nothing — and assumes a `/24` around it, which is what most home
/// and office networks are. `None` when there is no IPv4 route.
#[must_use]
pub fn local_network_guess() -> Option<String> {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    // 192.0.2.1 is TEST-NET-1 (RFC 5737): routable syntax, never a real host.
    socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(local) if !local.is_loopback() && !local.is_unspecified() => {
            let [a, b, c, _] = local.octets();
            Some(format!("{a}.{b}.{c}.0/24"))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(input: &str) -> (String, String, u32) {
        let target = ScanTarget::parse(input).expect("target should parse");
        (
            target.first().to_string(),
            target.last().to_string(),
            target.host_count(),
        )
    }

    #[test]
    fn cidr_leaves_out_network_and_broadcast() {
        assert_eq!(
            range("192.168.1.0/24"),
            ("192.168.1.1".into(), "192.168.1.254".into(), 254)
        );
    }

    #[test]
    fn cidr_accepts_any_address_of_the_network() {
        assert_eq!(range("192.168.1.77/24"), range("192.168.1.0/24"));
    }

    #[test]
    fn dotted_netmask_equals_prefix_length() {
        assert_eq!(range("10.0.0.0/255.255.255.192"), range("10.0.0.0/26"));
        assert_eq!(range("10.0.0.0/26").2, 62);
    }

    #[test]
    fn point_to_point_and_host_networks_keep_every_address() {
        assert_eq!(range("10.0.0.4/31").2, 2);
        assert_eq!(
            range("10.0.0.4/32"),
            ("10.0.0.4".into(), "10.0.0.4".into(), 1)
        );
    }

    #[test]
    fn explicit_range_includes_both_ends() {
        assert_eq!(
            range("192.168.1.10-192.168.1.50"),
            ("192.168.1.10".into(), "192.168.1.50".into(), 41)
        );
        assert_eq!(range(" 192.168.1.10 - 192.168.1.50 ").2, 41);
    }

    #[test]
    fn range_may_cross_a_subnet_boundary() {
        assert_eq!(range("192.168.1.250-192.168.2.5").2, 12);
    }

    #[test]
    fn short_range_replaces_the_last_octet() {
        assert_eq!(range("192.168.1.10-50"), range("192.168.1.10-192.168.1.50"));
    }

    #[test]
    fn single_address_is_a_range_of_one() {
        assert_eq!(
            range("192.168.1.10"),
            ("192.168.1.10".into(), "192.168.1.10".into(), 1)
        );
    }

    #[test]
    fn hosts_are_listed_in_ascending_order() {
        let target = ScanTarget::parse("10.0.0.254-10.0.1.1").expect("target should parse");
        let hosts: Vec<String> = target.hosts().map(|h| h.to_string()).collect();
        assert_eq!(hosts, ["10.0.0.254", "10.0.0.255", "10.0.1.0", "10.0.1.1"]);
    }

    #[test]
    fn rejects_what_is_not_a_range() {
        assert_eq!(ScanTarget::parse("  "), Err(NetworkScanError::Empty));
        assert_eq!(
            ScanTarget::parse("server.example.com"),
            Err(NetworkScanError::InvalidAddress(
                "server.example.com".into()
            ))
        );
        assert_eq!(
            ScanTarget::parse("192.168.1.0/33"),
            Err(NetworkScanError::InvalidMask("33".into()))
        );
        // Ones after a zero: not a netmask.
        assert_eq!(
            ScanTarget::parse("192.168.1.0/255.0.255.0"),
            Err(NetworkScanError::InvalidMask("255.0.255.0".into()))
        );
        assert!(matches!(
            ScanTarget::parse("192.168.1.50-192.168.1.10"),
            Err(NetworkScanError::ReversedRange { .. })
        ));
    }

    #[test]
    fn rejects_a_range_above_the_ceiling() {
        assert_eq!(range("10.0.0.0-10.0.255.255").2, MAX_SCAN_HOSTS);
        assert_eq!(
            ScanTarget::parse("10.0.0.0/8"),
            Err(NetworkScanError::TooLarge {
                count: 16_777_214,
                max: MAX_SCAN_HOSTS
            })
        );
        // The whole address space must not overflow the count.
        assert!(matches!(
            ScanTarget::parse("0.0.0.0/0"),
            Err(NetworkScanError::TooLarge { .. })
        ));
    }

    #[test]
    fn suggested_port_prefers_probe_order() {
        let host = HostResult {
            address: Ipv4Addr::LOCALHOST,
            open_ports: vec![443, 3389, 22],
        };
        assert_eq!(host.suggested_port().map(|p| p.port), Some(3389));

        let web_only = HostResult {
            address: Ipv4Addr::LOCALHOST,
            open_ports: vec![80, 443],
        };
        assert_eq!(web_only.suggested_port(), None);
    }

    #[tokio::test]
    async fn scan_finds_a_listening_port_and_a_refusing_host() {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("should bind a loopback port");
        let open_port = listener
            .local_addr()
            .expect("listener should have an address")
            .port();
        // A port nothing listens on: bind one, note its number, release it.
        let closed_port = {
            let probe = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
                .await
                .expect("should bind a loopback port");
            probe
                .local_addr()
                .expect("listener should have an address")
                .port()
        };

        let target = ScanTarget::parse("127.0.0.1").expect("target should parse");
        let cancel = AtomicBool::new(false);
        let mut steps = Vec::new();

        let options = ScanOptions {
            ports: vec![closed_port, open_port],
            ..ScanOptions::default()
        };
        let found = scan(&target, &options, &cancel, |step| steps.push(step)).await;
        assert_eq!(
            found,
            [HostResult {
                address: Ipv4Addr::LOCALHOST,
                open_ports: vec![open_port],
            }]
        );
        assert_eq!(steps.len(), 1);
        assert_eq!((steps[0].done, steps[0].total), (1, 1));

        // A refusal alone still proves the host is there.
        let options = ScanOptions {
            ports: vec![closed_port],
            ..ScanOptions::default()
        };
        let found = scan(&target, &options, &cancel, |_| {}).await;
        assert_eq!(
            found,
            [HostResult {
                address: Ipv4Addr::LOCALHOST,
                open_ports: Vec::new(),
            }]
        );
    }

    #[tokio::test]
    async fn cancelled_scan_stops_early() {
        let target = ScanTarget::parse("127.0.0.1-127.0.0.20").expect("target should parse");
        let cancel = AtomicBool::new(true);
        let mut steps = 0;
        let found = scan(&target, &ScanOptions::default(), &cancel, |_| steps += 1).await;
        assert!(found.is_empty());
        assert_eq!(steps, 0);
    }
}
