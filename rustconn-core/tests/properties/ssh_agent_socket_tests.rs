//! Property-based tests for Custom SSH Agent Socket feature
//!
//! Tests correctness properties defined in the design document for the
//! custom SSH agent socket resolution, validation, and serialization.

use proptest::prelude::*;
use rustconn_core::sftp::{SocketPathValidation, resolve_ssh_agent_socket, validate_socket_path};

/// Generates an arbitrary non-empty string suitable for socket paths.
/// Uses printable ASCII to avoid TOML encoding issues.
fn arb_nonempty_path() -> impl Strategy<Value = String> {
    prop::string::string_regex("/[a-zA-Z0-9/_.-]{1,50}")
        .expect("valid regex")
        .prop_filter("must not be empty", |s| !s.is_empty())
}

/// Generates an `Option<String>` that is either `None` or `Some(non-empty path)`.
fn arb_optional_path() -> impl Strategy<Value = Option<String>> {
    prop_oneof![Just(None), arb_nonempty_path().prop_map(Some),]
}

// ============================================================================
// Property 1: Priority chain ordering
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: custom-ssh-agent-socket, Property 1: Priority chain ordering**
    /// **Validates: Requirements 3.1, 3.2, 3.3, 3.4, 3.5, 3.6**
    ///
    /// For any combination of per-connection and global values,
    /// `resolve_ssh_agent_socket` returns the first non-empty value
    /// in priority order: per-connection → global → OnceLock → None.
    ///
    /// Note: OnceLock is global state and cannot be reliably controlled
    /// in property tests, so we test the per-connection vs global ordering.
    #[test]
    fn prop_priority_chain_ordering(
        per_conn in arb_optional_path(),
        global in arb_optional_path(),
    ) {
        let result = resolve_ssh_agent_socket(
            per_conn.as_deref(),
            global.as_deref(),
        );

        // Determine expected result based on priority chain
        // (ignoring OnceLock which is global state)
        let expected_from_args = per_conn
            .as_deref()
            .filter(|s| !s.is_empty())
            .or_else(|| global.as_deref().filter(|s| !s.is_empty()));

        match expected_from_args {
            Some(expected) => {
                // If we have a non-empty arg, result must match it
                prop_assert_eq!(
                    result.as_deref(),
                    Some(expected),
                    "Expected '{}' from priority chain, got {:?}",
                    expected,
                    result
                );
            }
            None => {
                // No args provided — result comes from OnceLock or is None.
                // We can't control OnceLock, but if result is Some it must be non-empty.
                if let Some(ref path) = result {
                    prop_assert!(
                        !path.is_empty(),
                        "OnceLock fallback must be non-empty, got empty string"
                    );
                }
            }
        }
    }
}

// ============================================================================
// Property 2: Empty strings treated as absent
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: custom-ssh-agent-socket, Property 2: Empty strings are treated as absent**
    /// **Validates: Requirements 1.3, 2.2, 3.1, 3.2**
    ///
    /// `Some("")` produces the same result as `None` for both
    /// per-connection and global parameters.
    #[test]
    fn prop_empty_strings_treated_as_absent(
        global in arb_optional_path(),
    ) {
        // Per-connection: Some("") vs None should give same result
        let with_empty = resolve_ssh_agent_socket(Some(""), global.as_deref());
        let with_none = resolve_ssh_agent_socket(None, global.as_deref());
        prop_assert_eq!(
            with_empty, with_none,
            "Some(\"\") per-connection should equal None per-connection"
        );
    }

    /// **Feature: custom-ssh-agent-socket, Property 2: Empty strings are treated as absent**
    /// **Validates: Requirements 1.3, 2.2, 3.1, 3.2**
    ///
    /// Same property for the global parameter.
    #[test]
    fn prop_empty_global_treated_as_absent(
        per_conn in arb_optional_path(),
    ) {
        let with_empty = resolve_ssh_agent_socket(per_conn.as_deref(), Some(""));
        let with_none = resolve_ssh_agent_socket(per_conn.as_deref(), None);
        prop_assert_eq!(
            with_empty, with_none,
            "Some(\"\") global should equal None global"
        );
    }
}

// ============================================================================
// Property 3: SshConfig serialization round-trip
// ============================================================================

use rustconn_core::SshConfig;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: custom-ssh-agent-socket, Property 3: Serialization round-trip for SshConfig**
    /// **Validates: Requirements 2.1, 2.2**
    ///
    /// Serialize SshConfig with arbitrary ssh_agent_socket to TOML,
    /// deserialize back, assert equality.
    #[test]
    fn prop_ssh_config_roundtrip(
        socket in arb_optional_path(),
    ) {
        let mut config = SshConfig::default();
        config.ssh_agent_socket = socket;

        let toml_str = toml::to_string(&config).expect("serialize SshConfig");
        let deserialized: SshConfig = toml::from_str(&toml_str).expect("deserialize SshConfig");

        prop_assert_eq!(
            config.ssh_agent_socket,
            deserialized.ssh_agent_socket,
            "ssh_agent_socket should survive TOML round-trip"
        );
    }
}

// ============================================================================
// Property 4: AppSettings serialization round-trip
// ============================================================================

use rustconn_core::AppSettings;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: custom-ssh-agent-socket, Property 4: Serialization round-trip for AppSettings**
    /// **Validates: Requirements 1.2, 1.3**
    ///
    /// Serialize AppSettings with arbitrary ssh_agent_socket to TOML,
    /// deserialize back, assert equality.
    #[test]
    fn prop_app_settings_roundtrip(
        socket in arb_optional_path(),
    ) {
        let mut settings = AppSettings::default();
        settings.ssh_agent_socket = socket;

        let toml_str = toml::to_string(&settings).expect("serialize AppSettings");
        let deserialized: AppSettings = toml::from_str(&toml_str).expect("deserialize AppSettings");

        prop_assert_eq!(
            settings.ssh_agent_socket,
            deserialized.ssh_agent_socket,
            "ssh_agent_socket should survive TOML round-trip"
        );
    }
}

// ============================================================================
// Property 5: Socket path validation correctness
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: custom-ssh-agent-socket, Property 5: Socket path validation correctness**
    /// **Validates: Requirements 4.1, 4.2**
    ///
    /// For any string, `validate_socket_path` returns `NotAbsolute`
    /// if and only if the string is non-empty and does not start with `/`.
    #[test]
    fn prop_validation_not_absolute_iff(
        path in "[a-zA-Z0-9 /._~-]{0,80}",
    ) {
        let result = validate_socket_path(&path);
        let is_non_empty = !path.is_empty();
        let starts_with_slash = path.starts_with('/');

        if is_non_empty && !starts_with_slash {
            prop_assert_eq!(
                result,
                SocketPathValidation::NotAbsolute,
                "Non-empty path '{}' without leading '/' should be NotAbsolute, got {:?}",
                path,
                result
            );
        } else {
            prop_assert_ne!(
                result,
                SocketPathValidation::NotAbsolute,
                "Path '{}' should NOT be NotAbsolute (empty={}, starts_with_slash={}), got {:?}",
                path,
                !is_non_empty,
                starts_with_slash,
                result
            );
        }
    }
}

// ============================================================================
// Property 6: Per-connection override isolation
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: custom-ssh-agent-socket, Property 6: Per-connection override isolation**
    /// **Validates: Requirements 3.1, 3.2, 3.5, 3.6**
    ///
    /// Two connections with different per-connection values resolve
    /// independently. Connection A's override does not affect connection B.
    #[test]
    fn prop_per_connection_isolation(
        path_a in arb_nonempty_path(),
        global in arb_optional_path(),
    ) {
        // Connection A has a per-connection override
        let result_a = resolve_ssh_agent_socket(
            Some(path_a.as_str()),
            global.as_deref(),
        );

        // Connection B has no per-connection override
        let result_b = resolve_ssh_agent_socket(
            None,
            global.as_deref(),
        );

        // A must resolve to its per-connection path
        prop_assert_eq!(
            result_a.as_deref(),
            Some(path_a.as_str()),
            "Connection A should resolve to its per-connection path"
        );

        // B must NOT resolve to A's path (unless global happens to match)
        let global_val = global.as_deref().filter(|s| !s.is_empty());
        match global_val {
            Some(g) => {
                prop_assert_eq!(
                    result_b.as_deref(),
                    Some(g),
                    "Connection B should resolve to global setting"
                );
            }
            None => {
                // B falls through to OnceLock or None — just verify it's not A's path
                // (unless OnceLock happens to have the same value, which is astronomically unlikely)
                if let Some(ref b_path) = result_b {
                    // If OnceLock returned something, it must be non-empty
                    prop_assert!(!b_path.is_empty(), "OnceLock fallback must be non-empty");
                }
            }
        }
    }
}
