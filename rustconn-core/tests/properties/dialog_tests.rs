//! Property-based tests for dialog validation and round-trip
//!
//! **Feature: rustconn-enhancements, Property 3: Dialog Validation Round-Trip**
//! **Validates: Requirements 4.1, 4.2, 4.3, 4.4, 4.5**
//!
//! **Feature: rustconn-bugfixes, Property 1: Connection Validation**
//! **Validates: Requirements 1.1, 1.2**

use proptest::prelude::*;
use rustconn_core::dialog_utils::{
    format_args, format_custom_options, parse_args, parse_custom_options, validate_host,
    validate_name, validate_port,
};
use rustconn_core::{
    ConfigManager, Connection, ProtocolConfig, SshAuthMethod, SshConfig, SshKeySource,
};
use std::collections::HashMap;

// ========== Generators ==========

/// Strategy for generating valid option keys (alphanumeric, no special chars)
fn arb_option_key() -> impl Strategy<Value = String> {
    "[A-Za-z][A-Za-z0-9_]{0,19}".prop_map(|s| s)
}

/// Strategy for generating valid option values (no commas or equals signs)
fn arb_option_value() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9_/-]{1,20}".prop_map(|s| s)
}

/// Strategy for generating custom options HashMap
fn arb_custom_options() -> impl Strategy<Value = HashMap<String, String>> {
    prop::collection::hash_map(arb_option_key(), arb_option_value(), 0..5)
}

/// Strategy for generating valid argument strings (no whitespace)
fn arb_arg() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9_/:-]{1,30}".prop_map(|s| s)
}

/// Strategy for generating argument vectors
fn arb_args() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec(arb_arg(), 0..5)
}

/// Strategy for generating valid connection names
fn arb_valid_name() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9_-]{0,31}".prop_map(|s| s)
}

/// Strategy for generating valid hostnames
fn arb_valid_host() -> impl Strategy<Value = String> {
    "[a-z0-9]([a-z0-9-]{0,15}[a-z0-9])?(\\.[a-z0-9]([a-z0-9-]{0,15}[a-z0-9])?)*".prop_map(|s| s)
}

/// Strategy for generating valid ports
fn arb_valid_port() -> impl Strategy<Value = u16> {
    1u16..=65535u16
}

// ========== Property Tests ==========

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn-enhancements, Property 3: Dialog Validation Round-Trip**
    /// **Validates: Requirements 4.4, 4.5**
    ///
    /// For any valid custom options HashMap, formatting to string and parsing back
    /// should produce an equivalent HashMap.
    #[test]
    fn custom_options_round_trip(options in arb_custom_options()) {
        // Format to string
        let formatted = format_custom_options(&options);

        // Parse back
        let parsed = parse_custom_options(&formatted);

        // Verify round-trip preserves all key-value pairs
        prop_assert_eq!(
            options.len(),
            parsed.len(),
            "Number of options should be preserved"
        );

        for (key, value) in &options {
            prop_assert_eq!(
                parsed.get(key),
                Some(value),
                "Option '{}' should be preserved with value '{}'",
                key,
                value
            );
        }
    }

    /// **Feature: rustconn-enhancements, Property 3: Dialog Validation Round-Trip**
    /// **Validates: Requirements 4.4, 4.5**
    ///
    /// For any valid argument vector, formatting to string and parsing back
    /// should produce an equivalent vector.
    #[test]
    fn args_round_trip(args in arb_args()) {
        // Format to string
        let formatted = format_args(&args);

        // Parse back
        let parsed = parse_args(&formatted);

        // Verify round-trip preserves all arguments in order
        prop_assert_eq!(
            args.len(),
            parsed.len(),
            "Number of arguments should be preserved"
        );

        for (i, (original, parsed_arg)) in args.iter().zip(parsed.iter()).enumerate() {
            prop_assert_eq!(
                original,
                parsed_arg,
                "Argument at index {} should be preserved",
                i
            );
        }
    }

    /// **Feature: rustconn-enhancements, Property 3: Dialog Validation Round-Trip**
    /// **Validates: Requirements 4.1, 4.2**
    ///
    /// For any valid connection name, validation should succeed.
    #[test]
    fn valid_name_passes_validation(name in arb_valid_name()) {
        let result = validate_name(&name);
        prop_assert!(
            result.is_ok(),
            "Valid name '{}' should pass validation, got error: {:?}",
            name,
            result
        );
    }

    /// **Feature: rustconn-enhancements, Property 3: Dialog Validation Round-Trip**
    /// **Validates: Requirements 4.1, 4.2**
    ///
    /// For any valid hostname, validation should succeed.
    #[test]
    fn valid_host_passes_validation(host in arb_valid_host()) {
        let result = validate_host(&host);
        prop_assert!(
            result.is_ok(),
            "Valid host '{}' should pass validation, got error: {:?}",
            host,
            result
        );
    }

    /// **Feature: rustconn-enhancements, Property 3: Dialog Validation Round-Trip**
    /// **Validates: Requirements 4.1, 4.2**
    ///
    /// For any valid port, validation should succeed.
    #[test]
    fn valid_port_passes_validation(port in arb_valid_port()) {
        let result = validate_port(port);
        prop_assert!(
            result.is_ok(),
            "Valid port {} should pass validation, got error: {:?}",
            port,
            result
        );
    }

    /// **Feature: rustconn-enhancements, Property 3: Dialog Validation Round-Trip**
    /// **Validates: Requirements 4.1, 4.2**
    ///
    /// Empty or whitespace-only names should fail validation.
    #[test]
    fn empty_name_fails_validation(whitespace in "[ \\t\\n]*") {
        let result = validate_name(&whitespace);
        prop_assert!(
            result.is_err(),
            "Empty/whitespace name should fail validation"
        );
    }

    /// **Feature: rustconn-enhancements, Property 3: Dialog Validation Round-Trip**
    /// **Validates: Requirements 4.1, 4.2**
    ///
    /// Empty or whitespace-only hosts should fail validation.
    #[test]
    fn empty_host_fails_validation(whitespace in "[ \\t\\n]*") {
        let result = validate_host(&whitespace);
        prop_assert!(
            result.is_err(),
            "Empty/whitespace host should fail validation"
        );
    }

    /// **Feature: rustconn-enhancements, Property 3: Dialog Validation Round-Trip**
    /// **Validates: Requirements 4.1, 4.2**
    ///
    /// Hosts containing spaces should fail validation.
    #[test]
    fn host_with_spaces_fails_validation(
        prefix in "[a-z]{1,10}",
        suffix in "[a-z]{1,10}"
    ) {
        let host_with_space = format!("{} {}", prefix, suffix);
        let result = validate_host(&host_with_space);
        prop_assert!(
            result.is_err(),
            "Host with spaces '{}' should fail validation",
            host_with_space
        );
    }

    /// **Feature: rustconn-enhancements, Property 3: Dialog Validation Round-Trip**
    /// **Validates: Requirements 4.1, 4.2**
    ///
    /// Port 0 should fail validation.
    #[test]
    fn zero_port_fails_validation(_dummy in Just(())) {
        let result = validate_port(0);
        prop_assert!(
            result.is_err(),
            "Port 0 should fail validation"
        );
    }

    /// **Feature: rustconn-enhancements, Property 3: Dialog Validation Round-Trip**
    /// **Validates: Requirements 4.4**
    ///
    /// Parsing custom options should handle malformed input gracefully.
    #[test]
    fn parse_custom_options_handles_malformed_input(
        text in "[a-zA-Z0-9,= ]{0,50}"
    ) {
        // Should not panic on any input
        let result = parse_custom_options(&text);

        // All parsed keys should be non-empty
        for key in result.keys() {
            prop_assert!(
                !key.is_empty(),
                "Parsed keys should not be empty"
            );
        }
    }

    /// **Feature: rustconn-enhancements, Property 3: Dialog Validation Round-Trip**
    /// **Validates: Requirements 4.5**
    ///
    /// Parsing args should handle any whitespace-separated input.
    #[test]
    fn parse_args_handles_any_input(text in "[a-zA-Z0-9_/: -]{0,100}") {
        // Should not panic on any input
        let result = parse_args(&text);

        // All parsed args should be non-empty (whitespace is split)
        for arg in &result {
            prop_assert!(
                !arg.is_empty(),
                "Parsed args should not be empty"
            );
            prop_assert!(
                !arg.contains(char::is_whitespace),
                "Parsed args should not contain whitespace"
            );
        }
    }
}

// ========== Connection Validation Property Tests ==========

/// Strategy for generating SSH config
fn arb_ssh_config() -> impl Strategy<Value = SshConfig> {
    Just(SshConfig {
        auth_method: SshAuthMethod::Password,
        key_path: None,
        key_source: SshKeySource::Default,
        agent_key_fingerprint: None,
        identities_only: false,
        proxy_jump: None,
        use_control_master: false,
        custom_options: HashMap::new(),
        startup_command: None,
    })
}

/// Strategy for generating protocol config
fn arb_protocol_config() -> impl Strategy<Value = ProtocolConfig> {
    arb_ssh_config().prop_map(ProtocolConfig::Ssh)
}

/// Strategy for generating whitespace-only strings (empty or spaces/tabs/newlines)
fn arb_whitespace_string() -> impl Strategy<Value = String> {
    prop_oneof![Just(String::new()), "[ \\t\\n]+".prop_map(|s| s),]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn-bugfixes, Property 1: Connection Validation**
    /// **Validates: Requirements 1.1, 1.2**
    ///
    /// For any connection data with empty name, validation SHALL return an error.
    #[test]
    fn connection_with_empty_name_fails_validation(
        empty_name in arb_whitespace_string(),
        host in arb_valid_host(),
        port in arb_valid_port(),
        protocol_config in arb_protocol_config(),
    ) {
        let conn = Connection::new(empty_name.clone(), host, port, protocol_config);
        let result = ConfigManager::validate_connection(&conn);

        prop_assert!(
            result.is_err(),
            "Connection with empty/whitespace name '{}' should fail validation",
            empty_name
        );
    }

    /// **Feature: rustconn-bugfixes, Property 1: Connection Validation**
    /// **Validates: Requirements 1.1, 1.2**
    ///
    /// For any connection data with empty host, validation SHALL return an error.
    #[test]
    fn connection_with_empty_host_fails_validation(
        name in arb_valid_name(),
        empty_host in arb_whitespace_string(),
        port in arb_valid_port(),
        protocol_config in arb_protocol_config(),
    ) {
        let conn = Connection::new(name, empty_host.clone(), port, protocol_config);
        let result = ConfigManager::validate_connection(&conn);

        prop_assert!(
            result.is_err(),
            "Connection with empty/whitespace host '{}' should fail validation",
            empty_host
        );
    }

    /// **Feature: rustconn-bugfixes, Property 1: Connection Validation**
    /// **Validates: Requirements 1.1, 1.2**
    ///
    /// For any connection data with valid name, host, and port, validation SHALL succeed.
    #[test]
    fn connection_with_valid_data_passes_validation(
        name in arb_valid_name(),
        host in arb_valid_host(),
        port in arb_valid_port(),
        protocol_config in arb_protocol_config(),
    ) {
        let conn = Connection::new(name.clone(), host.clone(), port, protocol_config);
        let result = ConfigManager::validate_connection(&conn);

        prop_assert!(
            result.is_ok(),
            "Connection with valid name '{}', host '{}', port {} should pass validation, got error: {:?}",
            name,
            host,
            port,
            result
        );
    }

    /// **Feature: rustconn-bugfixes, Property 1: Connection Validation**
    /// **Validates: Requirements 1.1, 1.2**
    ///
    /// For any connection data with port 0, validation SHALL return an error.
    #[test]
    fn connection_with_zero_port_fails_validation(
        name in arb_valid_name(),
        host in arb_valid_host(),
        protocol_config in arb_protocol_config(),
    ) {
        let conn = Connection::new(name, host, 0, protocol_config);
        let result = ConfigManager::validate_connection(&conn);

        prop_assert!(
            result.is_err(),
            "Connection with port 0 should fail validation"
        );
    }
}
