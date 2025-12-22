//! Wake On LAN (WOL) support for `RustConn`
//!
//! This module provides functionality to wake sleeping machines before connecting
//! by sending magic packets to their MAC addresses.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::net::UdpSocket;
use std::str::FromStr;
use thiserror::Error;

/// Errors related to Wake On LAN operations
#[derive(Debug, Error)]
pub enum WolError {
    /// Invalid MAC address format
    #[error("Invalid MAC address format: {0}")]
    InvalidMacFormat(String),

    /// Invalid MAC address byte value
    #[error("Invalid MAC address byte: {0}")]
    InvalidMacByte(String),

    /// Failed to create UDP socket
    #[error("Failed to create UDP socket: {0}")]
    SocketError(String),

    /// Failed to send magic packet
    #[error("Failed to send magic packet: {0}")]
    SendError(String),

    /// Failed to set socket options
    #[error("Failed to set socket options: {0}")]
    SocketOptionError(String),
}

/// Result type alias for WOL operations
pub type WolResult<T> = std::result::Result<T, WolError>;

/// A MAC (Media Access Control) address for Wake On LAN
///
/// Represents a 6-byte hardware address used to identify network interfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct MacAddress([u8; 6]);

impl MacAddress {
    /// Creates a new `MacAddress` from raw bytes
    #[must_use]
    pub const fn new(bytes: [u8; 6]) -> Self {
        Self(bytes)
    }

    /// Returns the raw bytes of the MAC address
    #[must_use]
    pub const fn bytes(&self) -> &[u8; 6] {
        &self.0
    }

    /// Parses a MAC address from a string
    ///
    /// Supports both colon (`:`) and dash (`-`) separators.
    ///
    /// # Errors
    ///
    /// Returns `WolError::InvalidMacFormat` if the format is invalid or
    /// `WolError::InvalidMacByte` if a byte value is not valid hexadecimal.
    ///
    /// # Examples
    /// ```
    /// use rustconn_core::wol::MacAddress;
    /// use std::str::FromStr;
    ///
    /// let mac1 = MacAddress::from_str("AA:BB:CC:DD:EE:FF").unwrap();
    /// let mac2 = MacAddress::from_str("AA-BB-CC-DD-EE-FF").unwrap();
    /// assert_eq!(mac1, mac2);
    /// ```
    pub fn parse(input: &str) -> WolResult<Self> {
        input.parse()
    }

    /// Formats the MAC address using colon separators
    #[must_use]
    pub fn format_colon(&self) -> String {
        format!(
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            self.0[0], self.0[1], self.0[2], self.0[3], self.0[4], self.0[5]
        )
    }

    /// Formats the MAC address using dash separators
    #[must_use]
    pub fn format_dash(&self) -> String {
        format!(
            "{:02X}-{:02X}-{:02X}-{:02X}-{:02X}-{:02X}",
            self.0[0], self.0[1], self.0[2], self.0[3], self.0[4], self.0[5]
        )
    }
}

impl FromStr for MacAddress {
    type Err = WolError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();

        // Determine separator
        let separator = if trimmed.contains(':') {
            ':'
        } else if trimmed.contains('-') {
            '-'
        } else {
            return Err(WolError::InvalidMacFormat(
                "MAC address must use ':' or '-' as separator".to_string(),
            ));
        };

        let parts: Vec<&str> = trimmed.split(separator).collect();

        if parts.len() != 6 {
            return Err(WolError::InvalidMacFormat(format!(
                "MAC address must have 6 octets, found {}",
                parts.len()
            )));
        }

        let mut bytes = [0u8; 6];
        for (i, part) in parts.iter().enumerate() {
            if part.len() != 2 {
                return Err(WolError::InvalidMacFormat(format!(
                    "Each octet must be 2 hex digits, found '{}' with {} digits",
                    part,
                    part.len()
                )));
            }
            bytes[i] = u8::from_str_radix(part, 16)
                .map_err(|_| WolError::InvalidMacByte(format!("Invalid hex value: '{part}'")))?;
        }

        Ok(Self(bytes))
    }
}

impl fmt::Display for MacAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.format_colon())
    }
}

impl From<MacAddress> for String {
    fn from(mac: MacAddress) -> Self {
        mac.to_string()
    }
}

impl TryFrom<String> for MacAddress {
    type Error = WolError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

/// Default WOL port (discard protocol)
pub const DEFAULT_WOL_PORT: u16 = 9;

/// Default broadcast address
pub const DEFAULT_BROADCAST_ADDRESS: &str = "255.255.255.255";

/// Default wait time in seconds after sending WOL packet
pub const DEFAULT_WOL_WAIT_SECONDS: u32 = 30;

/// Wake On LAN configuration for a connection
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WolConfig {
    /// MAC address of the target machine
    pub mac_address: MacAddress,
    /// Broadcast address to send the magic packet to
    #[serde(default = "default_broadcast_address")]
    pub broadcast_address: String,
    /// UDP port to send the magic packet to
    #[serde(default = "default_wol_port")]
    pub port: u16,
    /// Seconds to wait after sending the packet before attempting connection
    #[serde(default = "default_wait_seconds")]
    pub wait_seconds: u32,
}

fn default_broadcast_address() -> String {
    DEFAULT_BROADCAST_ADDRESS.to_string()
}

const fn default_wol_port() -> u16 {
    DEFAULT_WOL_PORT
}

const fn default_wait_seconds() -> u32 {
    DEFAULT_WOL_WAIT_SECONDS
}

impl WolConfig {
    /// Creates a new WOL configuration with the given MAC address
    #[must_use]
    pub fn new(mac_address: MacAddress) -> Self {
        Self {
            mac_address,
            broadcast_address: DEFAULT_BROADCAST_ADDRESS.to_string(),
            port: DEFAULT_WOL_PORT,
            wait_seconds: DEFAULT_WOL_WAIT_SECONDS,
        }
    }

    /// Sets the broadcast address
    #[must_use]
    pub fn with_broadcast_address(mut self, address: impl Into<String>) -> Self {
        self.broadcast_address = address.into();
        self
    }

    /// Sets the UDP port
    #[must_use]
    pub const fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Sets the wait time in seconds
    #[must_use]
    pub const fn with_wait_seconds(mut self, seconds: u32) -> Self {
        self.wait_seconds = seconds;
        self
    }
}

/// Magic packet size: 6 bytes of 0xFF + 16 repetitions of 6-byte MAC address
pub const MAGIC_PACKET_SIZE: usize = 6 + (16 * 6);

/// Generates a Wake On LAN magic packet for the given MAC address
///
/// The magic packet consists of:
/// - 6 bytes of 0xFF (synchronization stream)
/// - 16 repetitions of the target MAC address (96 bytes)
///
/// Total size: 102 bytes
#[must_use]
pub fn generate_magic_packet(mac: &MacAddress) -> [u8; MAGIC_PACKET_SIZE] {
    let mut packet = [0u8; MAGIC_PACKET_SIZE];

    // First 6 bytes are 0xFF
    packet[..6].fill(0xFF);

    // Next 96 bytes are 16 repetitions of the MAC address
    let mac_bytes = mac.bytes();
    for i in 0..16 {
        let offset = 6 + (i * 6);
        packet[offset..offset + 6].copy_from_slice(mac_bytes);
    }

    packet
}

/// Sends a Wake On LAN magic packet to wake a sleeping machine
///
/// # Arguments
/// * `mac` - The MAC address of the target machine
/// * `broadcast` - The broadcast address to send to (e.g., "255.255.255.255")
/// * `port` - The UDP port to send to (typically 9 or 7)
///
/// # Errors
/// Returns an error if the socket cannot be created or the packet cannot be sent.
pub fn send_magic_packet(mac: &MacAddress, broadcast: &str, port: u16) -> WolResult<()> {
    let packet = generate_magic_packet(mac);

    // Create UDP socket
    let socket = UdpSocket::bind("0.0.0.0:0").map_err(|e| WolError::SocketError(e.to_string()))?;

    // Enable broadcast
    socket
        .set_broadcast(true)
        .map_err(|e| WolError::SocketOptionError(e.to_string()))?;

    // Send the magic packet
    let target = format!("{broadcast}:{port}");
    socket
        .send_to(&packet, &target)
        .map_err(|e| WolError::SendError(e.to_string()))?;

    Ok(())
}

/// Sends a Wake On LAN magic packet using the provided configuration
///
/// # Errors
/// Returns an error if the packet cannot be sent.
pub fn send_wol(config: &WolConfig) -> WolResult<()> {
    send_magic_packet(&config.mac_address, &config.broadcast_address, config.port)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mac_address_parse_colon() {
        let mac = MacAddress::parse("AA:BB:CC:DD:EE:FF").unwrap();
        assert_eq!(mac.bytes(), &[0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
    }

    #[test]
    fn test_mac_address_parse_dash() {
        let mac = MacAddress::parse("AA-BB-CC-DD-EE-FF").unwrap();
        assert_eq!(mac.bytes(), &[0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
    }

    #[test]
    fn test_mac_address_parse_lowercase() {
        let mac = MacAddress::parse("aa:bb:cc:dd:ee:ff").unwrap();
        assert_eq!(mac.bytes(), &[0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
    }

    #[test]
    fn test_mac_address_format_colon() {
        let mac = MacAddress::new([0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
        assert_eq!(mac.format_colon(), "AA:BB:CC:DD:EE:FF");
    }

    #[test]
    fn test_mac_address_format_dash() {
        let mac = MacAddress::new([0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
        assert_eq!(mac.format_dash(), "AA-BB-CC-DD-EE-FF");
    }

    #[test]
    fn test_mac_address_display() {
        let mac = MacAddress::new([0x00, 0x11, 0x22, 0x33, 0x44, 0x55]);
        assert_eq!(mac.to_string(), "00:11:22:33:44:55");
    }

    #[test]
    fn test_mac_address_invalid_format() {
        assert!(MacAddress::parse("AABBCCDDEEFF").is_err());
        assert!(MacAddress::parse("AA:BB:CC:DD:EE").is_err());
        assert!(MacAddress::parse("AA:BB:CC:DD:EE:FF:00").is_err());
        assert!(MacAddress::parse("GG:HH:II:JJ:KK:LL").is_err());
    }

    #[test]
    fn test_magic_packet_format() {
        let mac = MacAddress::new([0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
        let packet = generate_magic_packet(&mac);

        // Check size
        assert_eq!(packet.len(), MAGIC_PACKET_SIZE);
        assert_eq!(packet.len(), 102);

        // Check first 6 bytes are 0xFF
        assert!(packet[..6].iter().all(|&b| b == 0xFF));

        // Check 16 repetitions of MAC address
        for i in 0..16 {
            let offset = 6 + (i * 6);
            assert_eq!(&packet[offset..offset + 6], mac.bytes());
        }
    }

    #[test]
    fn test_wol_config_defaults() {
        let mac = MacAddress::new([0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
        let config = WolConfig::new(mac);

        assert_eq!(config.broadcast_address, "255.255.255.255");
        assert_eq!(config.port, 9);
        assert_eq!(config.wait_seconds, 30);
    }

    #[test]
    fn test_wol_config_builder() {
        let mac = MacAddress::new([0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
        let config = WolConfig::new(mac)
            .with_broadcast_address("192.168.1.255")
            .with_port(7)
            .with_wait_seconds(60);

        assert_eq!(config.broadcast_address, "192.168.1.255");
        assert_eq!(config.port, 7);
        assert_eq!(config.wait_seconds, 60);
    }

    #[test]
    fn test_wol_config_serialization() {
        let mac = MacAddress::new([0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
        let config = WolConfig::new(mac);

        let json = serde_json::to_string(&config).unwrap();
        let parsed: WolConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(config, parsed);
    }

    #[test]
    fn test_mac_address_serialization() {
        let mac = MacAddress::new([0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);

        let json = serde_json::to_string(&mac).unwrap();
        assert_eq!(json, "\"AA:BB:CC:DD:EE:FF\"");

        let parsed: MacAddress = serde_json::from_str(&json).unwrap();
        assert_eq!(mac, parsed);
    }
}
