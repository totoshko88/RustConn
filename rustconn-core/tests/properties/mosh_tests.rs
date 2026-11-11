//! Property-based tests for the MOSH protocol module.

use proptest::prelude::*;
use rustconn_core::models::{Connection, MoshConfig, MoshPredictMode, ProtocolConfig};
use rustconn_core::protocol::{MoshProtocol, Protocol};

// ---------------------------------------------------------------------------
// Generators
// ---------------------------------------------------------------------------

fn arb_predict_mode() -> impl Strategy<Value = MoshPredictMode> {
    prop_oneof![
        Just(MoshPredictMode::Adaptive),
        Just(MoshPredictMode::Always),
        Just(MoshPredictMode::Never),
    ]
}

fn arb_mosh_config() -> impl Strategy<Value = MoshConfig> {
    (
        proptest::option::of(1u16..=65535),
        proptest::option::of("[0-9]{4,5}:[0-9]{4,5}"),
        proptest::option::of("/[a-z/]{1,30}"),
        arb_predict_mode(),
        prop::collection::vec("[a-zA-Z0-9_-]{1,20}", 0..3),
    )
        .prop_map(
            |(ssh_port, port_range, server_binary, predict_mode, custom_args)| MoshConfig {
                ssh_port,
                port_range,
                server_binary,
                predict_mode,
                custom_args,
                // Erase modes (issue #271) are covered by the unit tests in
                // models::protocol; this strategy only needs them to compile.
                ..MoshConfig::default()
            },
        )
}

fn arb_hostname() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9.-]{0,30}".prop_filter("non-empty host", |s| !s.is_empty())
}

// ---------------------------------------------------------------------------
// Proptest 7: Serde round-trip for MoshConfig
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Serde round-trip: serialize then deserialize MoshConfig produces identical value.
    #[test]
    fn mosh_config_serde_roundtrip(config in arb_mosh_config()) {
        let json = serde_json::to_string(&config);
        prop_assert!(json.is_ok(), "Serialization failed: {:?}", json.err());
        let json = json.unwrap();

        let deserialized: Result<MoshConfig, _> = serde_json::from_str(&json);
        prop_assert!(
            deserialized.is_ok(),
            "Deserialization failed: {:?}",
            deserialized.err()
        );
        let deserialized = deserialized.unwrap();

        prop_assert_eq!(&config, &deserialized);
    }
}

// ---------------------------------------------------------------------------
// Proptest 8: build_command() generates valid command
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// build_command() with various option combinations generates a valid command
    /// where the first element is "mosh" and the host is present.
    #[test]
    fn mosh_build_command_valid(
        config in arb_mosh_config(),
        host in arb_hostname(),
        username in proptest::option::of("[a-z][a-z0-9]{0,10}"),
    ) {
        let protocol = MoshProtocol::new();
        let mut conn = Connection::new(
            "test".to_string(),
            host.clone(),
            22,
            ProtocolConfig::Mosh(config),
        );
        conn.username = username.clone();

        let cmd = protocol.build_command(&conn);
        prop_assert!(cmd.is_some(), "build_command should return Some for MOSH");
        let cmd = cmd.unwrap();

        // First element must be "mosh"
        prop_assert_eq!(&cmd[0], "mosh");

        // Last element must contain the host
        let last = cmd.last().unwrap();
        prop_assert!(
            last.contains(&host),
            "Last arg '{}' should contain host '{}'",
            last,
            host
        );

        // If username is set, last element should be user@host
        if let Some(ref user) = username {
            let expected = format!("{user}@{host}");
            prop_assert_eq!(last, &expected);
        }
    }
}

// ---------------------------------------------------------------------------
// Proptest 9: Empty host → validation error
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]

    /// Empty host should produce a validation error.
    #[test]
    fn mosh_empty_host_validation_error(config in arb_mosh_config()) {
        let protocol = MoshProtocol::new();
        let conn = Connection::new(
            "test".to_string(),
            String::new(),
            22,
            ProtocolConfig::Mosh(config),
        );

        let result = protocol.validate_connection(&conn);
        prop_assert!(result.is_err(), "Empty host should fail validation");
    }
}
