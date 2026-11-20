//! Port knocking and fwknop Single Packet Authorization (SPA)
//!
//! Provides pre-connect firewall traversal:
//! - **Port knocking**: sends a sequence of TCP SYN / UDP packets to open a firewall
//! - **fwknop SPA**: sends a single encrypted+HMAC'd UDP packet (AES-256-CBC + HMAC-SHA256)
//!
//! Both run before the main connection, inside the Flatpak sandbox (no external CLI needed).

use std::net::{SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ─────────────────────────────────────────────────────────────────────────────
// Port Knock
// ─────────────────────────────────────────────────────────────────────────────

/// Protocol for a single knock
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum KnockProtocol {
    /// TCP SYN (connect attempt)
    #[default]
    Tcp,
    /// UDP datagram
    Udp,
}

/// A single knock in the sequence
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Knock {
    /// Port number to knock
    pub port: u16,
    /// Protocol (TCP or UDP)
    #[serde(default)]
    pub protocol: KnockProtocol,
}

/// Configuration for a port knock sequence
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnockSequence {
    /// Ordered list of knocks to perform
    pub knocks: Vec<Knock>,
    /// Delay between knocks in milliseconds (default: 100)
    #[serde(default = "default_knock_delay_ms")]
    pub delay_ms: u32,
    /// Settle time after the last knock before connecting (ms, default: 200)
    #[serde(default = "default_settle_ms")]
    pub settle_ms: u32,
}

const fn default_knock_delay_ms() -> u32 {
    100
}
const fn default_settle_ms() -> u32 {
    200
}

impl KnockSequence {
    /// Creates a new knock sequence from port numbers (all TCP)
    #[must_use]
    pub fn from_tcp_ports(ports: &[u16]) -> Self {
        Self {
            knocks: ports
                .iter()
                .map(|&port| Knock {
                    port,
                    protocol: KnockProtocol::Tcp,
                })
                .collect(),
            delay_ms: default_knock_delay_ms(),
            settle_ms: default_settle_ms(),
        }
    }

    /// Parses a knock sequence from a string like "7000 8000/tcp 9000/udp"
    ///
    /// # Errors
    ///
    /// Returns an error if any token cannot be parsed.
    pub fn parse(input: &str) -> Result<Self, KnockError> {
        let mut knocks = Vec::new();
        for token in input.split([' ', ',', '\t']) {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            let (port_str, proto) = if let Some(p) = token.strip_suffix("/tcp") {
                (p, KnockProtocol::Tcp)
            } else if let Some(p) = token.strip_suffix("/udp") {
                (p, KnockProtocol::Udp)
            } else {
                (token, KnockProtocol::Tcp)
            };
            let port: u16 = port_str
                .parse()
                .map_err(|_| KnockError::InvalidSequence(format!("Invalid port: {token}")))?;
            knocks.push(Knock {
                port,
                protocol: proto,
            });
        }
        if knocks.is_empty() {
            return Err(KnockError::InvalidSequence(
                "Empty knock sequence".to_string(),
            ));
        }
        Ok(Self {
            knocks,
            delay_ms: default_knock_delay_ms(),
            settle_ms: default_settle_ms(),
        })
    }

    /// Returns a display string like "7000/tcp 8000/tcp 9000/udp"
    #[must_use]
    pub fn display(&self) -> String {
        self.knocks
            .iter()
            .map(|k| match k.protocol {
                KnockProtocol::Tcp => format!("{}/tcp", k.port),
                KnockProtocol::Udp => format!("{}/udp", k.port),
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SPA (fwknop) — configuration only; packet building is separate
// ─────────────────────────────────────────────────────────────────────────────

/// Source IP mode for SPA packets
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SpaAllowIp {
    /// Send 0.0.0.0 — fwknopd opens for the packet's source IP (default)
    #[default]
    SourceIp,
    /// Resolve public IP before sending (like `fwknop -R`)
    ResolvePublic,
    /// Use an explicit IP address
    Explicit(String),
}

/// fwknop SPA configuration per connection
///
/// Keys are wrapped in `SecretString` at runtime.
///
/// Where the keys live: a key is either in the secret store (`*_key_in_vault`
/// is `true` and the matching `*_key_ref` is `None`, see
/// [`super::spa_vault`]) or, only for files written before 0.26.0 and for a
/// key the connection dialog has just handed over, as text in
/// `*_key_ref` until the move to the vault is confirmed. `Debug` never prints a
/// key.
///
/// shortcut: the plaintext fields are `String`, not `SecretString`, because they
/// are (de)serialized for the pending/legacy state; upgrade when that state is
/// gone (0.27+).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaConfig {
    /// Rijndael (AES-256-CBC) key — passphrase or base64. Plaintext, pending
    /// the move to the vault; `None` once the key is in the vault.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rijndael_key_ref: Option<String>,
    /// HMAC key — passphrase or base64 (recommended). Plaintext, pending the
    /// move to the vault; `None` once the key is in the vault.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hmac_key_ref: Option<String>,
    /// The Rijndael key is in the secret store under
    /// [`super::spa_vault::spa_secret_key`].
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub rijndael_key_in_vault: bool,
    /// The HMAC key is in the secret store.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hmac_key_in_vault: bool,
    /// Access specification (e.g. "tcp/22" or "tcp/22,tcp/443")
    #[serde(default = "default_spa_access")]
    pub access: String,
    /// Destination UDP port for the SPA packet (default: 62201)
    #[serde(default = "default_spa_port")]
    pub dest_port: u16,
    /// How to determine the allow-IP in the packet
    #[serde(default)]
    pub allow_ip: SpaAllowIp,
}

impl std::fmt::Debug for SpaConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mask = |k: &Option<String>| k.as_ref().map(|_| "<redacted>");
        f.debug_struct("SpaConfig")
            .field("rijndael_key_ref", &mask(&self.rijndael_key_ref))
            .field("hmac_key_ref", &mask(&self.hmac_key_ref))
            .field("rijndael_key_in_vault", &self.rijndael_key_in_vault)
            .field("hmac_key_in_vault", &self.hmac_key_in_vault)
            .field("access", &self.access)
            .field("dest_port", &self.dest_port)
            .field("allow_ip", &self.allow_ip)
            .finish()
    }
}

fn default_spa_access() -> String {
    "tcp/22".to_string()
}

const fn default_spa_port() -> u16 {
    62201
}

impl SpaConfig {
    /// Creates a new SPA config with default access (tcp/22)
    #[must_use]
    pub fn new() -> Self {
        Self {
            rijndael_key_ref: None,
            hmac_key_ref: None,
            rijndael_key_in_vault: false,
            hmac_key_in_vault: false,
            access: default_spa_access(),
            dest_port: default_spa_port(),
            allow_ip: SpaAllowIp::default(),
        }
    }
}

impl Default for SpaConfig {
    fn default() -> Self {
        Self::new()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Errors
// ─────────────────────────────────────────────────────────────────────────────

/// Errors from knock/SPA operations
#[derive(Debug, Error)]
pub enum KnockError {
    /// Invalid knock sequence format
    #[error("Invalid knock sequence: {0}")]
    InvalidSequence(String),
    /// DNS resolution failed
    #[error("Failed to resolve host '{host}': {reason}")]
    ResolutionFailed {
        /// The hostname
        host: String,
        /// Why it failed
        reason: String,
    },
    /// A knock failed
    #[error("Knock to {host}:{port}/{proto} failed: {reason}")]
    KnockFailed {
        /// Target host
        host: String,
        /// Target port
        port: u16,
        /// Protocol
        proto: String,
        /// Reason
        reason: String,
    },
    /// I/O error
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Result of executing a knock sequence
#[derive(Debug, Clone)]
pub struct KnockResult {
    /// Per-knock timing in milliseconds
    pub timings_ms: Vec<u32>,
    /// Total time including settle
    pub total_ms: u32,
}

// ─────────────────────────────────────────────────────────────────────────────
// Execution
// ─────────────────────────────────────────────────────────────────────────────

/// Executes a port knock sequence against a host
///
/// Each TCP knock is a non-blocking connect attempt (the SYN itself is the knock;
/// we don't care whether the port is open or closed). Each UDP knock sends an
/// empty datagram.
///
/// # Errors
///
/// Returns an error if DNS resolution fails or I/O errors prevent sending.
pub fn execute_knock_sequence(
    host: &str,
    sequence: &KnockSequence,
) -> Result<KnockResult, KnockError> {
    let start = Instant::now();
    let mut timings = Vec::with_capacity(sequence.knocks.len());

    // ponytail: resolve once, reuse first address; fine for <20 knocks
    let base_addr: SocketAddr = format!("{host}:0")
        .to_socket_addrs()
        .map_err(|e| KnockError::ResolutionFailed {
            host: host.to_string(),
            reason: e.to_string(),
        })?
        .next()
        .ok_or_else(|| KnockError::ResolutionFailed {
            host: host.to_string(),
            reason: "No addresses found".to_string(),
        })?;

    let knock_timeout = Duration::from_millis(500);
    let delay = Duration::from_millis(u64::from(sequence.delay_ms));

    for (i, knock) in sequence.knocks.iter().enumerate() {
        let knock_start = Instant::now();
        let addr = SocketAddr::new(base_addr.ip(), knock.port);

        match knock.protocol {
            KnockProtocol::Tcp => {
                // TCP knock: attempt connect, ignore result (refused = port closed = knock received)
                let _ = TcpStream::connect_timeout(&addr, knock_timeout);
            }
            KnockProtocol::Udp => {
                // UDP knock: send empty datagram
                let socket = UdpSocket::bind("0.0.0.0:0")?;
                socket.set_write_timeout(Some(knock_timeout))?;
                let _ = socket.send_to(&[], addr);
            }
        }

        let elapsed = knock_start.elapsed().as_millis() as u32;
        timings.push(elapsed);

        tracing::debug!(
            knock_index = i,
            port = knock.port,
            proto = ?knock.protocol,
            elapsed_ms = elapsed,
            "Knock sent"
        );

        // Inter-knock delay (skip after last knock)
        if i + 1 < sequence.knocks.len() {
            std::thread::sleep(delay);
        }
    }

    // Post-knock settle time
    let settle = Duration::from_millis(u64::from(sequence.settle_ms));
    std::thread::sleep(settle);

    let total_ms = start.elapsed().as_millis() as u32;
    tracing::info!(
        host,
        knocks = sequence.knocks.len(),
        total_ms,
        "Knock sequence completed"
    );

    Ok(KnockResult {
        timings_ms: timings,
        total_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_knock_sequence_parse_simple() {
        let seq = KnockSequence::parse("7000 8000 9000").unwrap();
        assert_eq!(seq.knocks.len(), 3);
        assert_eq!(seq.knocks[0].port, 7000);
        assert_eq!(seq.knocks[0].protocol, KnockProtocol::Tcp);
    }

    #[test]
    fn test_knock_sequence_parse_mixed_protocols() {
        let seq = KnockSequence::parse("7000/tcp 8000/udp 9000/tcp").unwrap();
        assert_eq!(seq.knocks.len(), 3);
        assert_eq!(seq.knocks[1].protocol, KnockProtocol::Udp);
    }

    #[test]
    fn test_knock_sequence_parse_comma_separated() {
        let seq = KnockSequence::parse("7000,8000,9000").unwrap();
        assert_eq!(seq.knocks.len(), 3);
    }

    #[test]
    fn test_knock_sequence_parse_empty_rejected() {
        assert!(KnockSequence::parse("").is_err());
    }

    #[test]
    fn test_knock_sequence_parse_invalid_port() {
        assert!(KnockSequence::parse("7000 abc 9000").is_err());
    }

    #[test]
    fn test_knock_sequence_display() {
        let seq = KnockSequence::parse("7000/tcp 8000/udp").unwrap();
        assert_eq!(seq.display(), "7000/tcp 8000/udp");
    }

    #[test]
    fn test_knock_sequence_from_tcp_ports() {
        let seq = KnockSequence::from_tcp_ports(&[1000, 2000, 3000]);
        assert_eq!(seq.knocks.len(), 3);
        assert!(seq.knocks.iter().all(|k| k.protocol == KnockProtocol::Tcp));
    }

    #[test]
    fn test_spa_config_default() {
        let cfg = SpaConfig::new();
        assert_eq!(cfg.dest_port, 62201);
        assert_eq!(cfg.access, "tcp/22");
        assert_eq!(cfg.allow_ip, SpaAllowIp::SourceIp);
    }

    #[test]
    fn test_knock_sequence_serialization_roundtrip() {
        let seq = KnockSequence::parse("7000/tcp 8000/udp 9000").unwrap();
        let toml_str = toml::to_string(&seq).unwrap();
        let restored: KnockSequence = toml::from_str(&toml_str).unwrap();
        assert_eq!(restored.knocks.len(), 3);
        assert_eq!(restored.knocks[1].protocol, KnockProtocol::Udp);
        assert_eq!(restored.delay_ms, 100);
        assert_eq!(restored.settle_ms, 200);
    }

    #[test]
    fn test_spa_config_serialization_roundtrip() {
        let cfg = SpaConfig {
            rijndael_key_ref: Some("spa-key-myhost".to_string()),
            hmac_key_ref: Some("spa-hmac-myhost".to_string()),
            rijndael_key_in_vault: false,
            hmac_key_in_vault: false,
            access: "tcp/22,tcp/443".to_string(),
            dest_port: 62201,
            allow_ip: SpaAllowIp::ResolvePublic,
        };
        let toml_str = toml::to_string(&cfg).unwrap();
        let restored: SpaConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(restored.access, "tcp/22,tcp/443");
        assert_eq!(restored.allow_ip, SpaAllowIp::ResolvePublic);
    }
}
