//! Property-based tests for FFI bindings
//!
//! These tests validate the correctness properties for FFI wrappers
//! as defined in the design document for native protocol embedding.

use proptest::prelude::*;
use rustconn_core::ffi::{
    ConnectionState, FfiDisplay, RdpConnectionConfig, RdpDisplay, SpiceConnectionConfig,
    SpiceDisplay, SpiceError, SpiceTlsConfig, VncCredentialType, VncDisplay,
};
use std::path::PathBuf;

// ============================================================================
// Generators for VNC configurations
// ============================================================================

/// Strategy for generating valid hostnames
fn arb_host() -> impl Strategy<Value = String> {
    "[a-z0-9]([a-z0-9-]{0,15}[a-z0-9])?(\\.[a-z0-9]([a-z0-9-]{0,15}[a-z0-9])?)*"
}

/// Strategy for generating valid ports (non-zero)
fn arb_port() -> impl Strategy<Value = u16> {
    1u16..=65535u16
}

/// Strategy for generating VNC credential types
fn arb_credential_type() -> impl Strategy<Value = VncCredentialType> {
    prop_oneof![
        Just(VncCredentialType::Password),
        Just(VncCredentialType::Username),
        Just(VncCredentialType::ClientName),
    ]
}

/// Strategy for generating non-empty credential values
fn arb_credential_value() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9!@#$%^&*()_+-=]{1,64}"
}

/// Strategy for generating connection states
fn arb_connection_state() -> impl Strategy<Value = ConnectionState> {
    prop_oneof![
        Just(ConnectionState::Disconnected),
        Just(ConnectionState::Connecting),
        Just(ConnectionState::Authenticating),
        Just(ConnectionState::Connected),
        Just(ConnectionState::Error),
    ]
}

// ============================================================================
// Property Tests for VNC Display
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // **Feature: native-protocol-embedding, Property 10: FFI widgets integrate with GTK4 hierarchy**
    // **Validates: Requirements 8.2**
    //
    // For any VncDisplay instance, the widget should be properly initialized
    // and maintain consistent state throughout its lifecycle.
    //
    // Note: This test validates the Rust wrapper behavior. Actual GTK4 widget
    // integration requires a GTK runtime and is tested in integration tests.

    #[test]
    fn prop_vnc_display_initial_state_is_disconnected(_seed in any::<u64>()) {
        let display = VncDisplay::new();

        // Initial state should always be Disconnected
        prop_assert_eq!(
            display.connection_state(),
            ConnectionState::Disconnected,
            "New VncDisplay should start in Disconnected state"
        );

        // Should not be open
        prop_assert!(!display.is_open(), "New VncDisplay should not be open");

        // Host and port should be None
        prop_assert!(display.host().is_none(), "New VncDisplay should have no host");
        prop_assert!(display.port().is_none(), "New VncDisplay should have no port");
    }

    #[test]
    fn prop_vnc_display_open_host_transitions_to_connecting(
        host in arb_host(),
        port in arb_port()
    ) {
        let display = VncDisplay::new();

        // Open connection
        let result = display.open_host(&host, port);
        prop_assert!(result.is_ok(), "open_host should succeed with valid host and port");

        // State should transition to Connecting
        prop_assert_eq!(
            display.connection_state(),
            ConnectionState::Connecting,
            "State should be Connecting after open_host"
        );

        // Host and port should be set
        prop_assert_eq!(display.host(), Some(host), "Host should be set after open_host");
        prop_assert_eq!(display.port(), Some(port), "Port should be set after open_host");
    }

    #[test]
    fn prop_vnc_display_close_returns_to_disconnected(
        host in arb_host(),
        port in arb_port()
    ) {
        let display = VncDisplay::new();

        // Open and then close
        display.open_host(&host, port).unwrap();
        display.close();

        // State should return to Disconnected
        prop_assert_eq!(
            display.connection_state(),
            ConnectionState::Disconnected,
            "State should be Disconnected after close"
        );

        // Host and port should be cleared
        prop_assert!(display.host().is_none(), "Host should be None after close");
        prop_assert!(display.port().is_none(), "Port should be None after close");
    }

    #[test]
    fn prop_vnc_display_rejects_empty_host(port in arb_port()) {
        let display = VncDisplay::new();

        let result = display.open_host("", port);
        prop_assert!(result.is_err(), "open_host should reject empty host");

        // State should remain Disconnected
        prop_assert_eq!(
            display.connection_state(),
            ConnectionState::Disconnected,
            "State should remain Disconnected after rejected open_host"
        );
    }

    #[test]
    fn prop_vnc_display_rejects_zero_port(host in arb_host()) {
        let display = VncDisplay::new();

        let result = display.open_host(&host, 0);
        prop_assert!(result.is_err(), "open_host should reject zero port");

        // State should remain Disconnected
        prop_assert_eq!(
            display.connection_state(),
            ConnectionState::Disconnected,
            "State should remain Disconnected after rejected open_host"
        );
    }

    #[test]
    fn prop_vnc_display_rejects_duplicate_connection(
        host in arb_host(),
        port in arb_port()
    ) {
        let display = VncDisplay::new();

        // First connection should succeed
        let result1 = display.open_host(&host, port);
        prop_assert!(result1.is_ok(), "First open_host should succeed");

        // Second connection should fail
        let result2 = display.open_host(&host, port);
        prop_assert!(result2.is_err(), "Second open_host should fail while connecting");
    }

    #[test]
    fn prop_vnc_display_set_credential_accepts_valid_values(
        cred_type in arb_credential_type(),
        value in arb_credential_value()
    ) {
        let display = VncDisplay::new();

        let result = display.set_credential(cred_type, &value);
        prop_assert!(result.is_ok(), "set_credential should accept non-empty value");
    }

    #[test]
    fn prop_vnc_display_set_credential_rejects_empty_values(
        cred_type in arb_credential_type()
    ) {
        let display = VncDisplay::new();

        let result = display.set_credential(cred_type, "");
        prop_assert!(result.is_err(), "set_credential should reject empty value");
    }

    #[test]
    fn prop_vnc_display_scaling_toggle_is_consistent(enabled in any::<bool>()) {
        let display = VncDisplay::new();

        display.set_scaling(enabled);
        prop_assert_eq!(
            display.scaling_enabled(),
            enabled,
            "scaling_enabled should match what was set"
        );
    }

    #[test]
    fn prop_vnc_display_ffi_display_trait_consistency(
        host in arb_host(),
        port in arb_port()
    ) {
        let display = VncDisplay::new();

        // Test FfiDisplay trait methods match VncDisplay methods
        prop_assert_eq!(
            FfiDisplay::state(&display),
            display.connection_state(),
            "FfiDisplay::state should match connection_state"
        );

        prop_assert_eq!(
            FfiDisplay::is_connected(&display),
            display.is_open(),
            "FfiDisplay::is_connected should match is_open"
        );

        // Open connection
        display.open_host(&host, port).unwrap();

        // Close via trait
        FfiDisplay::close(&display);

        prop_assert_eq!(
            display.connection_state(),
            ConnectionState::Disconnected,
            "FfiDisplay::close should disconnect"
        );
    }
}

// ============================================================================
// Property Tests for Connection State
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    #[test]
    fn prop_connection_state_display_is_non_empty(state in arb_connection_state()) {
        let display_str = state.to_string();
        prop_assert!(!display_str.is_empty(), "ConnectionState display should not be empty");
    }

    #[test]
    fn prop_connection_state_default_is_disconnected(_seed in any::<u64>()) {
        let state: ConnectionState = Default::default();
        prop_assert_eq!(
            state,
            ConnectionState::Disconnected,
            "Default ConnectionState should be Disconnected"
        );
    }

    #[test]
    fn prop_connection_state_equality_is_reflexive(state in arb_connection_state()) {
        prop_assert_eq!(state, state, "ConnectionState equality should be reflexive");
    }
}

// ============================================================================
// Property Tests for VNC Credential Type
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    #[test]
    fn prop_vnc_credential_type_display_is_non_empty(cred_type in arb_credential_type()) {
        let display_str = cred_type.to_string();
        prop_assert!(!display_str.is_empty(), "VncCredentialType display should not be empty");
    }

    #[test]
    fn prop_vnc_credential_type_equality_is_reflexive(cred_type in arb_credential_type()) {
        prop_assert_eq!(cred_type, cred_type, "VncCredentialType equality should be reflexive");
    }
}

// ============================================================================
// Unit Tests for Edge Cases
// ============================================================================

/// **Feature: native-protocol-embedding, Property 10: FFI widgets integrate with GTK4 hierarchy**
/// **Validates: Requirements 8.2**
///
/// This test validates that VncDisplay can be created and used without panicking,
/// which is a prerequisite for GTK4 integration.
#[test]
fn test_vnc_display_lifecycle() {
    // Create display
    let display = VncDisplay::new();
    assert_eq!(display.connection_state(), ConnectionState::Disconnected);

    // Open connection
    display.open_host("localhost", 5900).unwrap();
    assert_eq!(display.connection_state(), ConnectionState::Connecting);

    // Set credentials
    display
        .set_credential(VncCredentialType::Password, "test")
        .unwrap();
    display
        .set_credential(VncCredentialType::Username, "user")
        .unwrap();

    // Configure scaling
    display.set_scaling(true);
    assert!(display.scaling_enabled());

    // Close connection
    display.close();
    assert_eq!(display.connection_state(), ConnectionState::Disconnected);
}

/// Test that VncDisplay properly cleans up on drop
#[test]
fn test_vnc_display_drop_cleanup() {
    {
        let display = VncDisplay::new();
        display.open_host("localhost", 5900).unwrap();
        // display goes out of scope here
    }
    // If we get here without panicking, cleanup worked
}

/// Test signal callback registration
#[test]
fn test_vnc_display_signal_callbacks() {
    use std::cell::Cell;
    use std::rc::Rc;

    let display = VncDisplay::new();

    let connected_called = Rc::new(Cell::new(false));
    let disconnected_called = Rc::new(Cell::new(false));
    let auth_called = Rc::new(Cell::new(false));
    let auth_failure_called = Rc::new(Cell::new(false));

    // Register callbacks
    let cc = connected_called.clone();
    display.connect_vnc_connected(move |_| cc.set(true));

    let dc = disconnected_called.clone();
    display.connect_vnc_disconnected(move |_| dc.set(true));

    let ac = auth_called.clone();
    display.connect_vnc_auth_credential(move |_, _| ac.set(true));

    let afc = auth_failure_called.clone();
    display.connect_vnc_auth_failure(move |_, _| afc.set(true));

    // Callbacks should not be called yet
    assert!(!connected_called.get());
    assert!(!disconnected_called.get());
    assert!(!auth_called.get());
    assert!(!auth_failure_called.get());
}

// ============================================================================
// Property Tests for RDP Display
// **Feature: native-protocol-embedding, Property 1: Protocol widget creation returns valid GTK widget**
// **Validates: Requirements 1.2, 3.1**
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: native-protocol-embedding, Property 1: Protocol widget creation returns valid GTK widget**
    /// **Validates: Requirements 1.2, 3.1**
    ///
    /// For any RdpDisplay instance, the widget should be properly initialized
    /// and maintain consistent state throughout its lifecycle.
    #[test]
    fn prop_rdp_display_initial_state_is_disconnected(_seed in any::<u64>()) {
        let display = RdpDisplay::new();

        // Initial state should always be Disconnected
        prop_assert_eq!(
            display.connection_state(),
            ConnectionState::Disconnected,
            "New RdpDisplay should start in Disconnected state"
        );

        // Host and port should be None
        prop_assert!(display.host().is_none(), "New RdpDisplay should have no host");
        prop_assert!(display.port().is_none(), "New RdpDisplay should have no port");
        prop_assert!(display.config().is_none(), "New RdpDisplay should have no config");
    }

    /// **Feature: native-protocol-embedding, Property 1: Protocol widget creation returns valid GTK widget**
    /// **Validates: Requirements 1.2, 3.1**
    #[test]
    fn prop_rdp_display_open_transitions_to_connecting(
        host in arb_host(),
        port in arb_port()
    ) {
        let display = RdpDisplay::new();
        let config = RdpConnectionConfig::new(&host).with_port(port);

        // Open connection
        let result = display.open(&config);
        prop_assert!(result.is_ok(), "open should succeed with valid config");

        // State should transition to Connecting
        prop_assert_eq!(
            display.connection_state(),
            ConnectionState::Connecting,
            "State should be Connecting after open"
        );

        // Host and port should be set
        prop_assert_eq!(display.host(), Some(host), "Host should be set after open");
        prop_assert_eq!(display.port(), Some(port), "Port should be set after open");
    }

    /// **Feature: native-protocol-embedding, Property 1: Protocol widget creation returns valid GTK widget**
    /// **Validates: Requirements 1.2, 3.1**
    #[test]
    fn prop_rdp_display_close_returns_to_disconnected(
        host in arb_host(),
        port in arb_port()
    ) {
        let display = RdpDisplay::new();
        let config = RdpConnectionConfig::new(&host).with_port(port);

        // Open and then close
        display.open(&config).unwrap();
        display.close();

        // State should return to Disconnected
        prop_assert_eq!(
            display.connection_state(),
            ConnectionState::Disconnected,
            "State should be Disconnected after close"
        );

        // Host and port should be cleared
        prop_assert!(display.host().is_none(), "Host should be None after close");
        prop_assert!(display.port().is_none(), "Port should be None after close");
        prop_assert!(display.config().is_none(), "Config should be None after close");
    }

    /// **Feature: native-protocol-embedding, Property 1: Protocol widget creation returns valid GTK widget**
    /// **Validates: Requirements 1.2, 3.1**
    #[test]
    fn prop_rdp_display_rejects_empty_host(port in arb_port()) {
        let display = RdpDisplay::new();
        let config = RdpConnectionConfig {
            host: String::new(),
            port,
            ..Default::default()
        };

        let result = display.open(&config);
        prop_assert!(result.is_err(), "open should reject empty host");

        // State should remain Disconnected
        prop_assert_eq!(
            display.connection_state(),
            ConnectionState::Disconnected,
            "State should remain Disconnected after rejected open"
        );
    }

    /// **Feature: native-protocol-embedding, Property 1: Protocol widget creation returns valid GTK widget**
    /// **Validates: Requirements 1.2, 3.1**
    #[test]
    fn prop_rdp_display_rejects_zero_port(host in arb_host()) {
        let display = RdpDisplay::new();
        let config = RdpConnectionConfig {
            host,
            port: 0,
            ..Default::default()
        };

        let result = display.open(&config);
        prop_assert!(result.is_err(), "open should reject zero port");

        // State should remain Disconnected
        prop_assert_eq!(
            display.connection_state(),
            ConnectionState::Disconnected,
            "State should remain Disconnected after rejected open"
        );
    }

    /// **Feature: native-protocol-embedding, Property 1: Protocol widget creation returns valid GTK widget**
    /// **Validates: Requirements 1.2, 3.1**
    #[test]
    fn prop_rdp_display_rejects_duplicate_connection(
        host in arb_host(),
        port in arb_port()
    ) {
        let display = RdpDisplay::new();
        let config = RdpConnectionConfig::new(&host).with_port(port);

        // First connection should succeed
        let result1 = display.open(&config);
        prop_assert!(result1.is_ok(), "First open should succeed");

        // Second connection should fail
        let result2 = display.open(&config);
        prop_assert!(result2.is_err(), "Second open should fail while connecting");
    }

    /// **Feature: native-protocol-embedding, Property 1: Protocol widget creation returns valid GTK widget**
    /// **Validates: Requirements 1.2, 3.1**
    #[test]
    fn prop_rdp_display_set_credentials_accepts_valid_values(
        username in "[a-zA-Z][a-zA-Z0-9_]{0,31}",
        password in "[a-zA-Z0-9!@#$%^&*()_+-=]{1,64}"
    ) {
        let display = RdpDisplay::new();

        let result = display.set_credentials(&username, &password, None);
        prop_assert!(result.is_ok(), "set_credentials should accept valid values");
    }

    /// **Feature: native-protocol-embedding, Property 1: Protocol widget creation returns valid GTK widget**
    /// **Validates: Requirements 1.2, 3.1**
    #[test]
    fn prop_rdp_display_set_credentials_rejects_empty_username(
        password in "[a-zA-Z0-9!@#$%^&*()_+-=]{1,64}"
    ) {
        let display = RdpDisplay::new();

        let result = display.set_credentials("", &password, None);
        prop_assert!(result.is_err(), "set_credentials should reject empty username");
    }

    /// **Feature: native-protocol-embedding, Property 1: Protocol widget creation returns valid GTK widget**
    /// **Validates: Requirements 1.2, 3.1**
    #[test]
    fn prop_rdp_display_set_credentials_rejects_empty_password(
        username in "[a-zA-Z][a-zA-Z0-9_]{0,31}"
    ) {
        let display = RdpDisplay::new();

        let result = display.set_credentials(&username, "", None);
        prop_assert!(result.is_err(), "set_credentials should reject empty password");
    }

    /// **Feature: native-protocol-embedding, Property 1: Protocol widget creation returns valid GTK widget**
    /// **Validates: Requirements 1.2, 3.1**
    #[test]
    fn prop_rdp_display_clipboard_toggle_is_consistent(enabled in any::<bool>()) {
        let display = RdpDisplay::new();

        display.set_clipboard_enabled(enabled);
        prop_assert_eq!(
            display.clipboard_enabled(),
            enabled,
            "clipboard_enabled should match what was set"
        );
    }

    /// **Feature: native-protocol-embedding, Property 1: Protocol widget creation returns valid GTK widget**
    /// **Validates: Requirements 1.2, 3.1**
    #[test]
    fn prop_rdp_display_ffi_display_trait_consistency(
        host in arb_host(),
        port in arb_port()
    ) {
        let display = RdpDisplay::new();
        let config = RdpConnectionConfig::new(&host).with_port(port);

        // Test FfiDisplay trait methods match RdpDisplay methods
        prop_assert_eq!(
            FfiDisplay::state(&display),
            display.connection_state(),
            "FfiDisplay::state should match connection_state"
        );

        // Open connection
        display.open(&config).unwrap();

        // Close via trait
        FfiDisplay::close(&display);

        prop_assert_eq!(
            display.connection_state(),
            ConnectionState::Disconnected,
            "FfiDisplay::close should disconnect"
        );
    }
}

// ============================================================================
// Unit Tests for RDP Display
// ============================================================================

/// **Feature: native-protocol-embedding, Property 1: Protocol widget creation returns valid GTK widget**
/// **Validates: Requirements 1.2, 3.1**
///
/// This test validates that RdpDisplay can be created and used without panicking,
/// which is a prerequisite for GTK4 integration.
#[test]
fn test_rdp_display_lifecycle() {
    // Create display
    let display = RdpDisplay::new();
    assert_eq!(display.connection_state(), ConnectionState::Disconnected);

    // Open connection
    let config = RdpConnectionConfig::new("localhost").with_port(3389);
    display.open(&config).unwrap();
    assert_eq!(display.connection_state(), ConnectionState::Connecting);

    // Set credentials
    display
        .set_credentials("user", "password", Some("DOMAIN"))
        .unwrap();

    // Configure clipboard
    display.set_clipboard_enabled(true);
    assert!(display.clipboard_enabled());

    // Close connection
    display.close();
    assert_eq!(display.connection_state(), ConnectionState::Disconnected);
}

/// Test that RdpDisplay properly cleans up on drop
#[test]
fn test_rdp_display_drop_cleanup() {
    {
        let display = RdpDisplay::new();
        let config = RdpConnectionConfig::new("localhost").with_port(3389);
        display.open(&config).unwrap();
        // display goes out of scope here
    }
    // If we get here without panicking, cleanup worked
}

/// Test signal callback registration for RDP
#[test]
fn test_rdp_display_signal_callbacks() {
    use std::cell::Cell;
    use std::rc::Rc;

    let display = RdpDisplay::new();

    let connected_called = Rc::new(Cell::new(false));
    let disconnected_called = Rc::new(Cell::new(false));
    let auth_called = Rc::new(Cell::new(false));
    let auth_failure_called = Rc::new(Cell::new(false));
    let error_called = Rc::new(Cell::new(false));

    // Register callbacks
    let cc = connected_called.clone();
    display.connect_rdp_connected(move |_| cc.set(true));

    let dc = disconnected_called.clone();
    display.connect_rdp_disconnected(move |_| dc.set(true));

    let ac = auth_called.clone();
    display.connect_rdp_auth_required(move |_| ac.set(true));

    let afc = auth_failure_called.clone();
    display.connect_rdp_auth_failure(move |_, _| afc.set(true));

    let ec = error_called.clone();
    display.connect_rdp_error(move |_, _| ec.set(true));

    // Callbacks should not be called yet
    assert!(!connected_called.get());
    assert!(!disconnected_called.get());
    assert!(!auth_called.get());
    assert!(!auth_failure_called.get());
    assert!(!error_called.get());
}

// ============================================================================
// Property Tests for SPICE Display TLS Validation
// **Feature: native-protocol-embedding, Property 9: TLS certificate validation respects configuration**
// **Validates: Requirements 4.6**
// ============================================================================

/// Strategy for generating valid CA certificate paths (non-empty paths)
fn arb_ca_cert_path() -> impl Strategy<Value = PathBuf> {
    "[a-zA-Z0-9/_.-]{1,64}\\.crt".prop_map(PathBuf::from)
}

/// Strategy for generating SPICE TLS configurations
fn arb_spice_tls_config() -> impl Strategy<Value = SpiceTlsConfig> {
    (
        any::<bool>(),
        prop::option::of(arb_ca_cert_path()),
        any::<bool>(),
    )
        .prop_map(|(enabled, ca_cert_path, skip_cert_verify)| SpiceTlsConfig {
            enabled,
            ca_cert_path,
            skip_cert_verify,
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // **Feature: native-protocol-embedding, Property 9: TLS certificate validation respects configuration**
    // **Validates: Requirements 4.6**
    //
    // For any SPICE connection with TLS enabled, if `skip_cert_verify` is false,
    // invalid certificates SHALL cause connection failure; if true, connection SHALL proceed.

    /// Property: When TLS is enabled and skip_cert_verify is true, connection should proceed
    /// regardless of certificate validity.
    #[test]
    fn prop_spice_tls_skip_verify_allows_connection(
        host in arb_host(),
        port in arb_port()
    ) {
        let display = SpiceDisplay::new();

        // TLS enabled with skip_cert_verify = true should always succeed
        let tls = SpiceTlsConfig {
            enabled: true,
            ca_cert_path: None, // No CA cert provided
            skip_cert_verify: true, // But we skip verification
        };
        let config = SpiceConnectionConfig::new(&host, port).with_tls(tls);

        let result = display.open(&config);
        prop_assert!(
            result.is_ok(),
            "Connection should succeed when skip_cert_verify is true, got: {:?}",
            result
        );
    }

    /// Property: When TLS is enabled with a valid CA cert path, connection should proceed.
    #[test]
    fn prop_spice_tls_with_ca_cert_allows_connection(
        host in arb_host(),
        port in arb_port(),
        ca_path in arb_ca_cert_path()
    ) {
        let display = SpiceDisplay::new();

        // TLS enabled with CA cert should succeed
        let tls = SpiceTlsConfig {
            enabled: true,
            ca_cert_path: Some(ca_path),
            skip_cert_verify: false,
        };
        let config = SpiceConnectionConfig::new(&host, port).with_tls(tls);

        let result = display.open(&config);
        prop_assert!(
            result.is_ok(),
            "Connection should succeed when CA cert is provided, got: {:?}",
            result
        );
    }

    /// Property: When TLS is enabled without CA cert and skip_cert_verify is false,
    /// connection should fail with certificate validation error.
    #[test]
    fn prop_spice_tls_no_cert_no_skip_fails(
        host in arb_host(),
        port in arb_port()
    ) {
        let display = SpiceDisplay::new();

        // TLS enabled without CA cert and without skip_verify should fail
        let tls = SpiceTlsConfig {
            enabled: true,
            ca_cert_path: None,
            skip_cert_verify: false,
        };
        let config = SpiceConnectionConfig::new(&host, port).with_tls(tls);

        let result = display.open(&config);
        prop_assert!(
            result.is_err(),
            "Connection should fail when TLS enabled without CA cert and skip_verify is false"
        );

        // Verify it's the right kind of error (InvalidConfiguration from validation)
        if let Err(err) = result {
            prop_assert!(
                matches!(err, SpiceError::InvalidConfiguration(_)),
                "Error should be InvalidConfiguration, got: {:?}",
                err
            );
        }
    }

    /// Property: When TLS is disabled, connection should proceed regardless of other TLS settings.
    #[test]
    fn prop_spice_tls_disabled_ignores_cert_settings(
        host in arb_host(),
        port in arb_port(),
        skip_verify in any::<bool>()
    ) {
        let display = SpiceDisplay::new();

        // TLS disabled should always succeed regardless of other settings
        let tls = SpiceTlsConfig {
            enabled: false,
            ca_cert_path: None,
            skip_cert_verify: skip_verify,
        };
        let config = SpiceConnectionConfig::new(&host, port).with_tls(tls);

        let result = display.open(&config);
        prop_assert!(
            result.is_ok(),
            "Connection should succeed when TLS is disabled, got: {:?}",
            result
        );
    }

    /// Property: SpiceTlsConfig validation is consistent with SpiceDisplay::open behavior.
    #[test]
    fn prop_spice_tls_config_validation_consistency(tls in arb_spice_tls_config()) {
        let display = SpiceDisplay::new();
        let config = SpiceConnectionConfig::new("localhost", 5900).with_tls(tls.clone());

        let validation_result = tls.validate();
        let open_result = display.open(&config);

        // If TLS config validation fails, open should also fail
        if validation_result.is_err() {
            prop_assert!(
                open_result.is_err(),
                "If TLS validation fails, open should also fail"
            );
        }

        // If TLS config validation passes, open should succeed (for valid host/port)
        if validation_result.is_ok() {
            prop_assert!(
                open_result.is_ok(),
                "If TLS validation passes, open should succeed for valid host/port, got: {:?}",
                open_result
            );
        }
    }

    /// Property: TLS configuration with empty CA cert path behaves like no CA cert.
    #[test]
    fn prop_spice_tls_empty_ca_path_treated_as_none(
        host in arb_host(),
        port in arb_port()
    ) {
        let display = SpiceDisplay::new();

        // Empty path should be treated same as None
        let tls = SpiceTlsConfig {
            enabled: true,
            ca_cert_path: Some(PathBuf::new()), // Empty path
            skip_cert_verify: false,
        };
        let config = SpiceConnectionConfig::new(&host, port).with_tls(tls);

        let result = display.open(&config);

        // Should fail because empty path is not a valid certificate
        prop_assert!(
            result.is_err(),
            "Connection should fail with empty CA cert path when skip_verify is false"
        );
    }
}

// ============================================================================
// Unit Tests for SPICE TLS Edge Cases
// ============================================================================

/// **Feature: native-protocol-embedding, Property 9: TLS certificate validation respects configuration**
/// **Validates: Requirements 4.6**
///
/// Test that TLS validation correctly handles the boundary between valid and invalid configurations.
#[test]
fn test_spice_tls_validation_boundary_cases() {
    // Case 1: TLS enabled, no cert, no skip -> should fail validation
    let tls = SpiceTlsConfig {
        enabled: true,
        ca_cert_path: None,
        skip_cert_verify: false,
    };
    assert!(tls.validate().is_err());

    // Case 2: TLS enabled, no cert, skip -> should pass validation
    let tls = SpiceTlsConfig {
        enabled: true,
        ca_cert_path: None,
        skip_cert_verify: true,
    };
    assert!(tls.validate().is_ok());

    // Case 3: TLS enabled, cert provided, no skip -> should pass validation
    let tls = SpiceTlsConfig {
        enabled: true,
        ca_cert_path: Some(PathBuf::from("/path/to/ca.crt")),
        skip_cert_verify: false,
    };
    assert!(tls.validate().is_ok());

    // Case 4: TLS disabled -> should always pass validation
    let tls = SpiceTlsConfig {
        enabled: false,
        ca_cert_path: None,
        skip_cert_verify: false,
    };
    assert!(tls.validate().is_ok());
}

/// Test that SpiceDisplay correctly enforces TLS validation during open.
#[test]
fn test_spice_display_tls_enforcement() {
    let display = SpiceDisplay::new();

    // Should fail: TLS enabled without cert and without skip
    let tls = SpiceTlsConfig::new(); // enabled=true, no cert, skip=false
    let config = SpiceConnectionConfig::new("localhost", 5900).with_tls(tls);
    let result = display.open(&config);
    assert!(result.is_err());
    assert!(matches!(result, Err(SpiceError::InvalidConfiguration(_))));

    // Reset display
    display.close();

    // Should succeed: TLS enabled with skip_verify
    let tls = SpiceTlsConfig::new().with_skip_verify(true);
    let config = SpiceConnectionConfig::new("localhost", 5900).with_tls(tls);
    let result = display.open(&config);
    assert!(result.is_ok());
}

// ============================================================================
// Property Tests for SPICE Display Widget Creation
// **Feature: native-protocol-embedding, Property 1: Protocol widget creation returns valid GTK widget**
// **Validates: Requirements 1.2, 4.2**
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: native-protocol-embedding, Property 1: Protocol widget creation returns valid GTK widget**
    /// **Validates: Requirements 1.2, 4.2**
    ///
    /// For any SpiceDisplay instance, the widget should be properly initialized
    /// and maintain consistent state throughout its lifecycle.
    #[test]
    fn prop_spice_display_initial_state_is_disconnected(_seed in any::<u64>()) {
        let display = SpiceDisplay::new();

        // Initial state should always be Disconnected
        prop_assert_eq!(
            display.connection_state(),
            ConnectionState::Disconnected,
            "New SpiceDisplay should start in Disconnected state"
        );

        // Should not be connected
        prop_assert!(!display.is_connected(), "New SpiceDisplay should not be connected");

        // Host and port should be None
        prop_assert!(display.host().is_none(), "New SpiceDisplay should have no host");
        prop_assert!(display.port().is_none(), "New SpiceDisplay should have no port");
        prop_assert!(display.config().is_none(), "New SpiceDisplay should have no config");
    }

    /// **Feature: native-protocol-embedding, Property 1: Protocol widget creation returns valid GTK widget**
    /// **Validates: Requirements 1.2, 4.2**
    #[test]
    fn prop_spice_display_open_transitions_to_connecting(
        host in arb_host(),
        port in arb_port()
    ) {
        let display = SpiceDisplay::new();
        let config = SpiceConnectionConfig::new(&host, port);

        // Open connection
        let result = display.open(&config);
        prop_assert!(result.is_ok(), "open should succeed with valid config");

        // State should transition to Connecting
        prop_assert_eq!(
            display.connection_state(),
            ConnectionState::Connecting,
            "State should be Connecting after open"
        );

        // Host and port should be set
        prop_assert_eq!(display.host(), Some(host), "Host should be set after open");
        prop_assert_eq!(display.port(), Some(port), "Port should be set after open");
    }

    /// **Feature: native-protocol-embedding, Property 1: Protocol widget creation returns valid GTK widget**
    /// **Validates: Requirements 1.2, 4.2**
    #[test]
    fn prop_spice_display_close_returns_to_disconnected(
        host in arb_host(),
        port in arb_port()
    ) {
        let display = SpiceDisplay::new();
        let config = SpiceConnectionConfig::new(&host, port);

        // Open and then close
        display.open(&config).unwrap();
        display.close();

        // State should return to Disconnected
        prop_assert_eq!(
            display.connection_state(),
            ConnectionState::Disconnected,
            "State should be Disconnected after close"
        );

        // Host and port should be cleared
        prop_assert!(display.host().is_none(), "Host should be None after close");
        prop_assert!(display.port().is_none(), "Port should be None after close");
        prop_assert!(display.config().is_none(), "Config should be None after close");
    }

    /// **Feature: native-protocol-embedding, Property 1: Protocol widget creation returns valid GTK widget**
    /// **Validates: Requirements 1.2, 4.2**
    #[test]
    fn prop_spice_display_rejects_empty_host(port in arb_port()) {
        let display = SpiceDisplay::new();
        let config = SpiceConnectionConfig {
            host: String::new(),
            port,
            ..Default::default()
        };

        let result = display.open(&config);
        prop_assert!(result.is_err(), "open should reject empty host");

        // State should remain Disconnected
        prop_assert_eq!(
            display.connection_state(),
            ConnectionState::Disconnected,
            "State should remain Disconnected after rejected open"
        );
    }

    /// **Feature: native-protocol-embedding, Property 1: Protocol widget creation returns valid GTK widget**
    /// **Validates: Requirements 1.2, 4.2**
    #[test]
    fn prop_spice_display_rejects_zero_port(host in arb_host()) {
        let display = SpiceDisplay::new();
        let config = SpiceConnectionConfig {
            host,
            port: 0,
            ..Default::default()
        };

        let result = display.open(&config);
        prop_assert!(result.is_err(), "open should reject zero port");

        // State should remain Disconnected
        prop_assert_eq!(
            display.connection_state(),
            ConnectionState::Disconnected,
            "State should remain Disconnected after rejected open"
        );
    }

    /// **Feature: native-protocol-embedding, Property 1: Protocol widget creation returns valid GTK widget**
    /// **Validates: Requirements 1.2, 4.2**
    #[test]
    fn prop_spice_display_rejects_duplicate_connection(
        host in arb_host(),
        port in arb_port()
    ) {
        let display = SpiceDisplay::new();
        let config = SpiceConnectionConfig::new(&host, port);

        // First connection should succeed
        let result1 = display.open(&config);
        prop_assert!(result1.is_ok(), "First open should succeed");

        // Second connection should fail
        let result2 = display.open(&config);
        prop_assert!(result2.is_err(), "Second open should fail while connecting");
    }

    /// **Feature: native-protocol-embedding, Property 1: Protocol widget creation returns valid GTK widget**
    /// **Validates: Requirements 1.2, 4.2**
    #[test]
    fn prop_spice_display_clipboard_toggle_is_consistent(enabled in any::<bool>()) {
        let display = SpiceDisplay::new();

        display.set_clipboard_enabled(enabled);
        prop_assert_eq!(
            display.clipboard_enabled(),
            enabled,
            "clipboard_enabled should match what was set"
        );
    }

    /// **Feature: native-protocol-embedding, Property 1: Protocol widget creation returns valid GTK widget**
    /// **Validates: Requirements 1.2, 4.2**
    #[test]
    fn prop_spice_display_ffi_display_trait_consistency(
        host in arb_host(),
        port in arb_port()
    ) {
        let display = SpiceDisplay::new();
        let config = SpiceConnectionConfig::new(&host, port);

        // Test FfiDisplay trait methods match SpiceDisplay methods
        prop_assert_eq!(
            FfiDisplay::state(&display),
            display.connection_state(),
            "FfiDisplay::state should match connection_state"
        );

        // Open connection
        display.open(&config).unwrap();

        // Close via trait
        FfiDisplay::close(&display);

        prop_assert_eq!(
            display.connection_state(),
            ConnectionState::Disconnected,
            "FfiDisplay::close should disconnect"
        );
    }

    /// **Feature: native-protocol-embedding, Property 1: Protocol widget creation returns valid GTK widget**
    /// **Validates: Requirements 1.2, 4.2**
    ///
    /// Test that shared folders can be added and removed consistently.
    #[test]
    fn prop_spice_display_shared_folders_consistency(
        folder_name in "[a-zA-Z][a-zA-Z0-9_]{0,15}",
        folder_path in "[a-zA-Z0-9/_.-]{1,32}"
    ) {
        let display = SpiceDisplay::new();

        // Initially no shared folders
        prop_assert!(display.shared_folders().is_empty(), "Should start with no shared folders");

        // Add a folder
        let path = std::path::Path::new(&folder_path);
        let result = display.add_shared_folder(path, &folder_name);
        prop_assert!(result.is_ok(), "Adding shared folder should succeed");

        // Folder should be in the list
        let folders = display.shared_folders();
        prop_assert_eq!(folders.len(), 1, "Should have one shared folder");
        prop_assert_eq!(&folders[0].name, &folder_name, "Folder name should match");

        // Remove the folder
        let removed = display.remove_shared_folder(&folder_name);
        prop_assert!(removed, "Removing existing folder should return true");

        // Folder should be gone
        prop_assert!(display.shared_folders().is_empty(), "Should have no shared folders after removal");

        // Removing again should return false
        let removed_again = display.remove_shared_folder(&folder_name);
        prop_assert!(!removed_again, "Removing non-existent folder should return false");
    }
}

// ============================================================================
// Unit Tests for SPICE Display Widget
// ============================================================================

/// **Feature: native-protocol-embedding, Property 1: Protocol widget creation returns valid GTK widget**
/// **Validates: Requirements 1.2, 4.2**
///
/// This test validates that SpiceDisplay can be created and used without panicking,
/// which is a prerequisite for GTK4 integration.
#[test]
fn test_spice_display_lifecycle() {
    // Create display
    let display = SpiceDisplay::new();
    assert_eq!(display.connection_state(), ConnectionState::Disconnected);

    // Open connection
    let config = SpiceConnectionConfig::new("localhost", 5900);
    display.open(&config).unwrap();
    assert_eq!(display.connection_state(), ConnectionState::Connecting);

    // Configure features
    display.set_clipboard_enabled(true);
    assert!(display.clipboard_enabled());

    // Close connection
    display.close();
    assert_eq!(display.connection_state(), ConnectionState::Disconnected);
}

/// Test that SpiceDisplay properly cleans up on drop
#[test]
fn test_spice_display_drop_cleanup() {
    {
        let display = SpiceDisplay::new();
        let config = SpiceConnectionConfig::new("localhost", 5900);
        display.open(&config).unwrap();
        // display goes out of scope here
    }
    // If we get here without panicking, cleanup worked
}

/// Test signal callback registration for SPICE
#[test]
fn test_spice_display_signal_callbacks() {
    use std::cell::Cell;
    use std::rc::Rc;

    let display = SpiceDisplay::new();

    let connected_called = Rc::new(Cell::new(false));
    let disconnected_called = Rc::new(Cell::new(false));
    let error_called = Rc::new(Cell::new(false));
    let channel_event_called = Rc::new(Cell::new(false));

    // Register callbacks
    let cc = connected_called.clone();
    display.connect_spice_connected(move |_| cc.set(true));

    let dc = disconnected_called.clone();
    display.connect_spice_disconnected(move |_| dc.set(true));

    let ec = error_called.clone();
    display.connect_spice_error(move |_, _| ec.set(true));

    let cec = channel_event_called.clone();
    display.connect_spice_channel_event(move |_, _| cec.set(true));

    // Callbacks should not be called yet
    assert!(!connected_called.get());
    assert!(!disconnected_called.get());
    assert!(!error_called.get());
    assert!(!channel_event_called.get());
}
