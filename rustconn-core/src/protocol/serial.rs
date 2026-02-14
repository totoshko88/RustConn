//! Serial protocol handler

use crate::error::ProtocolError;
use crate::models::{Connection, ProtocolConfig, SerialFlowControl, SerialParity, SerialStopBits};

use super::{Protocol, ProtocolCapabilities, ProtocolResult};

/// Serial protocol handler
///
/// Implements the Protocol trait for serial console connections.
/// Serial sessions are spawned via VTE terminal using an external `picocom` client.
pub struct SerialProtocol;

impl SerialProtocol {
    /// Creates a new Serial protocol handler
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for SerialProtocol {
    fn default() -> Self {
        Self::new()
    }
}

impl Protocol for SerialProtocol {
    fn protocol_id(&self) -> &'static str {
        "serial"
    }

    fn display_name(&self) -> &'static str {
        "Serial"
    }

    fn default_port(&self) -> u16 {
        0
    }

    fn validate_connection(&self, connection: &Connection) -> ProtocolResult<()> {
        if let ProtocolConfig::Serial(ref config) = connection.protocol_config {
            if config.device.is_empty() {
                return Err(ProtocolError::InvalidConfig(
                    "Device path cannot be empty".to_string(),
                ));
            }
        } else {
            return Err(ProtocolError::InvalidConfig(
                "Expected Serial configuration".to_string(),
            ));
        }

        Ok(())
    }

    fn capabilities(&self) -> ProtocolCapabilities {
        ProtocolCapabilities::terminal()
    }

    fn build_command(&self, connection: &Connection) -> Option<Vec<String>> {
        let ProtocolConfig::Serial(ref config) = connection.protocol_config else {
            return None;
        };

        let mut cmd = vec![
            "picocom".to_string(),
            "--baud".to_string(),
            config.baud_rate.value().to_string(),
            "--databits".to_string(),
            config.data_bits.value().to_string(),
            "--stopbits".to_string(),
            match config.stop_bits {
                SerialStopBits::One => "1",
                SerialStopBits::Two => "2",
            }
            .to_string(),
            "--parity".to_string(),
            match config.parity {
                SerialParity::None => "n",
                SerialParity::Odd => "o",
                SerialParity::Even => "e",
            }
            .to_string(),
            "--flow".to_string(),
            match config.flow_control {
                SerialFlowControl::None => "n",
                SerialFlowControl::Hardware => "h",
                SerialFlowControl::Software => "s",
            }
            .to_string(),
        ];

        cmd.extend(config.custom_args.clone());
        cmd.push(config.device.clone());

        Some(cmd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ProtocolConfig, SerialBaudRate, SerialConfig};

    fn create_serial_connection(config: SerialConfig) -> Connection {
        Connection::new(
            "Test Serial".to_string(),
            String::new(),
            0,
            ProtocolConfig::Serial(config),
        )
    }

    #[test]
    fn test_serial_protocol_metadata() {
        let protocol = SerialProtocol::new();
        assert_eq!(protocol.protocol_id(), "serial");
        assert_eq!(protocol.display_name(), "Serial");
        assert_eq!(protocol.default_port(), 0);
    }

    #[test]
    fn test_validate_valid_connection() {
        let protocol = SerialProtocol::new();
        let config = SerialConfig {
            device: "/dev/ttyUSB0".to_string(),
            ..SerialConfig::default()
        };
        let connection = create_serial_connection(config);
        assert!(protocol.validate_connection(&connection).is_ok());
    }

    #[test]
    fn test_validate_empty_device() {
        let protocol = SerialProtocol::new();
        let config = SerialConfig::default();
        let connection = create_serial_connection(config);
        assert!(protocol.validate_connection(&connection).is_err());
    }

    #[test]
    fn test_build_command_default() {
        let protocol = SerialProtocol::new();
        let config = SerialConfig {
            device: "/dev/ttyUSB0".to_string(),
            ..SerialConfig::default()
        };
        let connection = create_serial_connection(config);
        let cmd = protocol.build_command(&connection).unwrap();
        assert_eq!(cmd[0], "picocom");
        assert_eq!(cmd[1], "--baud");
        assert_eq!(cmd[2], "115200");
        assert_eq!(cmd[3], "--databits");
        assert_eq!(cmd[4], "8");
        assert_eq!(cmd[5], "--stopbits");
        assert_eq!(cmd[6], "1");
        assert_eq!(cmd[7], "--parity");
        assert_eq!(cmd[8], "n");
        assert_eq!(cmd[9], "--flow");
        assert_eq!(cmd[10], "n");
        assert_eq!(cmd[11], "/dev/ttyUSB0");
    }

    #[test]
    fn test_build_command_custom_baud() {
        let protocol = SerialProtocol::new();
        let config = SerialConfig {
            device: "/dev/ttyACM0".to_string(),
            baud_rate: SerialBaudRate::B9600,
            ..SerialConfig::default()
        };
        let connection = create_serial_connection(config);
        let cmd = protocol.build_command(&connection).unwrap();
        assert_eq!(cmd[2], "9600");
    }
}
