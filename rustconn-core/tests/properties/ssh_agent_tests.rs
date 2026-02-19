//! Property-based tests for SSH Agent functionality
//!
//! Tests correctness properties for SSH agent output parsing and key management.

use proptest::prelude::*;
use rustconn_core::ssh_agent::{parse_agent_output, AgentError};

/// Generates a valid socket path component (alphanumeric with some special chars)
fn arb_socket_path_component() -> impl Strategy<Value = String> {
    prop::string::string_regex("[a-zA-Z0-9_-]{1,20}")
        .unwrap()
        .prop_filter("component must not be empty", |s| !s.is_empty())
}

/// Generates a valid PID number
fn arb_pid() -> impl Strategy<Value = u32> {
    1u32..99999
}

/// Generates a valid SSH agent socket path
fn arb_socket_path() -> impl Strategy<Value = String> {
    (arb_socket_path_component(), arb_pid())
        .prop_map(|(component, pid)| format!("/tmp/ssh-{}/agent.{}", component, pid))
}

/// Generates bash-format ssh-agent output
fn arb_bash_agent_output(socket_path: String, pid: u32) -> String {
    format!(
        "SSH_AUTH_SOCK={}; export SSH_AUTH_SOCK;\nSSH_AGENT_PID={}; export SSH_AGENT_PID;\necho Agent pid {};",
        socket_path, pid, pid
    )
}

/// Generates csh-format ssh-agent output
fn arb_csh_agent_output(socket_path: String, pid: u32) -> String {
    format!(
        "setenv SSH_AUTH_SOCK {};\nsetenv SSH_AGENT_PID {};\necho Agent pid {};",
        socket_path, pid, pid
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: ssh-agent-cli, Property 1: SSH Agent Output Parsing**
    /// **Validates: Requirements 1.2**
    ///
    /// For any valid ssh-agent -s output string (bash format), parsing should
    /// extract the SSH_AUTH_SOCK path correctly.
    #[test]
    fn prop_parse_bash_agent_output_extracts_socket(
        socket_path in arb_socket_path(),
        pid in arb_pid(),
    ) {
        let output = arb_bash_agent_output(socket_path.clone(), pid);

        let result = parse_agent_output(&output);

        prop_assert!(
            result.is_ok(),
            "Failed to parse bash agent output: {:?}. Output:\n{}",
            result.err(),
            output
        );

        prop_assert_eq!(
            result.unwrap(),
            socket_path,
            "Socket path mismatch"
        );
    }

    /// **Feature: ssh-agent-cli, Property 1: SSH Agent Output Parsing**
    /// **Validates: Requirements 1.2**
    ///
    /// For any valid ssh-agent -c output string (csh format), parsing should
    /// extract the SSH_AUTH_SOCK path correctly.
    #[test]
    fn prop_parse_csh_agent_output_extracts_socket(
        socket_path in arb_socket_path(),
        pid in arb_pid(),
    ) {
        let output = arb_csh_agent_output(socket_path.clone(), pid);

        let result = parse_agent_output(&output);

        prop_assert!(
            result.is_ok(),
            "Failed to parse csh agent output: {:?}. Output:\n{}",
            result.err(),
            output
        );

        prop_assert_eq!(
            result.unwrap(),
            socket_path,
            "Socket path mismatch"
        );
    }

    /// **Feature: ssh-agent-cli, Property 1: SSH Agent Output Parsing**
    /// **Validates: Requirements 1.2**
    ///
    /// For any output that doesn't contain SSH_AUTH_SOCK, parsing should fail
    /// with an appropriate error.
    #[test]
    fn prop_parse_invalid_output_returns_error(
        random_text in prop::string::string_regex("[a-zA-Z0-9 _=;]+").unwrap(),
    ) {
        // Ensure the random text doesn't accidentally contain SSH_AUTH_SOCK
        prop_assume!(!random_text.contains("SSH_AUTH_SOCK"));

        let result = parse_agent_output(&random_text);

        prop_assert!(
            result.is_err(),
            "Expected error for invalid output, got: {:?}",
            result
        );

        // Verify it's a ParseError
        if let Err(AgentError::ParseError(_)) = result {
            // Expected
        } else {
            prop_assert!(false, "Expected ParseError, got: {:?}", result);
        }
    }
}

// ============================================================================
// SSH Key List Parsing Property Tests
// ============================================================================

use rustconn_core::ssh_agent::parse_key_list;

/// Generates a valid SSH key type
fn arb_key_type() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("RSA".to_string()),
        Just("ED25519".to_string()),
        Just("ECDSA".to_string()),
        Just("DSA".to_string()),
    ]
}

/// Generates a valid key bit size based on key type
fn arb_key_bits(key_type: &str) -> u32 {
    match key_type {
        "RSA" => 4096,
        "ED25519" => 256,
        "ECDSA" => 256,
        "DSA" => 1024,
        _ => 2048,
    }
}

/// Generates a valid SHA256 fingerprint
fn arb_fingerprint() -> impl Strategy<Value = String> {
    prop::string::string_regex("[a-zA-Z0-9+/]{43}")
        .unwrap()
        .prop_map(|s| format!("SHA256:{}", s))
}

/// Generates a valid key comment (file path or email-like)
fn arb_key_comment() -> impl Strategy<Value = String> {
    prop_oneof![
        // File path style
        prop::string::string_regex("[a-z_][a-z0-9_]{0,15}")
            .unwrap()
            .prop_filter("name must not be empty", |s| !s.is_empty())
            .prop_map(|name| format!("/home/user/.ssh/id_{}", name)),
        // Email style
        (
            prop::string::string_regex("[a-z][a-z0-9]{0,10}").unwrap(),
            prop::string::string_regex("[a-z][a-z0-9]{0,10}").unwrap(),
        )
            .prop_filter("parts must not be empty", |(u, h)| !u.is_empty()
                && !h.is_empty())
            .prop_map(|(user, host)| format!("{}@{}", user, host)),
    ]
}

/// Represents a generated SSH key entry for testing
#[derive(Debug, Clone)]
struct KeyEntry {
    bits: u32,
    fingerprint: String,
    comment: String,
    key_type: String,
}

impl KeyEntry {
    /// Converts to ssh-add -l output format
    fn to_ssh_add_line(&self) -> String {
        format!(
            "{} {} {} ({})",
            self.bits, self.fingerprint, self.comment, self.key_type
        )
    }
}

/// Strategy for generating SSH key entries
fn arb_key_entry() -> impl Strategy<Value = KeyEntry> {
    (arb_key_type(), arb_fingerprint(), arb_key_comment()).prop_map(
        |(key_type, fingerprint, comment)| {
            let bits = arb_key_bits(&key_type);
            KeyEntry {
                bits,
                fingerprint,
                comment,
                key_type,
            }
        },
    )
}

/// Strategy for generating multiple key entries
fn arb_key_entries() -> impl Strategy<Value = Vec<KeyEntry>> {
    prop::collection::vec(arb_key_entry(), 1..10)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: ssh-agent-cli, Property 2: SSH Key List Parsing**
    /// **Validates: Requirements 1.6**
    ///
    /// For any valid ssh-add -l output string, parsing should extract all key
    /// fingerprints, types, and comments correctly.
    #[test]
    fn prop_parse_key_list_extracts_all_keys(entries in arb_key_entries()) {
        // Generate ssh-add -l output
        let output = entries
            .iter()
            .map(|e| e.to_ssh_add_line())
            .collect::<Vec<_>>()
            .join("\n");

        let result = parse_key_list(&output);

        prop_assert!(
            result.is_ok(),
            "Failed to parse key list: {:?}. Output:\n{}",
            result.err(),
            output
        );

        let keys = result.unwrap();

        // Property: All keys should be extracted
        prop_assert_eq!(
            keys.len(),
            entries.len(),
            "Expected {} keys, got {}. Output:\n{}",
            entries.len(),
            keys.len(),
            output
        );

        // Property: Each key should have correct parameters
        for (i, entry) in entries.iter().enumerate() {
            let key = &keys[i];

            prop_assert_eq!(
                key.bits,
                entry.bits,
                "Bits mismatch for key {}: expected {}, got {}",
                i,
                entry.bits,
                key.bits
            );

            prop_assert_eq!(
                &key.fingerprint,
                &entry.fingerprint,
                "Fingerprint mismatch for key {}",
                i
            );

            prop_assert_eq!(
                &key.comment,
                &entry.comment,
                "Comment mismatch for key {}",
                i
            );

            prop_assert_eq!(
                &key.key_type,
                &entry.key_type,
                "Key type mismatch for key {}",
                i
            );
        }
    }

    /// **Feature: ssh-agent-cli, Property 2: SSH Key List Parsing**
    /// **Validates: Requirements 1.6**
    ///
    /// For empty agent output, parsing should return an empty list.
    #[test]
    fn prop_parse_empty_key_list_returns_empty(
        whitespace in prop::string::string_regex("[ \t\n]*").unwrap(),
    ) {
        let result = parse_key_list(&whitespace);

        prop_assert!(
            result.is_ok(),
            "Failed to parse empty key list: {:?}",
            result.err()
        );

        prop_assert!(
            result.unwrap().is_empty(),
            "Expected empty key list for whitespace input"
        );
    }

    /// **Feature: ssh-agent-cli, Property 2: SSH Key List Parsing**
    /// **Validates: Requirements 1.6**
    ///
    /// For "no identities" message, parsing should return an empty list.
    #[test]
    fn prop_parse_no_identities_returns_empty(
        prefix in prop::string::string_regex("[ \t]*").unwrap(),
        suffix in prop::string::string_regex("[ \t\n]*").unwrap(),
    ) {
        let output = format!("{}The agent has no identities.{}", prefix, suffix);

        let result = parse_key_list(&output);

        prop_assert!(
            result.is_ok(),
            "Failed to parse 'no identities' message: {:?}",
            result.err()
        );

        prop_assert!(
            result.unwrap().is_empty(),
            "Expected empty key list for 'no identities' message"
        );
    }
}

// ============================================================================
// Agent Key Fingerprint Storage Property Tests
// ============================================================================

use rustconn_core::{Connection, ProtocolConfig, SshAuthMethod, SshConfig, SshKeySource};

/// Generates a valid hostname
fn arb_hostname() -> impl Strategy<Value = String> {
    prop::string::string_regex("[a-z][a-z0-9-]{0,20}(\\.[a-z][a-z0-9-]{0,10})*")
        .unwrap()
        .prop_filter("hostname must not be empty", |s| !s.is_empty())
}

/// Generates a valid connection name
fn arb_connection_name() -> impl Strategy<Value = String> {
    prop::string::string_regex("[a-zA-Z][a-zA-Z0-9_-]{0,30}")
        .unwrap()
        .prop_filter("name must not be empty", |s| !s.is_empty())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: ssh-agent-cli, Property 3: Agent Key Fingerprint Storage**
    /// **Validates: Requirements 2.3**
    ///
    /// For any SSH connection configured with an agent key, the fingerprint
    /// should be stored and retrievable for identification.
    #[test]
    fn prop_agent_key_fingerprint_stored_and_retrievable(
        name in arb_connection_name(),
        host in arb_hostname(),
        port in 1u16..65535u16,
        fingerprint in arb_fingerprint(),
        comment in arb_key_comment(),
    ) {
        // Create SSH config with agent key
        let ssh_config = SshConfig {
            auth_method: SshAuthMethod::Agent,
            key_path: None,
            key_source: SshKeySource::Agent {
                fingerprint: fingerprint.clone(),
                comment: comment.clone(),
            },
            agent_key_fingerprint: Some(fingerprint.clone()),
            identities_only: false,
            proxy_jump: None,
            use_control_master: false,
            agent_forwarding: false,
            x11_forwarding: false,
            compression: false,
            custom_options: std::collections::HashMap::new(),
            startup_command: None, jump_host_id: None, sftp_enabled: false, port_forwards: Vec::new(),
        };

        // Create connection
        let connection = Connection::new(
            name,
            host,
            port,
            ProtocolConfig::Ssh(ssh_config),
        );

        // Property: Fingerprint should be retrievable from the connection
        if let ProtocolConfig::Ssh(config) = &connection.protocol_config {
            // Check agent_key_fingerprint field
            prop_assert_eq!(
                config.agent_key_fingerprint.as_ref(),
                Some(&fingerprint),
                "agent_key_fingerprint should match"
            );

            // Check key_source contains the fingerprint
            if let SshKeySource::Agent { fingerprint: stored_fp, comment: stored_comment } = &config.key_source {
                prop_assert_eq!(
                    stored_fp,
                    &fingerprint,
                    "key_source fingerprint should match"
                );
                prop_assert_eq!(
                    stored_comment,
                    &comment,
                    "key_source comment should match"
                );
            } else {
                prop_assert!(false, "Expected SshKeySource::Agent");
            }
        } else {
            prop_assert!(false, "Expected SSH protocol config");
        }
    }

    /// **Feature: ssh-agent-cli, Property 3: Agent Key Fingerprint Storage**
    /// **Validates: Requirements 2.3**
    ///
    /// For any SSH connection with agent key, serialization and deserialization
    /// should preserve the fingerprint.
    #[test]
    fn prop_agent_key_fingerprint_survives_serialization(
        name in arb_connection_name(),
        host in arb_hostname(),
        port in 1u16..65535u16,
        fingerprint in arb_fingerprint(),
        comment in arb_key_comment(),
    ) {
        let ssh_config = SshConfig {
            auth_method: SshAuthMethod::Agent,
            key_path: None,
            key_source: SshKeySource::Agent {
                fingerprint: fingerprint.clone(),
                comment: comment.clone(),
            },
            agent_key_fingerprint: Some(fingerprint.clone()),
            identities_only: false,
            proxy_jump: None,
            use_control_master: false,
            agent_forwarding: false,
            x11_forwarding: false,
            compression: false,
            custom_options: std::collections::HashMap::new(),
            startup_command: None, jump_host_id: None, sftp_enabled: false, port_forwards: Vec::new(),
        };

        let connection = Connection::new(
            name,
            host,
            port,
            ProtocolConfig::Ssh(ssh_config),
        );

        // Serialize to JSON
        let json = serde_json::to_string(&connection).expect("Failed to serialize");

        // Deserialize back
        let deserialized: Connection = serde_json::from_str(&json).expect("Failed to deserialize");

        // Property: Fingerprint should be preserved
        if let ProtocolConfig::Ssh(config) = &deserialized.protocol_config {
            prop_assert_eq!(
                config.agent_key_fingerprint.as_ref(),
                Some(&fingerprint),
                "agent_key_fingerprint should survive serialization"
            );

            if let SshKeySource::Agent { fingerprint: stored_fp, .. } = &config.key_source {
                prop_assert_eq!(
                    stored_fp,
                    &fingerprint,
                    "key_source fingerprint should survive serialization"
                );
            } else {
                prop_assert!(false, "Expected SshKeySource::Agent after deserialization");
            }
        } else {
            prop_assert!(false, "Expected SSH protocol config after deserialization");
        }
    }
}
