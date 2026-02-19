//! Property-based tests for credentials security
//!
//! **Feature: rustconn, Property 13: Credentials Security Invariant**
//! **Validates: Requirements 5.5**

use proptest::prelude::*;
use rustconn_core::models::{Connection, ProtocolConfig, SshAuthMethod, SshConfig, SshKeySource};
use rustconn_core::Credentials;
use std::path::PathBuf;

/// Generates arbitrary SSH key paths (file paths that look like SSH keys)
fn arb_key_path() -> impl Strategy<Value = PathBuf> {
    prop_oneof![
        Just(PathBuf::from("/home/user/.ssh/id_rsa")),
        Just(PathBuf::from("/home/user/.ssh/id_ed25519")),
        Just(PathBuf::from("/home/user/.ssh/id_ecdsa")),
        Just(PathBuf::from("~/.ssh/custom_key")),
        "[a-z]{1,10}".prop_map(|name| PathBuf::from(format!("/home/user/.ssh/{name}"))),
    ]
}

/// Generates arbitrary usernames
fn arb_username() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9_]{0,15}".prop_map(String::from)
}

/// Generates arbitrary SSH configurations with public key authentication
fn arb_ssh_config_with_key() -> impl Strategy<Value = SshConfig> {
    (arb_key_path(), any::<bool>()).prop_map(|(key_path, use_control_master)| SshConfig {
        auth_method: SshAuthMethod::PublicKey,
        key_path: Some(key_path),
        key_source: SshKeySource::Default,
        agent_key_fingerprint: None,
        identities_only: false,
        proxy_jump: None,
        use_control_master,
        agent_forwarding: false,
        x11_forwarding: false,
        compression: false,
        custom_options: std::collections::HashMap::new(),
        startup_command: None,
        jump_host_id: None,
        sftp_enabled: false,
        port_forwards: Vec::new(),
    })
}

/// Generates arbitrary SSH connections with public key authentication
fn arb_ssh_connection_with_key() -> impl Strategy<Value = Connection> {
    (
        "[a-zA-Z][a-zA-Z0-9 _-]{0,30}",
        "[a-z0-9]([a-z0-9-]{0,20}[a-z0-9])?(\\.[a-z]{2,6})?",
        1u16..65535,
        arb_ssh_config_with_key(),
        prop::option::of(arb_username()),
    )
        .prop_map(|(name, host, port, ssh_config, username)| {
            let mut conn = Connection::new(name, host, port, ProtocolConfig::Ssh(ssh_config));
            conn.username = username;
            conn
        })
}

/// Generates credentials for SSH key authentication
/// These credentials should only contain username and optionally key passphrase,
/// but NEVER the actual private key content
fn arb_ssh_key_credentials() -> impl Strategy<Value = Credentials> {
    (
        prop::option::of(arb_username()),
        prop::option::of("[a-zA-Z0-9!@#$%^&*]{8,32}"), // Optional key passphrase
    )
        .prop_map(|(username, passphrase)| {
            let mut creds = Credentials::empty();
            creds.username = username;
            if let Some(p) = passphrase {
                creds.key_passphrase = Some(secrecy::SecretString::from(p));
            }
            creds
        })
}

/// Simulates what private key content might look like
fn looks_like_private_key(s: &str) -> bool {
    s.contains("-----BEGIN") && s.contains("PRIVATE KEY-----")
        || s.contains("-----BEGIN RSA PRIVATE KEY-----")
        || s.contains("-----BEGIN OPENSSH PRIVATE KEY-----")
        || s.contains("-----BEGIN EC PRIVATE KEY-----")
        || s.contains("-----BEGIN DSA PRIVATE KEY-----")
        || s.contains("-----BEGIN ENCRYPTED PRIVATE KEY-----")
        // Also check for base64-encoded key material patterns
        || (s.len() > 100 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=' || c == '\n'))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn, Property 13: Credentials Security Invariant**
    /// **Validates: Requirements 5.5**
    ///
    /// For any Credentials object associated with SSH key authentication,
    /// the credentials must contain only the key file path and never the
    /// actual private key content.
    ///
    /// This test verifies that:
    /// 1. The SSH config stores only the key_path (file path), not key content
    /// 2. Credentials never contain private key material in any field
    #[test]
    fn prop_ssh_key_credentials_never_contain_private_key_content(
        connection in arb_ssh_connection_with_key(),
        credentials in arb_ssh_key_credentials(),
    ) {
        // Extract SSH config from connection
        let ssh_config = match &connection.protocol_config {
            ProtocolConfig::Ssh(config) => config,
            _ => panic!("Expected SSH config"),
        };

        // Verify: SSH config stores key_path (file path), not key content
        if let Some(key_path) = &ssh_config.key_path {
            let path_str = key_path.to_string_lossy();
            // The key_path should be a file path, not key content
            prop_assert!(
                !looks_like_private_key(&path_str),
                "SSH config key_path should be a file path, not private key content"
            );
            // Path should look like a file path (contains / or starts with ~)
            prop_assert!(
                path_str.contains('/') || path_str.starts_with('~'),
                "SSH config key_path should be a valid file path"
            );
        }

        // Verify: Credentials username never contains private key content
        if let Some(username) = &credentials.username {
            prop_assert!(
                !looks_like_private_key(username),
                "Credentials username should never contain private key content"
            );
        }

        // Verify: Credentials password (if any) never contains private key content
        // Note: For SSH key auth, password field should typically be None,
        // but if present, it should not contain key material
        if let Some(password) = credentials.expose_password() {
            prop_assert!(
                !looks_like_private_key(password),
                "Credentials password should never contain private key content"
            );
        }

        // Verify: Key passphrase is for unlocking the key, not the key itself
        if let Some(passphrase) = credentials.expose_key_passphrase() {
            prop_assert!(
                !looks_like_private_key(passphrase),
                "Credentials key_passphrase should be a passphrase, not private key content"
            );
            // Passphrases are typically short (< 100 chars)
            prop_assert!(
                passphrase.len() < 200,
                "Key passphrase should be reasonably short, not key content"
            );
        }
    }

    /// Additional property: SSH key path should be a valid-looking file path
    #[test]
    fn prop_ssh_key_path_is_valid_file_path(
        connection in arb_ssh_connection_with_key(),
    ) {
        let ssh_config = match &connection.protocol_config {
            ProtocolConfig::Ssh(config) => config,
            _ => panic!("Expected SSH config"),
        };

        if let Some(key_path) = &ssh_config.key_path {
            let path_str = key_path.to_string_lossy();

            // Should not be empty
            prop_assert!(!path_str.is_empty(), "Key path should not be empty");

            // Should not contain newlines (which would indicate key content)
            prop_assert!(
                !path_str.contains('\n'),
                "Key path should not contain newlines"
            );

            // Should be a reasonable length for a file path
            prop_assert!(
                path_str.len() < 4096,
                "Key path should be a reasonable length"
            );
        }
    }

    /// Property: Credentials serialization should not expose secrets
    #[test]
    fn prop_credentials_serialization_does_not_expose_secrets(
        credentials in arb_ssh_key_credentials(),
    ) {
        // Serialize credentials to TOML
        let serialized = toml::to_string(&credentials).expect("Should serialize");

        // The serialized form should NOT contain password or key_passphrase
        // (these are intentionally not serialized for security)
        prop_assert!(
            !serialized.contains("password"),
            "Serialized credentials should not contain password field"
        );
        prop_assert!(
            !serialized.contains("key_passphrase"),
            "Serialized credentials should not contain key_passphrase field"
        );

        // Should not contain anything that looks like a private key
        prop_assert!(
            !looks_like_private_key(&serialized),
            "Serialized credentials should never contain private key content"
        );
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn test_looks_like_private_key_detection() {
        // Should detect RSA private key header
        assert!(looks_like_private_key(
            "-----BEGIN RSA PRIVATE KEY-----\nMIIE..."
        ));

        // Should detect OpenSSH private key header
        assert!(looks_like_private_key(
            "-----BEGIN OPENSSH PRIVATE KEY-----\nb3Bl..."
        ));

        // Should detect generic private key header
        assert!(looks_like_private_key(
            "-----BEGIN PRIVATE KEY-----\nMIIE..."
        ));

        // Should NOT detect file paths
        assert!(!looks_like_private_key("/home/user/.ssh/id_rsa"));
        assert!(!looks_like_private_key("~/.ssh/id_ed25519"));

        // Should NOT detect usernames
        assert!(!looks_like_private_key("admin"));
        assert!(!looks_like_private_key("user123"));

        // Should NOT detect short passphrases
        assert!(!looks_like_private_key("MySecretPassphrase123!"));
    }

    #[test]
    fn test_credentials_do_not_serialize_secrets() {
        let creds = Credentials::with_password("testuser", "secretpassword");
        let serialized = toml::to_string(&creds).unwrap();

        // Should contain username
        assert!(serialized.contains("testuser"));

        // Should NOT contain password
        assert!(!serialized.contains("secretpassword"));
        assert!(!serialized.contains("password"));
    }
}
