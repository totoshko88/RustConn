//! Property-based tests for Serial protocol
//!
//! Tests SerialConfig creation, serialization round-trip,
//! protocol validation, and command building.

use proptest::prelude::*;
use rustconn_core::models::{
    Connection, ProtocolConfig, SerialBaudRate, SerialConfig, SerialDataBits, SerialFlowControl,
    SerialParity, SerialStopBits,
};
use rustconn_core::protocol::{Protocol, SerialProtocol};

// ============================================================================
// Strategies for Serial enums
// ============================================================================

fn arb_baud_rate() -> impl Strategy<Value = SerialBaudRate> {
    prop_oneof![
        Just(SerialBaudRate::B9600),
        Just(SerialBaudRate::B19200),
        Just(SerialBaudRate::B38400),
        Just(SerialBaudRate::B57600),
        Just(SerialBaudRate::B115200),
        Just(SerialBaudRate::B230400),
        Just(SerialBaudRate::B460800),
        Just(SerialBaudRate::B921600),
    ]
}

fn arb_data_bits() -> impl Strategy<Value = SerialDataBits> {
    prop_oneof![
        Just(SerialDataBits::Five),
        Just(SerialDataBits::Six),
        Just(SerialDataBits::Seven),
        Just(SerialDataBits::Eight),
    ]
}

fn arb_stop_bits() -> impl Strategy<Value = SerialStopBits> {
    prop_oneof![Just(SerialStopBits::One), Just(SerialStopBits::Two),]
}

fn arb_parity() -> impl Strategy<Value = SerialParity> {
    prop_oneof![
        Just(SerialParity::None),
        Just(SerialParity::Odd),
        Just(SerialParity::Even),
    ]
}

fn arb_flow_control() -> impl Strategy<Value = SerialFlowControl> {
    prop_oneof![
        Just(SerialFlowControl::None),
        Just(SerialFlowControl::Hardware),
        Just(SerialFlowControl::Software),
    ]
}

fn arb_device_path() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("/dev/ttyUSB0".to_string()),
        Just("/dev/ttyUSB1".to_string()),
        Just("/dev/ttyACM0".to_string()),
        Just("/dev/ttyS0".to_string()),
        Just("/dev/ttyS1".to_string()),
        "/dev/tty[A-Z]{1,3}[0-9]{1,2}".prop_map(|s| s),
    ]
}

fn arb_custom_args() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec("[a-z0-9-]{1,10}", 0..3)
}

fn arb_serial_config() -> impl Strategy<Value = SerialConfig> {
    (
        arb_device_path(),
        arb_baud_rate(),
        arb_data_bits(),
        arb_stop_bits(),
        arb_parity(),
        arb_flow_control(),
        arb_custom_args(),
    )
        .prop_map(
            |(device, baud_rate, data_bits, stop_bits, parity, flow_control, custom_args)| {
                SerialConfig {
                    device,
                    baud_rate,
                    data_bits,
                    stop_bits,
                    parity,
                    flow_control,
                    custom_args,
                }
            },
        )
}

fn arb_serial_connection() -> impl Strategy<Value = Connection> {
    (arb_serial_config(), "[a-zA-Z][a-zA-Z0-9 _-]{0,20}").prop_map(|(config, name)| {
        let mut conn = Connection::new_serial(name, config.device.clone());
        if let ProtocolConfig::Serial(ref mut cfg) = conn.protocol_config {
            *cfg = config;
        }
        conn
    })
}

// ============================================================================
// Property Tests
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Default SerialConfig should have standard 115200/8N1 settings
    #[test]
    fn prop_default_serial_config_is_standard(
        device in arb_device_path()
    ) {
        let config = SerialConfig {
            device,
            ..SerialConfig::default()
        };
        prop_assert_eq!(config.baud_rate, SerialBaudRate::B115200);
        prop_assert_eq!(config.data_bits, SerialDataBits::Eight);
        prop_assert_eq!(config.stop_bits, SerialStopBits::One);
        prop_assert_eq!(config.parity, SerialParity::None);
        prop_assert_eq!(config.flow_control, SerialFlowControl::None);
        prop_assert!(config.custom_args.is_empty());
    }

    /// SerialConfig serialization round-trip preserves all fields
    #[test]
    fn prop_serial_config_serde_roundtrip(config in arb_serial_config()) {
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: SerialConfig = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(&config.device, &deserialized.device);
        prop_assert_eq!(config.baud_rate, deserialized.baud_rate);
        prop_assert_eq!(config.data_bits, deserialized.data_bits);
        prop_assert_eq!(config.stop_bits, deserialized.stop_bits);
        prop_assert_eq!(config.parity, deserialized.parity);
        prop_assert_eq!(config.flow_control, deserialized.flow_control);
        prop_assert_eq!(&config.custom_args, &deserialized.custom_args);
    }

    /// Serial connection round-trip preserves protocol config
    #[test]
    fn prop_serial_connection_serde_roundtrip(conn in arb_serial_connection()) {
        let json = serde_json::to_string(&conn).unwrap();
        let deserialized: Connection = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(conn.protocol, deserialized.protocol);
        prop_assert_eq!(conn.name, deserialized.name);
        if let (
            ProtocolConfig::Serial(ref orig),
            ProtocolConfig::Serial(ref deser),
        ) = (&conn.protocol_config, &deserialized.protocol_config)
        {
            prop_assert_eq!(&orig.device, &deser.device);
            prop_assert_eq!(orig.baud_rate, deser.baud_rate);
            prop_assert_eq!(orig.data_bits, deser.data_bits);
            prop_assert_eq!(orig.stop_bits, deser.stop_bits);
            prop_assert_eq!(orig.parity, deser.parity);
            prop_assert_eq!(orig.flow_control, deser.flow_control);
        } else {
            prop_assert!(false, "Expected Serial protocol config");
        }
    }

    /// SerialProtocol validates connections with non-empty device
    #[test]
    fn prop_serial_validation_requires_device(conn in arb_serial_connection()) {
        let protocol = SerialProtocol::new();
        let result = protocol.validate_connection(&conn);
        // All generated connections have non-empty device paths
        prop_assert!(
            result.is_ok(),
            "Valid serial connection should pass validation"
        );
    }

    /// SerialProtocol rejects empty device path
    #[test]
    fn prop_serial_validation_rejects_empty_device(
        name in "[a-zA-Z][a-zA-Z0-9]{0,10}"
    ) {
        let mut conn = Connection::new_serial(name, String::new());
        if let ProtocolConfig::Serial(ref mut cfg) = conn.protocol_config {
            cfg.device = String::new();
        }
        conn.host = String::new();
        let protocol = SerialProtocol::new();
        let result = protocol.validate_connection(&conn);
        prop_assert!(
            result.is_err(),
            "Empty device path should fail validation"
        );
    }

    /// SerialProtocol build_command produces picocom with correct args
    #[test]
    fn prop_serial_build_command_uses_picocom(conn in arb_serial_connection()) {
        let protocol = SerialProtocol::new();
        let cmd = protocol.build_command(&conn);
        let cmd = cmd.expect("Serial should produce a command");
        prop_assert_eq!(&cmd[0], "picocom");
        // Device path should be in the arguments
        if let ProtocolConfig::Serial(ref cfg) = conn.protocol_config {
            let args_str = cmd[1..].join(" ");
            prop_assert!(
                args_str.contains(&cfg.device),
                "Command should contain device path '{}', got: {}",
                cfg.device,
                args_str
            );
        }
    }

    /// SerialProtocol has correct protocol metadata
    #[test]
    fn prop_serial_protocol_metadata(_dummy in 0u32..1) {
        let protocol = SerialProtocol::new();
        prop_assert_eq!(protocol.protocol_id(), "serial");
        prop_assert_eq!(protocol.display_name(), "Serial");
        prop_assert_eq!(protocol.default_port(), 0);
    }

    /// All baud rate variants have valid display names
    #[test]
    fn prop_baud_rate_display_name(baud in arb_baud_rate()) {
        let name = baud.display_name();
        prop_assert!(!name.is_empty(), "Baud rate display name should not be empty");
        // Display name should contain digits (it's a number)
        prop_assert!(
            name.chars().any(|c| c.is_ascii_digit()),
            "Baud rate display name should contain digits: {}",
            name
        );
    }

    /// All data bits variants have valid display names
    #[test]
    fn prop_data_bits_display_name(bits in arb_data_bits()) {
        let name = bits.display_name();
        prop_assert!(!name.is_empty());
    }

    /// All flow control variants have valid display names
    #[test]
    fn prop_flow_control_display_name(fc in arb_flow_control()) {
        let name = fc.display_name();
        prop_assert!(!name.is_empty());
    }
}

// ============================================================================
// Non-proptest unit tests
// ============================================================================

#[test]
fn test_serial_protocol_default_port_is_zero() {
    let protocol = SerialProtocol::new();
    assert_eq!(protocol.default_port(), 0);
}

#[test]
fn test_serial_protocol_capabilities_include_terminal() {
    let protocol = SerialProtocol::new();
    let caps = protocol.capabilities();
    assert!(
        caps.terminal_based,
        "Serial should have terminal_based capability"
    );
}

#[test]
fn test_new_serial_connection_has_correct_protocol() {
    let conn = Connection::new_serial("Test Serial".to_string(), "/dev/ttyUSB0".to_string());
    assert_eq!(conn.protocol, rustconn_core::models::ProtocolType::Serial);
    assert_eq!(conn.port, 0);
    // Device is stored in SerialConfig, not in conn.host
    if let ProtocolConfig::Serial(ref cfg) = conn.protocol_config {
        assert_eq!(cfg.device, "/dev/ttyUSB0");
    } else {
        panic!("Expected Serial protocol config");
    }
}
