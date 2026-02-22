//! Property-based tests for the Wake On LAN (WOL) system
//!
//! These tests validate the correctness properties defined in the design document
//! for the WOL system (Requirements 5.x).

use proptest::prelude::*;
use rustconn_core::wol::{MAGIC_PACKET_SIZE, MacAddress, WolConfig, generate_magic_packet};

// ========== Strategies ==========

/// Strategy for generating valid MAC address bytes
fn arb_mac_bytes() -> impl Strategy<Value = [u8; 6]> {
    prop::array::uniform6(any::<u8>())
}

/// Strategy for generating a valid MAC address
fn arb_mac_address() -> impl Strategy<Value = MacAddress> {
    arb_mac_bytes().prop_map(MacAddress::new)
}

/// Strategy for generating MAC address strings with colon separator
fn arb_mac_string_colon() -> impl Strategy<Value = String> {
    arb_mac_bytes().prop_map(|bytes| {
        format!(
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5]
        )
    })
}

/// Strategy for generating MAC address strings with dash separator
fn arb_mac_string_dash() -> impl Strategy<Value = String> {
    arb_mac_bytes().prop_map(|bytes| {
        format!(
            "{:02X}-{:02X}-{:02X}-{:02X}-{:02X}-{:02X}",
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5]
        )
    })
}

/// Strategy for generating MAC address strings with either separator
fn arb_mac_string() -> impl Strategy<Value = String> {
    prop_oneof![arb_mac_string_colon(), arb_mac_string_dash()]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // ========== Property 14: MAC Address Parse Round-Trip ==========
    // **Feature: rustconn-enhancements, Property 14: MAC Address Parse Round-Trip**
    // **Validates: Requirements 5.1**
    //
    // For any valid MAC address string, parsing and formatting should produce
    // an equivalent string.

    #[test]
    fn mac_address_parse_round_trip_colon(mac_str in arb_mac_string_colon()) {
        // Parse the MAC address string
        let mac = MacAddress::parse(&mac_str).expect("Should parse valid MAC string");

        // Format back to string (using colon format)
        let formatted = mac.format_colon();

        // Should be equal (both use uppercase hex)
        prop_assert_eq!(
            mac_str, formatted,
            "Round-trip should preserve MAC address"
        );
    }

    #[test]
    fn mac_address_parse_round_trip_dash(mac_str in arb_mac_string_dash()) {
        // Parse the MAC address string
        let mac = MacAddress::parse(&mac_str).expect("Should parse valid MAC string");

        // Format back to string (using dash format)
        let formatted = mac.format_dash();

        // Should be equal (both use uppercase hex)
        prop_assert_eq!(
            mac_str, formatted,
            "Round-trip should preserve MAC address"
        );
    }

    #[test]
    fn mac_address_bytes_round_trip(bytes in arb_mac_bytes()) {
        // Create MAC from bytes
        let mac = MacAddress::new(bytes);

        // Format to string and parse back
        let formatted = mac.format_colon();
        let parsed = MacAddress::parse(&formatted).expect("Should parse formatted MAC");

        // Bytes should be identical
        prop_assert_eq!(
            mac.bytes(), parsed.bytes(),
            "Round-trip through string should preserve bytes"
        );
    }

    #[test]
    fn mac_address_parse_either_separator(mac_str in arb_mac_string()) {
        // Should successfully parse with either separator
        let result = MacAddress::parse(&mac_str);
        prop_assert!(
            result.is_ok(),
            "Should parse MAC with either separator: {}",
            mac_str
        );
    }

    #[test]
    fn mac_address_colon_and_dash_equivalent(bytes in arb_mac_bytes()) {
        let mac = MacAddress::new(bytes);

        // Parse both formats
        let colon_str = mac.format_colon();
        let dash_str = mac.format_dash();

        let from_colon = MacAddress::parse(&colon_str).unwrap();
        let from_dash = MacAddress::parse(&dash_str).unwrap();

        // Both should produce the same MAC address
        prop_assert_eq!(
            from_colon, from_dash,
            "Colon and dash formats should parse to same MAC"
        );
    }

    #[test]
    fn mac_address_json_round_trip(mac in arb_mac_address()) {
        // Serialize to JSON
        let json = serde_json::to_string(&mac).expect("JSON serialization should succeed");

        // Deserialize back
        let parsed: MacAddress = serde_json::from_str(&json).expect("JSON deserialization should succeed");

        // Should be equal
        prop_assert_eq!(
            mac, parsed,
            "JSON round-trip should preserve MAC address"
        );
    }

    #[test]
    fn wol_config_json_round_trip(mac in arb_mac_address()) {
        let config = WolConfig::new(mac);

        // Serialize to JSON
        let json = serde_json::to_string(&config).expect("JSON serialization should succeed");

        // Deserialize back
        let parsed: WolConfig = serde_json::from_str(&json).expect("JSON deserialization should succeed");

        // Should be equal
        prop_assert_eq!(
            config, parsed,
            "JSON round-trip should preserve WOL config"
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // ========== Property 15: WOL Magic Packet Format ==========
    // **Feature: rustconn-enhancements, Property 15: WOL Magic Packet Format**
    // **Validates: Requirements 5.2**
    //
    // For any MAC address, the generated magic packet should contain 6 bytes
    // of 0xFF followed by 16 repetitions of the MAC address.

    #[test]
    fn magic_packet_has_correct_size(mac in arb_mac_address()) {
        let packet = generate_magic_packet(&mac);

        prop_assert_eq!(
            packet.len(), MAGIC_PACKET_SIZE,
            "Magic packet should be exactly {} bytes", MAGIC_PACKET_SIZE
        );
        prop_assert_eq!(
            packet.len(), 102,
            "Magic packet should be exactly 102 bytes"
        );
    }

    #[test]
    fn magic_packet_starts_with_sync_stream(mac in arb_mac_address()) {
        let packet = generate_magic_packet(&mac);

        // First 6 bytes should all be 0xFF
        for (i, &byte) in packet[..6].iter().enumerate() {
            prop_assert_eq!(
                byte, 0xFF,
                "Byte {} of sync stream should be 0xFF, got 0x{:02X}",
                i, byte
            );
        }
    }

    #[test]
    fn magic_packet_contains_16_mac_repetitions(mac in arb_mac_address()) {
        let packet = generate_magic_packet(&mac);
        let mac_bytes = mac.bytes();

        // Check all 16 repetitions of the MAC address
        for rep in 0..16 {
            let offset = 6 + (rep * 6);
            let slice = &packet[offset..offset + 6];

            prop_assert_eq!(
                slice, mac_bytes.as_slice(),
                "MAC repetition {} at offset {} should match original MAC",
                rep, offset
            );
        }
    }

    #[test]
    fn magic_packet_deterministic(mac in arb_mac_address()) {
        // Generate packet twice
        let packet1 = generate_magic_packet(&mac);
        let packet2 = generate_magic_packet(&mac);

        // Should be identical
        prop_assert_eq!(
            packet1, packet2,
            "Magic packet generation should be deterministic"
        );
    }

    #[test]
    fn magic_packet_different_for_different_macs(
        mac1 in arb_mac_address(),
        mac2 in arb_mac_address()
    ) {
        // Skip if MACs happen to be the same
        prop_assume!(mac1 != mac2);

        let packet1 = generate_magic_packet(&mac1);
        let packet2 = generate_magic_packet(&mac2);

        // Packets should be different (at least in the MAC portion)
        prop_assert_ne!(
            packet1, packet2,
            "Different MACs should produce different packets"
        );

        // But sync stream should be the same
        prop_assert_eq!(
            &packet1[..6], &packet2[..6],
            "Sync stream should be identical regardless of MAC"
        );
    }
}

// ========== Unit Tests for Edge Cases ==========

#[cfg(test)]
mod edge_case_tests {
    use rustconn_core::wol::{MacAddress, WolConfig, WolError, generate_magic_packet};

    #[test]
    fn test_mac_address_all_zeros() {
        let mac = MacAddress::new([0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
        assert_eq!(mac.format_colon(), "00:00:00:00:00:00");

        let packet = generate_magic_packet(&mac);
        // First 6 bytes are 0xFF, rest are 0x00
        assert!(packet[..6].iter().all(|&b| b == 0xFF));
        assert!(packet[6..].iter().all(|&b| b == 0x00));
    }

    #[test]
    fn test_mac_address_all_ones() {
        let mac = MacAddress::new([0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(mac.format_colon(), "FF:FF:FF:FF:FF:FF");

        let packet = generate_magic_packet(&mac);
        // Entire packet should be 0xFF
        assert!(packet.iter().all(|&b| b == 0xFF));
    }

    #[test]
    fn test_mac_address_lowercase_parsing() {
        let mac = MacAddress::parse("aa:bb:cc:dd:ee:ff").unwrap();
        assert_eq!(mac.bytes(), &[0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
    }

    #[test]
    fn test_mac_address_mixed_case_parsing() {
        let mac = MacAddress::parse("Aa:Bb:Cc:Dd:Ee:Ff").unwrap();
        assert_eq!(mac.bytes(), &[0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
    }

    #[test]
    fn test_mac_address_with_whitespace() {
        let mac = MacAddress::parse("  AA:BB:CC:DD:EE:FF  ").unwrap();
        assert_eq!(mac.bytes(), &[0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
    }

    #[test]
    fn test_mac_address_invalid_no_separator() {
        let result = MacAddress::parse("AABBCCDDEEFF");
        assert!(matches!(result, Err(WolError::InvalidMacFormat(_))));
    }

    #[test]
    fn test_mac_address_invalid_too_few_octets() {
        let result = MacAddress::parse("AA:BB:CC:DD:EE");
        assert!(matches!(result, Err(WolError::InvalidMacFormat(_))));
    }

    #[test]
    fn test_mac_address_invalid_too_many_octets() {
        let result = MacAddress::parse("AA:BB:CC:DD:EE:FF:00");
        assert!(matches!(result, Err(WolError::InvalidMacFormat(_))));
    }

    #[test]
    fn test_mac_address_invalid_hex() {
        let result = MacAddress::parse("GG:HH:II:JJ:KK:LL");
        assert!(matches!(result, Err(WolError::InvalidMacByte(_))));
    }

    #[test]
    fn test_mac_address_invalid_single_digit() {
        let result = MacAddress::parse("A:B:C:D:E:F");
        assert!(matches!(result, Err(WolError::InvalidMacFormat(_))));
    }

    #[test]
    fn test_mac_address_invalid_three_digits() {
        let result = MacAddress::parse("AAA:BBB:CCC:DDD:EEE:FFF");
        assert!(matches!(result, Err(WolError::InvalidMacFormat(_))));
    }

    #[test]
    fn test_wol_config_custom_values() {
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
    fn test_wol_config_serialization_with_defaults() {
        let mac = MacAddress::new([0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
        let config = WolConfig::new(mac);

        let json = serde_json::to_string(&config).unwrap();
        let parsed: WolConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(config.mac_address, parsed.mac_address);
        assert_eq!(config.broadcast_address, parsed.broadcast_address);
        assert_eq!(config.port, parsed.port);
        assert_eq!(config.wait_seconds, parsed.wait_seconds);
    }

    #[test]
    fn test_mac_address_display_trait() {
        let mac = MacAddress::new([0x00, 0x11, 0x22, 0x33, 0x44, 0x55]);
        let display = format!("{}", mac);
        assert_eq!(display, "00:11:22:33:44:55");
    }

    #[test]
    fn test_mac_address_equality() {
        let mac1 = MacAddress::new([0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
        let mac2 = MacAddress::parse("AA:BB:CC:DD:EE:FF").unwrap();
        let mac3 = MacAddress::parse("AA-BB-CC-DD-EE-FF").unwrap();

        assert_eq!(mac1, mac2);
        assert_eq!(mac2, mac3);
        assert_eq!(mac1, mac3);
    }
}
